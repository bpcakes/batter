use std::{fmt, sync::Arc, time::Instant};

use runlimit_core::{
    AdmissionObservation, Allowance, BatchDecision, CapacityObservation, Check, CleanupObservation,
    CleanupOutcome, ConsumptionStatus, CounterKey, Decision, GcraPolicy, Limiter, Observation,
    Observer, QuotaMode,
    gcra::{ArithmeticOverflow, Evaluation as QuotaEvaluation, PendingAllowance, evaluate},
    observe_safely, validate_batch,
};
use thiserror::Error;

use crate::{
    Clock, MemoryBatchError, MemoryStoreConfig, MemoryStoreStats, PoisonedShardError, SystemClock,
    shards::{
        BatchTopology, BoundedShards, CleanupEffect, Shard, ShardEffect, collect_shard_effects,
        usize_to_u64,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    tat_scaled: u128,
}

impl Shard<Entry> {
    fn evaluate(
        &self,
        check: &PreparedCheck<'_>,
        now_millis: u128,
    ) -> Result<QuotaEvaluation, ArithmeticOverflow> {
        let active_tat = self
            .active_entry(check.counter_key, now_millis)
            .map(|(entry, _)| entry.tat_scaled);
        evaluate(&check.check, now_millis, active_tat)
    }

    fn consume(&mut self, check: &PreparedCheck<'_>, allowance: PendingAllowance) -> Allowance {
        self.replace(
            check.counter_key,
            Entry {
                tat_scaled: allowance.tat_scaled,
            },
            allowance.expires_at_millis,
        );

        allowance.allowance
    }
}

/// Failure of a single GCRA memory-store check.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum GcraCheckError {
    /// The target shard was poisoned and remains unavailable.
    #[error(transparent)]
    PoisonedShard(#[from] PoisonedShardError),
    /// Exact GCRA arithmetic could not represent the clock-relative state.
    #[error("GCRA arithmetic exceeded the supported exact range")]
    ArithmeticOverflow,
}

/// Failure of a GCRA memory-store atomic batch.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GcraBatchError {
    /// The shared bounded memory store rejected the batch.
    #[error(transparent)]
    Store(#[from] MemoryBatchError),
    /// Exact GCRA arithmetic could not represent the clock-relative state.
    #[error("GCRA arithmetic exceeded the supported exact range")]
    ArithmeticOverflow,
}

impl From<PoisonedShardError> for GcraBatchError {
    fn from(error: PoisonedShardError) -> Self {
        Self::Store(MemoryBatchError::PoisonedShard(error))
    }
}

#[derive(Clone, Copy, Debug)]
struct PreparedCheck<'a> {
    input_index: usize,
    counter_key: CounterKey,
    shard_position: usize,
    check: Check<'a, GcraPolicy>,
    quota_mode: QuotaMode,
}

impl<'a> PreparedCheck<'a> {
    fn new(input_index: usize, check: &Check<'a, GcraPolicy>, shard_position: usize) -> Self {
        Self {
            input_index,
            counter_key: check.counter_key(),
            shard_position,
            check: *check,
            quota_mode: check.policy().quota_mode(),
        }
    }
}

#[derive(Debug)]
struct Evaluation<T, E> {
    value: T,
    effect: E,
}

/// A hard-bounded, process-local generic-cell-rate-algorithm store.
///
/// Each key occupies one constant-size entry. The store refuses to evict
/// active entries, performs bounded cleanup per check, and preserves atomic
/// all-or-nothing batches in caller order.
pub struct GcraStore<C = SystemClock> {
    config: MemoryStoreConfig,
    clock: C,
    shards: BoundedShards<Entry>,
    observer: Option<Arc<dyn Observer>>,
}

/// Builds an empty [`GcraStore`] with its clock chosen before any quota state
/// exists.
///
/// Clock readings are coordinates for every timestamp stored in the shards,
/// so the clock is part of the store's identity rather than mutable runtime
/// configuration. A built store therefore exposes no clock-replacement API.
///
/// ```compile_fail,E0599
/// use std::time::Duration;
/// use runlimit_memory::{Clock, GcraStore, MemoryStoreConfig};
///
/// struct TestClock;
/// impl Clock for TestClock {
///     fn now(&self) -> Duration { Duration::ZERO }
/// }
///
/// let store = GcraStore::new(MemoryStoreConfig::new(1)?);
/// let store = store.with_clock(TestClock);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub struct GcraStoreBuilder<C = SystemClock> {
    config: MemoryStoreConfig,
    clock: C,
}

