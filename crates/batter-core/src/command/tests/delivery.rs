//! Where a delivery lands relative to the factory, work, its return and cleanup.

use super::{
    Marker, assert_removed_once, budget, entries, failed_report, injected, parent, push,
    start_injected, succeeded,
};
use crate::{
    BoxError, RegistrationError,
    cleanup::{CleanupOutcome, CleanupReport},
    command::{Command, CommandCause},
    operation::{Interruption, OperationContext, OperationOwner},
};
use std::{
    convert::Infallible,
    future::{Future, pending},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

fn is_cancelled<T, E>(work: &Result<T, CommandCause<E>>) -> bool {
    matches!(
        work,
        Err(CommandCause::Interrupted(Interruption::Cancelled))
    )
}

#[tokio::test]
async fn queued_delivery_destroys_the_factory_uninvoked_before_removing_listeners() {
    let (source, sender, log) = injected();
    sender.send(()).unwrap();
    let parent = parent();
    let invoked = Arc::new(AtomicBool::new(false));
    let invoking = invoked.clone();
    let factory = Marker::new(&log, "factory-destroyed");
    let command = Command::new(parent.context().clone(), budget(), move |_| {
        invoking.store(true, Ordering::SeqCst);
        Box::pin(async move {
            drop(factory);
            Ok::<_, Infallible>(())
        })
    });
    let report = failed_report(start_injected(command, source).finish().await);
    assert!(!invoked.load(Ordering::SeqCst), "a visible signal ran work");
    assert!(is_cancelled(&report.work));
    assert_eq!(report.interruption_after_work, None);
    assert!(report.destruction_panic.is_none());
    assert_eq!(report.stage, "command");
    let cleanup = report
        .cleanup
        .as_ref()
        .expect("cleanup coordinator reported");
    assert!(cleanup.is_success());
    assert!(cleanup.records.is_empty() && cleanup.skipped.is_empty());
    assert!(
        parent.context().check().is_ok(),
        "a signal reached the parent"
    );
    assert_eq!(entries(&log), ["factory-destroyed", "listeners-dropped"]);
    assert!(sender.is_closed());
}

#[tokio::test]
async fn delivery_during_work_removes_listeners_before_cleanup_and_spares_the_parent() {
    let (source, sender, log) = injected();
    let parent = parent();
    let (started, running) = oneshot::channel();
    let (observed, observation) = oneshot::channel();
    let (work_log, hook_log, removal) = (log.clone(), log.clone(), sender.clone());
    let command = Command::new(parent.context().clone(), budget(), move |scope| {
        Box::pin(async move {
            let _destroyed = Marker::new(&work_log, "work-destroyed");
            let descendant = scope.context().child(Duration::from_secs(30))?;
            let descendant = descendant.into_context();
            scope
                .reserve_cleanup("signal.observer")?
                .register(move || async move {
                    push(&hook_log, "cleanup");
                    observed
                        .send((descendant.check(), removal.is_closed()))
                        .map_err(|_| "cleanup observer dropped")?;
                    Ok::<(), BoxError>(())
                });
            started.send(()).map_err(|()| "test stopped waiting")?;
            pending::<Result<(), BoxError>>().await
        })
    });
    let owner = start_injected(command, source);
    running.await.expect("work started");
    sender
        .send(())
        .expect("listeners are installed while work runs");
    let report = failed_report(owner.finish().await);
    let (descendant, removed) = observation.await.expect("cleanup hook ran");
    assert_eq!(descendant, Err(Interruption::Cancelled));
    assert!(removed, "a cleanup hook could still receive a signal");
    assert_eq!(
        entries(&log),
        ["work-destroyed", "listeners-dropped", "cleanup"]
    );
    assert!(is_cancelled(&report.work));
    assert_eq!(report.interruption_after_work, None);
    let cleanup = report
        .cleanup
        .as_ref()
        .expect("cleanup coordinator reported");
    assert_eq!(cleanup.records.len(), 1);
    assert_eq!(cleanup.records[0].name, "signal.observer");
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(
        parent.context().check().is_ok(),
        "a signal reached the parent"
    );
}

/// Deliberately no derives: the value is neither `Clone` nor `Debug`.
struct Exported {
    rows: u32,
}

#[tokio::test]
async fn success_moves_an_uncloneable_value_out_after_listeners_are_removed() {
    let (source, sender, log) = injected();
    let removal = sender.clone();
    let (observed, observation) = oneshot::channel();
    let command = Command::new(parent().into_context(), budget(), move |scope| {
        Box::pin(async move {
            scope.stage("export.rows")?;
            scope
                .reserve_cleanup("export.source")?
                .register(move || async move {
                    observed
                        .send(removal.is_closed())
                        .map_err(|_| "cleanup observer dropped")?;
                    Ok::<(), BoxError>(())
                });
            Ok::<_, RegistrationError>(Exported { rows: 3 })
        })
    });
    let success = succeeded(start_injected(command, source).finish().await);
    assert!(observation.await.expect("cleanup hook ran"));
    assert_eq!(success.stage(), "export.rows");
    let cleanup = success.cleanup();
    assert!(cleanup.skipped.is_empty());
    assert_eq!(cleanup.records.len(), 1);
    assert_eq!(cleanup.records[0].name, "export.source");
    assert_eq!(cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(success.value().rows, 3);
    let exported: Exported = success.into_value();
    assert_eq!(exported.rows, 3);
    assert_removed_once(&sender, &log);
}

/// Work whose only poll makes a delivery visible and then returns, as a
/// signal arriving just before work completes.
struct DeliveredOnReturn {
    delivery: mpsc::UnboundedSender<()>,
}

impl Future for DeliveredOnReturn {
    type Output = Result<u32, Infallible>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.delivery
            .send(())
            .expect("listeners are installed while work runs");
        Poll::Ready(Ok(7))
    }
}

#[tokio::test]
async fn delivery_visible_when_work_returns_is_a_late_cancellation_not_success() {
    let (source, sender, log) = injected();
    let delivery = sender.clone();
    let parent = parent();
    let command = Command::new(parent.context().clone(), budget(), move |_| {
        Box::pin(DeliveredOnReturn { delivery })
    });
    let report = failed_report(start_injected(command, source).finish().await);
    assert_eq!(report.work.as_ref().ok(), Some(&7));
    assert_eq!(
        report.interruption_after_work,
        Some(Interruption::Cancelled)
    );
    assert!(report.destruction_panic.is_none());
    assert!(report.cleanup.as_ref().is_ok_and(CleanupReport::is_success));
    assert!(!report.is_success());
    assert!(
        parent.context().check().is_ok(),
        "a signal reached the parent"
    );
    assert_removed_once(&sender, &log);
}

/// Work that spends the task's cooperative budget in its final poll, makes a
/// delivery visible and then returns, so a budget-gated check would miss it.
struct BudgetSpentOnReturn {
    filler: mpsc::UnboundedReceiver<()>,
    drained: Arc<AtomicUsize>,
    delivery: mpsc::UnboundedSender<()>,
}

impl Future for BudgetSpentOnReturn {
    type Output = Result<u32, Infallible>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // The filler holds more items than one budget, so Pending here means
        // the budget is spent, not that the channel is empty.
        while self.filler.poll_recv(cx).is_ready() {
            self.drained.fetch_add(1, Ordering::SeqCst);
        }
        self.delivery
            .send(())
            .expect("listeners are installed while work runs");
        Poll::Ready(Ok(7))
    }
}

