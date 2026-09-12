use super::{ManagedRecord, ShutdownCause, TaskOutcome, TaskRecord};
use crate::cleanup::CleanupReport;

/// Complete process report, including teardown failures and unreaped work.
/// Awaiting the driver does not by itself establish successful shutdown.
/// Owned drivers return a [`super::SharedShutdownReport`]; binding or explicit dropping bypasses either lint.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::lifecycle::Supervisor;
/// async fn ignored(supervisor: Supervisor) {
///     supervisor.run_until(async {}).await;
/// }
/// ```
#[derive(Debug)]
#[must_use = "inspect the report for failures and incomplete cleanup"]
pub struct ShutdownReport {
    /// Trigger selected by the coordinator; inspect task/cleanup outcomes for all failures.
    pub cause: ShutdownCause,
    /// Directly joined tasks, in observation order.
    pub tasks: Vec<TaskRecord>,
    /// Native initialization and descendant settlement retained independently of
    /// direct wrapper joins. Pending settlement prevents dependency cleanup.
    pub managed: Vec<ManagedRecord>,
    /// Successful finite tasks are counted instead of retained individually.
    pub completed_process_tasks: u64,
    /// Whether directly registered tasks remained after the drain phase.
    pub forced_cancellation: bool,
    /// Tasks for which abort was requested, sorted for stable reporting.
    pub abort_requested: Vec<&'static str>,
    /// Direct tasks whose completion could not be observed, sorted by name.
    pub unjoined: Vec<&'static str>,
    /// Finalizer outcomes, including explicitly skipped hooks.
    pub cleanup: CleanupReport,
}

impl ShutdownReport {
    /// Success excludes early exits, returned errors, panics, aborts,
    /// unobserved termination, and incomplete cleanup.
    pub fn is_success(&self) -> bool {
        self.cause != ShutdownCause::EmptySupervisor
            && self
                .tasks
                .iter()
                .all(|task| task.outcome == TaskOutcome::Stopped)
            && self.abort_requested.is_empty()
            && self
                .managed
                .iter()
                .all(|record| record.outcome.is_success())
            && self.unjoined.is_empty()
            && self.cleanup.is_success()
    }

    /// Only direct tasks. This does NOT prove detached descendants terminated.
    pub fn all_direct_tasks_joined(&self) -> bool {
        self.unjoined.is_empty()
    }
}

impl std::fmt::Display for ShutdownReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let failures = self
            .tasks
            .iter()
            .filter(|t| t.outcome != TaskOutcome::Stopped)
            .count();
        write!(
            f,
            "shutdown {:?}: {failures} task failure(s), {} unjoined, {} unsuccessful managed component(s), {} pending native settlement(s); {}",
            self.cause,
            self.unjoined.len(),
            self.managed
                .iter()
                .filter(|record| !record.outcome.is_success())
                .count(),
            self.managed
                .iter()
                .filter(|record| !record.outcome.finished)
                .count(),
            self.cleanup
        )
    }
}
impl std::error::Error for ShutdownReport {}
