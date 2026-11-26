use runlimit_core::{
    BatchDecision, BatchError, Check, Denial, QuotaMode, RateLimitPolicy, validate_batch,
};

use super::{BatchSessionError, QuotaAlgorithm};
use crate::{
    Clock, MemoryBatchError,
    shards::{BatchTopology, BoundedShards, LockedShards, ShardEffect},
};

mod preflight;
#[cfg(test)]
pub(super) use preflight::PreflightedBatch;
#[cfg(test)]
mod tests;

pub(crate) enum BatchEffects {
    Disabled,
    Recorded(Vec<ShardEffect>),
}

// Arithmetic must succeed before the clock watermark or expiration index changes.
pub(crate) trait BatchAlgorithm: QuotaAlgorithm + Sized {
    type Arithmetic;
    type Writes;

    fn check_arithmetic(
        locked: &LockedBatch<'_, Self>,
    ) -> Result<Self::Arithmetic, Self::EvalError>;

    fn preflight_and_commit(
        session: BatchSession<'_, Self>,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError>;
}

struct ValidatedBatch<'a, P: RateLimitPolicy> {
    checks: &'a [Check<'a, P>],
    quota_mode: QuotaMode,
}

impl<'a, P: RateLimitPolicy> ValidatedBatch<'a, P> {
    fn new(checks: &'a [Check<'a, P>], max_batch_size: usize) -> Result<Self, BatchError> {
        validate_batch(checks, max_batch_size)?;
        let first = checks.first().ok_or(BatchError::EmptyBatch)?;
        Ok(Self {
            checks,
            quota_mode: first.policy().quota_mode(),
        })
    }

    fn denied(&self, index: usize, denial: Denial) -> BatchDecision {
        match (self.quota_mode, denial) {
            (QuotaMode::Shadow, Denial::QuotaExceeded(quota)) => {
                BatchDecision::shadow_denied(index, self.checks.len(), quota)
            }
            (QuotaMode::Enforce, Denial::QuotaExceeded(_))
            | (_, Denial::StorageCapacity { .. }) => {
                BatchDecision::denied(index, self.checks.len(), denial)
            }
        }
        .expect("a batch check's input index is below the batch size")
    }
}

pub(crate) struct LockedBatch<'a, A: QuotaAlgorithm> {
    batch: ValidatedBatch<'a, A::Policy>,
    topology: BatchTopology,
    shards: LockedShards<'a, A::Entry>,
    now_millis: u128,
    cleanup_per_check: usize,
}

impl<'a, A: QuotaAlgorithm> LockedBatch<'a, A> {
    pub(super) fn lock<C: Clock>(
        shards: &'a BoundedShards<A::Entry>,
        checks: &'a [Check<'a, A::Policy>],
        clock: &C,
    ) -> Result<Self, MemoryBatchError> {
        let batch = ValidatedBatch::new(checks, shards.config().max_batch_size())?;
        let (topology, locked) = shards.lock_batch(batch.checks.iter().map(Check::counter_key))?;
        let observed_millis = clock.now().as_millis();
        let now_millis = locked
            .iter()
            .map(|(_, shard)| shard.latest_observed_millis())
            .fold(observed_millis, u128::max);
        Ok(Self {
            batch,
            topology,
            shards: locked,
            now_millis,
            cleanup_per_check: shards.config().max_expired_removals_per_check(),
        })
    }
}

impl<A: BatchAlgorithm> LockedBatch<'_, A> {
    pub(super) fn run(
        mut self,
        record_effects: bool,
    ) -> Result<(BatchDecision, BatchEffects), BatchSessionError<A::EvalError>> {
        let arithmetic = A::check_arithmetic(&self).map_err(BatchSessionError::Evaluation)?;
        let mut effects = if record_effects {
            BatchEffects::Recorded(Vec::with_capacity(self.shards.len()))
        } else {
            BatchEffects::Disabled
        };
        for ((shard_index, shard), count) in
            self.shards.iter_mut().zip(self.topology.check_counts())
        {
            let limit = self.cleanup_per_check.saturating_mul(count);
            let cleanup = shard.advance_and_prune(self.now_millis, limit, record_effects);
            if let BatchEffects::Recorded(recorded) = &mut effects {
                recorded.push(ShardEffect {
                    shard_index: *shard_index,
                    cleanup,
                    used: shard.used_entries(),
                    capacity: shard.capacity(),
                });
            }
        }
        A::preflight_and_commit(BatchSession {
            locked: self,
            effects,
            arithmetic,
        })
        .map_err(BatchSessionError::Store)
    }
}

pub(crate) struct BatchSession<'a, A: BatchAlgorithm> {
    locked: LockedBatch<'a, A>,
    effects: BatchEffects,
    arithmetic: A::Arithmetic,
}

impl<A: BatchAlgorithm> BatchSession<'_, A> {
    fn denied(self, index: usize, denial: Denial) -> (BatchDecision, BatchEffects) {
        (self.locked.batch.denied(index, denial), self.effects)
    }
}

/// Counts only genuinely vacant keys. An expired entry still in the map occupies its slot.
struct CapacityPreflight<'a, 'shard, A: QuotaAlgorithm> {
    locked: &'a LockedBatch<'shard, A>,
    pending: Vec<usize>,
}

impl<'a, 'shard, A: QuotaAlgorithm> CapacityPreflight<'a, 'shard, A> {
    fn new(locked: &'a LockedBatch<'shard, A>) -> Self {
        Self {
            locked,
            pending: Vec::new(),
        }
    }

    fn reserve_vacant(&mut self, position: usize) -> Option<Denial> {
        let shard = &self.locked.shards[position].1;
        if self.pending.is_empty() {
            self.pending.resize(self.locked.shards.len(), 0);
        }
        let pending = &mut self.pending[position];
        if shard.used_entries().saturating_add(*pending) >= shard.capacity() {
            return Some(shard.storage_capacity_denial(self.locked.now_millis));
        }
        *pending += 1;
        None
    }
}
