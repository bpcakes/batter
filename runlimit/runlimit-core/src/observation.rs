use std::{
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    time::Duration,
};

use crate::{
    BatchDecision, BatchDecisionView, Check, Decision, DecisionView, Denial, PolicyFingerprint,
    PolicyId, RateLimitPolicy, ScopeId,
};

/// Receives synchronous, backend-neutral operational observations.
///
/// Implementations must return quickly and should hand expensive work to
/// another thread. Backends invoke observers only after releasing internal
/// locks and finalizing database transactions. A panic from an observer is
/// caught and ignored so telemetry cannot change an admission result.
///
/// Observations deliberately contain no subject keys, backend error text, or
/// other high-cardinality sensitive values.
pub trait Observer: Send + Sync + 'static {
    /// Records one operational observation.
    fn observe(&self, observation: &Observation<'_>);
}

/// A backend-neutral operational observation.
///
/// This enum and the classification enums it carries are exhaustive: an
/// observer matches every variant, so a new observation kind or outcome is a
/// compile error in every consumer rather than a silently unmetered fallback
/// arm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Observation<'a> {
    /// One admission operation completed.
    Admission(AdmissionObservation<'a>),
    /// A bounded cleanup pass completed.
    Cleanup(CleanupObservation),
    /// A bounded store reported local capacity use.
    Capacity(CapacityObservation),
}

/// The identity of the policy an admission observation is about.
///
/// The identifier, scope, and storage fingerprint are always present together,
/// so an observer never sees one without the others.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AdmissionPolicy<'a> {
    id: &'a PolicyId,
    scope: &'a ScopeId,
    fingerprint: PolicyFingerprint,
}

impl fmt::Debug for AdmissionPolicy<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmissionPolicy")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl<'a> AdmissionPolicy<'a> {
    /// Captures the identity of `policy`.
    pub fn new<P: RateLimitPolicy + ?Sized>(policy: &'a P) -> Self {
        Self {
            id: policy.id(),
            scope: policy.scope(),
            fingerprint: policy.fingerprint(),
        }
    }

    fn from_check<P: RateLimitPolicy + ?Sized>(check: &Check<'a, P>) -> Self {
        Self::new(check.policy())
    }

    /// Returns the application-defined policy identifier.
    pub const fn id(self) -> &'a PolicyId {
        self.id
    }

    /// Returns the application-defined policy scope.
    pub const fn scope(self) -> &'a ScopeId {
        self.scope
    }

    /// Returns the policy's storage configuration fingerprint.
    pub const fn fingerprint(self) -> PolicyFingerprint {
        self.fingerprint
    }
}

/// Whether an admission evaluated one check or an atomic batch, with the
/// policy metadata each shape can provide.
///
/// A single check always names its policy. A batch names a policy only when
/// exactly one of its checks is relevant: the sole check of a one-check batch,
/// or the check that a denial singled out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOperation<'a> {
    /// One check against one policy.
    Check {
        /// The evaluated policy.
        policy: AdmissionPolicy<'a>,
    },
    /// An atomic batch.
    Batch {
        /// Number of checks submitted, including for a rejected empty batch.
        batch_size: usize,
        /// The policy of the one relevant check, when the batch singled one
        /// out: the sole check of a one-check batch that was allowed or failed
        /// with policy metadata, or the check named by a denial.
        policy: Option<AdmissionPolicy<'a>>,
    },
}

/// Classification of a completed admission operation.
///
/// Every variant is a named match arm, so a metrics mapper cannot leave a new
/// outcome unmetered behind a wildcard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionOutcome {
    /// Quota was consumed successfully.
    Allowed,
    /// Quota exhaustion was enforced.
    QuotaDenied,
    /// Quota exhaustion was observed but not enforced.
    ShadowDenied,
    /// A hard storage bound denied admission.
    CapacityDenied,
    /// The backend or input validation failed.
    Failed,
}

/// What a caller can know about quota consumption after an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsumptionStatus {
    /// The operation definitely consumed quota.
    Consumed,
    /// The operation definitely did not consume quota.
    NotConsumed,
    /// A failure occurred after the backend may have committed consumption.
    PossiblyConsumed,
}

