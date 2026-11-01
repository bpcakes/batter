//! Bounded blocking work polled inside a timer-enabled Tokio runtime records
//! exactly one bulkhead decision, after its post-admission context check, and
//! never an operation completion for its result wait; a call polled outside a
//! runtime records none.

use super::{assert_catalog, capture::Capture, owner};
use batter_core::{
    admission::{Admission, AdmissionError, BlockingError, Bulkhead, BulkheadCapacity},
    operation::{Interruption, OperationOwner},
    telemetry::metrics::*,
};
use std::{
    panic::AssertUnwindSafe,
    sync::mpsc,
    task::{Context, Waker},
    time::Duration,
};
use tokio::sync::oneshot;

fn bulkhead() -> Bulkhead {
    Bulkhead::new(BulkheadCapacity::new(1).unwrap())
}

fn decisions(capture: &Capture, decision: &str) -> f64 {
    capture.count(
        ADMISSION_DECISIONS,
        &[("admission", "bulkhead"), ("decision", decision)],
    )
}

/// Work a refused call destroys uncalled; running it would surface as a
/// `Panicked` result instead of the asserted refusal.
fn refused() -> impl FnOnce() -> u8 + Send + 'static {
    || -> u8 { unreachable!("refused blocking work never runs") }
}

/// A closure that reports its start, then parks its blocking thread until
/// released. Dropping the release sender, including while a failed assertion
/// unwinds, also lets it return, so runtime destruction cannot hang on it.
fn parked(
    value: u8,
) -> (
    oneshot::Receiver<()>,
    mpsc::Sender<()>,
    impl FnOnce() -> u8 + Send + 'static,
) {
    let (started_tx, started) = oneshot::channel();
    let (release, released) = mpsc::channel::<()>();
    let work = move || {
        let _ = started_tx.send(());
        let _ = released.recv();
        value
    };
    (started, release, work)
}

#[tokio::test]
async fn handed_off_calls_record_one_admitted_decision() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let bulkhead = bulkhead();

    let value = bulkhead
        .run_blocking(&context, Admission::Reject, || 7_u8)
        .await;
    assert_eq!(value.unwrap(), 7);
    let panicked = bulkhead
        .run_blocking(&context, Admission::Reject, || -> u8 {
            panic!("blocking defect")
        })
        .await;
    assert!(
        matches!(panicked, Err(BlockingError::Panicked(_))),
        "{panicked:?}"
    );

    let abandoning = owner();
    let (started, release, work) = parked(9);
    let caller = tokio::spawn({
        let bulkhead = bulkhead.clone();
        let context = abandoning.context().clone();
        async move {
            bulkhead
                .run_blocking(&context, Admission::Reject, work)
                .await
        }
    });
    started.await.unwrap();
    abandoning.cancel();
    let abandoned = caller.await.unwrap();
    assert!(
        matches!(
            abandoned,
            Err(BlockingError::Abandoned(Interruption::Cancelled))
        ),
        "{abandoned:?}"
    );
    release.send(()).unwrap();

    // The decision is recorded at handoff, so success, panic and abandonment
    // after handoff are each `admitted`, and nothing else is recorded.
    assert_eq!(decisions(&capture, "admitted"), 3.0);
    assert_eq!(capture.samples(ADMISSION_DECISIONS).len(), 3);
    // The result wait is a foundation-owned boundary, not an operation.
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_eq!(capture.samples(OPERATION_DURATION), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn refused_calls_record_their_refusal_once() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let bulkhead = bulkhead();
    let held = bulkhead.enter(&context, Admission::Reject).await.unwrap();

    let overloaded = bulkhead
        .run_blocking(&context, Admission::Reject, refused())
        .await;
    assert!(
        matches!(
            overloaded,
            Err(BlockingError::NotAdmitted(AdmissionError::Overloaded))
        ),
        "{overloaded:?}"
    );
    let cancelled_owner = owner();
    cancelled_owner.cancel();
    let cancelled = bulkhead
        .run_blocking(cancelled_owner.context(), Admission::Wait, refused())
        .await;
    assert!(
        matches!(
            cancelled,
            Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
                Interruption::Cancelled
            )))
        ),
        "{cancelled:?}"
    );
    // A zero timeout polls the admission wait while it stays pending, then
    // destroys it unfinished.
    let dropped = tokio::time::timeout(
        Duration::ZERO,
        bulkhead.run_blocking(&context, Admission::Wait, refused()),
    )
    .await;
    assert!(dropped.is_err());
    // A call that is never polled decides nothing.
    drop(bulkhead.run_blocking(&context, Admission::Wait, refused()));
    drop(held);
    bulkhead.close();
    let closed = bulkhead
        .run_blocking(&context, Admission::Reject, refused())
        .await;
    assert!(
        matches!(
            closed,
            Err(BlockingError::NotAdmitted(AdmissionError::Closed))
        ),
        "{closed:?}"
    );

    // `admitted` is the held `enter`; each refusal is recorded exactly once.
    for decision in ["admitted", "overloaded", "cancelled", "dropped", "closed"] {
        assert_eq!(decisions(&capture, decision), 1.0, "{decision}");
    }
    assert_eq!(capture.samples(ADMISSION_DECISIONS).len(), 5);
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_catalog(&capture);
}

