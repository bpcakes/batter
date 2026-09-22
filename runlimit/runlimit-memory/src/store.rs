use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use runlimit_core::{
    AdmissionObservation, Allowance, BatchDecision, BatchError, Capacity, CapacityObservation,
    Check, CleanupObservation, CleanupOutcome, ConsumptionStatus, CounterKey, Decision, Limiter,
    Observation, Observer, QuotaDenial, QuotaMode, observe_safely, validate_batch,
};
use thiserror::Error;

use crate::{
    Clock, MemoryStoreConfig, SystemClock,
    shards::{
        BatchTopology, BoundedShards, CleanupEffect, Shard, ShardEffect, collect_shard_effects,
        usize_to_u64,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    used: u64,
}

impl Shard<Entry> {
    fn quota_denial(&self, check: &PreparedCheck, now_millis: u128) -> Option<QuotaDenial> {
        let (entry, expires_at_millis) = self.active_entry(check.counter_key, now_millis)?;
        let remaining = remaining_quota(check.limit, entry.used);
        (check.cost > remaining).then(|| {
            QuotaDenial::new(
                check.limit,
                duration_from_millis(expires_at_millis.saturating_sub(now_millis)),
            )
        })
    }

    fn consume(&mut self, check: &PreparedCheck, now_millis: u128) -> Allowance {
        let expires_at_millis = now_millis + u128::from(check.window_millis);
        if let Some((entry, active_expires_at_millis)) =
            self.active_entry(check.counter_key, now_millis)
        {
            let remaining = remaining_quota(check.limit, entry.used);
            debug_assert!(
                check.cost <= remaining,
                "quota denial must be checked before consumption"
            );
            self.update_value(check.counter_key, |entry| {
                entry.used = entry.used.saturating_add(check.cost);
            })
            .expect("the active entry was present");
            return allowance(
                check.limit,
                remaining.saturating_sub(check.cost),
                duration_from_millis(active_expires_at_millis.saturating_sub(now_millis)),
            );
        }

        self.replace(
            check.counter_key,
            Entry { used: check.cost },
            expires_at_millis,
        );
        allowance(
            check.limit,
            remaining_quota(check.limit, check.cost),
            duration_from_millis(expires_at_millis.saturating_sub(now_millis)),
        )
    }
}

/// Builds an allowance whose `available` was derived by subtracting from
/// `limit`, so it can never exceed the capacity.
fn allowance(limit: Capacity, available: u64, replenishes_after: Duration) -> Allowance {
    Allowance::new(limit, available, replenishes_after)
        .expect("remaining quota is derived from the limit and cannot exceed it")
}

#[derive(Debug)]
struct PreparedCheck {
    input_index: usize,
    counter_key: CounterKey,
    shard_position: usize,
    limit: Capacity,
    window_millis: u64,
    cost: u64,
}

impl PreparedCheck {
    fn new(input_index: usize, check: &Check<'_>, shard_position: usize) -> Self {
        Self {
            input_index,
            counter_key: check.counter_key(),
            shard_position,
            limit: check.policy().limit(),
            window_millis: check.policy().window().millis(),
            cost: check.cost(),
        }
    }
}

#[derive(Debug)]
struct Evaluation<T, E> {
    value: T,
    effect: E,
}

/// A sharded, hard-bounded, process-local fixed-window store.
///
/// The store never evicts an active key to admit a new one. Every check touches
/// only the shards required by that check and performs a bounded amount of
/// expiry cleanup. An atomic batch locks its shards in deterministic order.
///
/// If application code panics while a shard is locked, that shard remains
/// poisoned and every later operation touching it fails closed with
/// [`PoisonedShardError`]. [`MemoryStore::recover_poisoned`] is an explicit
/// availability tradeoff: it restores poisoned shards by discarding their
/// counters rather than trusting state whose consistency is unknown.
pub struct MemoryStore<C = SystemClock> {
    config: MemoryStoreConfig,
    clock: C,
    shards: BoundedShards<Entry>,
    observer: Option<Arc<dyn Observer>>,
}

/// Builds an empty [`MemoryStore`] with its clock chosen before any quota
/// state exists.
///
/// Clock readings are coordinates for every timestamp stored in the shards,
/// so the clock is part of the store's identity rather than mutable runtime
/// configuration. A built store therefore exposes no clock-replacement API.
///
/// ```compile_fail,E0599
/// use std::time::Duration;
/// use runlimit_memory::{Clock, MemoryStore, MemoryStoreConfig};
///
/// struct TestClock;
/// impl Clock for TestClock {
///     fn now(&self) -> Duration { Duration::ZERO }
/// }
///
/// let store = MemoryStore::new(MemoryStoreConfig::new(1)?);
/// let store = store.with_clock(TestClock);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub struct MemoryStoreBuilder<C = SystemClock> {
    config: MemoryStoreConfig,
    clock: C,
}

impl<C> fmt::Debug for MemoryStore<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let poisoned_shards = self.shards.poisoned_count();

        formatter
            .debug_struct("MemoryStore")
            .field("config", &self.config)
            .field("has_observer", &self.observer.is_some())
            .field("poisoned_shards", &poisoned_shards)
            .field("shard_state", &Redacted)
            .finish_non_exhaustive()
    }
}

struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl MemoryStore<SystemClock> {
    /// Creates a store using the system monotonic clock.
    pub fn new(config: MemoryStoreConfig) -> Self {
        Self::builder(config).build()
    }

    /// Begins constructing an empty store.
    ///
    /// Use [`MemoryStoreBuilder::with_clock`] before [`MemoryStoreBuilder::build`]
    /// when deterministic time is required. Once built, the store's clock
    /// cannot be replaced because its counters contain timestamps in that
    /// clock's coordinate system.
    pub fn builder(config: MemoryStoreConfig) -> MemoryStoreBuilder<SystemClock> {
        MemoryStoreBuilder {
            config,
            clock: SystemClock::new(),
        }
    }
}

impl<C> MemoryStoreBuilder<C> {
    /// Returns this builder with a caller-supplied monotonic clock.
    pub fn with_clock<D: Clock>(self, clock: D) -> MemoryStoreBuilder<D> {
        MemoryStoreBuilder {
            config: self.config,
            clock,
        }
    }
}

impl<C: Clock> MemoryStoreBuilder<C> {
    /// Builds an empty store whose timestamps use the selected clock.
    pub fn build(self) -> MemoryStore<C> {
        let shards = BoundedShards::new(&self.config);
        MemoryStore {
            config: self.config,
            clock: self.clock,
            shards,
            observer: None,
        }
    }
}

impl<C> MemoryStore<C> {
    /// Returns this store with an operational observer.
    ///
    /// Observer callbacks run after internal locks are released. A callback
    /// panic is isolated and cannot change the admission result.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }
}

impl<C: Clock> MemoryStore<C> {
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