/// Metadata for one completed admission operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionObservation<'a> {
    operation: AdmissionOperation<'a>,
    outcome: AdmissionOutcome,
    consumption: ConsumptionStatus,
    elapsed: Duration,
}

impl<'a> AdmissionObservation<'a> {
    /// Builds metadata for a failed single-check operation.
    pub fn failed_check<P: RateLimitPolicy + ?Sized>(
        check: &Check<'a, P>,
        consumption: ConsumptionStatus,
        elapsed: Duration,
    ) -> Self {
        Self {
            operation: AdmissionOperation::Check {
                policy: AdmissionPolicy::from_check(check),
            },
            outcome: AdmissionOutcome::Failed,
            consumption,
            elapsed,
        }
    }

    /// Builds metadata for a failed batch from the checks the caller submitted.
    ///
    /// A one-check batch carries that check's policy. Empty and multi-check
    /// batches have no single relevant policy, so their policy is absent.
    pub fn failed_batch<P: RateLimitPolicy + ?Sized>(
        checks: &[Check<'a, P>],
        consumption: ConsumptionStatus,
        elapsed: Duration,
    ) -> Self {
        Self {
            operation: AdmissionOperation::Batch {
                batch_size: checks.len(),
                policy: match checks {
                    [check] => Some(AdmissionPolicy::from_check(check)),
                    [] | [_, _, ..] => None,
                },
            },
            outcome: AdmissionOutcome::Failed,
            consumption,
            elapsed,
        }
    }

    /// Builds admission metadata from one completed check decision.
    ///
    /// Only an allowed decision is recorded as having consumed quota. Quota,
    /// shadow-quota, and storage-capacity denials receive their corresponding
    /// backend-neutral outcome classifications.
    pub fn from_check<P: RateLimitPolicy + ?Sized>(
        check: &Check<'a, P>,
        decision: &Decision,
        elapsed: Duration,
    ) -> Self {
        let (outcome, consumption) = classify_decision(decision);
        Self {
            operation: AdmissionOperation::Check {
                policy: AdmissionPolicy::from_check(check),
            },
            outcome,
            consumption,
            elapsed,
        }
    }

    /// Builds admission metadata from one completed atomic batch decision.
    ///
    /// An allowed batch is recorded as consuming quota. An allowed batch
    /// receives policy metadata only when its decision contains exactly one
    /// allowed check; a denied batch receives the metadata of its reported
    /// failing input when that input is present.
    pub fn from_batch<P: RateLimitPolicy + ?Sized>(
        checks: &[Check<'a, P>],
        decision: &BatchDecision,
        elapsed: Duration,
    ) -> Self {
        let (outcome, consumption) = classify_batch(decision);
        Self {
            operation: AdmissionOperation::Batch {
                batch_size: checks.len(),
                policy: batch_relevant_check(checks, decision).map(AdmissionPolicy::from_check),
            },
            outcome,
            consumption,
            elapsed,
        }
    }

    /// Returns the operation shape and the policy metadata it carries.
    pub const fn operation(self) -> AdmissionOperation<'a> {
        self.operation
    }

    /// Returns the admission outcome class.
    pub const fn outcome(self) -> AdmissionOutcome {
        self.outcome
    }

    /// Returns the quota-consumption certainty.
    pub const fn consumption(self) -> ConsumptionStatus {
        self.consumption
    }

    /// Returns wall-clock evaluation latency measured by the backend process.
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

fn classify_decision(decision: &Decision) -> (AdmissionOutcome, ConsumptionStatus) {
    match decision.view() {
        DecisionView::Allowed { .. } => (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed),
        DecisionView::Denied { denial } => classify_denial(denial),
        DecisionView::ShadowDenied { .. } => (
            AdmissionOutcome::ShadowDenied,
            ConsumptionStatus::NotConsumed,
        ),
    }
}

fn classify_batch(decision: &BatchDecision) -> (AdmissionOutcome, ConsumptionStatus) {
    match decision.view() {
        BatchDecisionView::Allowed { .. } => {
            (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed)
        }
        BatchDecisionView::Denied { denial, .. } => classify_denial(denial),
        BatchDecisionView::ShadowDenied { .. } => (
            AdmissionOutcome::ShadowDenied,
            ConsumptionStatus::NotConsumed,
        ),
    }
}

const fn classify_denial(denial: Denial) -> (AdmissionOutcome, ConsumptionStatus) {
    match denial {
        Denial::QuotaExceeded(_) => (
            AdmissionOutcome::QuotaDenied,
            ConsumptionStatus::NotConsumed,
        ),
        Denial::StorageCapacity { .. } => (
            AdmissionOutcome::CapacityDenied,
            ConsumptionStatus::NotConsumed,
        ),
    }
}

fn batch_relevant_check<'checks, 'policy, P: RateLimitPolicy + ?Sized>(
    checks: &'checks [Check<'policy, P>],
    decision: &BatchDecision,
) -> Option<&'checks Check<'policy, P>> {
    match decision.view() {
        BatchDecisionView::Allowed { allowances } if allowances.len() == 1 => checks.first(),
        BatchDecisionView::Allowed { .. } => None,
        BatchDecisionView::Denied { index, .. } | BatchDecisionView::ShadowDenied { index, .. } => {
            checks.get(index)
        }
    }
}

/// The effect of one bounded cleanup pass, as far as the backend can know it.
///
/// Every variant is a named match arm. An observer that reports removed rows
/// matches [`CleanupOutcome::Confirmed`] and never has to interpret an
/// optional count or a borrowed quota-consumption vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupOutcome {
    /// The pass committed and removed exactly `removed` rows or entries.
    Confirmed {
        /// Rows or entries removed.
        removed: u64,
    },
    /// The pass failed before it could remove anything.
    NoEffect,
    /// The pass failed after its removals may already have committed.
    Unknown,
}