#[tokio::test(start_paused = true)]
async fn deadline_refusals_record_deadline_exceeded_not_admitted() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let context = owner().into_context();
    let bulkhead = bulkhead();
    let expired = |result: &Result<u8, BlockingError>| {
        matches!(
            result,
            Err(BlockingError::NotAdmitted(AdmissionError::Interrupted(
                Interruption::DeadlineExceeded
            )))
        )
    };

    // The admission wait reaches its deadline. Paused time auto-advances
    // because no blocking task exists to inhibit it.
    let held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let short = OperationOwner::new(Duration::from_secs(1)).unwrap();
    let waited = bulkhead
        .run_blocking(short.context(), Admission::Wait, refused())
        .await;
    assert!(expired(&waited), "{waited:?}");
    drop(held);

    // A permit granted after the deadline but before its timer fires is
    // refused by the post-admission check, and only that refusal is recorded.
    let held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let waiter = tokio::spawn({
        let bulkhead = bulkhead.clone();
        let context = OperationOwner::new(Duration::from_secs(1))
            .unwrap()
            .into_context();
        async move {
            bulkhead
                .run_blocking(&context, Admission::Wait, refused())
                .await
        }
    });
    // The waiter registers its deadline timer and queues for the permit.
    tokio::task::yield_now().await;
    // Releasing grants and wakes the waiter; advancing moves the clock past
    // its deadline and yields, so the waiter runs before its timer fires.
    drop(held);
    tokio::time::advance(Duration::from_secs(2)).await;
    let rechecked = waiter.await.unwrap();
    assert!(expired(&rechecked), "{rechecked:?}");

    assert_eq!(decisions(&capture, "deadline_exceeded"), 2.0);
    // Only the two held `enter` calls were admitted.
    assert_eq!(decisions(&capture, "admitted"), 2.0);
    assert_eq!(capture.samples(ADMISSION_DECISIONS).len(), 4);
    assert_eq!(capture.samples(OPERATION_COMPLETIONS), []);
    assert_catalog(&capture);
}

#[tokio::test]
async fn run_blocking_destroyed_while_unwinding_records_panicked() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let bulkhead = bulkhead();
    let context = owner().into_context();
    let _held = bulkhead.enter(&context, Admission::Reject).await.unwrap();
    let waiting = tokio::spawn({
        let bulkhead = bulkhead.clone();
        async move {
            let wait = bulkhead.run_blocking(&context, Admission::Wait, refused());
            tokio::pin!(wait);
            // A zero timeout polls the wait while it stays pending, then returns
            // `Elapsed` and leaves it owned and pending.
            let polled = tokio::time::timeout(Duration::ZERO, wait.as_mut()).await;
            assert!(polled.is_err());
            panic!("sibling defect");
        }
    });
    assert!(waiting.await.unwrap_err().is_panic());
    assert_eq!(decisions(&capture, "admitted"), 1.0);
    assert_eq!(decisions(&capture, "panicked"), 1.0);
    assert_eq!(decisions(&capture, "dropped"), 0.0);
    assert_eq!(capture.samples(ADMISSION_DECISIONS).len(), 2);
    assert_catalog(&capture);
}

#[test]
fn run_blocking_outside_a_runtime_records_nothing() {
    let capture = Capture::unbounded();
    let _recorder = facade::set_default_local_recorder(&capture);
    let bulkhead = bulkhead();
    let context = owner().into_context();
    let mut call = Box::pin(bulkhead.run_blocking(&context, Admission::Reject, refused()));
    let polled = std::panic::catch_unwind(AssertUnwindSafe(|| {
        call.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    }));
    assert!(polled.is_err(), "polling outside a runtime must panic");
    drop(call);
    assert_eq!(capture.samples(ADMISSION_DECISIONS), []);
    assert!(capture.series().is_empty(), "{:?}", capture.series());
}
