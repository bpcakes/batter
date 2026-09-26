//! Closed label vocabularies, each defined once with `closed_domain!`.

use super::{AdmissionKind, CleanupHook, Coalesce, End, NameDomain};
use crate::{
    admission::AdmissionError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, Readiness, ShutdownCause, TaskOutcome},
    operation::Interruption,
    retry::{RetryExecutionError, StopReason},
    telemetry::{OUTCOME_LABELS, Outcome, outcome_label},
};

/// Metric-only outcome for an operation destroyed while its thread unwinds.
const PANICKED: &str = "panicked";

const OUTCOME_VALUES: usize = OUTCOME_LABELS.len() + 1;

/// Values of the `outcome` label on operation and attempt metrics: the
/// tracing outcome spellings plus `panicked`, which distinguishes a boundary
/// destroyed during unwinding from one abandoned by its caller.
pub const OUTCOMES: [&str; OUTCOME_VALUES] = {
    let mut values = [PANICKED; OUTCOME_VALUES];
    let mut index = 0;
    while index < OUTCOME_LABELS.len() {
        values[index] = OUTCOME_LABELS[index];
        index += 1;
    }
    values
};

pub(super) const fn outcome(value: Outcome, unwinding: bool) -> &'static str {
    match (value, unwinding) {
        (Outcome::Dropped, true) => PANICKED,
        (value, _) => outcome_label(value),
    }
}

closed_domain! {
    /// Values of the `result` label on [`super::RETRY_EXECUTIONS`].
    pub RETRY_RESULTS, fn retry<E>(value: End<Result<(), &RetryExecutionError<E>>>) {
        End::Finished(Ok(())) => "succeeded",
        End::Finished(Err(RetryExecutionError::Stopped { reason: StopReason::NotRetryable, .. })) => "not_retryable",
        End::Finished(Err(RetryExecutionError::Stopped { reason: StopReason::ReplayForbidden, .. })) => "replay_forbidden",
        End::Finished(Err(RetryExecutionError::Stopped { reason: StopReason::AttemptsExhausted, .. })) => "attempts_exhausted",
        End::Finished(Err(RetryExecutionError::Stopped { reason: StopReason::InsufficientBudget, .. })) => "insufficient_budget",
        End::Finished(Err(RetryExecutionError::Interrupted { reason: Interruption::Cancelled, .. })) => "cancelled",
        End::Finished(Err(RetryExecutionError::Interrupted { reason: Interruption::DeadlineExceeded, .. })) => "deadline_exceeded",
        End::Finished(Err(RetryExecutionError::AttemptDeadlineExceeded { .. })) => "attempt_deadline_exceeded",
        End::Dropped => "dropped",
        End::Panicked => "panicked",
    }
}

closed_domain! {
    /// Values of the `admission` label on [`super::ADMISSION_DECISIONS`].
    pub ADMISSIONS, fn admission_kind(value: AdmissionKind) {
        AdmissionKind::Bulkhead => "bulkhead",
        AdmissionKind::Process => "process",
        AdmissionKind::Root => "root",
    }
}

closed_domain! {
    /// `decision` values for `admission="bulkhead"`. `dropped` and `panicked`
    /// are polled `enter` futures destroyed before their decision.
    pub BULKHEAD_DECISIONS, fn bulkhead(value: End<Result<(), &AdmissionError>>) {
        End::Finished(Ok(())) => "admitted",
        End::Finished(Err(AdmissionError::Overloaded)) => "overloaded",
        End::Finished(Err(AdmissionError::Closed)) => "closed",
        End::Finished(Err(AdmissionError::Interrupted(Interruption::Cancelled))) => "cancelled",
        End::Finished(Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded))) => "deadline_exceeded",
        End::Dropped => "dropped",
        End::Panicked => "panicked",
    }
}

