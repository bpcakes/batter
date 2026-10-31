use super::{BlockingCall, BlockingError, join_failure, recheck};
use crate::{
    admission::AdmissionError,
    operation::{Interruption, OperationContext, OperationOwner},
};
use std::{
    io::{self, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::{sync::Semaphore, time::Instant};

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

fn single_blocking_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap()
}

#[test]
fn cancelled_blocking_join_maps_to_not_started() {
    let runtime = single_blocking_thread();
    runtime.block_on(async {
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let occupant = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = release_rx.recv();
        });
        started_rx.recv().unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        let marker = Arc::new(());
        let queued = tokio::task::spawn_blocking({
            let ran = ran.clone();
            let marker = marker.clone();
            move || {
                ran.store(true, Ordering::SeqCst);
                drop(marker);
            }
        });
        // A queued task is only marked cancelled; the busy worker later
        // destroys it unstarted instead of running it.
        queued.abort();
        drop(release);
        occupant.await.unwrap();
        let error = queued.await.unwrap_err();
        assert!(error.is_cancelled());
        assert!(matches!(join_failure(error), BlockingError::NotStarted));
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(Arc::strong_count(&marker), 1);
    });
}

#[tokio::test]
async fn panicked_blocking_join_maps_to_its_retained_payload() {
    let error = tokio::task::spawn_blocking(|| std::panic::panic_any(7_u8))
        .await
        .unwrap_err();
    let BlockingError::Panicked(payload) = join_failure(error) else {
        panic!("a panicking join must map to Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<u8>().copied()),
        Ok(Some(7))
    );
}

/// Records how many permits were free when the closure's capture was destroyed.
struct Probe {
    semaphore: Arc<Semaphore>,
    seen: Arc<AtomicUsize>,
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.seen
            .store(self.semaphore.available_permits(), Ordering::SeqCst);
    }
}

fn probed_call(
    finish: fn() -> u8,
) -> (
    BlockingCall<impl FnOnce() -> u8>,
    Arc<Semaphore>,
    Arc<AtomicUsize>,
) {
    let semaphore = Arc::new(Semaphore::new(1));
    let seen = Arc::new(AtomicUsize::new(usize::MAX));
    let probe = Probe {
        semaphore: semaphore.clone(),
        seen: seen.clone(),
    };
    let permit = semaphore.clone().try_acquire_owned().unwrap();
    let call = BlockingCall::new(
        move || {
            let _probe = probe;
            finish()
        },
        permit,
    );
    (call, semaphore, seen)
}

#[test]
fn permit_outlives_work_and_its_captures() {
    let (returned, semaphore, seen) = probed_call(|| 7);
    assert_eq!(returned.run(), 7);
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(semaphore.available_permits(), 1);

    let (unwound, semaphore, seen) = probed_call(|| panic!("blocking defect"));
    assert!(catch_unwind(AssertUnwindSafe(|| unwound.run())).is_err());
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(semaphore.available_permits(), 1);

    let (unrun, semaphore, seen) = probed_call(|| unreachable!("never called"));
    drop(unrun);
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(semaphore.available_permits(), 1);
}

/// Emits an event when Tokio would destroy an unstarted closure's capture.
struct Announce;

impl Drop for Announce {
    fn drop(&mut self) {
        tracing::info!("unrun capture dropped");
    }
}

