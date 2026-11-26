//! Only the consuming methods on the two preflighted owners expose quota
//! writes. Keeping them here gives them access to both owners' private fields.

use std::collections::{BTreeSet, hash_map};

use runlimit_core::{Allowance, BatchDecision, BatchError, CounterKey, Decision};

use super::{BatchAlgorithm, BatchEffects, PreflightedBatch};
use crate::shards::{
    Expiration, PrehashedCounterKey, QuotaAlgorithm, Shard, ShardEffect, StoredEntry,
    session::{EntryWrite, PendingToken, PreflightedSingle},
};
use crate::{
    MemoryBatchError,
    gcra::GcraAlgorithm,
    store::{Entry, FixedWindowAlgorithm},
};

impl<A: QuotaAlgorithm> PreflightedSingle<'_, A> {
    pub(in crate::shards::session) fn commit(mut self) -> (Decision, ShardEffect) {
        let allowance = self.token.allowance();
        apply(&mut self.locked.guard, self.token);
        let effect = self.locked.effect(self.cleanup);
        (Decision::allowed(allowance), effect)
    }
}

impl PreflightedBatch<'_, FixedWindowAlgorithm> {
    pub(in crate::shards::session) fn commit(
        mut self,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError> {
        let locked = &mut self.locked;
        let now = locked.now_millis;
        let mut allowances = Vec::with_capacity(locked.batch.checks.len());
        for (index, check) in locked.batch.checks.iter().enumerate() {
            let shard: &mut Shard<Entry> =
                &mut locked.shards[locked.topology.shard_position(index)].1;
            let key = check.counter_key();
            let allowance = match shard.entries.entry(PrehashedCounterKey(key)) {
                hash_map::Entry::Occupied(mut slot) if slot.get().expires_at_millis > now => {
                    let stored = slot.get_mut();
                    let (entry, allowance) =
                        stored.value.charge(check, stored.expires_at_millis, now);
                    stored.value = entry;
                    allowance
                }
                slot => {
                    let (entry, allowance, expires) = Entry::open(check, now);
                    replace_slot(slot, &mut shard.expirations, entry, expires);
                    allowance
                }
            };
            allowances.push(allowance);
        }
        self.finish(allowances)
    }
}

impl PreflightedBatch<'_, GcraAlgorithm> {
    pub(in crate::shards::session) fn commit(
        mut self,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError> {
        let mut allowances = Vec::with_capacity(self.writes.len());
        for target in self.writes.drain(..) {
            allowances.push(target.token.allowance());
            apply(&mut self.locked.shards[target.position].1, target.token);
        }
        self.finish(allowances)
    }
}

impl<A: BatchAlgorithm> PreflightedBatch<'_, A> {
    fn finish(
        mut self,
        allowances: Vec<Allowance>,
    ) -> Result<(BatchDecision, BatchEffects), MemoryBatchError> {
        // Both commit loops produce one allowance per write, so an empty result made no writes.
        let decision = BatchDecision::allowed(allowances)
            .map_err(|_| MemoryBatchError::InvalidBatch(BatchError::EmptyBatch))?;
        if let BatchEffects::Recorded(effects) = &mut self.effects {
            for ((_, shard), effect) in self.locked.shards.iter().zip(effects.iter_mut()) {
                effect.used = shard.used_entries();
            }
        }
        Ok((decision, self.effects))
    }
}

fn apply<T: PendingToken>(shard: &mut Shard<T::Entry>, token: T) {
    let key = token.counter_key();
    match token.into_write() {
        EntryWrite::UpdateActive(entry) => {
            shard
                .entries
                .get_mut(&PrehashedCounterKey(key))
                .expect("preflighted active entries stay locked through commit")
                .value = entry;
        }
        EntryWrite::Replace {
            entry,
            expires_at_millis,
        } => {
            shard.replace_entry(key, entry, expires_at_millis);
        }
    }
}

impl<E> Shard<E> {
    fn replace_entry(&mut self, key: CounterKey, value: E, expires_at_millis: u128) -> Option<E> {
        replace_slot(
            self.entries.entry(PrehashedCounterKey(key)),
            &mut self.expirations,
            value,
            expires_at_millis,
        )
    }

    #[cfg(test)]
    pub(crate) fn replace(
        &mut self,
        key: CounterKey,
        value: E,
        expires_at_millis: u128,
    ) -> Option<E> {
        self.replace_entry(key, value, expires_at_millis)
    }
}

fn replace_slot<E>(
    slot: hash_map::Entry<'_, PrehashedCounterKey, StoredEntry<E>>,
    expirations: &mut BTreeSet<Expiration>,
    value: E,
    expires_at_millis: u128,
) -> Option<E> {
    let key = slot.key().0;
    let stored = StoredEntry {
        value,
        expires_at_millis,
    };
    let previous = match slot {
        hash_map::Entry::Occupied(mut slot) => {
            let previous = slot.insert(stored);
            let removed = expirations.remove(&Expiration {
                expires_at_millis: previous.expires_at_millis,
                key,
            });
            debug_assert!(removed, "a replaced entry must have one expiration");
            Some(previous.value)
        }
        hash_map::Entry::Vacant(slot) => {
            slot.insert(stored);
            None
        }
    };
    let inserted = expirations.insert(Expiration {
        expires_at_millis,
        key,
    });
    debug_assert!(inserted, "an inserted entry must have one expiration");
    previous
}
