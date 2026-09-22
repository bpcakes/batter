//! Outcome-aware, consecutive-failure throttling, separate from request quotas.
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{Delay, PolicyFingerprint, PolicyId, QuotaPeriod, ScopeId, SubjectKey};

/// Validated exponential-backoff policy with exactly one in-flight attempt.
/// Every failure doubles the previous delay, up to the cap. Success resets
/// retry state; abandonment and expired leases count as failures. Audit history
/// is application-owned and is never removed by this policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptPolicy {
    id: PolicyId,
    scope: ScopeId,
    initial_delay: QuotaPeriod,
    maximum_delay: QuotaPeriod,
    quiet_period: QuotaPeriod,
    lease: QuotaPeriod,
    fingerprint: PolicyFingerprint,
}

/// An invalid relationship between validated policy durations.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AttemptPolicyError {
    /// The maximum delay was below the initial delay.
    #[error("maximum delay must be at least the initial delay")]
    MaximumBelowInitial,
    /// Quiet expiry could bypass an active failure delay.
    #[error("quiet period must be at least the maximum delay")]
    QuietBelowMaximum,
}

impl AttemptPolicy {
    /// Constructs a policy. All timing is whole milliseconds.
    ///
    /// # Errors
    /// Rejects a maximum below the initial delay or quiet expiry below the cap.
    pub fn new(
        id: PolicyId,
        scope: ScopeId,
        initial_delay: QuotaPeriod,
        maximum_delay: QuotaPeriod,
        quiet_period: QuotaPeriod,
        lease: QuotaPeriod,
    ) -> Result<Self, AttemptPolicyError> {
        if maximum_delay < initial_delay {
            return Err(AttemptPolicyError::MaximumBelowInitial);
        }
        if quiet_period < maximum_delay {
            return Err(AttemptPolicyError::QuietBelowMaximum);
        }
        let mut hash = Sha256::new();
        hash.update(b"runlimit/attempt-policy/v1\0");
        hash.update(id.as_str().as_bytes());
        hash.update([0]);
        hash.update(scope.as_str().as_bytes());
        hash.update([0]);
        for value in [initial_delay, maximum_delay, quiet_period, lease] {
            hash.update(value.millis().to_be_bytes());
        }
        Ok(Self {
            id,
            scope,
            initial_delay,
            maximum_delay,
            quiet_period,
            lease,
            fingerprint: PolicyFingerprint::from_digest(hash.finalize().into()),
        })
    }
    /// Policy namespace.
    pub const fn id(&self) -> &PolicyId {
        &self.id
    }
    /// Subject namespace.
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }
    /// First failure delay.
    pub const fn initial_delay(&self) -> QuotaPeriod {
        self.initial_delay
    }
    /// Upper bound on failure delay.
    pub const fn maximum_delay(&self) -> QuotaPeriod {
        self.maximum_delay
    }
    /// Inactivity after which prior failures expire.
    pub const fn quiet_period(&self) -> QuotaPeriod {
        self.quiet_period
    }
    /// Maximum duration of one admitted attempt.
    pub const fn lease(&self) -> QuotaPeriod {
        self.lease
    }
    /// Stable storage protocol fingerprint, including every timing parameter.
    pub const fn fingerprint(&self) -> PolicyFingerprint {
        self.fingerprint
    }
    /// Delay after this many consecutive failures; zero means no delay.
    pub fn delay(&self, failures: u32) -> Duration {
        if failures == 0 {
            return Duration::ZERO;
        }
        Duration::from_millis(
            self.initial_delay
                .millis()
                .saturating_mul(1_u64.checked_shl(failures - 1).unwrap_or(u64::MAX))
                .min(self.maximum_delay.millis()),
        )
    }
}

/// A subject bound to the precise attempt policy used during keyed derivation.
#[derive(Clone, Copy, Debug)]
pub struct AttemptSubject<'a> {
    pub(crate) policy: &'a AttemptPolicy,
    pub(crate) subject: SubjectKey,
}

