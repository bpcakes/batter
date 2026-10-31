//! The sole-owner path through the public API only. These tests never install
//! signal listeners: process-wide signal selection belongs to executable roots.

use super::*;
use batter_core::{
    BoxError, RegistrationError,
    cleanup::CleanupReport,
    command::{CommandFailure, CommandReport},
};
use std::{cell::Cell, error::Error as _};

#[derive(Debug, thiserror::Error)]
#[error("secret original failure")]
struct Original;

#[derive(Debug, thiserror::Error)]
#[error("secret cleanup failure in {0}")]
struct CleanupSecret(&'static str);

/// Deliberately neither `Clone` nor `Debug`.
struct NotClone(&'static str);

/// Sends the work child's state when the work future is destroyed.
struct ContextProbe {
    context: OperationContext,
    observed: Option<oneshot::Sender<Result<(), Interruption>>>,
}

impl Drop for ContextProbe {
    fn drop(&mut self) {
        if let Some(observed) = self.observed.take() {
            let _ = observed.send(self.context.check());
        }
    }
}

/// Compile-time proof that a type lacks a trait: the call is ambiguous, and
/// fails to compile, exactly when both blanket implementations apply.
macro_rules! assert_not_impl {
    ($type:ty: $trait:path) => {{
        trait AmbiguousIfImpl<A> {
            fn some_item() {}
        }
        impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
        impl<T: ?Sized + $trait> AmbiguousIfImpl<u8> for T {}
        let _ = <$type as AmbiguousIfImpl<_>>::some_item;
    }};
}

fn names(report: &CleanupReport) -> Vec<&'static str> {
    report.records.iter().map(|record| record.name).collect()
}

#[tokio::test(start_paused = true)]
async fn success_moves_out_a_value_that_is_neither_clone_nor_sync() {
    type Value = (Cell<u32>, NotClone);
    assert_not_impl!(Value: Clone);
    assert_not_impl!(Value: Sync);
    let closed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closing = closed.clone();
    let owner = Command::new(context(), budget(), move |scope| {
        Box::pin(async move {
            for name in ["dependency", "dependent"] {
                let closing = closing.clone();
                scope.reserve_cleanup(name)?.register(move || async move {
                    closing.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
            }
            scope.stage("command.export")?;
            Ok::<Value, RegistrationError>((Cell::new(7), NotClone("moved value")))
        })
    })
    .start_without_signals();
    let success = owner.finish().await.expect("complete contract succeeded");
    assert_eq!(closed.load(Ordering::SeqCst), 2);
    assert_eq!(success.stage(), "command.export");
    let cleanup = success.cleanup();
    assert!(cleanup.is_success());
    assert!(cleanup.skipped.is_empty());
    assert_eq!(names(cleanup), ["dependent", "dependency"]);
    assert!(
        cleanup.records.iter().all(|record| {
            record.outcome == CleanupOutcome::Succeeded && record.error.is_none()
        })
    );
    assert_eq!(success.value().0.get(), 7);
    let text = format!("{success} {success:?}");
    assert!(text.contains("command.export"), "{text}");
    assert!(!text.contains("moved value"), "{text}");
    let (count, moved) = success.into_value();
    count.set(8);
    assert_eq!((count.get(), moved.0), (8, "moved value"));
}

#[tokio::test(start_paused = true)]
async fn failure_retains_original_work_error_and_every_cleanup_error_unformatted() {
    let owner = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope.stage("command.prepare").unwrap();
            for name in ["first", "second"] {
                scope
                    .reserve_cleanup(name)
                    .unwrap()
                    .register(
                        move || async move { Err(Box::new(CleanupSecret(name)) as BoxError) },
                    );
            }
            Err::<u32, _>(Original)
        })
    })
    .start_without_signals();
    let failure = owner.finish().await.expect_err("work and cleanup failed");
    let CommandFailure::Report(report) = &failure else {
        panic!("complete report missing: {failure}")
    };
    assert!(matches!(report.work, Err(CommandCause::Failed(Original))));
    assert_eq!(report.stage, "command.prepare");
    assert!(report.interruption_after_work.is_none());
    assert!(report.destruction_panic.is_none());
    let cleanup = report.cleanup.as_ref().unwrap();
    assert!(cleanup.skipped.is_empty());
    assert_eq!(names(cleanup), ["second", "first"]);
    for record in &cleanup.records {
        assert_eq!(record.outcome, CleanupOutcome::Failed);
        let retained = record.error.as_ref().unwrap();
        assert_eq!(
            retained.downcast_ref::<CleanupSecret>().unwrap().0,
            record.name
        );
    }
    let source = failure.source().expect("failure exposes its report");
    let chained = source
        .downcast_ref::<CommandReport<u32, Original>>()
        .expect("the source is the complete report");
    assert!(std::ptr::eq(chained, &**report));
    let original = chained.source().expect("report exposes the original error");
    assert!(original.downcast_ref::<Original>().is_some());
    let text = format!("{failure} {failure:?}");
    assert!(!text.contains("secret"), "{text}");
    assert!(text.contains("command.prepare"), "{text}");
    assert!(text.contains("2 unsuccessful"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn cleanup_failure_exposes_the_completed_value_only_by_reference() {
    let owner = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("resource")?
                .register(|| async { Err(Box::new(CleanupSecret("resource")) as BoxError) });
            Ok::<_, RegistrationError>(NotClone("completed value"))
        })
    })
    .start_without_signals();
    let Err(CommandFailure::Report(report)) = owner.finish().await else {
        panic!("failed cleanup must not yield success")
    };
    assert_eq!(
        report.work.as_ref().ok().map(|value| value.0),
        Some("completed value")
    );
    assert!(report.interruption_after_work.is_none());
    assert!(report.destruction_panic.is_none());
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(names(cleanup), ["resource"]);
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn final_poll_cancellation_keeps_the_value_without_success() {
    let owner = Command::new(context(), budget(), |scope| {
        Box::pin(async move {
            scope.cancel_work();
            Ok::<_, Infallible>(42)
        })
    })
    .start_without_signals();
    let Err(CommandFailure::Report(report)) = owner.finish().await else {
        panic!("late cancellation must not yield success")
    };
    assert_eq!(report.work.as_ref().ok(), Some(&42));
    assert_eq!(
        report.interruption_after_work,
        Some(Interruption::Cancelled)
    );
    assert!(report.cleanup.as_ref().unwrap().is_success());
    assert!(!report.is_success());
}

