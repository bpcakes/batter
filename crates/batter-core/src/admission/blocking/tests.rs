use super::{BlockingCall, BlockingError, join_failure, recheck};
use crate::{
    admission::AdmissionError,
    operation::{Interruption, OperationContext, OperationOwner},
};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::{sync::Semaphore, time::Instant};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// An INFO fmt subscriber that writes to memory and logs `span_events`.
fn capture(span_events: FmtSpan) -> (Buffer, tracing::Dispatch) {
    let output = Buffer(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let dispatch = crate::test_dispatch::new(
        tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .with_span_events(span_events)
            .with_max_level(tracing::Level::INFO)
            .finish(),
    );
    (output, dispatch)
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
    // Each permit is free once `run` returns, before Tokio could publish the
    // outcome that is still held here.
    let (returned, semaphore, seen) = probed_call(|| 7);
    let delivered = returned.run();
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(semaphore.available_permits(), 1);
    assert!(matches!(delivered.into_outcome(), Ok(7)));

    // The unwind is caught as the outcome instead of escaping into Tokio.
    let (unwound, semaphore, seen) = probed_call(|| panic!("blocking defect"));
    let delivered = unwound.run();
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert_eq!(semaphore.available_permits(), 1);
    assert!(delivered.into_outcome().is_err());

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
    let (output, dispatch) = capture(FmtSpan::NONE);
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
    let text = output.text();
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
    let (output, dispatch) = capture(FmtSpan::CLOSE);
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
    assert!(matches!(ran.join().unwrap().into_outcome(), Ok(7)));
    std::thread::spawn(move || tracing::dispatcher::with_default(&fallback, || drop(unrun)))
        .join()
        .unwrap();
    assert_eq!(semaphore.available_permits(), 2);
    let text = output.text();
    assert_eq!(closes_of(&text, "caller.ran"), 1, "{text}");
    assert_eq!(closes_of(&text, "caller.unrun"), 1, "{text}");
}

/// Holds a span created inside the closure and logs inside it when destroyed.
struct Leftover(tracing::Span);

impl Leftover {
    fn new() -> Self {
        Self(tracing::info_span!("closure.outcome"))
    }
}

impl Drop for Leftover {
    fn drop(&mut self) {
        self.0
            .in_scope(|| tracing::info!("undelivered outcome dropped"));
    }
}

#[test]
fn abandoned_outcomes_are_destroyed_under_the_captured_dispatcher() {
    let (output, dispatch) = capture(FmtSpan::CLOSE);
    let semaphore = Arc::new(Semaphore::new(2));
    let permit = || semaphore.clone().try_acquire_owned().unwrap();
    let (value, panic) = tracing::dispatcher::with_default(&dispatch, || {
        // Each caller span is dropped here, so only its child keeps it open.
        let value = tracing::info_span!("caller.value")
            .in_scope(|| BlockingCall::new(Leftover::new, permit()));
        let panic = tracing::info_span!("caller.panic").in_scope(|| {
            BlockingCall::new(
                || -> u8 { std::panic::panic_any(Leftover::new()) },
                permit(),
            )
        });
        (value, panic)
    });
    // Tokio's blocking threads have no default dispatcher of their own.
    let value = std::thread::spawn(move || value.run()).join().unwrap();
    let panic = std::thread::spawn(move || panic.run()).join().unwrap();
    assert_eq!(semaphore.available_permits(), 2);
    assert!(panic.outcome.as_ref().is_some_and(Result::is_err));
    assert!(!output.text().contains("undelivered outcome dropped"));
    // As when Tokio destroys an abandoned outcome: another default registry
    // would swallow the events and panic on, or leak, the caller's span.
    let fallback = crate::test_dispatch::new(tracing_subscriber::registry());
    std::thread::spawn(move || {
        tracing::dispatcher::with_default(&fallback, || drop((value, panic)))
    })
    .join()
    .unwrap();
    let text = output.text();
    for caller in ["caller.value", "caller.panic"] {
        let dropped = text
            .lines()
            .filter(|line| line.contains("undelivered outcome dropped") && line.contains(caller));
        assert_eq!(dropped.count(), 1, "{caller} in {text}");
        assert_eq!(closes_of(&text, caller), 1, "{text}");
    }
    assert_eq!(closes_of(&text, "batter.blocking_work"), 2, "{text}");
}

/// A capture whose destructor panics with a `Leftover` payload.
struct Explodes;

impl Drop for Explodes {
    fn drop(&mut self) {
        std::panic::panic_any(Leftover::new());
    }
}

#[test]
fn a_capture_panic_while_destroyed_unstarted_is_retained_under_the_captured_dispatcher() {
    let (output, dispatch) = capture(FmtSpan::CLOSE);
    let semaphore = Arc::new(Semaphore::new(2));
    let permit = || semaphore.clone().try_acquire_owned().unwrap();
    let (observed, abandoned) = tracing::dispatcher::with_default(&dispatch, || {
        let (first, second) = (Explodes, Explodes);
        let observed = tracing::info_span!("caller.observed")
            .in_scope(|| BlockingCall::new(move || drop(first), permit()));
        let abandoned = tracing::info_span!("caller.abandoned")
            .in_scope(|| BlockingCall::new(move || drop(second), permit()));
        (observed, abandoned)
    });
    let runtime = single_blocking_thread();
    let error = runtime.block_on(async {
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let occupant = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = release_rx.recv();
        });
        started_rx.recv().unwrap();
        let observed = tokio::task::spawn_blocking(move || observed.run());
        let abandoned = tokio::task::spawn_blocking(move || abandoned.run());
        // As at runtime shutdown, the busy worker destroys both unstarted.
        observed.abort();
        abandoned.abort();
        // Nobody takes this one's panic: Tokio destroys it on its thread.
        drop(abandoned);
        drop(release);
        occupant.await.unwrap();
        observed
            .await
            .err()
            .expect("the unstarted closure must fail")
    });
    // Waits for the blocking thread, which destroys the abandoned payload.
    drop(runtime);
    assert_eq!(semaphore.available_permits(), 2);
    let BlockingError::Panicked(payload) = join_failure(error) else {
        panic!("a capture's panic must map to Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.is::<Leftover>()),
        Ok(true)
    );
    tracing::dispatcher::with_default(&dispatch, || drop(payload));
    let text = output.text();
    for caller in ["caller.observed", "caller.abandoned"] {
        let dropped = text
            .lines()
            .filter(|line| line.contains("undelivered outcome dropped") && line.contains(caller));
        assert_eq!(dropped.count(), 1, "{caller} in {text}");
        assert_eq!(closes_of(&text, caller), 1, "{text}");
    }
}

#[tokio::test]
async fn a_caught_closure_panic_still_records_a_failed_wait() {
    let (output, dispatch) = capture(FmtSpan::NONE);
    let bulkhead = super::Bulkhead::new(super::super::BulkheadCapacity::new(1).unwrap());
    let context = OperationOwner::new(Duration::from_secs(10))
        .unwrap()
        .into_context();
    let result = bulkhead
        .run_blocking(&context, super::Admission::Reject, || -> u8 {
            panic!("blocking defect")
        })
        .with_subscriber(dispatch)
        .await;
    assert!(matches!(result, Err(BlockingError::Panicked(_))));
    let text = output.text();
    let finished: Vec<_> = text
        .lines()
        .filter(|line| line.contains("batter.blocking_wait") && line.contains("boundary finished"))
        .collect();
    assert_eq!(finished.len(), 1, "{text}");
    assert!(finished[0].contains("outcome=\"failed\""), "{text}");
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
