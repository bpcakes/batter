//! Owned sequencing, independent evidence and cancellation through the public API.
#[path = "phased_operation/adversarial.rs"]
mod adversarial;

use batter_core::{
    command::CommandCause,
    operation::{
        FailedPhasedReport, Interruption, OperationOwner, PhasedFailure, PhasedOperation,
        PhasedSuccess,
    },
};
use std::{
    convert::Infallible,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

fn parent() -> OperationOwner {
    OperationOwner::new(Duration::from_secs(10)).expect("valid parent budget")
}
fn reserve() -> Duration {
    Duration::from_secs(2)
}

#[derive(Debug)]
struct Secret(&'static str);

struct Destroyed(Arc<AtomicBool>);
impl Drop for Destroyed {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn early_work_error_is_retained_before_fallible_finalization() {
    let parent = parent();
    let destroyed = Arc::new(AtomicBool::new(false));
    let mark = destroyed.clone();
    let check = destroyed.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |_work| {
            let _guard = Destroyed(mark);
            Err::<(), _>(Secret("secret-work"))?;
            Ok::<_, Secret>(())
        },
        async move |scope| {
            assert!(
                check.load(Ordering::SeqCst),
                "work destroyed before finalization"
            );
            assert!(matches!(
                scope.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Failed(Secret("secret-work")))
            ));
            Err::<(), _>(Secret("secret-finalization"))
        },
    )
    .unwrap()
    .start();
    let outcome = running.wait().await;
    assert!(
        matches!(outcome, Err(PhasedFailure::Report(_))),
        "phase failures cannot return Ok"
    );
    let report = retained(outcome);
    assert!(report.work().classify().is_err() && report.finalization().classify().is_err());
    assert!(matches!(
        report.work().classify().unwrap_err().returned_result(),
        Err(CommandCause::Failed(Secret("secret-work")))
    ));
    assert!(matches!(
        report
            .finalization()
            .classify()
            .unwrap_err()
            .returned_result(),
        Err(CommandCause::Failed(Secret("secret-finalization")))
    ));
    let text = format!("{report:?}");
    assert!(!text.contains("secret-"), "redacted: {text}");
}

#[tokio::test(start_paused = true)]
async fn deadline_stops_work_and_preserves_original_finalization_deadline() {
    let parent = parent();
    let deadline = parent.context().deadline();
    let destroyed = Arc::new(AtomicBool::new(false));
    let mark = destroyed.clone();
    let check = destroyed.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |work| {
            assert_eq!(work.deadline(), deadline - reserve());
            let _guard = Destroyed(mark);
            pending::<Result<(), Infallible>>().await
        },
        async move |scope| {
            assert!(check.load(Ordering::SeqCst), "timed out work destroyed");
            assert!(matches!(
                scope.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
            ));
            assert_eq!(scope.context().deadline(), deadline);
            assert_eq!(scope.context().remaining(), reserve());
            Ok::<_, Infallible>(42)
        },
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert_eq!(report.finalization().classify().ok(), Some(&42));
    assert!(parent.context().check().is_ok(), "no upward cancellation");
}

#[tokio::test(start_paused = true)]
async fn lost_owner_before_first_poll_skips_work_but_runs_finalization() {
    let parent = parent();
    let started = Arc::new(AtomicBool::new(false));
    let mark = started.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |_work| {
            mark.store(true, Ordering::SeqCst);
            Ok::<_, Infallible>(())
        },
        async |scope| {
            assert!(matches!(
                scope.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Interrupted(Interruption::Cancelled))
            ));
            assert!(
                scope.context().check().is_ok(),
                "owner cancellation spares finalization"
            );
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    let observer = running.observer();
    drop(running);
    let report = retained(observer.wait().await);
    assert!(!started.load(Ordering::SeqCst), "uncalled work factory");
    assert!(report.finalization().is_success());
}

#[tokio::test(start_paused = true)]
async fn borrowed_waiter_cancellation_leaves_work_alive_and_reports_are_retained() {
    let parent = parent();
    let release = Arc::new(Notify::new());
    let wait = release.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |work| {
            wait.notified().await;
            assert!(work.check().is_ok(), "dropped waiter did not cancel work");
            Ok::<_, Infallible>(7)
        },
        async |scope| Ok::<_, Infallible>(*scope.work().classify().unwrap()),
    )
    .unwrap()
    .start();
    let mut waiting = Box::pin(running.wait());
    tokio::select! { biased;
        _ = &mut waiting => panic!("work not released"),
        _ = tokio::task::yield_now() => {}
    }
    drop(waiting);
    release.notify_one();
    let report = running.wait().await.unwrap();
    let observer = running.observer();
    drop(running);
    let retained = observer.wait().await.unwrap();
    assert!(
        std::ptr::eq(report.work(), retained.work()),
        "same retained value"
    );
    assert_eq!(*report.work(), 7);
    assert_eq!(*report.finalization(), 7);
}