/// Metadata for one bounded cleanup pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupObservation {
    requested: usize,
    outcome: CleanupOutcome,
    elapsed: Duration,
}

impl CleanupObservation {
    /// Constructs metadata for a cleanup pass.
    pub const fn new(requested: usize, outcome: CleanupOutcome, elapsed: Duration) -> Self {
        Self {
            requested,
            outcome,
            elapsed,
        }
    }

    /// Returns the configured maximum work for this cleanup pass.
    pub const fn requested(self) -> usize {
        self.requested
    }

    /// Returns what the backend knows about the pass's effect.
    pub const fn outcome(self) -> CleanupOutcome {
        self.outcome
    }

    /// Returns cleanup latency.
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

/// Local capacity use reported by one shard of a bounded backend.
///
/// Every bounded backend that reports capacity is sharded, so the shard index
/// is always present and an observer never handles a missing one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityObservation {
    used: u64,
    capacity: u64,
    shard_index: usize,
}

impl CapacityObservation {
    /// Constructs capacity metadata for one shard.
    pub const fn new(used: u64, capacity: u64, shard_index: usize) -> Self {
        Self {
            used,
            capacity,
            shard_index,
        }
    }

    /// Returns occupied capacity units.
    pub const fn used(self) -> u64 {
        self.used
    }

    /// Returns the local hard capacity.
    pub const fn capacity(self) -> u64 {
        self.capacity
    }

    /// Returns remaining capacity.
    pub const fn headroom(self) -> u64 {
        self.capacity.saturating_sub(self.used)
    }

    /// Returns the backend-local shard index.
    pub const fn shard_index(self) -> usize {
        self.shard_index
    }
}