    /// Evaluates and, when allowed, consumes one check.
    ///
    /// # Errors
    ///
    /// Returns an error without allowing the request when the target shard is
    /// poisoned.
    pub fn check(&self, check: &Check<'_>) -> Result<Decision, PoisonedShardError> {
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
        check: &Check<'_>,
    ) -> Result<Evaluation<Decision, ShardEffect>, PoisonedShardError> {
        let counter_key = check.counter_key();
        let shard_index = self.shards.shard_index(&counter_key);
        let prepared = PreparedCheck::new(0, check, 0);
        let mut shard = self.shards.lock_shard(shard_index)?;
        let observed_millis = self.clock.now().as_millis();
        let now_millis = observed_millis.max(shard.latest_observed_millis());
        shard.record_observed_millis(now_millis);
        let cleanup_requested = self.config.max_expired_removals_per_check();
        let cleanup_started = Instant::now();
        let cleanup_removed = shard.prune_expired(now_millis, cleanup_requested);
        let cleanup_elapsed = cleanup_started.elapsed();

        let decision = if let Some(denial) = shard.quota_denial(&prepared, now_millis) {
            if check.policy().quota_mode() == QuotaMode::Shadow {
                Decision::shadow_denied(denial)
            } else {
                Decision::denied(denial)
            }
        } else if !shard.contains_key(counter_key) && shard.used() >= shard.capacity() {
            Decision::denied(shard.storage_capacity_denial(now_millis))
        } else {
            Decision::allowed(shard.consume(&prepared, now_millis))
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

    /// Evaluates a nonempty batch atomically.
    ///
    /// If any check is denied, none of the checks consumes quota. Returned
    /// allowed decisions preserve input order.
    ///
    /// # Errors
    ///
    /// Returns an error before quota consumption when the batch is empty,
    /// exceeds its configured bound, mixes quota modes, contains a duplicate
    /// key, targets more distinct keys at one shard than that shard can ever
    /// store, or a touched shard is poisoned.
    pub fn check_all(&self, checks: &[Check<'_>]) -> Result<BatchDecision, MemoryBatchError> {
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
        checks: &[Check<'_>],
    ) -> Result<Evaluation<BatchDecision, Vec<ShardEffect>>, MemoryBatchError> {
        validate_batch(checks, self.config.max_batch_size())?;

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
        let mut cleanup_effects = Vec::with_capacity(locked_shards.len());
        for ((_, shard), check_count) in locked_shards.iter_mut().zip(topology.check_counts()) {
            shard.record_observed_millis(now_millis);
            let cleanup_limit = self
                .config
                .max_expired_removals_per_check()
                .saturating_mul(check_count);
            let cleanup_started = Instant::now();
            let removed = shard.prune_expired(now_millis, cleanup_limit);
            cleanup_effects.push(CleanupEffect {
                requested: cleanup_limit,
                removed,
                elapsed: cleanup_started.elapsed(),
            });
        }

        let mut pending_insertions = vec![0_usize; locked_shards.len()];
        for check in &prepared {
            let shard = &locked_shards[check.shard_position].1;
            if let Some(denial) = shard.quota_denial(check, now_millis) {
                let value = if checks[check.input_index].policy().quota_mode() == QuotaMode::Shadow
                {
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
        }

        let mut allowances = Vec::with_capacity(prepared.len());
        for check in &prepared {
            let shard = &mut locked_shards[check.shard_position].1;
            allowances.push(shard.consume(check, now_millis));
        }

        Ok(Evaluation {
            value: BatchDecision::allowed(allowances)
                .expect("batch validation rejected the empty batch"),
            effect: collect_shard_effects(&locked_shards, &cleanup_effects),
        })
    }

    /// Returns current storage usage.
    ///
    /// # Errors
    ///
    /// Returns an error if a shard is poisoned.
    pub fn stats(&self) -> Result<MemoryStoreStats, PoisonedShardError> {
        self.shards.stats(&self.config)
    }

    /// Removes every stored counter.
    ///
    /// # Errors
    ///
    /// Returns an error if a shard is poisoned.
    pub fn clear(&self) -> Result<(), PoisonedShardError> {
        self.shards.clear()
    }

    /// Rebuilds every poisoned shard and returns the number recovered.
    ///
    /// A poisoned shard's internal consistency is unknown, so recovery
    /// discards all counters in that shard before clearing its mutex poison
    /// flag. This can admit requests that the discarded counters would have
    /// denied. Healthy shards and their active counters are left unchanged.
    ///
    /// Until this method is called, operations touching a poisoned shard
    /// continue to fail closed with [`PoisonedShardError`].
    /// Recovery is safe to invoke through an [`std::sync::Arc`]. It locks every
    /// shard in index order before changing any of them, so concurrent checks
    /// observe either the pre-recovery poisoned store or the complete recovered
    /// state, never a partially reopened multi-shard store.
    pub fn recover_poisoned(&self) -> usize {
        self.shards.recover_poisoned(&self.config)
    }
}

fn remaining_quota(limit: Capacity, used: u64) -> u64 {
    debug_assert!(
        used <= limit.get(),
        "stored quota usage ({used}) exceeded its policy limit ({limit})"
    );
    limit.get().saturating_sub(used)
}

// Both async bodies contain no `.await`: they exist to defer the synchronous
// check until the first poll, which the `Limiter` contract requires.
// Rust 1.94 predates this Clippy lint; retain the deferral contract on both toolchains.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
impl<C: Clock> Limiter for MemoryStore<C> {
    type Policy = runlimit_core::FixedWindowPolicy;
    type CheckError = PoisonedShardError;
    type CheckAllError = MemoryBatchError;

    async fn check(&self, check: &Check<'_>) -> Result<Decision, Self::CheckError> {
        MemoryStore::check(self, check)
    }

    async fn check_all(&self, checks: &[Check<'_>]) -> Result<BatchDecision, Self::CheckAllError> {
        MemoryStore::check_all(self, checks)
    }
}

fn duration_from_millis(millis: u128) -> Duration {
    Duration::from_millis(
        u64::try_from(millis)
            .expect("a counter's remaining duration cannot exceed its portable policy window"),
    )
}

/// Current bounded-store usage.
///
/// With the `serde` feature, this serializes as an object containing `entries`,
/// `capacity`, and `shard_count`. Telemetry is intentionally serialize-only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryStoreStats {
    entries: usize,
    capacity: usize,
    shard_count: usize,
}

impl MemoryStoreStats {
    pub(crate) const fn from_parts(entries: usize, capacity: usize, shard_count: usize) -> Self {
        Self {
            entries,
            capacity,
            shard_count,
        }
    }

    /// Returns the number of stored counters.
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the hard maximum number of stored counters.
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// Returns the number of independently locked shards.
    pub const fn shard_count(self) -> usize {
        self.shard_count
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct MemoryStoreStatsWire {
    entries: usize,
    capacity: usize,
    shard_count: usize,
}

#[cfg(feature = "serde")]
impl serde::Serialize for MemoryStoreStats {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &MemoryStoreStatsWire {
                entries: self.entries(),
                capacity: self.capacity(),
                shard_count: self.shard_count(),
            },
            serializer,
        )
    }
}

/// A shard mutex was poisoned by a panic and remains unavailable.
///
/// This is the only failure a single check, [`MemoryStore::stats`], or
/// [`MemoryStore::clear`] can report. Every operation touching the shard
/// fails closed until [`MemoryStore::recover_poisoned`] rebuilds it.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("memory-store shard {shard_index} is poisoned and unavailable")]
pub struct PoisonedShardError {
    /// Poisoned shard index.
    pub shard_index: usize,
}

/// A fail-closed failure of an in-memory atomic batch.
///
/// Every variant is reachable from [`MemoryStore::check_all`]; failures that
/// only a batch can produce never appear on the single-check path.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MemoryBatchError {
    /// The atomic batch violated a backend-independent structural requirement.
    #[error(transparent)]
    InvalidBatch(#[from] BatchError),
    /// An atomic batch can never fit in one shard, regardless of expiry.
    #[error(
        "batch targets {key_count} distinct keys at memory-store shard {shard_index}, \
         but that shard can store at most {capacity}"
    )]
    BatchExceedsShardCapacity {
        /// Index of the shard that cannot hold all targeted keys.
        shard_index: usize,
        /// Number of distinct batch keys targeting the shard.
        key_count: usize,
        /// Hard key capacity of the shard.
        capacity: usize,
    },
    /// A touched shard was poisoned and remains unavailable.
    #[error(transparent)]
    PoisonedShard(#[from] PoisonedShardError),
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Barrier, Mutex, Weak,
            atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use runlimit_core::{
        AdmissionOperation, AdmissionOutcome, Allowance, BatchDecision, BatchDecisionView,
        BatchError, Capacity, Check, CleanupOutcome, ConsumptionStatus, Decision, DecisionView,
        Delay, Denial, FixedWindowPolicy, KeyHasher, MAX_LIMIT, MAX_WINDOW, MAX_WINDOW_MILLIS,
        Observation, Observer, PolicyId, QuotaDenial, QuotaMode, ScopeId, SubjectKey,
    };

    use super::{Entry, MemoryBatchError, MemoryStore, PoisonedShardError, remaining_quota};
    use crate::{Clock, MemoryStoreConfig, shards::counter_key_hash};

    fn capacity(value: u64) -> Capacity {
        Capacity::new(value).unwrap()
    }

    fn quota(capacity_value: u64, retry_after: Duration) -> QuotaDenial {
        QuotaDenial::new(capacity(capacity_value), retry_after)
    }

    fn allowance(capacity_value: u64, available: u64, replenishes_after: Duration) -> Allowance {
        Allowance::new(capacity(capacity_value), available, replenishes_after).unwrap()
    }

    /// Shapes a one-check batch into the decision its single check would have
    /// produced, so the single and batch paths can be compared exactly.
    fn single_decision(batch: &BatchDecision) -> Decision {
        match batch.view() {
            BatchDecisionView::Allowed {
                allowances: [allowance],
            } => Decision::allowed(*allowance),
            BatchDecisionView::Denied {
                index: 0,
                batch_size,
                denial,
            } if batch_size.get() == 1 => Decision::denied(denial),
            BatchDecisionView::ShadowDenied {
                index: 0,
                batch_size,
                denial,
            } if batch_size.get() == 1 => Decision::shadow_denied(denial),
            other => panic!("a one-check batch yields exactly one decision: {other:?}"),
        }
    }

    fn storage_capacity(retry_after: Option<Duration>) -> Denial {
        Denial::StorageCapacity {
            retry_after: retry_after.map(Delay::new),
        }
    }

    fn with_clock<C: Clock>(config: MemoryStoreConfig, clock: C) -> MemoryStore<C> {
        MemoryStore::builder(config).with_clock(clock).build()
    }

    #[derive(Clone, Default)]
    struct ManualClock {
        now_millis: Arc<AtomicU64>,
    }

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            let millis = u64::try_from(duration.as_millis()).unwrap();
            self.now_millis.fetch_add(millis, Ordering::Relaxed);
        }

        fn set(&self, duration: Duration) {
            let millis = u64::try_from(duration.as_millis()).unwrap();
            self.now_millis.store(millis, Ordering::Relaxed);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.now_millis.load(Ordering::Relaxed))
        }
    }

    #[derive(Clone, Default)]
    struct PanicOnceClock {
        panic_next: Arc<AtomicBool>,
    }

    impl PanicOnceClock {
        fn panic_once(&self) {
            self.panic_next.store(true, Ordering::Relaxed);
        }
    }

    impl Clock for PanicOnceClock {
        fn now(&self) -> Duration {
            assert!(
                !self.panic_next.swap(false, Ordering::Relaxed),
                "panic from test clock"
            );
            Duration::ZERO
        }
    }

    #[derive(Clone)]
    struct WideManualClock {
        now_millis: Arc<Mutex<u128>>,
    }

    impl WideManualClock {
        fn new(now_millis: u128) -> Self {
            Self {
                now_millis: Arc::new(Mutex::new(now_millis)),
            }
        }

        fn advance(&self, millis: u128) {
            let mut now_millis = self.now_millis.lock().unwrap();
            *now_millis += millis;
        }
    }

    impl Clock for WideManualClock {
        fn now(&self) -> Duration {
            let millis = *self.now_millis.lock().unwrap();
            let seconds = u64::try_from(millis / 1_000).unwrap();
            let subsec_millis = u32::try_from(millis % 1_000).unwrap();
            Duration::new(seconds, subsec_millis * 1_000_000)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedObservation {
        Admission(AdmissionOutcome, ConsumptionStatus, usize),
        Cleanup(usize, CleanupOutcome),
        Capacity(u64, u64, u64),
    }

    #[derive(Default)]
    struct RecordingObserver {
        observations: Mutex<Vec<RecordedObservation>>,
    }

    impl Observer for RecordingObserver {
        fn observe(&self, observation: &Observation<'_>) {
            let owned = match observation {
                Observation::Admission(admission) => RecordedObservation::Admission(
                    admission.outcome(),
                    admission.consumption(),
                    match admission.operation() {
                        AdmissionOperation::Check { .. } => 1,
                        AdmissionOperation::Batch { batch_size, .. } => batch_size,
                    },
                ),
                Observation::Cleanup(cleanup) => {
                    RecordedObservation::Cleanup(cleanup.requested(), cleanup.outcome())
                }
                Observation::Capacity(capacity) => RecordedObservation::Capacity(
                    capacity.used(),
                    capacity.capacity(),
                    capacity.headroom(),
                ),
            };
            self.observations.lock().unwrap().push(owned);
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedOperation {
        Check,
        Batch { batch_size: usize, has_policy: bool },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct RecordedAdmissionMetadata {
        operation: RecordedOperation,
        outcome: AdmissionOutcome,
        consumption: ConsumptionStatus,
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
                .push(RecordedAdmissionMetadata {
                    operation: match admission.operation() {
                        AdmissionOperation::Check { .. } => RecordedOperation::Check,
                        AdmissionOperation::Batch { batch_size, policy } => {
                            RecordedOperation::Batch {
                                batch_size,
                                has_policy: policy.is_some(),
                            }
                        }
                    },
                    outcome: admission.outcome(),
                    consumption: admission.consumption(),
                });
        }
    }

    struct ReentrantObserver {
        store: Mutex<Option<Weak<MemoryStore<ManualClock>>>>,
        calls: AtomicUsize,
    }

    impl Observer for ReentrantObserver {
        fn observe(&self, _observation: &Observation<'_>) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(store) = self.store.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                store.stats().unwrap();
            }
        }
    }

    struct PanickingObserver;

    impl Observer for PanickingObserver {
        fn observe(&self, _observation: &Observation<'_>) {
            panic!("observer failure");
        }
    }

    fn policy(policy_id: &str, scope_id: &str, limit: u64, window: Duration) -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new(policy_id).unwrap(),
            ScopeId::new(scope_id).unwrap(),
            limit,
            window,
        )
        .unwrap()
    }

    fn subject(byte: u8) -> SubjectKey {
        SubjectKey::from_digest([byte; 32])
    }

    fn subjects_for_shard<C: Clock>(
        store: &MemoryStore<C>,
        policy: &FixedWindowPolicy,
        shard_index: usize,
        count: usize,
    ) -> Vec<SubjectKey> {
        assert!(shard_index < store.shards.len());
        let mut subjects = Vec::with_capacity(count);

        for value in 0_u64..10_000 {
            let mut digest = [0_u8; 32];
            digest[..8].copy_from_slice(&value.to_le_bytes());
            let candidate = SubjectKey::from_digest(digest);
            let counter_key = Check::new(candidate.bind(policy)).counter_key();
            if store.shards.shard_index(&counter_key) == shard_index {
                subjects.push(candidate);
                if subjects.len() == count {
                    break;
                }
            }
        }

        assert_eq!(
            subjects.len(),
            count,
            "could not find enough subjects for shard {shard_index}"
        );
        subjects
    }

    #[test]
    fn hmac_subjects_are_evenly_and_deterministically_sharded() {
        const SHARD_COUNT: usize = 16;
        const SUBJECT_COUNT: u64 = 4_096;

        let config = MemoryStoreConfig::new(SHARD_COUNT)
            .unwrap()
            .with_shard_count(SHARD_COUNT)
            .unwrap();
        let first = with_clock(config.clone(), ManualClock::default());
        let second = with_clock(config, ManualClock::default());
        let policy = policy("auth.login", "client", 2, Duration::from_mins(1));
        let key_hasher = KeyHasher::new([0x42; 32]).unwrap();
        let mut counts = [0_usize; SHARD_COUNT];

        for value in 0_u64..SUBJECT_COUNT {
            let subject = key_hasher.hash_for(&policy, value.to_le_bytes());
            let key = Check::new(subject).counter_key();
            let first_index = first.shards.shard_index(&key);

            assert_eq!(first_index, second.shards.shard_index(&key));
            counts[first_index] += 1;
        }

        assert!(
            counts.iter().all(|&count| (176..=336).contains(&count)),
            "uniform HMAC output should spread across every shard: {counts:?}"
        );
    }

    #[test]
    fn the_entries_map_distinguishes_deliberate_prehash_collisions() {
        let first_policy = policy("auth.login", "client", 1, Duration::from_mins(1));
        let second_policy = policy("auth.reset", "client", 2, Duration::from_mins(1));
        let first_subject = SubjectKey::from_digest([0_u8; 32]);
        let first_key = Check::new(first_subject.bind(&first_policy)).counter_key();

        let zero_second_key =
            Check::new(SubjectKey::from_digest([0_u8; 32]).bind(&second_policy)).counter_key();
        let colliding_subject_word =
            counter_key_hash(first_key) ^ counter_key_hash(zero_second_key);
        let mut second_digest = [0_u8; 32];
        second_digest[..8].copy_from_slice(&colliding_subject_word.to_le_bytes());
        let second_subject = SubjectKey::from_digest(second_digest);
        let second_key = Check::new(second_subject.bind(&second_policy)).counter_key();

        assert_ne!(first_key, second_key);
        assert_eq!(counter_key_hash(first_key), counter_key_hash(second_key));

        let store = with_clock(MemoryStoreConfig::new(4).unwrap(), ManualClock::default());
        let first_check = Check::new(first_subject.bind(&first_policy));
        let second_check = Check::new(second_subject.bind(&second_policy));

        assert!(store.check(&first_check).unwrap().permits_request());
        assert!(store.check(&second_check).unwrap().permits_request());
        assert_eq!(store.stats().unwrap().entries(), 2);
        assert!(!store.check(&first_check).unwrap().permits_request());
        assert!(store.check(&second_check).unwrap().permits_request());
    }

    #[test]
    fn remaining_quota_reaches_zero_without_wrapping() {
        assert_eq!(remaining_quota(capacity(3), 0), 3);
        assert_eq!(remaining_quota(capacity(3), 3), 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "stored quota usage (4) exceeded its policy limit (3)")]
    fn corrupt_quota_state_trips_the_debug_invariant() {
        let _ = remaining_quota(capacity(3), 4);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn corrupt_quota_state_fails_closed_in_release_builds() {
        let policy = policy("auth.login", "client", 3, Duration::from_secs(60));
        let check = Check::new(subject(1).bind(&policy));
        let key = check.counter_key();
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        {
            let mut shard = store.shards.shard(0).lock().unwrap();
            assert!(shard.replace(key, Entry { used: 4 }, 60_000).is_none());
        }

        assert_eq!(
            store.check(&check),
            Ok(Decision::denied(quota(3, Duration::from_secs(60))))
        );
    }

    #[test]
    fn debug_output_is_useful_without_exposing_counter_state() {
        let store = with_clock(MemoryStoreConfig::new(8).unwrap(), ManualClock::default());
        let policy = policy("auth.login", "client", 2, Duration::from_mins(1));
        store
            .check(&Check::new(subject(0xab).bind(&policy)))
            .expect("store is available");

        let output = format!("{store:?}");

        assert!(output.starts_with("MemoryStore { config: MemoryStoreConfig"));
        assert!(output.contains("max_keys: 8"));
        assert!(output.contains("poisoned_shards: 0"));
        assert!(output.contains("shard_state: [REDACTED]"));
        assert!(!output.contains("CounterKey"));
        assert!(!output.contains("expires_at_millis"));
        assert!(!output.contains("used:"));
    }

    #[test]
    fn enforces_quota_and_reports_exact_retry_duration() {
        let clock = ManualClock::default();
        let store = with_clock(MemoryStoreConfig::new(8).unwrap(), clock.clone());
        let policy = policy("auth.login", "client", 2, Duration::from_mins(1));
        let check = Check::new(subject(1).bind(&policy));

        let first = store.check(&check).unwrap();
        assert_eq!(
            first,
            Decision::allowed(allowance(2, 1, Duration::from_mins(1)))
        );

        let second = store.check(&check).unwrap();
        assert_eq!(
            second,
            Decision::allowed(allowance(2, 0, Duration::from_mins(1)))
        );

        let denied = store.check(&check).unwrap();
        assert!(!denied.permits_request());
        assert_eq!(denied, Decision::denied(quota(2, Duration::from_mins(1))));

        clock.advance(Duration::from_secs(10));
        assert_eq!(
            store.check(&check).unwrap(),
            Decision::denied(quota(2, Duration::from_secs(50)))
        );

        clock.advance(Duration::from_secs(50));
        let reset = store.check(&check).unwrap();
        assert_eq!(
            reset,
            Decision::allowed(allowance(2, 1, Duration::from_mins(1)))
        );
    }

    #[test]
    fn shadow_mode_reports_quota_without_consuming_or_rejecting() {
        let clock = ManualClock::default();
        let store = with_clock(MemoryStoreConfig::new(4).unwrap(), clock.clone());
        let enforced = policy("auth.login", "client", 1, Duration::from_mins(1));
        let shadow = enforced.clone().with_quota_mode(QuotaMode::Shadow);
        let subject = subject(7);

        assert!(
            store
                .check(&Check::new(subject.bind(&shadow)))
                .unwrap()
                .permits_request()
        );
        let shadow_denial = store.check(&Check::new(subject.bind(&shadow))).unwrap();
        assert!(shadow_denial.permits_request());
        assert_eq!(
            shadow_denial,
            Decision::shadow_denied(quota(1, Duration::from_mins(1)))
        );

        let enforced_denial = store.check(&Check::new(subject.bind(&enforced))).unwrap();
        assert!(!enforced_denial.permits_request());
        assert_eq!(
            enforced_denial,
            Decision::denied(quota(1, Duration::from_mins(1)))
        );

        clock.advance(Duration::from_mins(1));
        assert!(
            store
                .check(&Check::new(subject.bind(&enforced)))
                .unwrap()
                .permits_request()
        );
    }

    #[test]
    fn shadow_batches_roll_back_and_capacity_denials_remain_enforced() {
        let store = with_clock(MemoryStoreConfig::new(3).unwrap(), ManualClock::default());
        let shadow = policy("auth.login", "client", 1, Duration::from_mins(1))
            .with_quota_mode(QuotaMode::Shadow);
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
            "a shadow-denied atomic batch must not consume another check"
        );

        let full_store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default());
        assert!(full_store.check(&first).unwrap().permits_request());
        let capacity = full_store.check(&second).unwrap();
        assert!(matches!(
            capacity.view(),
            DecisionView::Denied {
                denial: Denial::StorageCapacity { .. },
            }
        ));
    }

    #[test]
    fn mixed_shadow_and_enforced_batches_fail_before_consumption() {
        let store = with_clock(MemoryStoreConfig::new(4).unwrap(), ManualClock::default());
        let enforced = policy("auth.login", "client", 1, Duration::from_mins(1));
        let shadow = policy("auth.reset", "client", 1, Duration::from_mins(1))
            .with_quota_mode(QuotaMode::Shadow);
        let result = store.check_all(&[
            Check::new(subject(1).bind(&enforced)),
            Check::new(subject(2).bind(&shadow)),
        ]);

        assert!(matches!(
            result,
            Err(MemoryBatchError::InvalidBatch(
                BatchError::MixedQuotaModes { index: 1, .. }
            ))
        ));
        assert_eq!(store.stats().unwrap().entries(), 0);
    }

    #[test]
    fn observer_reports_outcomes_cleanup_and_capacity_headroom() {
        let observer = Arc::new(RecordingObserver::default());
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default())
            .with_observer(observer.clone());
        let limited = policy("auth.login", "client", 1, Duration::from_mins(1));

        assert!(
            store
                .check(&Check::new(subject(1).bind(&limited)))
                .unwrap()
                .permits_request()
        );
        assert!(
            !store
                .check(&Check::new(subject(1).bind(&limited)))
                .unwrap()
                .permits_request()
        );
        assert!(
            !store
                .check(&Check::new(subject(2).bind(&limited)))
                .unwrap()
                .permits_request()
        );

        let observations = observer.observations.lock().unwrap().clone();
        assert_eq!(observations.len(), 9);
        assert!(observations.contains(&RecordedObservation::Admission(
            AdmissionOutcome::Allowed,
            ConsumptionStatus::Consumed,
            1,
        )));
        assert!(observations.contains(&RecordedObservation::Admission(
            AdmissionOutcome::QuotaDenied,
            ConsumptionStatus::NotConsumed,
            1,
        )));
        assert!(observations.contains(&RecordedObservation::Admission(
            AdmissionOutcome::CapacityDenied,
            ConsumptionStatus::NotConsumed,
            1,
        )));
        assert_eq!(
            observations
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        RecordedObservation::Cleanup(8, CleanupOutcome::Confirmed { removed: 0 })
                    )
                })
                .count(),
            3
        );
        assert_eq!(
            observations
                .iter()
                .filter(|event| matches!(event, RecordedObservation::Capacity(1, 1, 0)))
                .count(),
            3
        );
    }

    #[test]
    fn observers_run_after_unlock_and_panics_are_isolated() {
        let observer = Arc::new(ReentrantObserver {
            store: Mutex::new(None),
            calls: AtomicUsize::new(0),
        });
        let store = Arc::new(
            with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default())
                .with_observer(observer.clone()),
        );
        *observer.store.lock().unwrap() = Some(Arc::downgrade(&store));
        let limited = policy("auth.login", "client", 1, Duration::from_mins(1));
        let check = Check::new(subject(1).bind(&limited));

        assert!(store.check(&check).unwrap().permits_request());
        assert_eq!(observer.calls.load(Ordering::Relaxed), 3);

        let panicking_store =
            with_clock(MemoryStoreConfig::new(1).unwrap(), ManualClock::default())
                .with_observer(Arc::new(PanickingObserver));
        assert!(panicking_store.check(&check).unwrap().permits_request());
        assert!(!panicking_store.check(&check).unwrap().permits_request());
    }

    #[test]
    fn single_check_matches_a_one_element_batch_across_state_transitions() {
        #[derive(Clone, Copy)]
        struct Step {
            now: Duration,
            subject: u8,
            cost: u64,
            expected: Decision,
        }

        let config = MemoryStoreConfig::new(2)
            .unwrap()
            .with_shard_count(1)
            .unwrap()
            .with_max_expired_removals_per_check(1)
            .unwrap();
        let single_clock = ManualClock::default();
        let batch_clock = ManualClock::default();
        let single_store = with_clock(config.clone(), single_clock.clone());
        let batch_store = with_clock(config, batch_clock.clone());
        let policy = policy("auth.login", "client", 5, Duration::from_millis(10));
        let steps = [
            Step {
                now: Duration::ZERO,
                subject: 1,
                cost: 2,
                expected: Decision::allowed(allowance(5, 3, Duration::from_millis(10))),
            },
            Step {
                now: Duration::from_millis(3),
                subject: 1,
                cost: 2,
                expected: Decision::allowed(allowance(5, 1, Duration::from_millis(7))),
            },
            Step {
                now: Duration::from_millis(3),
                subject: 1,
                cost: 2,
                expected: Decision::denied(quota(5, Duration::from_millis(7))),
            },
            Step {
                now: Duration::from_millis(9),
                subject: 1,
                cost: 1,
                expected: Decision::allowed(allowance(5, 0, Duration::from_millis(1))),
            },
            Step {
                now: Duration::from_millis(10),
                subject: 1,
                cost: 3,
                expected: Decision::allowed(allowance(5, 2, Duration::from_millis(10))),
            },
            Step {
                now: Duration::from_millis(8),
                subject: 1,
                cost: 1,
                expected: Decision::allowed(allowance(5, 1, Duration::from_millis(10))),
            },
            Step {
                now: Duration::from_millis(10),
                subject: 2,
                cost: 1,
                expected: Decision::allowed(allowance(5, 4, Duration::from_millis(10))),
            },
            Step {
                now: Duration::from_millis(10),
                subject: 3,
                cost: 1,
                expected: Decision::denied(storage_capacity(Some(Duration::from_millis(10)))),
            },
            Step {
                now: Duration::from_millis(20),
                subject: 3,
                cost: 1,
                expected: Decision::allowed(allowance(5, 4, Duration::from_millis(10))),
            },
        ];

        for (index, step) in steps.into_iter().enumerate() {
            single_clock.set(step.now);
            batch_clock.set(step.now);
            let check = Check::new(subject(step.subject).bind(&policy))
                .with_cost(step.cost)
                .unwrap();

            let single = single_store.check(&check).unwrap();
            let batch =
                single_decision(&batch_store.check_all(std::slice::from_ref(&check)).unwrap());

            assert_eq!(single, batch, "step {index}");
            assert_eq!(single, step.expected, "step {index}");
        }

        assert_eq!(single_store.stats().unwrap(), batch_store.stats().unwrap());
    }

    #[test]
    fn windows_remain_exact_across_the_u64_millisecond_boundary() {
        let clock = WideManualClock::new(u128::from(u64::MAX));
        let store = with_clock(MemoryStoreConfig::new(1).unwrap(), clock.clone());
        let policy = policy("auth.login", "client", MAX_LIMIT, MAX_WINDOW);
        let check = Check::new(subject(1).bind(&policy))
            .with_cost(MAX_LIMIT)
            .unwrap();

        let first = store.check(&check).unwrap();
        assert_eq!(
            first,
            Decision::allowed(allowance(MAX_LIMIT, 0, MAX_WINDOW))
        );

        let denied = store.check(&check).unwrap();
        assert_eq!(denied, Decision::denied(quota(MAX_LIMIT, MAX_WINDOW)));

        clock.advance(u128::from(MAX_WINDOW_MILLIS - 1));
        let nearly_reset = store.check(&check).unwrap();
        assert_eq!(
            nearly_reset,
            Decision::denied(quota(MAX_LIMIT, Duration::from_millis(1)))
        );

        clock.advance(1);
        let reset = store.check(&check).unwrap();
        assert_eq!(
            reset,
            Decision::allowed(allowance(MAX_LIMIT, 0, MAX_WINDOW))
        );
    }

    #[test]
    fn policy_configuration_changes_use_independent_counters() {
        let store = with_clock(
            MemoryStoreConfig::new(2)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            ManualClock::default(),
        );
        let strict = policy("auth.login", "client", 1, Duration::from_mins(1));
        let relaxed = policy("auth.login", "client", 2, Duration::from_mins(1));
        let key = subject(1);

        assert!(
            store
                .check(&Check::new(key.bind(&strict)))
                .unwrap()
                .permits_request()
        );
        assert!(
            store
                .check(&Check::new(key.bind(&relaxed)))
                .unwrap()
                .permits_request()
        );
        assert_eq!(store.stats().unwrap().entries(), 2);
    }

    #[test]
    fn capacity_exhaustion_does_not_evict_an_active_entry() {
        let store = with_clock(
            MemoryStoreConfig::new(1)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            ManualClock::default(),
        );
        let policy = policy("auth.login", "client", 10, Duration::from_mins(1));

        assert!(
            store
                .check(&Check::new(subject(1).bind(&policy)))
                .unwrap()
                .permits_request()
        );
        let denied = store.check(&Check::new(subject(2).bind(&policy))).unwrap();
        assert_eq!(
            denied,
            Decision::denied(storage_capacity(Some(Duration::from_mins(1))))
        );
        assert_eq!(store.stats().unwrap().entries(), 1);
    }

    #[test]
    fn default_configuration_can_use_its_entire_capacity() {
        let store = with_clock(MemoryStoreConfig::new(64).unwrap(), ManualClock::default());
        let policy = policy("auth.login", "client", 10, Duration::from_mins(1));

        for byte in 0..64 {
            assert!(
                store
                    .check(&Check::new(subject(byte).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }

        let denied = store.check(&Check::new(subject(64).bind(&policy))).unwrap();
        assert!(!denied.permits_request());
        assert_eq!(store.stats().unwrap().entries(), 64);
    }

    #[test]
    fn a_cross_shard_batch_preserves_input_order_and_updates_each_shard() {
        let store = with_clock(
            MemoryStoreConfig::new(4)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            ManualClock::default(),
        );
        let policy = policy("auth.login", "client", 10, Duration::from_mins(1));
        let shard_zero = subjects_for_shard(&store, &policy, 0, 1)[0];
        let shard_one = subjects_for_shard(&store, &policy, 1, 2);
        let checks = [
            Check::new(shard_one[0].bind(&policy)).with_cost(2).unwrap(),
            Check::new(shard_zero.bind(&policy)).with_cost(3).unwrap(),
            Check::new(shard_one[1].bind(&policy)).with_cost(4).unwrap(),
        ];

        assert_eq!(
            store.check_all(&checks),
            Ok(BatchDecision::allowed(vec![
                allowance(10, 8, Duration::from_mins(1)),
                allowance(10, 7, Duration::from_mins(1)),
                allowance(10, 6, Duration::from_mins(1)),
            ])
            .unwrap())
        );
        assert_eq!(store.shards.shard(0).lock().unwrap().used(), 1);
        assert_eq!(store.shards.shard(1).lock().unwrap().used(), 2);
    }

    #[test]
    fn a_full_shard_denies_a_batch_while_another_shard_has_room() {
        let store = with_clock(
            MemoryStoreConfig::new(6)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            ManualClock::default(),
        );
        let policy = policy("auth.login", "client", 10, Duration::from_mins(1));
        let crowded = subjects_for_shard(&store, &policy, 0, 4);
        let other = subjects_for_shard(&store, &policy, 1, 1)[0];

        for key in &crowded[..2] {
            assert!(
                store
                    .check(&Check::new((*key).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }
        let checks = [
            Check::new(other.bind(&policy)),
            Check::new(crowded[2].bind(&policy)),
            Check::new(crowded[3].bind(&policy)),
        ];

        assert_eq!(
            store.check_all(&checks),
            Ok(
                BatchDecision::denied(2, 3, storage_capacity(Some(Duration::from_mins(1))))
                    .unwrap()
            )
        );
        assert_eq!(store.shards.shard(0).lock().unwrap().used(), 2);
        assert_eq!(store.shards.shard(1).lock().unwrap().used(), 0);

        assert!(
            store
                .check(&Check::new(crowded[2].bind(&policy)))
                .unwrap()
                .permits_request()
        );
        assert!(
            store
                .check(&Check::new(other.bind(&policy)))
                .unwrap()
                .permits_request()
        );
    }

    #[test]
    fn an_unsatisfiable_per_shard_batch_returns_a_structural_error() {
        let store = with_clock(
            MemoryStoreConfig::new(4)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            ManualClock::default(),
        );
        let policy = policy("auth.login", "client", 2, Duration::from_mins(1));
        let keys = subjects_for_shard(&store, &policy, 0, 3);
        let other = subjects_for_shard(&store, &policy, 1, 1)[0];
        let other_check = Check::new(other.bind(&policy));
        assert_eq!(
            store.check(&other_check),
            Ok(Decision::allowed(allowance(2, 1, Duration::from_mins(1))))
        );
        let checks = [
            other_check,
            Check::new(keys[0].bind(&policy)),
            Check::new(keys[1].bind(&policy)),
            Check::new(keys[2].bind(&policy)),
        ];

        assert_eq!(
            store.check_all(&checks),
            Err(MemoryBatchError::BatchExceedsShardCapacity {
                shard_index: 0,
                key_count: 3,
                capacity: 2,
            })
        );
        assert_eq!(store.stats().unwrap().entries(), 1);
        assert_eq!(
            store.check(&other_check),
            Ok(Decision::allowed(allowance(2, 0, Duration::from_mins(1))))
        );
    }

    #[test]
    fn expiry_cleanup_is_bounded_and_capacity_remains_hard_bounded() {
        let clock = ManualClock::default();
        let config = MemoryStoreConfig::new(3)
            .unwrap()
            .with_shard_count(1)
            .unwrap()
            .with_max_expired_removals_per_check(1)
            .unwrap();
        let store = with_clock(config, clock.clone());
        let policy = policy("auth.login", "client", 10, Duration::from_secs(1));

        for byte in 1..=3 {
            assert!(
                store
                    .check(&Check::new(subject(byte).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }
        clock.advance(Duration::from_secs(1));

        assert!(
            store
                .check(&Check::new(subject(4).bind(&policy)))
                .unwrap()
                .permits_request()
        );
        assert_eq!(store.stats().unwrap().entries(), 3);

        assert!(
            store
                .check(&Check::new(subject(5).bind(&policy)))
                .unwrap()
                .permits_request()
        );
        assert_eq!(store.stats().unwrap().entries(), 3);
    }

    #[test]
    fn a_cross_shard_batch_attributes_cleanup_to_each_checks_target() {
        let clock = ManualClock::default();
        let store = with_clock(
            MemoryStoreConfig::new(8)
                .unwrap()
                .with_shard_count(2)
                .unwrap()
                .with_max_expired_removals_per_check(1)
                .unwrap(),
            clock.clone(),
        );
        let policy = policy("auth.login", "client", 10, Duration::from_secs(1));
        let shard_zero = subjects_for_shard(&store, &policy, 0, 6);
        let shard_one = subjects_for_shard(&store, &policy, 1, 5);

        for key in shard_zero[..4].iter().chain(&shard_one[..4]) {
            assert!(
                store
                    .check(&Check::new((*key).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }
        clock.advance(Duration::from_secs(1));

        let checks = [
            Check::new(shard_zero[4].bind(&policy)),
            Check::new(shard_one[4].bind(&policy)),
            Check::new(shard_zero[5].bind(&policy)),
        ];
        let cleanup_batch = store.check_all(&checks).unwrap();
        assert!(
            matches!(
                cleanup_batch.view(),
                BatchDecisionView::Allowed { allowances } if allowances.len() == 3
            ),
            "the two checks targeting shard 0 need two cleanup slots: {cleanup_batch:?}"
        );

        let shard = store.shards.shard(0).lock().unwrap();
        assert_eq!(shard.used(), 4);
        assert_eq!(shard.expiration_count_at_or_before(1_000), 2);
        drop(shard);

        let shard = store.shards.shard(1).lock().unwrap();
        assert_eq!(shard.used(), 4);
        assert_eq!(shard.expiration_count_at_or_before(1_000), 3);
    }

    #[test]
    fn an_atomic_batch_receives_the_cleanup_budget_for_each_check() {
        let clock = ManualClock::default();
        let config = MemoryStoreConfig::new(3)
            .unwrap()
            .with_shard_count(1)
            .unwrap()
            .with_max_expired_removals_per_check(1)
            .unwrap();
        let store = with_clock(config, clock.clone());
        let policy = policy("auth.login", "client", 10, Duration::from_secs(1));

        for byte in 1..=3 {
            assert!(
                store
                    .check(&Check::new(subject(byte).bind(&policy)))
                    .unwrap()
                    .permits_request()
            );
        }
        clock.advance(Duration::from_secs(1));

        let checks = [4, 5, 6].map(|byte| Check::new(subject(byte).bind(&policy)));
        let result = store.check_all(&checks).unwrap();
        assert!(matches!(
            result.view(),
            BatchDecisionView::Allowed { allowances } if allowances.len() == 3
        ));
        assert_eq!(store.stats().unwrap().entries(), 3);
    }

    #[test]
    fn auxiliary_expiry_index_remains_bounded_across_many_windows() {
        let clock = ManualClock::default();
        let store = with_clock(
            MemoryStoreConfig::new(1)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            clock.clone(),
        );
        let policy = policy("auth.login", "client", 1, Duration::from_millis(1));
        let check = Check::new(subject(1).bind(&policy));

        for _ in 0..1_000 {
            assert!(store.check(&check).unwrap().permits_request());
            clock.advance(Duration::from_millis(1));
        }

        let shard = store.shards.shard(0).lock().unwrap();
        assert_eq!(shard.used(), 1);
        assert_eq!(shard.expiration_count(), 1);
    }

    #[test]
    fn targeted_expired_refresh_keeps_indexes_aligned_for_single_and_batch() {
        for use_batch in [false, true] {
            let clock = ManualClock::default();
            let store = with_clock(
                MemoryStoreConfig::new(2)
                    .unwrap()
                    .with_shard_count(1)
                    .unwrap()
                    .with_max_expired_removals_per_check(1)
                    .unwrap(),
                clock.clone(),
            );
            let policy = policy("auth.login", "client", 10, Duration::from_millis(2));
            let earlier = Check::new(subject(1).bind(&policy));
            let target = Check::new(subject(2).bind(&policy));

            assert!(store.check(&earlier).unwrap().permits_request());
            clock.advance(Duration::from_millis(1));
            assert!(store.check(&target).unwrap().permits_request());
            clock.advance(Duration::from_millis(2));

            let decision = if use_batch {
                single_decision(&store.check_all(std::slice::from_ref(&target)).unwrap())
            } else {
                store.check(&target).unwrap()
            };
            assert_eq!(
                decision,
                Decision::allowed(allowance(10, 9, Duration::from_millis(2)))
            );

            let key = target.counter_key();
            let shard = store.shards.shard(0).lock().unwrap();
            assert_eq!(shard.used(), 1);
            assert_eq!(shard.expiration_count(), 1);
            assert_eq!(shard.entry(key), Some((&Entry { used: 1 }, 5)));
            assert!(shard.contains_expiration(key, 5));
        }
    }

    #[test]
    fn a_denied_batch_does_not_consume_other_checks() {
        let store = with_clock(
            MemoryStoreConfig::new(8)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            ManualClock::default(),
        );
        let exhausted = policy("auth.login", "client", 1, Duration::from_mins(1));
        let untouched = policy("auth.login", "identity", 1, Duration::from_mins(1));
        let exhausted_check = Check::new(subject(1).bind(&exhausted));
        let untouched_check = Check::new(subject(2).bind(&untouched));

        assert!(store.check(&exhausted_check).unwrap().permits_request());
        let result = store
            .check_all(&[exhausted_check, untouched_check])
            .unwrap();
        assert!(matches!(
            result.view(),
            BatchDecisionView::Denied {
                index: 0,
                batch_size,
                ..
            } if batch_size.get() == 2
        ));

        assert!(
            store
                .check(&Check::new(subject(2).bind(&untouched)))
                .unwrap()
                .permits_request()
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_before_consumption() {
        let store = with_clock(MemoryStoreConfig::new(8).unwrap(), ManualClock::default());
        let alpha = policy("auth.alpha", "client", 2, Duration::from_mins(1));
        let beta = policy("auth.beta", "client", 2, Duration::from_mins(1));
        let checks = [
            Check::new(subject(1).bind(&beta)),
            Check::new(subject(1).bind(&beta)),
            Check::new(subject(2).bind(&alpha)),
            Check::new(subject(2).bind(&alpha)),
        ];

        assert_eq!(
            store.check_all(&checks),
            Err(MemoryBatchError::InvalidBatch(BatchError::DuplicateKey {
                first_index: 0,
                duplicate_index: 1,
            }))
        );
        assert_eq!(store.stats().unwrap().entries(), 0);
    }

    #[test]
    fn concurrent_checks_never_over_admit() {
        let store = Arc::new(with_clock(
            MemoryStoreConfig::new(8).unwrap(),
            ManualClock::default(),
        ));
        let policy = Arc::new(policy("auth.login", "client", 25, Duration::from_mins(1)));

        let allowed = (0..100)
            .map(|_| {
                let store = Arc::clone(&store);
                let policy = Arc::clone(&policy);
                thread::spawn(move || {
                    store
                        .check(&Check::new(subject(1).bind(&policy)))
                        .unwrap()
                        .permits_request()
                })
            })
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>();

        assert_eq!(allowed, 25);
    }

    #[test]
    fn concurrent_cross_shard_batches_lock_consistently_and_never_over_admit() {
        const THREAD_COUNT: usize = 8;
        const CALLS_PER_THREAD: usize = 64;

        let store = with_clock(
            MemoryStoreConfig::new(2)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            ManualClock::default(),
        );
        let limit = u64::try_from(THREAD_COUNT * CALLS_PER_THREAD).unwrap();
        let policy = policy("auth.login", "client", limit, Duration::from_mins(1));
        let first = subjects_for_shard(&store, &policy, 0, 1)[0];
        let second = subjects_for_shard(&store, &policy, 1, 1)[0];
        let store = Arc::new(store);
        let policy = Arc::new(policy);
        let barrier = Arc::new(Barrier::new(THREAD_COUNT));
        let (finished_sender, finished_receiver) = mpsc::channel();

        let threads = (0..THREAD_COUNT)
            .map(|thread_index| {
                let store = Arc::clone(&store);
                let policy = Arc::clone(&policy);
                let barrier = Arc::clone(&barrier);
                let finished_sender = finished_sender.clone();
                thread::spawn(move || {
                    for call_index in 0..CALLS_PER_THREAD {
                        barrier.wait();
                        let subjects = if (thread_index + call_index) % 2 == 0 {
                            [first, second]
                        } else {
                            [second, first]
                        };
                        let checks =
                            subjects.map(|subject| Check::new(subject.bind(policy.as_ref())));
                        let result = store.check_all(&checks).unwrap();
                        assert!(
                            matches!(
                                result.view(),
                                BatchDecisionView::Allowed { allowances }
                                    if allowances.len() == checks.len()
                            ),
                            "every call through the configured limit must be allowed: {result:?}"
                        );
                    }
                    finished_sender.send(()).unwrap();
                })
            })
            .collect::<Vec<_>>();
        drop(finished_sender);

        for _ in 0..THREAD_COUNT {
            finished_receiver
                .recv_timeout(Duration::from_secs(5))
                .expect("cross-shard batches must not deadlock");
        }
        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(store.stats().unwrap().entries(), 2);
        for key in [first, second] {
            assert_eq!(
                store.check(&Check::new(key.bind(policy.as_ref()))),
                Ok(Decision::denied(quota(limit, Duration::from_mins(1))))
            );
        }
    }

    #[test]
    fn a_poisoned_default_shard_remains_fail_closed() {
        let clock = PanicOnceClock::default();
        let store = with_clock(
            MemoryStoreConfig::new(1)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            clock.clone(),
        );
        let policy = policy("auth.login", "client", 1, Duration::from_mins(1));
        let exhausted = Check::new(subject(1).bind(&policy));
        let new_key = Check::new(subject(2).bind(&policy));

        assert!(store.check(&exhausted).unwrap().permits_request());
        clock.panic_once();
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.check(&new_key);
        }));
        assert!(panic_result.is_err());

        let poisoned = Err(PoisonedShardError { shard_index: 0 });
        assert_eq!(store.check(&exhausted), poisoned);
        assert_eq!(store.check(&new_key), poisoned);
        assert_eq!(store.check(&exhausted), poisoned);
        assert_eq!(store.stats(), Err(PoisonedShardError { shard_index: 0 }));
        assert_eq!(store.clear(), Err(PoisonedShardError { shard_index: 0 }));
    }

    #[test]
    fn batch_preflight_preserves_validation_capacity_and_lock_precedence() {
        let clock = PanicOnceClock::default();
        let store = with_clock(
            MemoryStoreConfig::new(1)
                .unwrap()
                .with_shard_count(1)
                .unwrap()
                .with_max_batch_size(2)
                .unwrap(),
            clock.clone(),
        );
        let policy = policy("auth.preflight", "client", 1, Duration::from_mins(1));
        let checks = [
            Check::new(subject(1).bind(&policy)),
            Check::new(subject(2).bind(&policy)),
            Check::new(subject(3).bind(&policy)),
        ];

        clock.panic_once();
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.check(&checks[0]);
        }));
        assert!(panic_result.is_err());

        assert_eq!(
            store.check_all(&[]),
            Err(MemoryBatchError::InvalidBatch(BatchError::EmptyBatch))
        );
        assert_eq!(
            store.check_all(&[checks[0], checks[0]]),
            Err(MemoryBatchError::InvalidBatch(BatchError::DuplicateKey {
                first_index: 0,
                duplicate_index: 1,
            }))
        );
        assert_eq!(
            store.check_all(&checks),
            Err(MemoryBatchError::InvalidBatch(BatchError::BatchTooLarge {
                actual: 3,
                maximum: 2,
            }))
        );
        assert_eq!(
            store.check_all(&checks[..2]),
            Err(MemoryBatchError::BatchExceedsShardCapacity {
                shard_index: 0,
                key_count: 2,
                capacity: 1,
            })
        );
        assert_eq!(
            store.check_all(&checks[..1]),
            Err(MemoryBatchError::PoisonedShard(PoisonedShardError {
                shard_index: 0
            }))
        );
    }

    #[test]
    fn failed_checks_and_one_check_batches_keep_policy_metadata() {
        let clock = PanicOnceClock::default();
        let observer = Arc::new(AdmissionMetadataObserver::default());
        let store = with_clock(
            MemoryStoreConfig::new(1)
                .unwrap()
                .with_shard_count(1)
                .unwrap(),
            clock.clone(),
        )
        .with_observer(observer.clone());
        let policy = policy("auth.observed-failure", "client", 1, Duration::from_mins(1));
        let check = Check::new(subject(1).bind(&policy));

        clock.panic_once();
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.check(&check);
        }));
        assert!(panic_result.is_err());
        assert_eq!(
            store.check(&check),
            Err(PoisonedShardError { shard_index: 0 })
        );
        assert_eq!(
            store.check_all(&[check]),
            Err(MemoryBatchError::PoisonedShard(PoisonedShardError {
                shard_index: 0
            }))
        );

        assert_eq!(
            observer.admissions.lock().unwrap().as_slice(),
            [
                RecordedAdmissionMetadata {
                    operation: RecordedOperation::Check,
                    outcome: AdmissionOutcome::Failed,
                    consumption: ConsumptionStatus::NotConsumed,
                },
                RecordedAdmissionMetadata {
                    operation: RecordedOperation::Batch {
                        batch_size: 1,
                        has_policy: true,
                    },
                    outcome: AdmissionOutcome::Failed,
                    consumption: ConsumptionStatus::NotConsumed,
                },
            ]
        );
    }

    #[test]
    fn explicit_recovery_resets_only_poisoned_shards_behind_an_arc() {
        let clock = PanicOnceClock::default();
        let store = Arc::new(with_clock(
            MemoryStoreConfig::new(4)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            clock.clone(),
        ));
        let policy = policy("auth.login", "client", 1, Duration::from_mins(1));
        let reset_subject = subjects_for_shard(&store, &policy, 0, 1)[0];
        let retained_subject = subjects_for_shard(&store, &policy, 1, 1)[0];
        let reset_check = Check::new(reset_subject.bind(&policy));
        let retained_check = Check::new(retained_subject.bind(&policy));

        assert!(store.check(&reset_check).unwrap().permits_request());
        assert!(store.check(&retained_check).unwrap().permits_request());
        clock.panic_once();
        let panicked_store = Arc::clone(&store);
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ = panicked_store.check(&reset_check);
        }));
        assert!(panic_result.is_err());

        assert_eq!(
            store.check(&reset_check),
            Err(PoisonedShardError { shard_index: 0 })
        );
        assert!(!store.check(&retained_check).unwrap().permits_request());
        assert!(format!("{store:?}").contains("poisoned_shards: 1"));

        assert_eq!(store.recover_poisoned(), 1);
        assert_eq!(
            store.recover_poisoned(),
            0,
            "recovery must be idempotent when no new panic occurs"
        );
        assert_eq!(store.stats().unwrap().entries(), 1);
        assert!(
            store.check(&reset_check).unwrap().permits_request(),
            "the recovered shard starts with empty quota state"
        );
        assert!(
            !store.check(&retained_check).unwrap().permits_request(),
            "healthy-shard quota state must survive recovery"
        );
        assert!(format!("{store:?}").contains("poisoned_shards: 0"));
    }

    #[test]
    fn recovery_reopens_every_shard_poisoned_by_an_unwinding_batch() {
        let clock = PanicOnceClock::default();
        let store = Arc::new(with_clock(
            MemoryStoreConfig::new(2)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
            clock.clone(),
        ));
        let policy = policy("auth.login", "client", 1, Duration::from_mins(1));
        let shard_zero = subjects_for_shard(&store, &policy, 0, 1)[0];
        let shard_one = subjects_for_shard(&store, &policy, 1, 1)[0];
        let checks = [
            Check::new(shard_one.bind(&policy)),
            Check::new(shard_zero.bind(&policy)),
        ];

        clock.panic_once();
        let panicked_store = Arc::clone(&store);
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _ = panicked_store.check_all(&checks);
        }));
        assert!(panic_result.is_err());
        assert_eq!(
            store.check(&checks[0]),
            Err(PoisonedShardError { shard_index: 1 })
        );
        assert_eq!(
            store.check(&checks[1]),
            Err(PoisonedShardError { shard_index: 0 })
        );

        assert_eq!(store.recover_poisoned(), 2);
        assert_eq!(store.stats().unwrap().entries(), 0);
        let recovered = store.check_all(&checks).unwrap();
        assert!(
            matches!(
                recovered.view(),
                BatchDecisionView::Allowed { allowances } if allowances.len() == 2
            ),
            "all shards must be usable after one atomic recovery: {recovered:?}"
        );
    }
}