#[tokio::test(start_paused = true)]
async fn dropping_an_unfinished_finish_cancels_work_downward_and_still_finalizes() {
    let parent = context();
    let (started, starting) = oneshot::channel();
    let (destroyed, destruction) = oneshot::channel();
    let (finalized, finalization) = oneshot::channel();
    let owner = Command::new(parent.clone(), budget(), move |scope| {
        Box::pin(async move {
            let child = scope
                .context()
                .child(Duration::from_secs(3))
                .unwrap()
                .into_context();
            let observed = child.clone();
            scope
                .reserve_cleanup("resource")?
                .register(move || async move {
                    finalized.send(observed.check()).unwrap();
                    Ok(())
                });
            let _probe = ContextProbe {
                context: child,
                observed: Some(destroyed),
            };
            started.send(()).unwrap();
            pending::<()>().await;
            Ok::<_, RegistrationError>(())
        })
    })
    .start_without_signals();
    starting.await.unwrap();
    // The pending finish future owns the owner; dropping it is owner drop.
    poll_once(owner.finish()).await;
    assert_eq!(destruction.await.unwrap(), Err(Interruption::Cancelled));
    assert_eq!(finalization.await.unwrap(), Err(Interruption::Cancelled));
    assert!(parent.check().is_ok());
}

#[tokio::test]
async fn owner_drop_before_the_coordinator_runs_invokes_no_factory() {
    let invoked = Arc::new(AtomicBool::new(false));
    let invoking = invoked.clone();
    let (probe, closed) = oneshot::channel::<()>();
    let owner = Command::new(context(), budget(), move |_| {
        invoking.store(true, Ordering::SeqCst);
        drop(probe);
        Box::pin(async { Ok::<_, Infallible>(()) })
    })
    .start_without_signals();
    // A current-thread coordinator cannot run before this task yields.
    drop(owner);
    // Invocation would set the flag before releasing the probe.
    assert!(closed.await.is_err(), "the factory was destroyed");
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn cancel_before_finish_interrupts_work_and_finalizes_without_cancelling_parent() {
    let parent = context();
    let (started, starting) = oneshot::channel();
    let owner = Command::new(parent.clone(), budget(), move |scope| {
        Box::pin(async move {
            scope
                .reserve_cleanup("resource")?
                .register(|| async { Ok(()) });
            started.send(()).unwrap();
            pending::<()>().await;
            Ok::<_, RegistrationError>(())
        })
    })
    .start_without_signals();
    starting.await.unwrap();
    owner.cancel();
    owner.cancel();
    let Err(CommandFailure::Report(report)) = owner.finish().await else {
        panic!("cancelled work must not yield success")
    };
    assert!(matches!(
        report.work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    ));
    assert!(report.interruption_after_work.is_none());
    let cleanup = report.cleanup.as_ref().unwrap();
    assert_eq!(names(cleanup), ["resource"]);
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(cleanup.is_success());
    assert!(parent.check().is_ok());
}

async fn finished_value(work: Result<u32, Original>) -> Result<u32, BoxError> {
    let owner = Command::new(context(), budget(), move |_| Box::pin(async move { work }))
        .start_without_signals();
    let success = owner.finish().await?;
    Ok(success.into_value())
}

#[tokio::test(start_paused = true)]
async fn value_and_failure_cross_a_box_error_boundary() {
    assert_eq!(finished_value(Ok(42)).await.unwrap(), 42);
    let error = finished_value(Err(Original)).await.unwrap_err();
    assert!(!format!("{error} {error:?}").contains("secret"));
    let failure = error
        .downcast_ref::<CommandFailure<u32, Original>>()
        .expect("the failure keeps its concrete type");
    let CommandFailure::Report(report) = failure else {
        panic!("complete report missing: {failure}")
    };
    assert!(matches!(report.work, Err(CommandCause::Failed(Original))));
    assert!(report.cleanup.as_ref().unwrap().is_success());
}