closed_domain! {
    /// `decision` values for `admission="process"`.
    pub PROCESS_DECISIONS, fn process(value: Result<(), &ProcessAdmissionError>) {
        Ok(()) => "admitted",
        Err(ProcessAdmissionError::NotRunning) => "not_running",
        Err(ProcessAdmissionError::NotReady) => "not_ready",
        Err(ProcessAdmissionError::Closed) => "closed",
        Err(ProcessAdmissionError::Full) => "full",
        Err(ProcessAdmissionError::InvalidName(_)) => "invalid_name",
    }
}

closed_domain! {
    /// `decision` values for `admission="root"`, the lifecycle gate of
    /// [`crate::lifecycle::OperationAdmission::admit_root`]: the observed
    /// readiness, which admits only when ready.
    pub ROOT_DECISIONS, fn root(value: Readiness) {
        Readiness::Ready => "admitted",
        Readiness::Starting => "starting",
        Readiness::Draining => "draining",
        Readiness::Stopped => "stopped",
    }
}

closed_domain! {
    /// Values of the `kind` label on [`super::TASK_EXITS`].
    pub TASK_KINDS, fn task_kind(finite: bool) {
        false => "component",
        true => "process",
    }
}

closed_domain! {
    /// Values of the `outcome` label on [`super::TASK_EXITS`].
    pub TASK_OUTCOMES, fn task_outcome(value: TaskOutcome) {
        TaskOutcome::Completed => "completed",
        TaskOutcome::Stopped => "stopped",
        TaskOutcome::UnexpectedExit => "unexpected_exit",
        TaskOutcome::Failed => "failed",
        TaskOutcome::Panicked => "panicked",
        TaskOutcome::Aborted => "aborted",
    }
}

closed_domain! {
    /// Values of the `outcome` label on [`super::CLEANUP_HOOKS`]. `dropped`
    /// counts registered hooks that never ran because their stack or in-flight
    /// close driver was destroyed.
    pub CLEANUP_OUTCOMES, fn cleanup(value: CleanupHook) {
        CleanupHook::Observed(CleanupOutcome::Succeeded) => "succeeded",
        CleanupHook::Observed(CleanupOutcome::Failed) => "failed",
        CleanupHook::Observed(CleanupOutcome::Panicked) => "panicked",
        CleanupHook::Observed(CleanupOutcome::Cancelled) => "cancelled",
        CleanupHook::Observed(CleanupOutcome::TimedOut) => "timed_out",
        CleanupHook::Observed(CleanupOutcome::Unjoined) => "unjoined",
        CleanupHook::Skipped => "skipped",
        CleanupHook::Dropped => "dropped",
    }
}

closed_domain! {
    /// Values of the `cause` label on [`super::SHUTDOWNS`]. `none` is a driver
    /// destroyed before it selected a shutdown cause.
    pub SHUTDOWN_CAUSES, fn shutdown_cause(value: Option<ShutdownCause>) {
        Some(ShutdownCause::Requested) => "requested",
        Some(ShutdownCause::ComponentExit(_)) => "component_exit",
        Some(ShutdownCause::FiniteTaskExit(_)) => "finite_task_exit",
        Some(ShutdownCause::EmptySupervisor) => "empty_supervisor",
        None => "none",
    }
}

closed_domain! {
    /// Values of the `result` label on shutdown metrics: a completed report's
    /// success, or a driver destroyed (`dropped`) or unwound (`panicked`)
    /// before its report.
    pub SHUTDOWN_RESULTS, fn shutdown_result(value: End<bool>) {
        End::Finished(true) => "success",
        End::Finished(false) => "failure",
        End::Dropped => "dropped",
        End::Panicked => "panicked",
    }
}

closed_domain! {
    /// Values of the `domain` label on [`super::LABELS_COALESCED`].
    pub COALESCE_DOMAINS, fn name_domain(value: NameDomain) {
        NameDomain::Operation => "operation",
        NameDomain::Task => "task",
    }
}

closed_domain! {
    /// Values of the `reason` label on [`super::LABELS_COALESCED`].
    pub COALESCE_REASONS, fn coalesce_reason(value: Coalesce) {
        Coalesce::Invalid => "invalid",
        Coalesce::Capacity => "capacity",
    }
}
