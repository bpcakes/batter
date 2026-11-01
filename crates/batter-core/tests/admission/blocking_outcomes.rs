//! Completed blocking work delivers its value, its concrete domain error or
//! its retained panic after freeing capacity, under the caller's tracing.
//! Work its shutting-down runtime destroyed unstarted reports `NotStarted`.

use crate::blocking_support::{Logs, Unrun, bulkhead, fresh};
use batter_core::admission::{Admission, BlockingError};
use std::{
    future::Future,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use tracing::{Instrument, instrument::WithSubscriber};

#[tokio::test]
async fn closure_panic_is_reported_with_its_original_payload() {
    let bulkhead = bulkhead(1);
    let result = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || -> u8 {
            panic!("blocking defect")
        })
        .await;
    let Err(BlockingError::Panicked(payload)) = result else {
        panic!("a panicking closure must report Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<&str>().copied()),
        Ok(Some("blocking defect"))
    );
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProbePayload(u8);

#[tokio::test]
async fn opaque_panic_payload_is_retained() {
    let result = bulkhead(1)
        .run_blocking(&fresh(), Admission::Reject, || -> u8 {
            std::panic::panic_any(ProbePayload(7))
        })
        .await;
    let Err(BlockingError::Panicked(payload)) = result else {
        panic!("a panicking closure must report Panicked");
    };
    assert_eq!(
        payload.try_inspect(|value| value.downcast_ref::<ProbePayload>().copied()),
        Ok(Some(ProbePayload(7)))
    );
}

#[tokio::test]
async fn returns_a_value_from_a_blocking_thread_and_frees_capacity_first() {
    let bulkhead = bulkhead(1);
    let caller = std::thread::current().id();
    let worker = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || std::thread::current().id())
        .await
        .unwrap();
    assert_ne!(worker, caller);
    // The permit was released before the value was delivered.
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn fallible_closure_keeps_its_concrete_domain_error() {
    let bulkhead = bulkhead(1);
    let result = bulkhead
        .run_blocking(&fresh(), Admission::Reject, || {
            Err::<u8, io::Error>(io::Error::other("domain refusal"))
        })
        .await;
    let Ok(Err(error)) = result else {
        panic!("a domain error stays inside Ok");
    };
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "domain refusal");
    assert!(bulkhead.enter(&fresh(), Admission::Reject).await.is_ok());
}

#[tokio::test]
async fn blocking_work_runs_under_the_callers_dispatcher_and_span() {
    let logs = Logs::new();
    let bulkhead = bulkhead(1);
    let context = fresh();
    let result = async {
        // Created inside the subscriber, so it is the caller's live span.
        let span = tracing::info_span!("caller.request");
        async {
            bulkhead
                .run_blocking(&context, Admission::Reject, || {
                    tracing::info!(marker = "blocking.event", "blocking work ran");
                })
                .await
        }
        .instrument(span)
        .await
    }
    .with_subscriber(logs.dispatch.clone())
    .await;
    assert!(result.is_ok());
    let text = logs.text();
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("marker=\"blocking.event\""))
        .collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert!(lines[0].contains("caller.request"), "{text}");
    assert!(lines[0].contains("batter.blocking_work"), "{text}");
    assert!(text.contains("batter.blocking_wait"), "{text}");
    // The child span closed under the caller's dispatcher, so the caller's
    // own span was released and closed too.
    assert_eq!(logs.closes_of("batter.blocking_work"), 1, "{text}");
    assert_eq!(logs.closes_of("caller.request"), 1, "{text}");
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn run_blocking_future_is_send() {
    let bulkhead = bulkhead(1);
    let context = fresh();
    let call = bulkhead.run_blocking(&context, Admission::Reject, || 1_u8);
    assert_send(&call);
}

fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
    pin!(future).poll(&mut Context::from_waker(Waker::noop()))
}

/// Holds the runtime's only spare blocking thread until the sender drops, so
/// later blocking tasks queue and shutting the pool down waits for it.
fn occupy_blocking_thread(handle: &tokio::runtime::Handle) -> mpsc::Sender<()> {
    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    drop(handle.spawn_blocking(move || {
        started.send(()).unwrap();
        let _ = release_rx.recv();
    }));
    started_rx.recv().unwrap();
    release
}

