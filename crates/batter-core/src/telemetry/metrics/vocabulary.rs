//! Closed label vocabularies, each defined once.
//!
//! Every domain is one exhaustive match whose published value list is
//! generated from the same arms, so a new enum variant cannot compile until it
//! has a label, and that label is then part of the documented domain.

use super::CleanupHook;
use crate::{
    admission::AdmissionError,
    cleanup::CleanupOutcome,
    lifecycle::{ProcessAdmissionError, Readiness, ShutdownCause, TaskOutcome},
    operation::Interruption,
    retry::{RetryExecutionError, StopReason},
    telemetry::Outcome,
};

macro_rules! closed_domain {
    (
        $(#[$doc:meta])*
        $values:ident, fn $label:ident $(<$generic:ident>)? ($value:ident: $input:ty) {
            $($pattern:pat => $text:literal,)+
        }
    ) => {
        $(#[$doc])*
        pub const $values: &[&str] = &[$($text),+];

        pub(super) fn $label $(<$generic>)? ($value: $input) -> &'static str {
            match $value {
                $($pattern => $text,)+
            }
        }
    };
}

closed_domain! {
    /// Values of the `outcome` label on operation and attempt metrics.
    OUTCOMES, fn outcome(value: Outcome) {
        Outcome::Succeeded => "succeeded",
        Outcome::Failed => "failed",
        Outcome::Cancelled => "cancelled",
        Outcome::DeadlineExceeded => "deadline_exceeded",
        Outcome::Dropped => "dropped",
    }
}

closed_domain! {
    /// Values of the `result` label on [`super::RETRY_EXECUTIONS`].
    RETRY_RESULTS, fn retry<E>(value: Option<Result<(), &RetryExecutionError<E>>>) {
        Some(Ok(())) => "succeeded",
        Some(Err(RetryExecutionError::Stopped { reason: StopReason::NotRetryable, .. })) => "not_retryable",
        Some(Err(RetryExecutionError::Stopped { reason: StopReason::ReplayForbidden, .. })) => "replay_forbidden",
        Some(Err(RetryExecutionError::Stopped { reason: StopReason::AttemptsExhausted, .. })) => "attempts_exhausted",
        Some(Err(RetryExecutionError::Stopped { reason: StopReason::InsufficientBudget, .. })) => "insufficient_budget",
        Some(Err(RetryExecutionError::Interrupted { reason: Interruption::Cancelled, .. })) => "cancelled",
        Some(Err(RetryExecutionError::Interrupted { reason: Interruption::DeadlineExceeded, .. })) => "deadline_exceeded",
        Some(Err(RetryExecutionError::AttemptDeadlineExceeded { .. })) => "attempt_deadline_exceeded",
        None => "dropped",
    }
}

closed_domain! {
    /// `decision` values for `admission="bulkhead"`. `dropped` is a polled
    /// `enter` future destroyed before its decision.
    BULKHEAD_DECISIONS, fn bulkhead(value: Option<Result<(), &AdmissionError>>) {
        Some(Ok(())) => "admitted",
        Some(Err(AdmissionError::Overloaded)) => "overloaded",
        Some(Err(AdmissionError::Closed)) => "closed",
        Some(Err(AdmissionError::Interrupted(Interruption::Cancelled))) => "cancelled",
        Some(Err(AdmissionError::Interrupted(Interruption::DeadlineExceeded))) => "deadline_exceeded",
        None => "dropped",
    }
}

closed_domain! {
    /// `decision` values for `admission="process"`.
    PROCESS_DECISIONS, fn process(value: Result<(), &ProcessAdmissionError>) {
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
    ROOT_DECISIONS, fn root(value: Readiness) {
        Readiness::Ready => "admitted",
        Readiness::Starting => "starting",
        Readiness::Draining => "draining",
        Readiness::Stopped => "stopped",
    }
}

closed_domain! {
    /// Values of the `kind` label on [`super::TASK_EXITS`].
    TASK_KINDS, fn task_kind(finite: bool) {
        false => "component",
        true => "process",
    }
}

closed_domain! {
    /// Values of the `outcome` label on [`super::TASK_EXITS`].
    TASK_OUTCOMES, fn task_outcome(value: TaskOutcome) {
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
    /// counts hooks abandoned by dropping an unclosed stack or an in-flight
    /// close driver.
    CLEANUP_OUTCOMES, fn cleanup(value: CleanupHook) {
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
    /// Values of the `cause` label on [`super::SHUTDOWNS`].
    SHUTDOWN_CAUSES, fn shutdown_cause(value: ShutdownCause) {
        ShutdownCause::Requested => "requested",
        ShutdownCause::ComponentExit(_) => "component_exit",
        ShutdownCause::FiniteTaskExit(_) => "finite_task_exit",
        ShutdownCause::EmptySupervisor => "empty_supervisor",
    }
}

closed_domain! {
    /// Values of the `result` label on shutdown metrics.
    SHUTDOWN_RESULTS, fn shutdown_result(success: bool) {
        true => "success",
        false => "failure",
    }
}
