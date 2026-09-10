use crate::events::EventLog;
use axum::body::{Bytes, HttpBody};
use batter::operation::{Interruption, OperationContext};
use http_body::Frame;
use std::{
    convert::Infallible,
    future::Future,
    io::Write,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct State {
    events: EventLog,
    changed: Arc<Notify>,
    pub release: Arc<Notify>,
    pub graceful: Arc<Notify>,
    pub context: Arc<Mutex<Option<OperationContext>>>,
}

impl State {
    pub fn record(&self, event: &'static str) {
        self.events.record(event);
        // Fixed fixture names only. Preserve progress even if the parent must kill
        // a stuck runtime; never panic from diagnostic I/O in resource Drop.
        let _ = writeln!(std::io::stdout().lock(), "batter-http-event:{event}");
        self.changed.notify_waiters();
    }

    pub fn count(&self, event: &str) -> usize {
        self.events.count(event)
    }

    pub async fn wait(&self, event: &str) {
        let _ = writeln!(std::io::stdout().lock(), "batter-http-wait:{event}");
        loop {
            let changed = self.changed.notified();
            if self.count(event) != 0 {
                return;
            }
            changed.await;
        }
    }

    pub fn before(&self, first: &str, second: &str) {
        self.events.before(first, second);
    }

    pub fn snapshot(&self) -> Vec<&'static str> {
        self.events.snapshot()
    }

    pub fn context_cancelled(&self) -> bool {
        self.context.lock().unwrap().as_ref().unwrap().check() == Err(Interruption::Cancelled)
    }
}

#[test]
fn failed_ordering_assertion_does_not_poison_drop_evidence() {
    let state = State::default();
    state.record("second");
    state.record("first");
    assert!(std::panic::catch_unwind(|| state.before("first", "second")).is_err());
    drop(Lifetime(state.clone(), "resource-dropped"));
    assert_eq!(state.count("resource-dropped"), 1);
}

pub struct Lifetime(pub State, pub &'static str);
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.record(self.1);
    }
}

pub struct ControlledBody {
    state: State,
    release: Pin<Box<dyn Future<Output = ()> + Send>>,
    phase: u8,
}

impl ControlledBody {
    pub fn new(state: State) -> Self {
        let release = state.release.clone();
        Self {
            state,
            release: Box::pin(async move { release.notified().await }),
            phase: 0,
        }
    }
}

impl HttpBody for ControlledBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.phase {
            0 => {
                self.phase = 1;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"start")))))
            }
            1 => {
                if self.release.as_mut().poll(cx).is_pending() {
                    if self.state.count("body-pending") == 0 {
                        self.state.record("body-pending");
                    }
                    return Poll::Pending;
                }
                self.phase = 2;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"end")))))
            }
            _ => {
                self.state.record("body-complete");
                Poll::Ready(None)
            }
        }
    }
}

impl Drop for ControlledBody {
    fn drop(&mut self) {
        self.state.record("body-dropped");
    }
}