#[tokio::test]
async fn delivery_visible_after_work_spends_its_budget_is_still_a_late_cancellation() {
    let (source, sender, log) = injected();
    let delivery = sender.clone();
    let (fill, filler) = mpsc::unbounded_channel();
    for _ in 0..1024 {
        fill.send(()).unwrap();
    }
    let drained = Arc::new(AtomicUsize::new(0));
    let counting = drained.clone();
    let parent = parent();
    let command = Command::new(parent.context().clone(), budget(), move |_| {
        Box::pin(BudgetSpentOnReturn {
            filler,
            drained: counting,
            delivery,
        })
    });
    let report = failed_report(start_injected(command, source).finish().await);
    assert!(
        drained.load(Ordering::SeqCst) < 1024,
        "work drained the filler instead of spending its budget"
    );
    drop(fill);
    assert_eq!(report.work.as_ref().ok(), Some(&7));
    assert_eq!(
        report.interruption_after_work,
        Some(Interruption::Cancelled)
    );
    assert!(!report.is_success());
    assert_removed_once(&sender, &log);
}

/// Reports, while the work future is destroyed, what a descendant observes.
struct DestructionWitness {
    descendant: OperationContext,
    seen: Option<oneshot::Sender<Result<(), Interruption>>>,
}

impl Drop for DestructionWitness {
    fn drop(&mut self) {
        if let Some(seen) = self.seen.take() {
            // The receiver outlives the command; a lost report fails the test.
            let _ = seen.send(self.descendant.check());
        }
    }
}

