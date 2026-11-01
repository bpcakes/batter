//! Shared fixtures for bounded blocking work. No fixture sleeps: closures park
//! on explicit gates that also release on unwind, so a failed assertion cannot
//! leave a closure blocking runtime drop.

use batter_core::{
    admission::{Bulkhead, BulkheadCapacity},
    operation::{OperationContext, OperationOwner},
};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::oneshot;

pub(crate) fn bulkhead(capacity: usize) -> Bulkhead {
    Bulkhead::new(BulkheadCapacity::new(capacity).unwrap())
}

pub(crate) fn owner(seconds: u64) -> OperationOwner {
    OperationOwner::new(Duration::from_secs(seconds)).unwrap()
}

/// A live context whose budget outlasts every probe in these tests.
pub(crate) fn fresh() -> OperationContext {
    owner(10).into_context()
}

/// Observes and releases one gated closure.
///
/// Dropping the gate, including while a failed assertion unwinds, drops
/// `release` and unblocks the closure.
pub(crate) struct Gate {
    started: oneshot::Receiver<()>,
    release: mpsc::Sender<()>,
    returned: Arc<AtomicBool>,
}

impl Gate {
    /// Wait until the closure is running on a blocking thread.
    pub(crate) async fn started(&mut self) {
        (&mut self.started)
            .await
            .expect("a gated closure signals its start");
    }

    /// Let the parked closure continue.
    pub(crate) fn release(&self) {
        self.release
            .send(())
            .expect("a gated closure awaits its release");
    }

    /// Whether the closure has returned its value.
    pub(crate) fn returned(&self) -> bool {
        self.returned.load(Ordering::SeqCst)
    }
}

/// A closure that signals its start, blocks until released and then runs
/// `finish`; the gate observes a return only after `finish` returns.
pub(crate) fn gated<T>(
    finish: impl FnOnce() -> T + Send + 'static,
) -> (Gate, impl FnOnce() -> T + Send + 'static) {
    let (started_tx, started) = oneshot::channel();
    let (release, release_rx) = mpsc::channel();
    let returned = Arc::new(AtomicBool::new(false));
    let finished = returned.clone();
    let work = move || {
        let _ = started_tx.send(());
        // A dropped gate also ends this wait.
        let _ = release_rx.recv();
        let value = finish();
        finished.store(true, Ordering::SeqCst);
        value
    };
    let gate = Gate {
        started,
        release,
        returned,
    };
    (gate, work)
}

/// A gated closure that returns `value` once released.
pub(crate) fn parked<T: Send + 'static>(value: T) -> (Gate, impl FnOnce() -> T + Send + 'static) {
    gated(move || value)
}

/// Proves that closures built from it were destroyed without being called.
#[derive(Default)]
pub(crate) struct Unrun {
    called: Arc<AtomicBool>,
    marker: Arc<()>,
}

impl Unrun {
    pub(crate) fn closure(&self) -> impl FnOnce() -> u8 + Send + 'static {
        let called = self.called.clone();
        let marker = self.marker.clone();
        move || {
            let _marker = marker;
            called.store(true, Ordering::SeqCst);
            0
        }
    }

    pub(crate) fn assert_destroyed_uncalled(&self) {
        assert!(!self.called.load(Ordering::SeqCst), "work was called");
        assert_eq!(Arc::strong_count(&self.marker), 1, "work was not destroyed");
    }
}

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// An INFO fmt subscriber that also logs span closes, captured in memory.
pub(crate) struct Logs {
    pub(crate) dispatch: tracing::Dispatch,
    output: Arc<Mutex<Vec<u8>>>,
}

impl Logs {
    pub(crate) fn new() -> Self {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Buffer(output.clone());
        let dispatch = crate::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_max_level(tracing::Level::INFO)
                .finish(),
        );
        Self { dispatch, output }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.output.lock().unwrap().clone()).unwrap()
    }

    /// Close lines of the span `name` itself, not of its children.
    pub(crate) fn closes_of(&self, name: &str) -> usize {
        let own = format!("{name}: ");
        self.text()
            .lines()
            .filter(|line| line.contains(&own) && line.trim_end().ends_with(" close"))
            .count()
    }
}