#[tokio::test(start_paused = true)]
async fn parent_cancellation_interrupts_both_phases() {
    let parent = parent();
    let finalized = Arc::new(AtomicBool::new(false));
    let mark = finalized.clone();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |_work| pending::<Result<(), Infallible>>().await,
        async move |_scope| {
            mark.store(true, Ordering::SeqCst);
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    parent.cancel();
    let report = retained(running.wait().await);
    assert!(matches!(
        report.work().classify().unwrap_err().returned_result(),
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(matches!(
        report
            .finalization()
            .classify()
            .unwrap_err()
            .returned_result(),
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(
        !finalized.load(Ordering::SeqCst),
        "cancelled finalization factory not invoked"
    );
}

#[tokio::test(start_paused = true)]
async fn work_panic_cannot_bypass_finalization_and_finalization_panic_keeps_work() {
    let parent = parent();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |_work| {
            panic!("work secret");
            #[expect(unreachable_code, reason = "types the panicking phase")]
            Ok::<(), Infallible>(())
        },
        async |scope| {
            assert!(matches!(
                scope.work().classify().unwrap_err().returned_result(),
                Err(CommandCause::Panicked(_))
            ));
            panic!("finalization secret");
            #[expect(unreachable_code, reason = "types the panicking phase")]
            Ok::<(), Infallible>(())
        },
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert!(matches!(
        report.work().classify().unwrap_err().returned_result(),
        Err(CommandCause::Panicked(_))
    ));
    assert!(matches!(
        report
            .finalization()
            .classify()
            .unwrap_err()
            .returned_result(),
        Err(CommandCause::Panicked(_))
    ));
    assert!(
        !format!("{report:?}").contains("secret"),
        "panic payload redacted"
    );
}

#[tokio::test(start_paused = true)]
async fn finalization_timeout_keeps_successful_work() {
    let parent = parent();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |_work| Ok::<_, Infallible>(7),
        async |_scope| pending::<Result<(), Infallible>>().await,
    )
    .unwrap()
    .start();
    let report = retained(running.wait().await);
    assert_eq!(report.work().classify().ok(), Some(&7));
    assert!(matches!(
        report
            .finalization()
            .classify()
            .unwrap_err()
            .returned_result(),
        Err(CommandCause::Interrupted(Interruption::DeadlineExceeded))
    ));
}

#[tokio::test(start_paused = true)]
async fn unstarted_specification_is_inert_and_invalid_reserve_is_rejected() {
    let parent = parent();
    let events = Arc::new(Mutex::new(Vec::new()));
    let work = events.clone();
    let finalization = events.clone();
    let specification = PhasedOperation::new(
        parent.context(),
        reserve(),
        async move |_scope| {
            work.lock().unwrap().push("work");
            Ok::<_, ()>(())
        },
        async move |_scope| {
            finalization.lock().unwrap().push("finalization");
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    drop(specification);
    assert!(events.lock().unwrap().is_empty(), "no factory before start");
    assert!(
        PhasedOperation::new(
            parent.context(),
            Duration::ZERO,
            async |_scope| Ok::<_, ()>(()),
            async |_scope| Ok::<_, ()>(())
        )
        .is_err()
    );
}

type Checked<T, E, U, F> = Result<PhasedSuccess<T, U>, PhasedFailure<T, E, U, F>>;

fn retained<T, E, U, F>(result: Checked<T, E, U, F>) -> FailedPhasedReport<T, E, U, F> {
    match result {
        Err(PhasedFailure::Report(report)) => report,
        Ok(_) => panic!("expected failed report"),
        Err(error) => panic!("unexpected coordinator failure: {error:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn successful_work_context_is_cancelled_before_finalization_begins() {
    let parent = parent();
    let running = PhasedOperation::new(
        parent.context(),
        reserve(),
        async |work| Ok::<_, Infallible>(work.clone()),
        async |scope| {
            let retained_work = scope.work().classify().unwrap();
            assert_eq!(retained_work.check(), Err(Interruption::Cancelled));
            assert!(scope.context().check().is_ok(), "sibling remains live");
            Ok::<_, Infallible>(())
        },
    )
    .unwrap()
    .start();
    let result = running.wait().await.unwrap();
    assert_eq!(result.work().check(), Err(Interruption::Cancelled));
    assert!(parent.context().check().is_ok());
}
