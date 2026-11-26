use super::{LockedBatch, PendingToken, QuotaAlgorithm, QuotaEvaluation};
use runlimit_core::Denial;
use std::convert::Infallible;

pub(super) struct TargetedToken<T> {
    pub(super) shard_position: usize,
    pub(super) token: T,
}

/// Fallible algorithms carry the evaluated prefix through cleanup. An
/// infallible algorithm can evaluate lazily against the cleaned shard.
pub(super) enum ArithmeticPreflight<T, E> {
    Infallible(fn(E) -> Infallible),
    Evaluated(QuotaPreflight<T>),
}

pub(super) enum QuotaPreflight<T> {
    AllAllowed(Vec<TargetedToken<T>>),
    Denied {
        allowed_prefix: Vec<TargetedToken<T>>,
        denial: Denial,
    },
}

impl<T> QuotaPreflight<T> {
    pub(super) fn collect<E>(
        evaluations: &mut impl ExactSizeIterator<Item = Result<(usize, QuotaEvaluation<T>), E>>,
        mut capacity_denial: impl FnMut(&TargetedToken<T>) -> Option<Denial>,
    ) -> Result<Self, E> {
        let batch_len = evaluations.len();
        let mut tokens = Vec::new();
        for evaluation in evaluations {
            let (shard_position, outcome) = evaluation?;
            match outcome {
                QuotaEvaluation::Allowed(token) => {
                    let target = TargetedToken {
                        shard_position,
                        token,
                    };
                    if let Some(denial) = capacity_denial(&target) {
                        return Ok(Self::Denied {
                            allowed_prefix: tokens,
                            denial,
                        });
                    }
                    if tokens.is_empty() {
                        tokens.reserve_exact(batch_len);
                    }
                    tokens.push(target);
                }
                QuotaEvaluation::Denied(denial) => {
                    return Ok(Self::Denied {
                        allowed_prefix: tokens,
                        denial: denial.into(),
                    });
                }
            }
        }
        Ok(Self::AllAllowed(tokens))
    }

    pub(super) fn allowed_prefix(&self) -> &[TargetedToken<T>] {
        match self {
            Self::AllAllowed(tokens) => tokens,
            Self::Denied { allowed_prefix, .. } => allowed_prefix,
        }
    }
}

pub(super) struct CapacityPreflight<'batch, 'shard, A: QuotaAlgorithm> {
    locked: &'batch LockedBatch<'shard, A>,
    pending_insertions: Vec<usize>,
}

impl<'batch, 'shard, A: QuotaAlgorithm> CapacityPreflight<'batch, 'shard, A> {
    pub(super) fn new(locked: &'batch LockedBatch<'shard, A>) -> Self {
        Self {
            locked,
            pending_insertions: Vec::new(),
        }
    }

    pub(super) fn denial(&mut self, target: &TargetedToken<A::Token>) -> Option<Denial> {
        let position = target.shard_position;
        let shard = &self.locked.shards[position].1;
        if shard.contains_key(target.token.counter_key()) {
            return None;
        }
        if self.pending_insertions.is_empty() {
            self.pending_insertions.resize(self.locked.shards.len(), 0);
        }
        let pending = &mut self.pending_insertions[position];
        if shard.used_entries().saturating_add(*pending) >= shard.capacity() {
            return Some(shard.storage_capacity_denial(self.locked.now_millis));
        }
        *pending += 1;
        None
    }
}