#[test]
fn work_and_unrun_work_use_the_captured_dispatcher_and_span() {
    let output = Buffer(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let dispatch = crate::test_dispatch::new(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .finish(),
    );
    let semaphore = Arc::new(Semaphore::new(2));
    let (ran, unrun) = tracing::dispatcher::with_default(&dispatch, || {
        let caller = tracing::info_span!("caller.request");
        let _entered = caller.enter();
        let ran = BlockingCall::new(
            || tracing::info!("blocking work ran"),
            semaphore.clone().try_acquire_owned().unwrap(),
        );
        let announce = Announce;
        let unrun = BlockingCall::new(
            move || drop(announce),
            semaphore.clone().try_acquire_owned().unwrap(),
        );
        (ran, unrun)
    });
    // Neither thread has a default subscriber of its own.
    std::thread::spawn(move || ran.run()).join().unwrap();
    std::thread::spawn(move || drop(unrun)).join().unwrap();
    assert_eq!(semaphore.available_permits(), 2);
    let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    for message in ["blocking work ran", "unrun capture dropped"] {
        let lines: Vec<_> = text.lines().filter(|line| line.contains(message)).collect();
        assert_eq!(lines.len(), 1, "{message} once in {text}");
        assert!(lines[0].contains("caller.request"), "{text}");
        assert!(lines[0].contains("batter.blocking_work"), "{text}");
    }
}

#[tokio::test]
async fn recheck_refuses_a_cancelled_or_expired_context_and_releases_the_permit() {
    let semaphore = Arc::new(Semaphore::new(1));

    let owner = OperationOwner::new(Duration::from_secs(10)).unwrap();
    owner.cancel();
    let permit = semaphore.clone().try_acquire_owned().unwrap();
    assert!(matches!(
        recheck(owner.context(), permit),
        Err(AdmissionError::Interrupted(Interruption::Cancelled))
    ));
    assert_eq!(semaphore.available_permits(), 1);

    let expired = OperationContext::at(Instant::now());
    let permit = semaphore.clone().try_acquire_owned().unwrap();
    assert!(matches!(
        recheck(&expired, permit),
        Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded))
    ));
    assert_eq!(semaphore.available_permits(), 1);

    let live = OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let permit = semaphore.clone().try_acquire_owned().unwrap();
    let kept = recheck(&live, permit).unwrap();
    assert_eq!(semaphore.available_permits(), 0);
    drop(kept);
    assert_eq!(semaphore.available_permits(), 1);
}

/// Close lines of `name` itself, not of its `batter.blocking_work` child.
fn closes_of(text: &str, name: &str) -> usize {
    let own = format!("{name}: ");
    text.lines()
        .filter(|line| line.contains(&own) && line.trim_end().ends_with(" close"))
        .count()
}

#[test]
fn span_destruction_releases_the_callers_span_under_the_captured_dispatcher() {
    let output = Buffer(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let dispatch = crate::test_dispatch::new(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .with_max_level(tracing::Level::INFO)
            .finish(),
    );
    let semaphore = Arc::new(Semaphore::new(2));
    let (ran, unrun) = tracing::dispatcher::with_default(&dispatch, || {
        // Each caller span is dropped here, so only its child keeps it open.
        let ran = tracing::info_span!("caller.ran").in_scope(|| {
            BlockingCall::new(|| 7_u8, semaphore.clone().try_acquire_owned().unwrap())
        });
        let unrun = tracing::info_span!("caller.unrun").in_scope(|| {
            BlockingCall::new(|| 0_u8, semaphore.clone().try_acquire_owned().unwrap())
        });
        (ran, unrun)
    });
    // A different default registry on the destroying thread would panic, or
    // leak the caller span, if the child closed outside the captured one.
    let fallback = crate::test_dispatch::new(tracing_subscriber::registry());
    let ran = std::thread::spawn({
        let fallback = fallback.clone();
        move || tracing::dispatcher::with_default(&fallback, || ran.run())
    });
    assert_eq!(ran.join().unwrap(), 7);
    std::thread::spawn(move || tracing::dispatcher::with_default(&fallback, || drop(unrun)))
        .join()
        .unwrap();
    assert_eq!(semaphore.available_permits(), 2);
    let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    assert_eq!(closes_of(&text, "caller.ran"), 1, "{text}");
    assert_eq!(closes_of(&text, "caller.unrun"), 1, "{text}");
}

#[tokio::test]
async fn close_keeps_a_running_closures_permit_until_it_returns() {
    let bulkhead = super::Bulkhead::new(super::super::BulkheadCapacity::new(1).unwrap());
    let semaphore = bulkhead.semaphore.clone();
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let context = OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let caller = tokio::spawn({
        let bulkhead = bulkhead.clone();
        async move {
            bulkhead
                .run_blocking(&context, super::Admission::Reject, move || {
                    let _ = started.send(());
                    // A dropped sender also ends this wait.
                    let _ = release_rx.recv();
                    7_u8
                })
                .await
        }
    });
    started_rx.await.unwrap();
    bulkhead.close();
    assert!(semaphore.is_closed());
    // The closed bit does not hide permits: the running closure holds the one.
    assert_eq!(semaphore.available_permits(), 0);
    drop(release);
    assert_eq!(caller.await.unwrap().unwrap(), 7);
    assert_eq!(semaphore.available_permits(), 1);
}
