use std::convert::Infallible;

use runlimit_core::BatchDecision;

use super::{BatchAlgorithm, BatchEffects, BatchSession, CapacityPreflight, LockedBatch};
use crate::{MemoryBatchError, shards::PrehashedCounterKey, store::FixedWindowAlgorithm};

mod commit;
mod gcra;

/// Constructed after every member passes preflight. Private fields keep the
/// original locks and algorithm-specific writes together through commit.
pub(in crate::shards::session) struct PreflightedBatch<'a, A: BatchAlgorithm> {
    locked: LockedBatch<'a, A>,
    effects: BatchEffects,
    writes: A::Writes,
}

impl BatchAlgorithm for FixedWindowAlgorithm {
    type Arithmetic = ();
    type Writes = ();

    fn check_arithmetic(_: &LockedBatch<'_, Self>) -> Result<(), Infallible> {
        Ok(())
    }

    fn preflight_and_commit(
        session: BatchSession<'_, Self>,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError> {
        let locked = &session.locked;
        let mut capacity = CapacityPreflight::new(locked);
        for (index, check) in locked.batch.checks.iter().enumerate() {
            #[cfg(test)]
            super::tests::count_evaluation();
            let position = locked.topology.shard_position(index);
            let shard = &locked.shards[position].1;
            let denial = match shard.entries.get(&PrehashedCounterKey(check.counter_key())) {
                Some(stored) if stored.expires_at_millis > locked.now_millis => stored
                    .value
                    .denial(check, stored.expires_at_millis, locked.now_millis)
                    .map(Into::into),
                Some(_) => None,
                None => capacity.reserve_vacant(position),
            };
            if let Some(denial) = denial {
                return Ok(session.denied(index, denial));
            }
        }
        PreflightedBatch {
            locked: session.locked,
            effects: session.effects,
            writes: (),
        }
        .commit()
    }
}