impl AttemptSubject<'_> {
    /// Exact policy reference retained by derivation.
    pub const fn policy(&self) -> &AttemptPolicy {
        self.policy
    }
    /// Opaque digest for backend storage. This explicitly leaves policy binding.
    pub const fn into_unbound_subject_key(self) -> SubjectKey {
        self.subject
    }
}

/// Admission either owns a single-use receipt or explains enforced denial.
#[derive(Debug)]
pub enum AttemptAdmission<R> {
    /// One attempt has been reserved until its lease expires.
    Admitted(R),
    /// No new attempt was reserved.
    Denied(AttemptDenial),
}

/// Attempt-specific enforced denial; ordinary quota decisions are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptDenial {
    /// Another attempt is active.
    Busy {
        /// Time until its lease expires.
        retry_after: Delay,
    },
    /// Consecutive failures require waiting.
    Backoff {
        /// Remaining delay.
        retry_after: Delay,
    },
    /// Backend cannot allocate a new subject without exceeding its hard bound.
    StorageCapacity,
}

/// Application outcome for an admitted attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    /// Full authentication succeeded; reset consecutive failure state.
    Success,
    /// Authentication failed; advance consecutive failure state.
    Failure,
    /// Explicit cancellation after admission; conservatively counts as failure.
    Abandoned,
}

/// Confirmed backend state transition, or provisional inside a host transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptCompletion {
    outcome: AttemptOutcome,
    consecutive_failures: u32,
    retry_after: Delay,
}
/// Completion metadata contradicted its application outcome.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("successful completion requires zero failures and delay; failure requires both nonzero")]
pub struct AttemptCompletionError;
impl AttemptCompletion {
    /// Constructs internally consistent completion metadata.
    ///
    /// # Errors
    /// Rejects success with failure state or failure without a nonzero delay/count.
    pub fn new(
        outcome: AttemptOutcome,
        consecutive_failures: u32,
        retry_after: Delay,
    ) -> Result<Self, AttemptCompletionError> {
        let valid = match outcome {
            AttemptOutcome::Success => {
                consecutive_failures == 0 && retry_after.duration().is_zero()
            }
            AttemptOutcome::Failure | AttemptOutcome::Abandoned => {
                consecutive_failures > 0 && !retry_after.duration().is_zero()
            }
        };
        if !valid {
            return Err(AttemptCompletionError);
        }
        Ok(Self {
            outcome,
            consecutive_failures,
            retry_after,
        })
    }
    /// Applied application outcome.
    pub const fn outcome(self) -> AttemptOutcome {
        self.outcome
    }
    /// Consecutive failures after the transition.
    pub const fn consecutive_failures(self) -> u32 {
        self.consecutive_failures
    }
    /// Delay before another attempt.
    pub const fn retry_after(self) -> Delay {
        self.retry_after
    }
}

/// Acknowledged result from a standalone completion owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptCompletionResult {
    /// State transition committed (or applied atomically in memory).
    Applied(AttemptCompletion),
    /// No matching live reservation; no completion was applied.
    Stale,
}

/// Completion may only apply to the still-live reservation that created it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagedAttemptCompletion {
    /// Transition staged; the caller must still commit before publishing it.
    Applied(AttemptCompletion),
    /// Reservation expired, was completed, or belongs to another store.
    Stale,
}

/// Attempt observations deliberately use a separate enum from quota events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptObservation {
    /// One reservation committed.
    Admitted,
    /// Admission denied without allocating a reservation.
    Denied(AttemptDenial),
    /// Completion acknowledged by the transaction owner.
    Completed(AttemptCompletion),
    /// A stale completion was rejected.
    Stale,
    /// Commit acknowledgement was lost; no automatic replay is safe.
    CommitUncertain,
}

/// Observer must not contain raw subjects. Panics are isolated by backends.
pub trait AttemptObserver: Send + Sync {
    /// Receives an attempt lifecycle event.
    fn observe(&self, observation: AttemptObservation);
}

/// Backend helper that isolates observer panics from decisions.
pub fn observe_attempt_safely(observer: &dyn AttemptObserver, observation: AttemptObservation) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        observer.observe(observation);
    }));
}
