use runlimit_core::{
    BatchDecision, BatchError, Check, Denial, QuotaMode, RateLimitPolicy, validate_batch,
};

use super::{BatchSessionError, PendingToken, QuotaAlgorithm, QuotaEvaluation};
use crate::{
    Clock, MemoryBatchError, MemoryStoreConfig,
    shards::{BatchTopology, BoundedShards, LockedShards, ShardEffect},
};

mod commit;
mod preflight;
#[cfg(test)]
mod tests;
use preflight::{ArithmeticPreflight, CapacityPreflight, QuotaPreflight, TargetedToken};

pub(super) enum BatchEffects {
    Disabled,
    Recorded(Vec<ShardEffect>),
}

pub(super) fn run_batch_session<A: QuotaAlgorithm, C: Clock>(
    shards: &BoundedShards<A::Entry>,
    config: &MemoryStoreConfig,
    clock: &C,
    observe: bool,
    checks: &[Check<'_, A::Policy>],
) -> Result<(BatchDecision, BatchEffects), BatchSessionError<A::EvalError>> {
    let locked = LockedBatch::<A>::lock(shards, checks, config, clock)?;
    let prepared = locked.prepare().map_err(BatchSessionError::Evaluation)?;
    let session = prepared.advance_and_prune(config.max_expired_removals_per_check(), observe);
    Ok(match session.preflight()? {
        BatchPreflight::Preflighted(preflighted) => preflighted.commit(),
        BatchPreflight::Denied { decision, effects } => (decision, effects),
    })
}

// ---------------------------------------------------------------------------
// Multi-shard consuming batch session:
// ValidatedBatch -> LockedBatch -> PreparedBatch -> BatchSession -> PreflightedBatch -> commit
// ---------------------------------------------------------------------------

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

pub(super) struct LockedBatch<'a, A: QuotaAlgorithm> {
    batch: ValidatedBatch<'a, A::Policy>,
    topology: BatchTopology,
    shards: LockedShards<'a, A::Entry>,
    now_millis: u128,
}

impl<'a, A: QuotaAlgorithm> LockedBatch<'a, A> {
    pub(super) fn lock<C: Clock>(
        shards: &'a BoundedShards<A::Entry>,
        checks: &'a [Check<'a, A::Policy>],
        config: &MemoryStoreConfig,
        clock: &C,
    ) -> Result<Self, MemoryBatchError> {
        let batch = ValidatedBatch::new(checks, config.max_batch_size())?;
        let topology = BatchTopology::new(
            batch
                .checks
                .iter()
                .map(|check| shards.shard_index(&check.counter_key())),
            config,
        )?;
        let locked = shards.lock_topology(&topology)?;
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
        })
    }

    pub(super) fn prepare(self) -> Result<PreparedBatch<'a, A>, A::EvalError> {
        let quota = if let Some(never) = A::INFALLIBLE {
            ArithmeticPreflight::Infallible(never)
        } else {
            let mut evaluations = self.evaluations();
            let prefix = QuotaPreflight::collect(&mut evaluations, |_| None)?;
            // A later arithmetic error takes precedence over a quota denial.
            for remaining in evaluations {
                remaining?;
            }
            ArithmeticPreflight::Evaluated(prefix)
        };
        Ok(PreparedBatch {
            locked: self,
            quota,
        })
    }

    fn evaluations(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<(usize, QuotaEvaluation<A::Token>), A::EvalError>>
    {
        self.batch
            .checks
            .iter()
            .enumerate()
            .map(|(input_index, check)| {
                let position = self.topology.shard_position(input_index);
                let outcome = A::evaluate(&self.shards[position].1, check, self.now_millis)?;
                Ok((position, outcome))
            })
    }
}

pub(super) struct PreparedBatch<'a, A: QuotaAlgorithm> {
    locked: LockedBatch<'a, A>,
    quota: ArithmeticPreflight<A::Token, A::EvalError>,
}

impl<'a, A: QuotaAlgorithm> PreparedBatch<'a, A> {
    pub(super) fn advance_and_prune(
        mut self,
        cleanup_per_check: usize,
        record_effects: bool,
    ) -> BatchSession<'a, A> {
        let locked = &mut self.locked;
        let mut effects = if record_effects {
            BatchEffects::Recorded(Vec::with_capacity(locked.shards.len()))
        } else {
            BatchEffects::Disabled
        };

        for ((shard_index, shard), check_count) in
            locked.shards.iter_mut().zip(locked.topology.check_counts())
        {
            let cleanup_limit = cleanup_per_check.saturating_mul(check_count);
            let cleanup = shard.advance_and_prune(locked.now_millis, cleanup_limit, record_effects);
            if let BatchEffects::Recorded(recorded) = &mut effects {
                recorded.push(ShardEffect {
                    shard_index: *shard_index,
                    cleanup,
                    used: shard.used_entries(),
                    capacity: shard.capacity(),
                });
            }
        }

        BatchSession {
            locked: self.locked,
            effects,
            quota: self.quota,
        }
    }
}

pub(super) struct BatchSession<'a, A: QuotaAlgorithm> {
    locked: LockedBatch<'a, A>,
    effects: BatchEffects,
    quota: ArithmeticPreflight<A::Token, A::EvalError>,
}

pub(super) enum BatchPreflight<'a, A: QuotaAlgorithm> {
    Preflighted(PreflightedBatch<'a, A>),
    Denied {
        decision: BatchDecision,
        effects: BatchEffects,
    },
}

impl<'a, A: QuotaAlgorithm> BatchSession<'a, A> {
    pub(super) fn preflight(self) -> Result<BatchPreflight<'a, A>, MemoryBatchError> {
        let Self {
            locked,
            effects,
            quota,
        } = self;
        let mut capacity = CapacityPreflight::new(&locked);
        let prefix = match quota {
            ArithmeticPreflight::Infallible(never) => {
                match QuotaPreflight::collect(&mut locked.evaluations(), |target| {
                    capacity.denial(target)
                }) {
                    Ok(prefix) => prefix,
                    Err(error) => match never(error) {},
                }
            }
            ArithmeticPreflight::Evaluated(prefix) => {
                for (index, target) in prefix.allowed_prefix().iter().enumerate() {
                    if let Some(denial) = capacity.denial(target) {
                        let decision = locked.batch.denied(index, denial);
                        return Ok(BatchPreflight::Denied { decision, effects });
                    }
                }
                prefix
            }
        };
        match prefix {
            QuotaPreflight::Denied {
                allowed_prefix,
                denial,
            } => {
                let decision = locked.batch.denied(allowed_prefix.len(), denial);
                Ok(BatchPreflight::Denied { decision, effects })
            }
            QuotaPreflight::AllAllowed(tokens) => {
                let allowances = tokens
                    .iter()
                    .map(|targeted| targeted.token.allowance())
                    .collect::<Vec<_>>();
                let decision = BatchDecision::allowed(allowances)
                    .map_err(|_| MemoryBatchError::InvalidBatch(BatchError::EmptyBatch))?;
                Ok(BatchPreflight::Preflighted(PreflightedBatch {
                    shards: locked.shards,
                    effects,
                    tokens,
                    decision,
                }))
            }
        }
    }
}

pub(super) struct PreflightedBatch<'a, A: QuotaAlgorithm> {
    shards: LockedShards<'a, A::Entry>,
    effects: BatchEffects,
    tokens: Vec<TargetedToken<A::Token>>,
    decision: BatchDecision,
}