impl<C> fmt::Debug for GcraStore<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let poisoned_shards = self.shards.poisoned_count();
        formatter
            .debug_struct("GcraStore")
            .field("config", &self.config)
            .field("has_observer", &self.observer.is_some())
            .field("poisoned_shards", &poisoned_shards)
            .field("shard_state", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl GcraStore<SystemClock> {
    /// Creates a store using the system monotonic clock.
    pub fn new(config: MemoryStoreConfig) -> Self {
        Self::builder(config).build()
    }

    /// Begins constructing an empty store.
    ///
    /// Use [`GcraStoreBuilder::with_clock`] before [`GcraStoreBuilder::build`]
    /// when deterministic time is required. Once built, the store's clock
    /// cannot be replaced because its counters contain timestamps in that
    /// clock's coordinate system.
    pub fn builder(config: MemoryStoreConfig) -> GcraStoreBuilder<SystemClock> {
        GcraStoreBuilder {
            config,
            clock: SystemClock::new(),
        }
    }
}

impl<C> GcraStoreBuilder<C> {
    /// Returns this builder with a caller-supplied monotonic clock.
    pub fn with_clock<D: Clock>(self, clock: D) -> GcraStoreBuilder<D> {
        GcraStoreBuilder {
            config: self.config,
            clock,
        }
    }
}

impl<C: Clock> GcraStoreBuilder<C> {
    /// Builds an empty store whose timestamps use the selected clock.
    pub fn build(self) -> GcraStore<C> {
        let shards = BoundedShards::new(&self.config);
        GcraStore {
            config: self.config,
            clock: self.clock,
            shards,
            observer: None,
        }
    }
}

impl<C> GcraStore<C> {
    /// Returns this store with an operational observer.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }
}

impl<C: Clock> GcraStore<C> {
    /// Evaluates and, when allowed, consumes one GCRA check.
    ///
    /// # Errors
    ///
    /// Fails closed when the target shard is poisoned or exact arithmetic
    /// cannot represent the caller-supplied clock value.
    pub fn check(&self, check: &Check<'_, GcraPolicy>) -> Result<Decision, GcraCheckError> {
        let started = Instant::now();
        let result = self.check_inner(check);
        let elapsed = started.elapsed();
        match &result {
            Ok(evaluation) => {
                self.observe_shard_effect(&evaluation.effect);
                self.observe_admission(|| {
                    AdmissionObservation::from_check(check, &evaluation.value, elapsed)
                });
            }
            Err(_) => self.observe_admission(|| {
                AdmissionObservation::failed_check(check, ConsumptionStatus::NotConsumed, elapsed)
            }),
        }
        result.map(|evaluation| evaluation.value)
    }

    fn check_inner(
        &self,
        check: &Check<'_, GcraPolicy>,
    ) -> Result<Evaluation<Decision, ShardEffect>, GcraCheckError> {
        let counter_key = check.counter_key();
        let shard_index = self.shards.shard_index(&counter_key);
        let prepared = PreparedCheck::new(0, check, 0);
        let mut shard = self.shards.lock_shard(shard_index)?;
        let observed_millis = self.clock.now().as_millis();
        let now_millis = observed_millis.max(shard.latest_observed_millis());
        let evaluated = shard
            .evaluate(&prepared, now_millis)
            .map_err(|ArithmeticOverflow| GcraCheckError::ArithmeticOverflow)?;
        shard.record_observed_millis(now_millis);
        let cleanup_requested = self.config.max_expired_removals_per_check();
        let cleanup_started = Instant::now();
        let cleanup_removed = shard.prune_expired(now_millis, cleanup_requested);
        let cleanup_elapsed = cleanup_started.elapsed();

        let decision = match evaluated {
            QuotaEvaluation::Allowed(allowance) => {
                if !shard.contains_key(counter_key) && shard.used() >= shard.capacity() {
                    Decision::denied(shard.storage_capacity_denial(now_millis))
                } else {
                    Decision::allowed(shard.consume(&prepared, allowance))
                }
            }
            QuotaEvaluation::Denied(denial) => {
                if prepared.quota_mode == QuotaMode::Shadow {
                    Decision::shadow_denied(denial)
                } else {
                    Decision::denied(denial)
                }
            }
        };

        Ok(Evaluation {
            value: decision,
            effect: ShardEffect {
                shard_index,
                cleanup: CleanupEffect {
                    requested: cleanup_requested,
                    removed: cleanup_removed,
                    elapsed: cleanup_elapsed,
                },
                used: shard.used(),
                capacity: shard.capacity(),
            },
        })
    }

    /// Evaluates a nonempty atomic batch of GCRA checks.
    ///
    /// Allowed decisions preserve input order. A denied or shadow-denied batch
    /// consumes no quota.
    ///
    /// # Errors
    ///
    /// Fails closed for empty or otherwise invalid batches, poisoned shards,
    /// an unsatisfiable per-shard batch, or arithmetic overflow.
    pub fn check_all(
        &self,
        checks: &[Check<'_, GcraPolicy>],
    ) -> Result<BatchDecision, GcraBatchError> {
        let started = Instant::now();
        let result = self.check_all_inner(checks);
        let elapsed = started.elapsed();
        match &result {
            Ok(evaluation) => {
                self.observe_shard_effects(&evaluation.effect);
                self.observe_admission(|| {
                    AdmissionObservation::from_batch(checks, &evaluation.value, elapsed)
                });
            }
            Err(_) => self.observe_admission(|| {
                AdmissionObservation::failed_batch(checks, ConsumptionStatus::NotConsumed, elapsed)
            }),
        }
        result.map(|evaluation| evaluation.value)
    }

    #[allow(clippy::too_many_lines)]
    fn check_all_inner(
        &self,
        checks: &[Check<'_, GcraPolicy>],
    ) -> Result<Evaluation<BatchDecision, Vec<ShardEffect>>, GcraBatchError> {
        validate_batch(checks, self.config.max_batch_size()).map_err(MemoryBatchError::from)?;

        let topology = BatchTopology::new(
            checks
                .iter()
                .map(|check| self.shards.shard_index(&check.counter_key())),
            &self.config,
        )?;
        let prepared = checks
            .iter()
            .enumerate()
            .map(|(input_index, check)| {
                PreparedCheck::new(input_index, check, topology.shard_position(input_index))
            })
            .collect::<Vec<_>>();

        let mut locked_shards = self.shards.lock_topology(&topology)?;
        let observed_millis = self.clock.now().as_millis();
        let now_millis = locked_shards
            .iter()
            .map(|(_, shard)| shard.latest_observed_millis())
            .fold(observed_millis, u128::max);
        let mut evaluations = Vec::with_capacity(prepared.len());
        for check in &prepared {
            evaluations.push(
                locked_shards[check.shard_position]
                    .1
                    .evaluate(check, now_millis)
                    .map_err(|ArithmeticOverflow| GcraBatchError::ArithmeticOverflow)?,
            );
        }
        let mut cleanup_effects = Vec::with_capacity(locked_shards.len());
        for ((_, shard), check_count) in locked_shards.iter_mut().zip(topology.check_counts()) {
            shard.record_observed_millis(now_millis);
            let requested = self
                .config
                .max_expired_removals_per_check()
                .saturating_mul(check_count);
            let cleanup_started = Instant::now();
            let removed = shard.prune_expired(now_millis, requested);
            cleanup_effects.push(CleanupEffect {
                requested,
                removed,
                elapsed: cleanup_started.elapsed(),
            });
        }

        let mut pending_insertions = vec![0_usize; locked_shards.len()];
        let mut allowances = Vec::with_capacity(prepared.len());
        for (check, evaluation) in prepared.iter().zip(evaluations) {
            let shard = &locked_shards[check.shard_position].1;
            let allowance = match evaluation {
                QuotaEvaluation::Allowed(allowance) => allowance,
                QuotaEvaluation::Denied(denial) => {
                    let value = if check.quota_mode == QuotaMode::Shadow {
                        BatchDecision::shadow_denied(check.input_index, checks.len(), denial)
                    } else {
                        BatchDecision::denied(check.input_index, checks.len(), denial)
                    }
                    .expect("a prepared check's input index is below the batch size");
                    return Ok(Evaluation {
                        value,
                        effect: collect_shard_effects(&locked_shards, &cleanup_effects),
                    });
                }
            };

            if !shard.contains_key(check.counter_key) {
                let projected_len = shard.used() + pending_insertions[check.shard_position];
                if projected_len >= shard.capacity() {
                    return Ok(Evaluation {
                        value: BatchDecision::denied(
                            check.input_index,
                            checks.len(),
                            shard.storage_capacity_denial(now_millis),
                        )
                        .expect("a prepared check's input index is below the batch size"),
                        effect: collect_shard_effects(&locked_shards, &cleanup_effects),
                    });
                }
                pending_insertions[check.shard_position] += 1;
            }
            allowances.push(allowance);
        }

        let mut consumed = Vec::with_capacity(prepared.len());
        for (check, allowance) in prepared.iter().zip(allowances) {
            consumed.push(
                locked_shards[check.shard_position]
                    .1
                    .consume(check, allowance),
            );
        }
        Ok(Evaluation {
            value: BatchDecision::allowed(consumed)
                .expect("batch validation rejected the empty batch"),
            effect: collect_shard_effects(&locked_shards, &cleanup_effects),
        })
    }

    /// Returns current bounded storage use.
    ///
    /// # Errors
    ///
    /// Returns an error if any shard is poisoned.
    pub fn stats(&self) -> Result<MemoryStoreStats, PoisonedShardError> {
        self.shards.stats(&self.config)
    }

    /// Removes every stored counter.
    ///
    /// # Errors
    ///
    /// Returns an error if any shard is poisoned.
    pub fn clear(&self) -> Result<(), PoisonedShardError> {
        self.shards.clear()
    }

    /// Rebuilds poisoned shards by discarding their untrusted counter state.
    pub fn recover_poisoned(&self) -> usize {
        self.shards.recover_poisoned(&self.config)
    }

    fn observe_shard_effects(&self, effects: &[ShardEffect]) {
        for effect in effects {
            self.observe_shard_effect(effect);
        }
    }

    fn observe_shard_effect(&self, effect: &ShardEffect) {
        let Some(observer) = &self.observer else {
            return;
        };
        observe_safely(
            observer.as_ref(),
            &Observation::Cleanup(CleanupObservation::new(
                effect.cleanup.requested,
                CleanupOutcome::Confirmed {
                    removed: usize_to_u64(effect.cleanup.removed),
                },
                effect.cleanup.elapsed,
            )),
        );
        observe_safely(
            observer.as_ref(),
            &Observation::Capacity(CapacityObservation::new(
                usize_to_u64(effect.used),
                usize_to_u64(effect.capacity),
                effect.shard_index,
            )),
        );
    }

    fn observe_admission<'a>(&self, build_observation: impl FnOnce() -> AdmissionObservation<'a>) {
        let Some(observer) = &self.observer else {
            return;
        };
        let admission = build_observation();
        observe_safely(observer.as_ref(), &Observation::Admission(admission));
    }
}

// Both async bodies contain no `.await`: they exist to defer the synchronous
// check until the first poll, which the `Limiter` contract requires.
// Rust 1.94 predates this Clippy lint; retain the deferral contract on both toolchains.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
impl<C: Clock> Limiter for GcraStore<C> {
    type Policy = GcraPolicy;
    type CheckError = GcraCheckError;
    type CheckAllError = GcraBatchError;

    async fn check(&self, check: &Check<'_, Self::Policy>) -> Result<Decision, Self::CheckError> {
        GcraStore::check(self, check)
    }

    async fn check_all(
        &self,
        checks: &[Check<'_, Self::Policy>],
    ) -> Result<BatchDecision, Self::CheckAllError> {
        GcraStore::check_all(self, checks)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier, Mutex, Weak,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::Duration,
    };

    use runlimit_core::{
        AdmissionOperation, AdmissionOutcome, Allowance, BatchDecision, BatchDecisionView,
        BatchError, Capacity, Check, CleanupOutcome, ConsumptionStatus, Decision, DecisionView,
        Denial, GcraPolicy, MAX_LIMIT, Observation, Observer, PolicyId, QuotaDenial, QuotaMode,
        ScopeId, SubjectKey,
    };

    use super::{GcraBatchError, GcraCheckError, GcraStore};
    use crate::{Clock, MemoryBatchError, MemoryStoreConfig, PoisonedShardError};

    fn capacity(value: u64) -> Capacity {
        Capacity::new(value).unwrap()
    }

    fn quota(capacity_value: u64, retry_after: Duration) -> QuotaDenial {
        QuotaDenial::new(capacity(capacity_value), retry_after)
    }

    fn allowance(capacity_value: u64, available: u64, replenishes_after: Duration) -> Allowance {
        Allowance::new(capacity(capacity_value), available, replenishes_after).unwrap()
    }

    fn with_clock<C: Clock>(config: MemoryStoreConfig, clock: C) -> GcraStore<C> {
        GcraStore::builder(config).with_clock(clock).build()
    }

    #[derive(Clone, Default)]
    struct ManualClock {
        millis: Arc<AtomicU64>,
    }

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            self.millis.fetch_add(
                u64::try_from(duration.as_millis()).unwrap(),
                Ordering::Relaxed,
            );
        }

        fn set(&self, duration: Duration) {
            self.millis.store(
                u64::try_from(duration.as_millis()).unwrap(),
                Ordering::Relaxed,
            );
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.millis.load(Ordering::Relaxed))
        }
    }

    #[derive(Clone, Copy)]
    struct MaximumClock;

    impl Clock for MaximumClock {
        fn now(&self) -> Duration {
            Duration::new(u64::MAX, 999_999_999)
        }
    }

    #[derive(Clone, Default)]
    struct SwitchableExtremeClock {
        extreme: Arc<AtomicBool>,
    }

    impl SwitchableExtremeClock {
        fn use_extreme_time(&self) {
            self.extreme.store(true, Ordering::Relaxed);
        }
    }

    impl Clock for SwitchableExtremeClock {
        fn now(&self) -> Duration {
            if self.extreme.load(Ordering::Relaxed) {
                Duration::new(u64::MAX, 999_999_999)
            } else {
                Duration::ZERO
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedObservation {
        Admission {
            outcome: AdmissionOutcome,
            consumption: ConsumptionStatus,
        },
        Cleanup {
            outcome: CleanupOutcome,
        },
        Capacity {
            used: u64,
            capacity: u64,
            shard_index: usize,
        },
    }

    impl RecordedObservation {
        fn from_observation(observation: &Observation<'_>) -> Self {
            match observation {
                Observation::Admission(admission) => Self::Admission {
                    outcome: admission.outcome(),
                    consumption: admission.consumption(),
                },
                Observation::Cleanup(cleanup) => Self::Cleanup {
                    outcome: cleanup.outcome(),
                },
                Observation::Capacity(capacity) => Self::Capacity {
                    used: capacity.used(),
                    capacity: capacity.capacity(),
                    shard_index: capacity.shard_index(),
                },
            }
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        observations: Mutex<Vec<RecordedObservation>>,
    }

    impl RecordingObserver {
        fn take(&self) -> Vec<RecordedObservation> {
            std::mem::take(
                &mut *self
                    .observations
                    .lock()
                    .expect("recording observer mutex remains healthy"),
            )
        }
    }

    impl Observer for RecordingObserver {
        fn observe(&self, observation: &Observation<'_>) {
            self.observations
                .lock()
                .expect("recording observer mutex remains healthy")
                .push(RecordedObservation::from_observation(observation));
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedAdmissionMetadata {
        Check,
        Batch { has_policy: bool },
    }

    #[derive(Default)]
    struct AdmissionMetadataObserver {
        admissions: Mutex<Vec<RecordedAdmissionMetadata>>,
    }

    impl Observer for AdmissionMetadataObserver {
        fn observe(&self, observation: &Observation<'_>) {
            let Observation::Admission(admission) = observation else {
                return;
            };
            self.admissions
                .lock()
                .unwrap()
                .push(match admission.operation() {
                    AdmissionOperation::Check { .. } => RecordedAdmissionMetadata::Check,
                    AdmissionOperation::Batch { policy, .. } => RecordedAdmissionMetadata::Batch {
                        has_policy: policy.is_some(),
                    },
                });
        }
    }

    struct ReentrantObserver {
        store: Mutex<Option<Weak<GcraStore<ManualClock>>>>,
        observations: Mutex<Vec<RecordedObservation>>,
    }

    impl Observer for ReentrantObserver {
        fn observe(&self, observation: &Observation<'_>) {
            if let Some(store) = self
                .store
                .lock()
                .expect("reentrant observer store mutex remains healthy")
                .as_ref()
                .and_then(Weak::upgrade)
            {
                store
                    .stats()
                    .expect("observer runs after shard locks release");
            }
            self.observations
                .lock()
                .expect("reentrant observer observations mutex remains healthy")
                .push(RecordedObservation::from_observation(observation));
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct ReferenceBucket {
        quota: u128,
        period_millis: u128,
        capacity: u64,
        tokens_scaled: u128,
        last_millis: u64,
    }

    impl ReferenceBucket {
        fn new(quota: u64, period_millis: u64, capacity: u64) -> Self {
            Self {
                quota: u128::from(quota),
                period_millis: u128::from(period_millis),
                capacity,
                tokens_scaled: u128::from(capacity) * u128::from(period_millis),
                last_millis: 0,
            }
        }

        fn check(&mut self, now_millis: u64, cost: u64) -> Decision {
            assert!(now_millis >= self.last_millis);
            let maximum = u128::from(self.capacity) * self.period_millis;
            let elapsed = u128::from(now_millis - self.last_millis);
            self.tokens_scaled = maximum.min(self.tokens_scaled + elapsed * self.quota);
            self.last_millis = now_millis;

            let requested = u128::from(cost) * self.period_millis;
            if requested > self.tokens_scaled {
                return Decision::denied(quota(
                    self.capacity,
                    reference_duration(reference_div_ceil(
                        requested - self.tokens_scaled,
                        self.quota,
                    )),
                ));
            }

            self.tokens_scaled -= requested;
            Decision::allowed(allowance(
                self.capacity,
                u64::try_from(self.tokens_scaled / self.period_millis)
                    .expect("reference availability fits the configured capacity"),
                reference_duration(reference_div_ceil(maximum - self.tokens_scaled, self.quota)),
            ))
        }
    }

    fn reference_div_ceil(numerator: u128, denominator: u128) -> u128 {
        numerator.div_ceil(denominator)
    }

    fn reference_duration(millis: u128) -> Duration {
        Duration::from_millis(
            u64::try_from(millis).expect("small reference-model durations fit in u64"),
        )
    }

    fn policy(id: &str, quota: u64, period: Duration, burst_capacity: u64) -> GcraPolicy {
        GcraPolicy::new(
            PolicyId::new(id).unwrap(),
            ScopeId::new("client").unwrap(),
            quota,
            period,
            burst_capacity,
        )
        .unwrap()
    }

    fn subject(byte: u8) -> SubjectKey {
        SubjectKey::from_digest([byte; 32])
    }

    #[test]
    fn matches_rational_token_bucket_reference_across_small_parameter_space() {
        for quota in 1_u64..=5 {
            for period_millis in 1_u64..=10 {
                for capacity in 1_u64..=5 {
                    let clock = ManualClock::default();
                    let store = with_clock(MemoryStoreConfig::new(1).unwrap(), clock.clone());
                    let id = format!("reference.{quota}.{period_millis}.{capacity}");
                    let policy = policy(&id, quota, Duration::from_millis(period_millis), capacity);
                    let mut reference = ReferenceBucket::new(quota, period_millis, capacity);
                    let mut now_millis = 0_u64;

                    for step in 0_u64..40 {
                        now_millis += match step % 5 {
                            0 | 2 => 0,
                            1 => 1,
                            3 => 2,
                            _ => 3,
                        };
                        clock.set(Duration::from_millis(now_millis));
                        let cost = 1 + (step * 7 + quota + period_millis) % capacity;
                        let expected = reference.check(now_millis, cost);
                        let actual = store
                            .check(
                                &Check::new(subject(1).bind(&policy))
                                    .with_cost(cost)
                                    .unwrap(),
                            )
                            .unwrap();

                        assert_eq!(
                            actual, expected,
                            "quota={quota}, period={period_millis}ms, capacity={capacity}, \
                             step={step}, now={now_millis}ms, cost={cost}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn quota_three_per_ten_millis_rounds_fractional_boundaries_up() {
        let clock = ManualClock::default();
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), clock.clone());
        let policy = policy("api.fractional", 3, Duration::from_millis(10), 4);
        let subject = subject(1);

        assert_eq!(
            store.check(&Check::new(subject.bind(&policy)).with_cost(4).unwrap()),
            Ok(Decision::allowed(allowance(
                4,
                0,
                Duration::from_millis(14)
            )))
        );
        clock.set(Duration::from_millis(3));
        assert_eq!(
            store.check(&Check::new(subject.bind(&policy))),
            Ok(Decision::denied(quota(4, Duration::from_millis(1))))
        );
        clock.set(Duration::from_millis(4));
        assert_eq!(
            store.check(&Check::new(subject.bind(&policy))),
            Ok(Decision::allowed(allowance(
                4,
                0,
                Duration::from_millis(13)
            )))
        );
    }

    #[test]
    fn quota_above_period_milliseconds_replenishes_multiple_units_per_tick() {
        let clock = ManualClock::default();
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), clock.clone());
        let policy = policy("api.fast", 10, Duration::from_millis(1), 12);
        let subject = subject(1);

        assert_eq!(
            store.check(&Check::new(subject.bind(&policy)).with_cost(12).unwrap()),
            Ok(Decision::allowed(allowance(
                12,
                0,
                Duration::from_millis(2)
            )))
        );
        assert_eq!(
            store.check(&Check::new(subject.bind(&policy))),
            Ok(Decision::denied(quota(12, Duration::from_millis(1))))
        );
        clock.set(Duration::from_millis(1));
        assert_eq!(
            store.check(&Check::new(subject.bind(&policy)).with_cost(10).unwrap()),
            Ok(Decision::allowed(allowance(
                12,
                0,
                Duration::from_millis(2)
            )))
        );
    }

    #[test]
    fn weighted_burst_refills_at_exact_integer_boundaries() {
        let clock = ManualClock::default();
        let store = with_clock(MemoryStoreConfig::new(8).unwrap(), clock.clone());
        let policy = policy("api.read", 2, Duration::from_secs(1), 4);
        let check = Check::new(subject(1).bind(&policy)).with_cost(3).unwrap();

        assert_eq!(
            store.check(&check),
            Ok(Decision::allowed(allowance(
                4,
                1,
                Duration::from_millis(1_500)
            )))
        );
        assert_eq!(
            store.check(&Check::new(subject(1).bind(&policy))),
            Ok(Decision::allowed(allowance(4, 0, Duration::from_secs(2))))
        );
        assert_eq!(
            store.check(&Check::new(subject(1).bind(&policy))),
            Ok(Decision::denied(quota(4, Duration::from_millis(500))))
        );

        clock.advance(Duration::from_millis(499));
        assert_eq!(
            store.check(&Check::new(subject(1).bind(&policy))),
            Ok(Decision::denied(quota(4, Duration::from_millis(1))))
        );
        clock.advance(Duration::from_millis(1));
        assert_eq!(
            store.check(&Check::new(subject(1).bind(&policy))),
            Ok(Decision::allowed(allowance(4, 0, Duration::from_secs(2))))
        );

        clock.advance(Duration::from_secs(2));
        assert_eq!(
            store.check(&Check::new(subject(1).bind(&policy))),
            Ok(Decision::allowed(allowance(
                4,
                3,
                Duration::from_millis(500)
            )))
        );
    }

    #[test]
    fn regressing_clocks_cannot_refill_quota_early() {
        let clock = ManualClock::default();
        clock.set(Duration::from_secs(10));
        let store = with_clock(MemoryStoreConfig::new(2).unwrap(), clock.clone());
        let policy = policy("api.write", 1, Duration::from_secs(1), 1);
        let check = Check::new(subject(1).bind(&policy));

        assert!(store.check(&check).unwrap().permits_request());
        clock.set(Duration::from_secs(1));
        assert_eq!(
            store.check(&check),
            Ok(Decision::denied(quota(1, Duration::from_secs(1))))
        );
    }

    #[test]
    fn bounded_expiry_cleanup_releases_hard_capacity() {
        let clock = ManualClock::default();
        let config = MemoryStoreConfig::new(3)
            .unwrap()
            .with_max_expired_removals_per_check(1)
            .unwrap();
        let store = with_clock(config, clock.clone());
        let policy = policy("api.read", 1, Duration::from_millis(10), 1);

        for byte in 1..=3 {
            assert!(
                store
                    .check(&Check::new(subject(byte).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }
        assert!(
            !store
                .check(&Check::new(subject(4).bind(&policy)))
                .unwrap()
                .permits_request()
        );

        clock.advance(Duration::from_millis(10));
        assert!(
            store
                .check(&Check::new(subject(4).bind(&policy)))
                .unwrap()
                .permits_request()
        );
        assert_eq!(store.stats().unwrap().entries(), 3);
    }

    #[test]
    fn concurrent_same_key_checks_consume_exactly_one_burst() {
        const ATTEMPTS: usize = 32;
        const BURST: usize = 7;

        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        let policy = policy(
            "api.concurrent",
            1,
            Duration::from_mins(1),
            u64::try_from(BURST).unwrap(),
        );
        let check = Check::new(subject(1).bind(&policy));
        let barrier = Arc::new(Barrier::new(ATTEMPTS + 1));

        let decisions = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(ATTEMPTS);
            let store_ref = &store;
            for _ in 0..ATTEMPTS {
                let barrier = Arc::clone(&barrier);
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    store_ref.check(&check).unwrap()
                }));
            }
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("admission thread does not panic"))
                .collect::<Vec<_>>()
        });

        assert_eq!(
            decisions
                .iter()
                .filter(|decision| matches!(decision.view(), DecisionView::Allowed { .. }))
                .count(),
            BURST
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| {
                    matches!(
                        decision.view(),
                        DecisionView::Denied {
                            denial: Denial::QuotaExceeded(_),
                        }
                    )
                })
                .count(),
            ATTEMPTS - BURST
        );
        assert_eq!(store.stats().unwrap().entries(), 1);
    }

    #[test]
    fn allowed_batches_return_exact_decisions_in_input_order() {
        let store = with_clock(MemoryStoreConfig::new(2).unwrap(), ManualClock::default());
        let slow = policy("api.slow", 1, Duration::from_millis(10), 3);
        let fast = policy("api.fast", 4, Duration::from_millis(10), 5);
        let checks = [
            Check::new(subject(1).bind(&fast)).with_cost(3).unwrap(),
            Check::new(subject(1).bind(&slow)).with_cost(2).unwrap(),
        ];

        assert_eq!(
            store.check_all(&checks),
            Ok(BatchDecision::allowed(vec![
                allowance(5, 2, Duration::from_millis(8)),
                allowance(3, 1, Duration::from_millis(20)),
            ])
            .unwrap())
        );
    }

    #[test]
    fn enforced_quota_denial_rolls_back_earlier_batch_members() {
        let store = with_clock(MemoryStoreConfig::new(3).unwrap(), ManualClock::default());
        let policy = policy("api.quota-rollback", 1, Duration::from_millis(100), 1);
        let exhausted = Check::new(subject(1).bind(&policy));
        let earlier = Check::new(subject(2).bind(&policy));
        assert!(store.check(&exhausted).unwrap().permits_request());

        let result = store.check_all(&[earlier, exhausted]).unwrap();
        assert!(matches!(
            result.view(),
            BatchDecisionView::Denied {
                index: 1,
                batch_size,
                denial: Denial::QuotaExceeded(_),
            } if batch_size.get() == 2
        ));
        assert!(
            store.check(&earlier).unwrap().permits_request(),
            "a quota-denied batch must not consume an earlier member"
        );
    }

    #[test]
    fn storage_capacity_denial_rolls_back_earlier_batch_members() {
        let store = with_clock(MemoryStoreConfig::new(2).unwrap(), ManualClock::default());
        let policy = policy("api.capacity-rollback", 1, Duration::from_millis(100), 2);
        let earlier = Check::new(subject(1).bind(&policy));
        let occupying = Check::new(subject(2).bind(&policy));
        let new_key = Check::new(subject(3).bind(&policy));
        assert!(store.check(&earlier).unwrap().permits_request());
        assert!(store.check(&occupying).unwrap().permits_request());

        let result = store.check_all(&[earlier, new_key]).unwrap();
        assert!(matches!(
            result.view(),
            BatchDecisionView::Denied {
                index: 1,
                batch_size,
                denial: Denial::StorageCapacity { .. },
            } if batch_size.get() == 2
        ));
        assert!(
            store.check(&earlier).unwrap().permits_request(),
            "a capacity-denied batch must not consume an earlier member"
        );
    }

    #[test]
    fn unsatisfiable_batch_uses_shared_topology_preflight_without_mutation() {
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        let policy = policy("api.unsatisfiable", 1, Duration::from_secs(1), 1);
        let checks = [
            Check::new(subject(1).bind(&policy)),
            Check::new(subject(2).bind(&policy)),
        ];

        assert_eq!(
            store.check_all(&checks),
            Err(GcraBatchError::Store(
                MemoryBatchError::BatchExceedsShardCapacity {
                    shard_index: 0,
                    key_count: 2,
                    capacity: 1,
                }
            ))
        );
        assert_eq!(store.stats().unwrap().entries(), 0);
    }

    #[test]
    fn atomic_batches_preserve_order_and_roll_back_on_shadow_denial() {
        let store = with_clock(MemoryStoreConfig::new(8).unwrap(), ManualClock::default());
        let shadow =
            policy("api.read", 1, Duration::from_secs(1), 1).with_quota_mode(QuotaMode::Shadow);
        let first = Check::new(subject(1).bind(&shadow));
        let second = Check::new(subject(2).bind(&shadow));

        assert!(store.check(&first).unwrap().permits_request());
        let result = store.check_all(&[first, second]).unwrap();
        assert!(matches!(
            result.view(),
            BatchDecisionView::ShadowDenied {
                index: 0,
                batch_size,
                ..
            } if batch_size.get() == 2
        ));
        assert!(
            store.check(&second).unwrap().permits_request(),
            "a shadow-denied batch must not consume any member"
        );
    }

    #[test]
    fn storage_capacity_is_enforced_for_shadow_policies() {
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        let shadow =
            policy("api.read", 1, Duration::from_secs(1), 1).with_quota_mode(QuotaMode::Shadow);

        assert!(
            store
                .check(&Check::new(subject(1).bind(&shadow)))
                .unwrap()
                .permits_request()
        );
        let decision = store.check(&Check::new(subject(2).bind(&shadow))).unwrap();
        assert!(matches!(
            decision.view(),
            DecisionView::Denied {
                denial: Denial::StorageCapacity { .. },
            }
        ));
    }

    #[test]
    fn batch_observers_run_after_unlock_in_sorted_shard_order() {
        let observer = Arc::new(ReentrantObserver {
            store: Mutex::new(None),
            observations: Mutex::new(Vec::new()),
        });
        let config = MemoryStoreConfig::new(2)
            .unwrap()
            .with_shard_count(2)
            .unwrap();
        let store =
            Arc::new(with_clock(config, ManualClock::default()).with_observer(observer.clone()));
        *observer
            .store
            .lock()
            .expect("reentrant observer store mutex remains healthy") =
            Some(Arc::downgrade(&store));
        let policy = policy("api.observed-batch", 1, Duration::from_secs(1), 1);
        let subject_for_shard = |target| {
            (0..=u8::MAX)
                .map(subject)
                .find(|subject| {
                    let check = Check::new((*subject).bind(&policy));
                    store.shards.shard_index(&check.counter_key()) == target
                })
                .expect("the deterministic subject space reaches each configured shard")
        };
        let checks = [
            Check::new(subject_for_shard(1).bind(&policy)),
            Check::new(subject_for_shard(0).bind(&policy)),
        ];

        assert!(matches!(
            store.check_all(&checks).unwrap().view(),
            BatchDecisionView::Allowed { .. }
        ));
        assert_eq!(
            *observer
                .observations
                .lock()
                .expect("reentrant observer observations mutex remains healthy"),
            vec![
                RecordedObservation::Cleanup {
                    outcome: CleanupOutcome::Confirmed { removed: 0 },
                },
                RecordedObservation::Capacity {
                    used: 1,
                    capacity: 1,
                    shard_index: 0,
                },
                RecordedObservation::Cleanup {
                    outcome: CleanupOutcome::Confirmed { removed: 0 },
                },
                RecordedObservation::Capacity {
                    used: 1,
                    capacity: 1,
                    shard_index: 1,
                },
                RecordedObservation::Admission {
                    outcome: AdmissionOutcome::Allowed,
                    consumption: ConsumptionStatus::Consumed,
                },
            ]
        );
    }

    #[test]
    fn extreme_clock_values_fail_closed_without_mutation() {
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), MaximumClock);
        let policy = policy("api.read", MAX_LIMIT, Duration::from_millis(1), 1);
        let check = Check::new(subject(1).bind(&policy));

        assert_eq!(store.check(&check), Err(GcraCheckError::ArithmeticOverflow));
        assert_eq!(store.stats().unwrap().entries(), 0);
    }

    #[test]
    fn failed_gcra_checks_and_one_check_batches_keep_policy_metadata() {
        let observer = Arc::new(AdmissionMetadataObserver::default());
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), MaximumClock)
            .with_observer(observer.clone());
        let policy = policy(
            "api.observed-overflow",
            MAX_LIMIT,
            Duration::from_millis(1),
            1,
        );
        let check = Check::new(subject(1).bind(&policy));

        assert_eq!(store.check(&check), Err(GcraCheckError::ArithmeticOverflow));
        assert_eq!(
            store.check_all(&[check]),
            Err(GcraBatchError::ArithmeticOverflow)
        );
        assert_eq!(
            observer.admissions.lock().unwrap().as_slice(),
            [
                RecordedAdmissionMetadata::Check,
                RecordedAdmissionMetadata::Batch { has_policy: true },
            ]
        );
    }

    #[test]
    fn arithmetic_preflight_preserves_expired_entries_and_emits_only_failure() {
        let clock = SwitchableExtremeClock::default();
        let observer = Arc::new(RecordingObserver::default());
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), clock.clone())
            .with_observer(observer.clone());
        let ordinary = policy("api.ordinary", 1, Duration::from_millis(1), 1);
        assert!(
            store
                .check(&Check::new(subject(1).bind(&ordinary)))
                .unwrap()
                .permits_request()
        );
        observer.take();

        clock.use_extreme_time();
        let overflowing = policy("api.overflowing", MAX_LIMIT, Duration::from_millis(1), 1);
        assert_eq!(
            store.check(&Check::new(subject(2).bind(&overflowing))),
            Err(GcraCheckError::ArithmeticOverflow)
        );
        assert_eq!(
            store.stats().unwrap().entries(),
            1,
            "arithmetic preflight must fail before pruning the now-expired entry"
        );
        assert_eq!(
            observer.take(),
            vec![RecordedObservation::Admission {
                outcome: AdmissionOutcome::Failed,
                consumption: ConsumptionStatus::NotConsumed,
            }]
        );
    }

    #[test]
    fn later_batch_overflow_supersedes_an_earlier_denial_without_effects() {
        let observer = Arc::new(RecordingObserver::default());
        let store = with_clock(MemoryStoreConfig::new(2).unwrap(), MaximumClock)
            .with_observer(observer.clone());
        let limited = policy("api.limited", 1, Duration::from_millis(1), 1);
        let exhausted = Check::new(subject(1).bind(&limited));
        assert!(store.check(&exhausted).unwrap().permits_request());
        observer.take();

        let overflowing = policy("api.overflowing", MAX_LIMIT, Duration::from_millis(1), 1);
        let later = Check::new(subject(2).bind(&overflowing));
        assert_eq!(
            store.check_all(&[exhausted, later]),
            Err(GcraBatchError::ArithmeticOverflow),
            "a later arithmetic failure must supersede an earlier quota denial"
        );
        assert_eq!(
            store.stats().unwrap().entries(),
            1,
            "arithmetic preflight must not clean up or insert entries"
        );
        assert_eq!(
            observer.take(),
            vec![RecordedObservation::Admission {
                outcome: AdmissionOutcome::Failed,
                consumption: ConsumptionStatus::NotConsumed,
            }]
        );
    }

    #[test]
    fn empty_batches_fail_closed() {
        let store = GcraStore::new(MemoryStoreConfig::new(1).unwrap());

        assert_eq!(
            store.check_all(&[]),
            Err(GcraBatchError::Store(MemoryBatchError::InvalidBatch(
                BatchError::EmptyBatch
            )))
        );
    }

    #[test]
    fn poisoned_shards_fail_single_checks_with_the_bare_error() {
        let store = GcraStore::new(MemoryStoreConfig::new(1).unwrap());
        let policy = policy("api.poisoned", 1, Duration::from_secs(1), 1);
        let check = Check::new(subject(1).bind(&policy));
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.shards.shard(0).lock().unwrap();
            panic!("poison the only shard");
        }));
        assert!(panic_result.is_err());

        assert_eq!(
            store.check(&check),
            Err(GcraCheckError::PoisonedShard(PoisonedShardError {
                shard_index: 0
            }))
        );
        assert_eq!(
            store.check_all(&[check]),
            Err(GcraBatchError::Store(MemoryBatchError::PoisonedShard(
                PoisonedShardError { shard_index: 0 }
            )))
        );
        assert_eq!(store.stats(), Err(PoisonedShardError { shard_index: 0 }));
        assert_eq!(store.recover_poisoned(), 1);
        assert!(store.check(&check).unwrap().permits_request());
    }

    #[test]
    fn generic_limiter_contract_selects_gcra_policy() {
        fn accepts_gcra_limiter<L: runlimit_core::Limiter<Policy = GcraPolicy>>(_limiter: &L) {}

        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        accepts_gcra_limiter(&store);
    }
}
