//! Batter operation phases derived from one native handler invocation.

use batter_core::{
    ConfigurationError,
    operation::{OperationOwner, OperationPhases, RootDeadline},
};
use runledger_core::jobs::JobExecution;
use std::{fmt, time::Duration};

/// Derive work and finalization contexts from one native handler invocation.
///
/// Call this once inside a `JobExecutionHandler` and run application work under
/// the returned [`OperationPhases`]. Both phases come from one root whose
/// deadline is exactly [`JobExecution::deadline`], the instant the Runledger
/// worker enforces, converted to Tokio's view of the same monotonic clock. It
/// is never rebuilt from the remaining budget, so deriving late does not move
/// it.
///
/// - `work()` ends at that deadline minus `reserve`, computed with checked
///   arithmetic by [`batter_core::operation::OperationContext::reserve_finalization`].
///   Use it for admission, provider calls, retries and child operations.
/// - `finalization()` keeps the invocation deadline. It is a sibling of work:
///   cancelling work with [`OperationPhases::cancel_work`] or exhausting its
///   deadline leaves finalization running, so a final-state write can use the
///   reserve after work expires.
/// - When the invocation exits for any reason, including runtime timeout,
///   lease loss and task abort, Runledger cancels the root, and with it both
///   phases and every context derived from them. The link is the native
///   invocation's own exit hook: no guard, forwarding task or paired call is
///   needed, and dropping the phases changes nothing. A graceful stop that
///   lets the invocation finish does not cancel it.
///
/// The worker destroys the handler future before it ends the invocation, so
/// work awaited inside the handler is dropped rather than seeing that
/// cancellation, and nothing the handler computes afterwards is stored. The
/// exit reaches work outside the handler future: give a spawned task a clone
/// of a phase, or a child derived from it. A `run` boundary's own scope is
/// also cancelled as soon as that boundary ends.
///
/// Await final-state work after work ends. Run it under `finalization()` when
/// it uses Batter boundaries such as admission, retries, child operations or
/// spawned tasks; a plain write awaited in the handler is bounded by the native
/// deadline either way, as in the reference delivery worker.
///
/// `Duration::ZERO` selects no reserve: both phases end at the invocation
/// deadline, through [`batter_core::operation::OperationContext::split_finalization`].
/// The worker accepts a handler result only when it is observed strictly before
/// that deadline, so with no reserve, a result returned or a final-state write
/// attempted after work expiry is classified `job.timeout_exceeded`. Nothing
/// here changes that precedence. A positive reserve is application policy:
/// it must cover the final-state work the handler still does after work
/// expires. Runledger persists the handler's outcome after it returns, outside
/// this budget.
///
/// Derivation and each later operation preflight reject expired or abandoned
/// work before invoking any application factory. Each successful call keeps
/// one cancellation link until the invocation ends. Cancellation is a
/// notification: it does not join detached tasks, shield cleanup, undo remote
/// effects or choose the job's failure classification, which stays an explicit
/// application mapping to `JobFailure`.
///
/// Facade consumers reach the same items as `batter::runledger::job_phases`
/// and `batter::runledger::native::core::jobs`; the facade's `runledger`
/// module shows the handler with those paths.
///
/// ```
/// use batter_core::operation::OperationError;
/// use batter_runledger::{JobPhasesRejection, job_phases};
/// use runledger_core::jobs::{
///     JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType,
/// };
/// use runledger_core::prelude::async_trait;
/// use serde_json::Value;
/// use std::time::Duration;
///
/// /// Time kept inside the native deadline for the final-state write.
/// const FINAL_STATE_RESERVE: Duration = Duration::from_millis(500);
///
/// struct Deliver;
///
/// #[async_trait]
/// impl JobExecutionHandler for Deliver {
///     fn job_type(&self) -> JobType<'static> {
///         JobType::new("example.deliver")
///     }
///
///     async fn execute(
///         &self,
///         execution: JobExecution<'_>,
///         _payload: Value,
///     ) -> Result<JobCompletion, JobFailure> {
///         let phases = job_phases(execution, FINAL_STATE_RESERVE).map_err(|rejection| {
///             match rejection {
///                 JobPhasesRejection::Exhausted | JobPhasesRejection::Ended => {
///                     JobFailure::timeout("example.no_work_time", "No work time remained.")
///                 }
///                 JobPhasesRejection::Unsupported | JobPhasesRejection::Reserve(_) => {
///                     JobFailure::terminal("example.unsupported", "Invocation phases unavailable.")
///                 }
///             }
///         })?;
///         let delivered = phases
///             .work()
///             .run("example.provider", |_scope| async {
///                 Ok::<_, std::io::Error>("receipt")
///             })
///             .await;
///         // Classify before recording: an interrupted call leaves delivery unknown.
///         let outcome = match delivered {
///             Ok(_receipt) => Ok(JobCompletion::success()),
///             Err(OperationError::Failed(_)) => Err(JobFailure::retryable(
///                 "example.refused",
///                 "The provider refused delivery.",
///             )),
///             Err(OperationError::Interrupted(_)) => Err(JobFailure::timeout(
///                 "example.unknown",
///                 "Delivery is unknown.",
///             )),
///         };
///         // The final-state write runs even when work was interrupted.
///         phases
///             .finalization()
///             .run("example.record", |_scope| async {
///                 Ok::<_, std::io::Error>(())
///             })
///             .await
///             .map_err(|_| JobFailure::retryable("example.unrecorded", "Outcome not recorded."))?;
///         outcome
///     }
/// }
/// # let _ = Deliver.into_job_handler();
/// ```
///
/// The phases carry no authority over the invocation or their shared root:
///
/// ```compile_fail,E0599
/// fn cannot_cancel_both(phases: batter_core::operation::OperationPhases) {
///     phases.cancel();
/// }
/// ```
///
/// ```compile_fail,E0624
/// fn cannot_cancel_finalization(phases: batter_core::operation::OperationPhases) {
///     phases.finalization().cancel();
/// }
/// ```
pub fn job_phases(
    execution: JobExecution<'_>,
    reserve: Duration,
) -> Result<OperationPhases, JobPhasesRejection> {
    let invocation = execution
        .invocation()
        .ok_or(JobPhasesRejection::Unsupported)?;
    if invocation.has_ended() {
        return Err(JobPhasesRejection::Ended);
    }
    let deadline = tokio::time::Instant::from_std(execution.deadline());
    let root = OperationOwner::at(RootDeadline::at(deadline));
    let phases = if reserve.is_zero() {
        root.context().split_finalization()
    } else {
        root.context().reserve_finalization(reserve)
    }
    .map_err(|error| match error {
        ConfigurationError::InvalidReserve => JobPhasesRejection::Exhausted,
        other => JobPhasesRejection::Reserve(other),
    })?;
    // Native ownership keeps the root's only cancellation authority. An end
    // racing this registration runs the hook at once and is reported below.
    invocation.on_end(move || root.cancel());
    if invocation.has_ended() {
        return Err(JobPhasesRejection::Ended);
    }
    Ok(phases)
}

/// Why [`job_phases`] returned no phases. No application work has run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobPhasesRejection {
    /// The execution services make no exit claim: their
    /// `JobExecutionServices::invocation` returned `None`, as custom services
    /// written before it existed do. Linked cancellation could never fire, so
    /// the bridge refuses them; the services must own a `JobInvocationOwner`.
    Unsupported,
    /// The native invocation had already ended.
    Ended,
    /// No positive work time remains before the invocation deadline minus the
    /// reserve, including when that subtraction leaves the clock's range.
    Exhausted,
    /// The reserve is not an accepted operational duration (more than one year
    /// or unrepresentable on the runtime clock).
    Reserve(ConfigurationError),
}

impl fmt::Display for JobPhasesRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "job execution services do not own the invocation's exit",
            Self::Ended => "job invocation already ended",
            Self::Exhausted => "no work time remains before the final-state reserve",
            Self::Reserve(_) => "final-state reserve is invalid",
        })
    }
}

impl std::error::Error for JobPhasesRejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Reserve(error) => Some(error),
            Self::Unsupported | Self::Ended | Self::Exhausted => None,
        }
    }
}