/// Invokes an observer while isolating callback panics.
///
/// Backend implementations use this after releasing locks and finalizing
/// transactions. Applications normally call [`Observer::observe`] only in
/// their observer implementation.
#[doc(hidden)]
pub fn observe_safely(observer: &dyn Observer, observation: &Observation<'_>) {
    let _ = catch_unwind(AssertUnwindSafe(|| observer.observe(observation)));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        AdmissionObservation, AdmissionOperation, AdmissionOutcome, AdmissionPolicy,
        CleanupObservation, CleanupOutcome, ConsumptionStatus,
    };
    use crate::{
        Allowance, BatchDecision, Capacity, Check, Decision, Denial, FixedWindowPolicy, PolicyId,
        QuotaDenial, RateLimitPolicy, ScopeId, SubjectKey,
    };

    fn policy(id: &str) -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new(id).unwrap(),
            ScopeId::new("client").unwrap(),
            3,
            Duration::from_mins(1),
        )
        .unwrap()
    }

    fn capacity(value: u64) -> Capacity {
        Capacity::new(value).unwrap()
    }

    fn assert_admission(
        admission: AdmissionObservation<'_>,
        operation: AdmissionOperation<'_>,
        outcome: AdmissionOutcome,
        consumption: ConsumptionStatus,
        elapsed: Duration,
    ) {
        assert_eq!(admission.operation(), operation);
        assert_eq!(admission.outcome(), outcome);
        assert_eq!(admission.consumption(), consumption);
        assert_eq!(admission.elapsed(), elapsed);
    }

    #[test]
    fn admission_policy_captures_identity_and_fingerprint_together() {
        let policy = policy("api.read");
        let captured = AdmissionPolicy::new(&policy);

        assert_eq!(captured.id(), RateLimitPolicy::id(&policy));
        assert_eq!(captured.scope(), RateLimitPolicy::scope(&policy));
        assert_eq!(captured.fingerprint(), policy.fingerprint());
    }

    #[test]
    fn admission_debug_omits_fingerprint_unless_explicitly_requested() {
        let policy = policy("api.read");
        let check = Check::new(SubjectKey::from_digest([1; 32]).bind(&policy));
        let admission = AdmissionObservation::failed_check(
            &check,
            ConsumptionStatus::NotConsumed,
            Duration::from_millis(7),
        );
        let captured = AdmissionPolicy::new(&policy);
        let operation = admission.operation();
        let observation = super::Observation::Admission(admission);
        let fingerprint = policy.fingerprint().to_string();

        assert_eq!(captured.fingerprint(), policy.fingerprint());
        for debug in [
            format!("{captured:?}"),
            format!("{operation:?}"),
            format!("{admission:?}"),
            format!("{observation:?}"),
        ] {
            assert!(!debug.contains("fingerprint"));
            assert!(!debug.contains(&fingerprint));
        }
    }

    #[test]
    fn check_decisions_keep_their_admission_classification() {
        let policy = policy("api.read");
        let check = Check::new(SubjectKey::from_digest([1; 32]).bind(&policy));
        let elapsed = Duration::from_millis(7);
        let quota_denial = QuotaDenial::new(capacity(3), Duration::from_secs(1));
        let capacity_denial = Denial::StorageCapacity { retry_after: None };

        for (decision, outcome, consumption) in [
            (
                Decision::allowed(Allowance::new(capacity(3), 2, Duration::from_mins(1)).unwrap()),
                AdmissionOutcome::Allowed,
                ConsumptionStatus::Consumed,
            ),
            (
                Decision::denied(quota_denial),
                AdmissionOutcome::QuotaDenied,
                ConsumptionStatus::NotConsumed,
            ),
            (
                Decision::shadow_denied(quota_denial),
                AdmissionOutcome::ShadowDenied,
                ConsumptionStatus::NotConsumed,
            ),
            (
                Decision::denied(capacity_denial),
                AdmissionOutcome::CapacityDenied,
                ConsumptionStatus::NotConsumed,
            ),
        ] {
            assert_admission(
                AdmissionObservation::from_check(&check, &decision, elapsed),
                AdmissionOperation::Check {
                    policy: AdmissionPolicy::new(&policy),
                },
                outcome,
                consumption,
                elapsed,
            );
        }
    }

    #[test]
    fn failure_factories_preserve_consumption_and_couple_policy_metadata() {
        let policy = policy("api.failed");
        let check = Check::new(SubjectKey::from_digest([1; 32]).bind(&policy));
        let elapsed = Duration::from_millis(7);

        assert_admission(
            AdmissionObservation::failed_check(
                &check,
                ConsumptionStatus::PossiblyConsumed,
                elapsed,
            ),
            AdmissionOperation::Check {
                policy: AdmissionPolicy::new(&policy),
            },
            AdmissionOutcome::Failed,
            ConsumptionStatus::PossiblyConsumed,
            elapsed,
        );
        let empty: [Check<'_, FixedWindowPolicy>; 0] = [];
        assert_admission(
            AdmissionObservation::failed_batch(&empty, ConsumptionStatus::NotConsumed, elapsed),
            AdmissionOperation::Batch {
                batch_size: 0,
                policy: None,
            },
            AdmissionOutcome::Failed,
            ConsumptionStatus::NotConsumed,
            elapsed,
        );
        let failed_batch = AdmissionObservation::failed_batch(
            std::slice::from_ref(&check),
            ConsumptionStatus::NotConsumed,
            elapsed,
        );
        assert_admission(
            failed_batch,
            AdmissionOperation::Batch {
                batch_size: 1,
                policy: Some(AdmissionPolicy::new(&policy)),
            },
            AdmissionOutcome::Failed,
            ConsumptionStatus::NotConsumed,
            elapsed,
        );
        let debug = format!("{failed_batch:?}");
        assert!(debug.contains("operation: Batch"));
        assert!(debug.contains("batch_size: 1"));
        assert!(debug.contains("PolicyId(\"api.failed\")"));
        assert!(debug.contains("ScopeId(\"client\")"));
        assert!(debug.contains("outcome: Failed"));
        assert!(debug.contains("consumption: NotConsumed"));
        assert!(debug.contains("elapsed: 7ms"));
    }

    #[test]
    fn batch_decisions_keep_consumption_and_relevant_policy_semantics() {
        let first = policy("api.read");
        let second = policy("api.write");
        let checks = [
            Check::new(SubjectKey::from_digest([1; 32]).bind(&first)),
            Check::new(SubjectKey::from_digest([2; 32]).bind(&second)),
        ];
        let elapsed = Duration::from_millis(11);
        let allowed = Allowance::new(capacity(3), 2, Duration::from_mins(1)).unwrap();
        let quota_denial = QuotaDenial::new(capacity(3), Duration::from_secs(1));
        let capacity_denial = Denial::StorageCapacity { retry_after: None };

        for (decision, policy, outcome, consumption) in [
            (
                BatchDecision::allowed(vec![allowed]).unwrap(),
                Some(&first),
                AdmissionOutcome::Allowed,
                ConsumptionStatus::Consumed,
            ),
            (
                BatchDecision::allowed(vec![allowed, allowed]).unwrap(),
                None,
                AdmissionOutcome::Allowed,
                ConsumptionStatus::Consumed,
            ),
            (
                BatchDecision::denied(1, 2, quota_denial).unwrap(),
                Some(&second),
                AdmissionOutcome::QuotaDenied,
                ConsumptionStatus::NotConsumed,
            ),
            (
                BatchDecision::shadow_denied(0, 2, quota_denial).unwrap(),
                Some(&first),
                AdmissionOutcome::ShadowDenied,
                ConsumptionStatus::NotConsumed,
            ),
            (
                BatchDecision::denied(1, 2, capacity_denial).unwrap(),
                Some(&second),
                AdmissionOutcome::CapacityDenied,
                ConsumptionStatus::NotConsumed,
            ),
        ] {
            assert_admission(
                AdmissionObservation::from_batch(&checks, &decision, elapsed),
                AdmissionOperation::Batch {
                    batch_size: checks.len(),
                    policy: policy.map(AdmissionPolicy::new),
                },
                outcome,
                consumption,
                elapsed,
            );
        }
    }

    #[test]
    fn cleanup_observations_expose_their_outcome_directly() {
        let elapsed = Duration::from_millis(5);
        for outcome in [
            CleanupOutcome::Confirmed { removed: 3 },
            CleanupOutcome::NoEffect,
            CleanupOutcome::Unknown,
        ] {
            let cleanup = CleanupObservation::new(8, outcome, elapsed);
            assert_eq!(cleanup.requested(), 8);
            assert_eq!(cleanup.outcome(), outcome);
            assert_eq!(cleanup.elapsed(), elapsed);
        }

        assert_eq!(
            format!(
                "{:?}",
                CleanupObservation::new(8, CleanupOutcome::Confirmed { removed: 3 }, elapsed)
            ),
            "CleanupObservation { requested: 8, outcome: Confirmed { removed: 3 }, elapsed: 5ms }"
        );
    }
}