/// Probes queue behind the occupied thread until the blocking pool starts
/// shutting down, which destroys new work at spawn.
fn await_blocking_pool_shutdown(handle: &tokio::runtime::Handle) {
    let limit = Instant::now() + Duration::from_secs(30);
    while !handle.spawn_blocking(|| ()).is_finished() {
        assert!(Instant::now() < limit, "the blocking pool never shut down");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A multi-thread runtime shuts its blocking pool down while a worker can
/// still poll the runtime's own tasks, even on a plain drop. Holding the only
/// worker inside a task's poll until the pool refuses work makes that
/// deterministic: the time driver stops only after that worker shuts down.
#[test]
fn a_task_on_a_dropping_multi_thread_runtime_is_not_started() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_time()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let release = occupy_blocking_thread(&handle);
    let (in_poll, in_poll_rx) = mpsc::channel();
    let (go, go_rx) = mpsc::channel::<()>();
    let (outcome, outcome_rx) = mpsc::channel();
    let unrun = Unrun::default();
    let work = unrun.closure();
    drop(runtime.spawn(async move {
        in_poll.send(()).unwrap();
        go_rx.recv().unwrap();
        let bulkhead = bulkhead(1);
        let context = fresh();
        let result = bulkhead
            .run_blocking(&context, Admission::Reject, work)
            .await;
        let freed = bulkhead.enter(&context, Admission::Reject).await.is_ok();
        let _ = outcome.send((matches!(result, Err(BlockingError::NotStarted)), freed));
    }));
    in_poll_rx.recv().unwrap();
    let dropping = std::thread::spawn(move || drop(runtime));
    await_blocking_pool_shutdown(&handle);
    go.send(()).unwrap();
    let (not_started, freed) = outcome_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(not_started, "the runtime's own task must see NotStarted");
    assert!(freed, "the destroyed closure must release its permit");
    unrun.assert_destroyed_uncalled();
    drop(release);
    dropping.join().unwrap();
}

/// Records whether it was destroyed outside an unwinding panic.
struct Calm(Arc<AtomicBool>);

impl Drop for Calm {
    fn drop(&mut self) {
        self.0.store(!std::thread::panicking(), Ordering::SeqCst);
    }
}

/// A current-thread runtime never polls its own tasks during
/// `shutdown_timeout`, which shuts the blocking pool down before the runtime
/// shuts its time driver down. A call polled from outside it in that window
/// sees `NotStarted`; after the runtime is gone, the shut-down timer panics.
#[test]
fn an_outside_call_during_current_thread_shutdown_is_not_started() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let release = occupy_blocking_thread(&handle);
    let shutdown = std::thread::spawn(move || runtime.shutdown_timeout(Duration::from_secs(30)));
    await_blocking_pool_shutdown(&handle);
    let bulkhead = bulkhead(1);
    let context = fresh();
    let unrun = Unrun::default();
    let entered = handle.enter();
    // The deadline timer has not shut down, so the first poll sees the result.
    let result = poll_once(bulkhead.run_blocking(&context, Admission::Reject, unrun.closure()));
    assert!(matches!(
        result,
        Poll::Ready(Err(BlockingError::NotStarted))
    ));
    unrun.assert_destroyed_uncalled();
    assert!(matches!(
        poll_once(bulkhead.enter(&context, Admission::Reject)),
        Poll::Ready(Ok(_))
    ));
    drop(entered);
    drop(release);
    shutdown.join().unwrap();
    // After handoff, the shut-down deadline timer panics instead.
    let late = Unrun::default();
    let calm = Arc::new(AtomicBool::new(false));
    let work = {
        let (capture, work) = (Calm(calm.clone()), late.closure());
        move || {
            drop(capture);
            work()
        }
    };
    let _entered = handle.enter();
    let payload = catch_unwind(AssertUnwindSafe(|| {
        poll_once(bulkhead.run_blocking(&context, Admission::Reject, work))
    }))
    .expect_err("a poll after the runtime is gone must panic");
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(
        message.contains("is being shutdown"),
        "unexpected panic: {message}"
    );
    assert!(
        calm.load(Ordering::SeqCst),
        "Tokio destroyed the closure at spawn"
    );
    late.assert_destroyed_uncalled();
    assert!(matches!(
        poll_once(bulkhead.enter(&context, Admission::Reject)),
        Poll::Ready(Ok(_))
    ));
}
