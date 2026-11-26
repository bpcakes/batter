use runlimit_core::{BatchDecision, QuotaDenial, gcra::ArithmeticOverflow};

use super::{
    BatchAlgorithm, BatchEffects, BatchSession, CapacityPreflight, LockedBatch, PreflightedBatch,
};
use crate::{
    MemoryBatchError,
    gcra::GcraAlgorithm,
    shards::{PendingToken, QuotaAlgorithm, QuotaEvaluation},
};

pub(crate) struct TargetedToken {
    pub(super) position: usize,
    pub(super) token: <GcraAlgorithm as QuotaAlgorithm>::Token,
}

pub(crate) enum GcraPreflight {
    AllAllowed(Vec<TargetedToken>),
    Denied {
        prefix: Vec<TargetedToken>,
        denial: QuotaDenial,
    },
}

impl BatchAlgorithm for GcraAlgorithm {
    type Arithmetic = GcraPreflight;
    type Writes = Vec<TargetedToken>;

    fn check_arithmetic(
        locked: &LockedBatch<'_, Self>,
    ) -> Result<GcraPreflight, ArithmeticOverflow> {
        let mut evaluations = locked
            .batch
            .checks
            .iter()
            .enumerate()
            .map(|(index, check)| {
                let position = locked.topology.shard_position(index);
                Self::evaluate(&locked.shards[position].1, check, locked.now_millis)
                    .map(|outcome| (position, outcome))
            });
        let mut prefix = Vec::new();
        while let Some(evaluation) = evaluations.next() {
            let (position, outcome) = evaluation?;
            match outcome {
                QuotaEvaluation::Allowed(token) => {
                    if prefix.is_empty() {
                        prefix.reserve_exact(locked.batch.checks.len());
                    }
                    prefix.push(TargetedToken { position, token });
                }
                QuotaEvaluation::Denied(denial) => {
                    // An arithmetic failure anywhere in the batch precedes cleanup and denial.
                    for remaining in evaluations {
                        remaining?;
                    }
                    return Ok(GcraPreflight::Denied { prefix, denial });
                }
            }
        }
        Ok(GcraPreflight::AllAllowed(prefix))
    }

    fn preflight_and_commit(
        session: BatchSession<'_, Self>,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError> {
        let mut capacity = CapacityPreflight::new(&session.locked);
        let prefix = match &session.arithmetic {
            GcraPreflight::AllAllowed(tokens) | GcraPreflight::Denied { prefix: tokens, .. } => {
                tokens
            }
        };
        for (index, target) in prefix.iter().enumerate() {
            if !session.locked.shards[target.position]
                .1
                .contains_key(target.token.counter_key())
                && let Some(denial) = capacity.reserve_vacant(target.position)
            {
                return Ok(session.denied(index, denial));
            }
        }
        match session.arithmetic {
            GcraPreflight::Denied { ref prefix, denial } => {
                let index = prefix.len();
                Ok(session.denied(index, denial.into()))
            }
            GcraPreflight::AllAllowed(writes) => PreflightedBatch {
                locked: session.locked,
                effects: session.effects,
                writes,
            }
            .commit(),
        }
    }
}
