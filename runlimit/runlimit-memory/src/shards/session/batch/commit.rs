//! Only the consuming methods on the two preflighted owners expose quota
//! writes. Keeping them here gives them access to both owners' private fields.

use runlimit_core::{BatchDecision, CounterKey, Decision};

use super::{BatchEffects, PreflightedBatch, QuotaAlgorithm};
use crate::shards::{
    Expiration, PrehashedCounterKey, Shard, ShardEffect, StoredEntry,
    session::{EntryWrite, PendingToken, PreflightedSingle},
};

impl<A: QuotaAlgorithm> PreflightedSingle<'_, A> {
    pub(in crate::shards::session) fn commit(mut self) -> (Decision, ShardEffect) {
        let allowance = self.token.allowance();
        apply(&mut self.locked.guard, self.token);
        let effect = self.locked.effect(self.cleanup);
        (Decision::allowed(allowance), effect)
    }
}

impl<A: QuotaAlgorithm> PreflightedBatch<'_, A> {
    pub(in crate::shards::session) fn commit(mut self) -> (BatchDecision, BatchEffects) {
        for targeted in self.tokens {
            apply(&mut self.shards[targeted.shard_position].1, targeted.token);
        }
        if let BatchEffects::Recorded(effects) = &mut self.effects {
            for ((_, shard), effect) in self.shards.iter().zip(effects.iter_mut()) {
                effect.used = shard.used_entries();
            }
        }
        (self.decision, self.effects)
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
        let previous = self.entries.insert(
            PrehashedCounterKey(key),
            StoredEntry {
                value,
                expires_at_millis,
            },
        );
        if let Some(previous) = &previous {
            let removed = self.expirations.remove(&Expiration {
                expires_at_millis: previous.expires_at_millis,
                key,
            });
            debug_assert!(removed, "a replaced entry must have one expiration");
        }
        let inserted = self.expirations.insert(Expiration {
            expires_at_millis,
            key,
        });
        debug_assert!(inserted, "an inserted entry must have one expiration");
        previous.map(|entry| entry.value)
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
