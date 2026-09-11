use super::{TerminationGate, TerminationState};
use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupReport, CleanupStack, SkipReason},
    lifecycle::TaskOutcome,
    lifecycle::{DriverOutcome, ShutdownReport, Supervisor},
};
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

/// Separately driven owner for resources used by the native worker.
///
/// The outer Batter cleanup hook captures only a clone of this owner. A caller
/// must retain another clone and call [`Self::finish`] after the process report,
/// because Batter may conservatively skip the outer hook after aborting the
/// wrapper task. Only `NotStarted` or `CooperativelyStopped` permits finalizers;
/// `Unproven` records every nested hook as `UnsafeTaskExit` without invoking it.
pub struct DependencyCleanup {
    stack: Mutex<Option<CleanupStack>>,
    started: AtomicBool,
    process_unsafe: Arc<AtomicBool>,
    completed: watch::Sender<Option<Arc<CleanupReport>>>,
    budget: CleanupBudget,
    termination: TerminationGate,
}

impl DependencyCleanup {
    /// Own a completed dependency stack before registering its grouped outer hook.
    #[must_use]
    pub fn new(stack: CleanupStack, budget: CleanupBudget, termination: TerminationGate) -> Self {
        let (completed, _) = watch::channel(None);
        Self {
            stack: Mutex::new(Some(stack)),
            started: AtomicBool::new(false),
            process_unsafe: Arc::new(AtomicBool::new(false)),
            completed,
            budget,
            termination,
        }
    }

    /// Register the grouped hook after all nested resources are owned.
    ///
    /// Acquisition code should reserve the outer name before acquiring a
    /// resource when registration failure would otherwise abandon ownership.
    /// The production root uses that stricter reservation pattern directly.
    pub fn register(
        self: &Arc<Self>,
        supervisor: &mut Supervisor,
    ) -> Result<(), batter::RegistrationError> {
        let cleanup = self.clone();
        supervisor.on_cleanup("worker.dependencies", move || async move {
            let report = cleanup.finish().await;
            if report.is_success() {
                Ok(())
            } else {
                Err(Box::new(DependencyCleanupError(report)) as BoxError)
            }
        })
    }

    /// Drive or observe the single nested cleanup result.
    ///
    /// Concurrent callers receive the same retained report. The nested stack is
    /// run at most once and is skipped, rather than dropped silently, whenever
    /// transitive worker termination remains unproved. After a complete Batter
    /// process run, prefer [`Self::finish_after_process`] so another component's
    /// unsafe exit also prevents teardown of shared dependencies.
    pub async fn finish(&self) -> Arc<CleanupReport> {
        let mut completion = self.completed.subscribe();
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let stack = self
                .stack
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
                .expect("the first cleanup driver owns the nested stack");
            let termination = self.termination.clone();
            let process_unsafe = self.process_unsafe.clone();
            let budget = self.budget;
            let completed = self.completed.clone();
            // The driver outlives any one outer-hook or caller waiter. Runtime
            // destruction remains outside the guarantee, as for Batter drivers.
            drop(tokio::spawn(async move {
                termination.wait_for_driver_observation().await;
                let report = match (process_unsafe.load(Ordering::Acquire), termination.state()) {
                    (true, _) => stack.skip(SkipReason::UnsafeTaskExit),
                    (
                        false,
                        TerminationState::NotStarted | TerminationState::CooperativelyStopped,
                    ) => stack.close(budget).await,
                    (false, TerminationState::Unproven) => stack.skip(SkipReason::UnsafeTaskExit),
                };
                completed.send_replace(Some(Arc::new(report)));
            }));
        }
        loop {
            if let Some(report) = completion.borrow_and_update().clone() {
                return report;
            }
            completion
                .changed()
                .await
                .expect("dependency cleanup owner retains publication");
        }
    }

    /// Drive or observe cleanup after the complete Batter driver outcome.
    ///
    /// Batter skips its cleanup stack after a panic, requested abort, or unjoined
    /// direct task. The same evidence forces this separately retained nested
    /// stack to skip even when its own worker driver later stops cooperatively,
    /// because another component can still hold the shared dependency. A lost
    /// Batter coordinator is treated conservatively for the same reason.
    pub async fn finish_after_process(&self, process: &DriverOutcome) -> Arc<CleanupReport> {
        let unsafe_exit = match process {
            Ok(report) => report_has_unsafe_exit(report),
            Err(_) => true,
        };
        if unsafe_exit {
            self.process_unsafe.store(true, Ordering::Release);
        }
        self.finish().await
    }
}

