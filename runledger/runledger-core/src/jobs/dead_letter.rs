use serde::Serialize;

use super::JobFailure;

/// Why a job stopped retrying and was dead-lettered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobDeadLetterReason {
    FailureKindNonRetryable,
    AttemptsExhausted,
    LeaseExpired,
}

/// Runtime component that dead-lettered the job and is invoking the hook.
///
/// Every dead-letter hook receives the durable attempt's worker identity in
/// [`JobContext::worker_id`](crate::jobs::JobContext::worker_id), whichever
/// component delivers it. The origin is the typed way to tell the two
/// deliveries apart; it is not encoded in the worker identity and is not
/// implied by [`JobDeadLetterReason`].
///
/// ```
/// use runledger_core::jobs::{
///     JobDeadLetterInfo, JobDeadLetterOrigin, JobDeadLetterReason, JobFailure,
/// };
///
/// let info = JobDeadLetterInfo::new(
///     JobFailure::lease_expired("job.lease_expired", "lease expired before completion"),
///     JobDeadLetterReason::LeaseExpired,
///     Some(3),
///     JobDeadLetterOrigin::Reaper,
/// );
///
/// let delivered_by = match info.origin {
///     JobDeadLetterOrigin::Worker => "the worker that executed the attempt",
///     JobDeadLetterOrigin::Reaper => "the lease reaper",
/// };
/// assert_eq!(delivered_by, "the lease reaper");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobDeadLetterOrigin {
    /// The worker that executed the attempt observed the terminal failure and
    /// invoked the hook from its own job task.
    Worker,
    /// The lease reaper expired the attempt's lease after the retry budget was
    /// spent and invoked the hook from the reaper loop.
    Reaper,
}

/// A terminal job failure and the component delivering its dead-letter hook.
///
/// Migration: `JobDeadLetterInfo` struct literals now require `origin`;
/// exhaustive patterns must bind it or use `..`. Choose the actual hook-delivery
/// component, not a value inferred from the failure reason or worker identity.
/// ```
/// use runledger_core::jobs::{
///     JobDeadLetterInfo, JobDeadLetterOrigin, JobDeadLetterReason, JobFailure,
/// };
/// let info = JobDeadLetterInfo {
///     failure: JobFailure::lease_expired("job.lease_expired", "lease expired"),
///     reason: JobDeadLetterReason::LeaseExpired,
///     max_attempts: Some(3),
///     origin: JobDeadLetterOrigin::Reaper,
/// };
/// let JobDeadLetterInfo { failure, reason, max_attempts, origin } = info;
/// assert_eq!(origin, JobDeadLetterOrigin::Reaper);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobDeadLetterInfo {
    pub failure: JobFailure,
    pub reason: JobDeadLetterReason,
    pub max_attempts: Option<i32>,
    /// Component that dead-lettered the job and is invoking the hook.
    pub origin: JobDeadLetterOrigin,
}

impl JobDeadLetterInfo {
    /// Construct hook information with an explicit delivery origin.
    ///
    /// Migration: `JobDeadLetterInfo::new(failure, reason, max_attempts)` now
    /// requires a fourth argument, the actual [`JobDeadLetterOrigin`]. There is
    /// no default origin valid for both worker and reaper delivery. See the
    /// [`JobDeadLetterOrigin`] example for the complete call.
    #[must_use]
    pub fn new(
        failure: JobFailure,
        reason: JobDeadLetterReason,
        max_attempts: Option<i32>,
        origin: JobDeadLetterOrigin,
    ) -> Self {
        Self {
            failure,
            reason,
            max_attempts,
            origin,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dead_letter_info_serializes_its_origin_beside_the_reason() {
        let serialized = serde_json::to_value(JobDeadLetterInfo::new(
            JobFailure::lease_expired("job.lease_expired", "lease expired"),
            JobDeadLetterReason::LeaseExpired,
            Some(2),
            JobDeadLetterOrigin::Reaper,
        ))
        .expect("serialize dead-letter info");

        assert_eq!(
            serialized,
            json!({
                "failure": {
                    "kind": "LEASE_EXPIRED",
                    "code": "job.lease_expired",
                    "message": "lease expired"
                },
                "reason": "LEASE_EXPIRED",
                "max_attempts": 2,
                "origin": "REAPER"
            })
        );
    }

    #[test]
    fn worker_origin_serializes_as_screaming_snake_case() {
        assert_eq!(
            serde_json::to_value(JobDeadLetterOrigin::Worker).expect("serialize origin"),
            json!("WORKER")
        );
    }
}