/// The coordinator cancels the command's context after work on every path, so
/// a cleanup hook cannot tell whether the signal itself cancelled. Observing
/// during work-future destruction can: only the signal branch has run then.
#[tokio::test]
async fn delivery_itself_cancels_descendants_before_work_is_destroyed() {
    let (source, sender, log) = injected();
    let parent = parent();
    let (started, running) = oneshot::channel();
    let (seen, destruction) = oneshot::channel();
    let command = Command::new(parent.context().clone(), budget(), move |scope| {
        Box::pin(async move {
            let descendant = scope.context().child(Duration::from_secs(30))?;
            let _witness = DestructionWitness {
                descendant: descendant.into_context(),
                seen: Some(seen),
            };
            started.send(()).map_err(|()| "test stopped waiting")?;
            pending::<Result<(), BoxError>>().await
        })
    });
    let owner = start_injected(command, source);
    running.await.expect("work started");
    sender
        .send(())
        .expect("listeners are installed while work runs");
    let report = failed_report(owner.finish().await);
    assert!(is_cancelled(&report.work));
    assert_eq!(
        destruction.await.expect("work future was destroyed"),
        Err(Interruption::Cancelled),
        "the signal did not cancel the command's own context"
    );
    assert!(
        parent.context().check().is_ok(),
        "a signal reached the parent"
    );
    assert_removed_once(&sender, &log);
}

/// Installation precedes the deadline; the deadline passes before the
/// coordinator's first poll. A queued delivery is still a cancellation, and the
/// parent is expired rather than cancelled.
#[tokio::test(start_paused = true)]
async fn queued_delivery_is_a_cancellation_even_after_the_deadline_passed() {
    let (source, sender, log) = injected();
    let parent = OperationOwner::new(Duration::from_secs(1)).unwrap();
    let invoked = Arc::new(AtomicBool::new(false));
    let invoking = invoked.clone();
    let command = Command::new(parent.context().clone(), budget(), move |_| {
        invoking.store(true, Ordering::SeqCst);
        Box::pin(async { Ok::<_, Infallible>(()) })
    });
    let owner = start_injected(command, source);
    sender.send(()).unwrap();
    // Moves the paused clock past the deadline, then yields to the coordinator.
    tokio::time::advance(Duration::from_secs(2)).await;
    let report = failed_report(owner.finish().await);
    assert!(!invoked.load(Ordering::SeqCst));
    assert!(is_cancelled(&report.work));
    assert_eq!(report.interruption_after_work, None);
    assert_eq!(
        parent.context().check(),
        Err(Interruption::DeadlineExceeded)
    );
    assert_removed_once(&sender, &log);
}