fn report_has_unsafe_exit(report: &ShutdownReport) -> bool {
    !report.abort_requested.is_empty()
        || !report.unjoined.is_empty()
        || report
            .tasks
            .iter()
            .any(|task| matches!(task.outcome, TaskOutcome::Panicked | TaskOutcome::Aborted))
        || report
            .cleanup
            .records
            .iter()
            .any(|cleanup| cleanup.outcome == CleanupOutcome::Unjoined)
        || report.cleanup.skipped.iter().any(|cleanup| {
            matches!(
                cleanup.reason,
                SkipReason::UnsafeTaskExit | SkipReason::PreviousCleanupUnjoined
            )
        })
}

/// Retained nested dependency cleanup failure used by the outer cleanup report.
pub struct DependencyCleanupError(pub(crate) Arc<CleanupReport>);

impl DependencyCleanupError {
    /// Inspect the exact attempted and skipped nested finalizers.
    pub fn report(&self) -> &CleanupReport {
        &self.0
    }
}

impl fmt::Display for DependencyCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("worker dependency cleanup did not complete")
    }
}

impl fmt::Debug for DependencyCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for DependencyCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::{Semaphore, oneshot};

    fn budget() -> CleanupBudget {
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn unproven_termination_skips_dependency_and_retains_one_report() {
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let mut stack = CleanupStack::new();
        stack
            .push("database", move || async move {
                invoked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let gate = TerminationGate::new();
        gate.arm().unwrap();
        gate.mark_driver_observed();
        let owner = Arc::new(DependencyCleanup::new(stack, budget(), gate));
        let first = owner.finish().await;
        let second = owner.finish().await;
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(first.records.is_empty());
        assert_eq!(first.skipped.len(), 1);
        assert_eq!(first.skipped[0].reason, SkipReason::UnsafeTaskExit);
    }

    #[tokio::test]
    async fn not_started_cleanup_runs_dependency() {
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let mut stack = CleanupStack::new();
        stack
            .push("database", move || async move {
                invoked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let owner = DependencyCleanup::new(stack, budget(), TerminationGate::new());
        assert!(owner.finish().await.is_success());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_waiter_does_not_cancel_the_cleanup_driver() {
        let (entered_tx, entered_rx) = oneshot::channel();
        let release = Arc::new(Semaphore::new(0));
        let finalizer_release = release.clone();
        let mut stack = CleanupStack::new();
        stack
            .push("database", move || async move {
                let _ = entered_tx.send(());
                finalizer_release.acquire().await.unwrap().forget();
                Ok(())
            })
            .unwrap();
        let owner = Arc::new(DependencyCleanup::new(
            stack,
            budget(),
            TerminationGate::new(),
        ));
        let waiting_owner = owner.clone();
        let waiter = tokio::spawn(async move { waiting_owner.finish().await });
        entered_rx.await.unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.add_permits(1);
        assert!(owner.finish().await.is_success());
    }

    #[tokio::test]
    async fn unsafe_process_outcome_overrides_safe_worker_termination() {
        use batter::lifecycle::{ShutdownBudget, ShutdownSignal};
        use std::future::Ready;

        fn panic_factory(_: ShutdownSignal) -> Ready<Result<(), BoxError>> {
            panic!("controlled unsafe component exit")
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let mut stack = CleanupStack::new();
        stack
            .push("database", move || async move {
                invoked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let gate = TerminationGate::new();
        gate.arm().unwrap();
        gate.mark_cooperatively_stopped();
        let owner = DependencyCleanup::new(stack, budget(), gate);
        let second = Duration::from_secs(1);
        let mut supervisor =
            Supervisor::new(ShutdownBudget::new(second, second, second, budget()).unwrap());
        supervisor.register("unsafe", panic_factory).unwrap();
        let process = supervisor.start().wait().await;
        assert!(matches!(&process, Ok(report) if report_has_unsafe_exit(report)));

        let report = owner.finish_after_process(&process).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(report.records.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SkipReason::UnsafeTaskExit);
    }

    #[test]
    fn unjoined_process_cleanup_vetoes_later_dependency_cleanup() {
        use batter::{
            cleanup::{CleanupRecord, SkippedCleanup},
            lifecycle::ShutdownCause,
        };

        let report = ShutdownReport {
            cause: ShutdownCause::Requested,
            tasks: Vec::new(),
            completed_process_tasks: 0,
            forced_cancellation: false,
            abort_requested: Vec::new(),
            unjoined: Vec::new(),
            cleanup: CleanupReport {
                records: vec![CleanupRecord {
                    name: "unfinished",
                    outcome: CleanupOutcome::Unjoined,
                    error: None,
                }],
                skipped: vec![SkippedCleanup {
                    name: "dependent",
                    reason: SkipReason::PreviousCleanupUnjoined,
                }],
            },
        };

        assert!(report_has_unsafe_exit(&report));
    }
}
