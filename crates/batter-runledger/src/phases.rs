//! Batter operation phases derived from one native handler invocation.

use batter_core::{
    ConfigurationError,
    operation::{OperationOwner, OperationPhases, RootDeadline},
};
use runledger_core::jobs::JobExecution;
use std::{fmt, time::Duration};

// The adapter owns this derivation; applications receive one phase at a time.
pub(crate) fn job_phases(
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

/// Why the handler invocation could not supply phases. No application work has run.
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

#[cfg(test)]
mod tests;
