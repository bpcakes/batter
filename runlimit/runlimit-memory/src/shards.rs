use std::{
    collections::{BTreeSet, HashMap},
    hash::{BuildHasherDefault, Hash, Hasher},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use runlimit_core::{CounterKey, Delay, Denial};

use crate::{
    MemoryStoreConfig,
    store::{MemoryBatchError, MemoryStoreStats, PoisonedShardError},
};

mod session;

/// The one conversion of a stored entry's remaining lifetime, in clock
/// milliseconds, to a delay. An entry never outlives its policy's window or
/// period, which `runlimit-core` bounds by `MAX_WINDOW`, far below
/// `u64::MAX` milliseconds, so the narrowing cannot fail.
pub(crate) fn remaining_duration(millis: u128) -> Duration {
    Duration::from_millis(
        u64::try_from(millis)
            .expect("a stored entry's remaining lifetime cannot exceed its policy window"),
    )
}

pub(crate) use session::{
    BatchSessionError, CheckSessionError, EntryWrite, PendingToken, QuotaAlgorithm,
    QuotaEvaluation, execute_batch, execute_check,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Expiration {
    expires_at_millis: u128,
    key: CounterKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrehashedCounterKey(CounterKey);

impl Hash for PrehashedCounterKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(counter_key_hash(self.0));
    }
}

#[derive(Default)]
struct PassThroughHasher(u64);

impl Hasher for PassThroughHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // `PrehashedCounterKey` always uses `write_u64`. Keep the required
        // byte-oriented fallback well-defined in case that implementation is
        // changed later.
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

type EntryMap<E> =
    HashMap<PrehashedCounterKey, StoredEntry<E>, BuildHasherDefault<PassThroughHasher>>;

#[derive(Debug)]
struct StoredEntry<E> {
    value: E,
    expires_at_millis: u128,
}

#[derive(Debug)]
pub(crate) struct Shard<E> {
    capacity: usize,
    latest_observed_millis: u128,
    entries: EntryMap<E>,
    expirations: BTreeSet<Expiration>,
}

impl<E> Shard<E> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            latest_observed_millis: 0,
            entries: HashMap::with_hasher(BuildHasherDefault::default()),
            expirations: BTreeSet::new(),
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.expirations.clear();
    }

    const fn capacity(&self) -> usize {
        self.capacity
    }

    fn used_entries(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used_entries()
    }

    const fn latest_observed_millis(&self) -> u128 {
        self.latest_observed_millis
    }

    const fn record_observed_millis(&mut self, now_millis: u128) {
        self.latest_observed_millis = now_millis;
    }

    fn contains_key(&self, key: CounterKey) -> bool {
        self.entries.contains_key(&PrehashedCounterKey(key))
    }

    pub(crate) fn active_entry(&self, key: CounterKey, now_millis: u128) -> Option<(&E, u128)> {
        let entry = self.entries.get(&PrehashedCounterKey(key))?;
        (entry.expires_at_millis > now_millis).then_some((&entry.value, entry.expires_at_millis))
    }

    fn advance_and_prune(
        &mut self,
        now_millis: u128,
        cleanup_limit: usize,
        time_cleanup: bool,
    ) -> CleanupEffect {
        self.record_observed_millis(now_millis);
        let started = time_cleanup.then(Instant::now);
        let removed = self.prune_expired(now_millis, cleanup_limit);
        CleanupEffect {
            requested: cleanup_limit,
            removed,
            elapsed: started.map_or_default(|start| start.elapsed()),
        }
    }

    fn prune_expired(&mut self, now_millis: u128, maximum: usize) -> usize {
        let mut removed_count = 0;
        for _ in 0..maximum {
            let Some(expiration) = self.expirations.first() else {
                break;
            };
            if expiration.expires_at_millis > now_millis {
                break;
            }

            let expiration = self
                .expirations
                .pop_first()
                .expect("the first expiration was present");
            let removed = self.entries.remove(&PrehashedCounterKey(expiration.key));
            debug_assert_eq!(
                removed.as_ref().map(|entry| entry.expires_at_millis),
                Some(expiration.expires_at_millis),
                "the entry and expiration indexes diverged"
            );
            removed_count += usize::from(removed.is_some());
        }
        removed_count
    }

    fn capacity_retry_after_millis(&self, now_millis: u128) -> Option<u128> {
        self.expirations.first().map(|expiration| {
            expiration
                .expires_at_millis
                .saturating_sub(now_millis)
                .max(1)
        })
    }

    /// Builds the denial for a new key that cannot fit in this full shard,
    /// with the delay until the earliest stored entry expires when one exists.
    fn storage_capacity_denial(&self, now_millis: u128) -> Denial {
        Denial::StorageCapacity {
            retry_after: self
                .capacity_retry_after_millis(now_millis)
                .map(|millis| Delay::new(remaining_duration(millis))),
        }
    }

    #[cfg(test)]
    pub(crate) fn entry(&self, key: CounterKey) -> Option<(&E, u128)> {
        self.entries
            .get(&PrehashedCounterKey(key))
            .map(|entry| (&entry.value, entry.expires_at_millis))
    }

    #[cfg(test)]
    fn snapshot(&self) -> Vec<(CounterKey, E, u128)>
    where
        E: Copy,
    {
        let mut entries = self
            .entries
            .iter()
            .map(|(key, stored)| (key.0, stored.value, stored.expires_at_millis))
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|(key, _, _)| *key);
        entries
    }

    #[cfg(test)]
    pub(crate) fn expiration_count(&self) -> usize {
        self.expirations.len()
    }

    #[cfg(test)]
    pub(crate) fn expiration_count_at_or_before(&self, now_millis: u128) -> usize {
        self.expirations
            .iter()
            .take_while(|expiration| expiration.expires_at_millis <= now_millis)
            .count()
    }

    #[cfg(test)]
    pub(crate) fn contains_expiration(&self, key: CounterKey, expires_at_millis: u128) -> bool {
        self.expirations.contains(&Expiration {
            expires_at_millis,
            key,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BatchShard {
    index: usize,
    check_count: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct BatchTopology {
    shards: Vec<BatchShard>,
    input_positions: Vec<usize>,
}

impl BatchTopology {
    fn shard_position(&self, input_index: usize) -> usize {
        self.input_positions[input_index]
    }

    fn check_counts(&self) -> impl ExactSizeIterator<Item = usize> + '_ {
        self.shards.iter().map(|shard| shard.check_count)
    }
}

pub(crate) struct BoundedShards<E> {
    config: MemoryStoreConfig,
    shards: Box<[Mutex<Shard<E>>]>,
}

type LockedShards<'a, E> = Vec<(usize, MutexGuard<'a, Shard<E>>)>;

impl<E> BoundedShards<E> {
    pub(crate) fn new(config: MemoryStoreConfig) -> Self {
        let shards = (0..config.shard_count())
            .map(|index| Mutex::new(Shard::new(config.shard_capacity(index))))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { config, shards }
    }

    pub(crate) const fn config(&self) -> &MemoryStoreConfig {
        &self.config
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.shards.len()
    }

    pub(crate) fn poisoned_count(&self) -> usize {
        self.shards
            .iter()
            .filter(|shard| shard.is_poisoned())
            .count()
    }

    pub(crate) fn shard_index(&self, key: &CounterKey) -> usize {
        #[allow(clippy::cast_possible_truncation)]
        let hash = counter_key_hash(*key) as usize;
        hash % self.shards.len()
    }

    fn lock_shard(&self, index: usize) -> Result<MutexGuard<'_, Shard<E>>, PoisonedShardError> {
        self.shards[index]
            .lock()
            .map_err(|_| PoisonedShardError { shard_index: index })
    }

    fn lock_shards<'a>(
        &'a self,
        indexes: &[usize],
    ) -> Result<LockedShards<'a, E>, PoisonedShardError> {
        let mut locked = Vec::with_capacity(indexes.len());
        for &index in indexes {
            locked.push((index, self.lock_shard(index)?));
        }
        Ok(locked)
    }

    fn batch_topology(
        &self,
        keys: impl Iterator<Item = CounterKey>,
    ) -> Result<BatchTopology, MemoryBatchError> {
        let mut input_positions = keys.map(|key| self.shard_index(&key)).collect::<Vec<_>>();
        let mut shards = input_positions
            .iter()
            .copied()
            .map(|index| BatchShard {
                index,
                check_count: 0,
            })
            .collect::<Vec<_>>();
        shards.sort_unstable_by_key(|shard| shard.index);
        shards.dedup_by_key(|shard| shard.index);

        for shard_position in &mut input_positions {
            let shard_index = *shard_position;
            *shard_position = shards.partition_point(|shard| shard.index < shard_index);
            let shard = &mut shards[*shard_position];
            debug_assert_eq!(shard.index, shard_index);
            shard.check_count += 1;
        }

        for shard in &shards {
            let capacity = self.config.shard_capacity(shard.index);
            if shard.check_count > capacity {
                return Err(MemoryBatchError::BatchExceedsShardCapacity {
                    shard_index: shard.index,
                    key_count: shard.check_count,
                    capacity,
                });
            }
        }

        Ok(BatchTopology {
            shards,
            input_positions,
        })
    }

    fn lock_batch(
        &self,
        keys: impl Iterator<Item = CounterKey>,
    ) -> Result<(BatchTopology, LockedShards<'_, E>), MemoryBatchError> {
        let topology = self.batch_topology(keys)?;
        let mut locked = Vec::with_capacity(topology.shards.len());
        for shard in &topology.shards {
            locked.push((shard.index, self.lock_shard(shard.index)?));
        }
        Ok((topology, locked))
    }

    pub(crate) fn stats(&self) -> Result<MemoryStoreStats, PoisonedShardError> {
        let indexes = (0..self.shards.len()).collect::<Vec<_>>();
        let locked = self.lock_shards(&indexes)?;
        Ok(MemoryStoreStats::from_parts(
            locked.iter().map(|(_, shard)| shard.used_entries()).sum(),
            self.config.max_keys(),
            self.config.shard_count(),
        ))
    }

    pub(crate) fn clear(&self) -> Result<(), PoisonedShardError> {
        let indexes = (0..self.shards.len()).collect::<Vec<_>>();
        let mut locked = self.lock_shards(&indexes)?;
        for (_, shard) in &mut locked {
            shard.clear();
        }
        Ok(())
    }

    pub(crate) fn recover_poisoned(&self) -> usize {
        let mut locked_shards = Vec::with_capacity(self.shards.len());

        for (shard_index, shard_mutex) in self.shards.iter().enumerate() {
            match shard_mutex.lock() {
                Ok(shard) => locked_shards.push((shard_index, shard_mutex, shard, false)),
                Err(poisoned) => {
                    locked_shards.push((shard_index, shard_mutex, poisoned.into_inner(), true));
                }
            }
        }

        let mut recovered = 0;
        for (shard_index, shard_mutex, shard, was_poisoned) in &mut locked_shards {
            if !*was_poisoned {
                continue;
            }

            **shard = Shard::new(self.config.shard_capacity(*shard_index));
            shard_mutex.clear_poison();
            recovered += 1;
        }

        recovered
    }

    #[cfg(test)]
    pub(crate) fn shard(&self, index: usize) -> &Mutex<Shard<E>> {
        &self.shards[index]
    }
}

#[derive(Clone, Copy, Debug)]
struct CleanupEffect {
    requested: usize,
    removed: usize,
    elapsed: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShardEffect {
    shard_index: usize,
    cleanup: CleanupEffect,
    used: usize,
    capacity: usize,
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Hashes a logical counter key for a local shard and map bucket.
///
/// Subject keys are opaque HMAC-SHA-256 output (or caller-supplied
/// cryptographically opaque digests), so their bits are already uniform and
/// attacker-resistant. Mix in the policy fingerprint so successive
/// configurations of one subject do not all collide, without paying for a
/// second keyed hash.
pub(crate) fn counter_key_hash(key: CounterKey) -> u64 {
    leading_u64(key.subject().as_bytes())
        ^ leading_u64(key.fingerprint().as_bytes()).rotate_left(32)
}

const fn leading_u64(bytes: &[u8; 32]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        time::Duration,
    };

    use runlimit_core::{Check, CounterKey, FixedWindowPolicy, PolicyId, ScopeId, SubjectKey};

    use super::{BatchShard, BoundedShards};
    use crate::{MemoryBatchError, MemoryStoreConfig, PoisonedShardError};

    #[derive(Clone, Copy)]
    struct TestEntry;

    fn counter_key(subject_byte: u8) -> CounterKey {
        let policy = FixedWindowPolicy::new(
            PolicyId::new("memory.kernel").unwrap(),
            ScopeId::new("test").unwrap(),
            1,
            Duration::from_mins(1),
        )
        .unwrap();
        Check::new(SubjectKey::from_digest([subject_byte; 32]).bind(&policy)).counter_key()
    }

    fn keys_for_shards<const N: usize>(
        shards: &BoundedShards<TestEntry>,
        indexes: [usize; N],
    ) -> [CounterKey; N] {
        let mut candidates = (0..=u8::MAX).map(counter_key);
        indexes.map(|index| {
            candidates
                .find(|key| shards.shard_index(key) == index)
                .expect("test subjects cover both shards")
        })
    }

    #[test]
    fn batch_topology_resolves_sorted_routes_and_counts_once() {
        let config = MemoryStoreConfig::new(5)
            .unwrap()
            .with_shard_count(2)
            .unwrap();
        let shards = BoundedShards::<TestEntry>::new(config);
        let keys = keys_for_shards(&shards, [1, 0, 1]);
        let (topology, locked) = shards.lock_batch(keys.into_iter()).unwrap();

        assert_eq!(
            topology.shards,
            vec![
                BatchShard {
                    index: 0,
                    check_count: 1,
                },
                BatchShard {
                    index: 1,
                    check_count: 2,
                },
            ]
        );
        assert_eq!(topology.input_positions, vec![1, 0, 1]);
        assert_eq!(topology.check_counts().collect::<Vec<_>>(), vec![1, 2]);

        assert_eq!(
            locked.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn batch_topology_uses_the_owners_capacity_to_reject_the_first_overfull_shard() {
        let config = MemoryStoreConfig::new(4)
            .unwrap()
            .with_shard_count(2)
            .unwrap();

        let shards = BoundedShards::<TestEntry>::new(config);
        let keys = keys_for_shards(&shards, [1, 0, 0, 0, 1, 1]);
        assert_eq!(
            shards.batch_topology(keys.into_iter()),
            Err(MemoryBatchError::BatchExceedsShardCapacity {
                shard_index: 0,
                key_count: 3,
                capacity: 2,
            })
        );
        let larger = BoundedShards::<TestEntry>::new(
            MemoryStoreConfig::new(6)
                .unwrap()
                .with_shard_count(2)
                .unwrap(),
        );
        let topology = larger.batch_topology(keys.into_iter()).unwrap();
        assert_eq!(topology.check_counts().collect::<Vec<_>>(), [3, 3]);
    }

    fn insert(shard: &mut super::Shard<TestEntry>, key: CounterKey, expires_at_millis: u128) {
        assert!(shard.replace(key, TestEntry, expires_at_millis).is_none());
    }

    #[test]
    fn expiry_cleanup_stays_bounded_and_reports_the_next_capacity_retry() {
        let config = MemoryStoreConfig::new(2).unwrap();
        let shards = BoundedShards::<TestEntry>::new(config);
        let first = counter_key(1);
        let second = counter_key(2);
        let mut shard = shards.shard(0).lock().unwrap();

        insert(&mut shard, first, 10);
        insert(&mut shard, second, 20);

        assert_eq!(shard.prune_expired(10, 1), 1);
        assert_eq!(shard.used(), 1);
        assert_eq!(shard.capacity_retry_after_millis(10), Some(10));

        assert_eq!(shard.prune_expired(20, 0), 0);
        assert_eq!(shard.capacity_retry_after_millis(20), Some(1));
        assert_eq!(shard.prune_expired(20, 1), 1);
        assert_eq!(shard.used(), 0);
        assert_eq!(shard.capacity_retry_after_millis(20), None);
    }

    #[test]
    fn replacing_an_entry_moves_its_expiration_atomically() {
        let config = MemoryStoreConfig::new(1).unwrap();
        let shards = BoundedShards::<TestEntry>::new(config);
        let key = counter_key(3);
        let mut shard = shards.shard(0).lock().unwrap();

        insert(&mut shard, key, 10);
        assert!(shard.replace(key, TestEntry, 20).is_some());

        assert_eq!(shard.used(), 1);
        assert_eq!(shard.expiration_count(), 1);
        assert!(!shard.contains_expiration(key, 10));
        assert!(shard.contains_expiration(key, 20));
        assert_eq!(shard.prune_expired(10, 1), 0);
        assert_eq!(shard.prune_expired(20, 1), 1);
        assert_eq!(shard.used(), 0);
    }

    #[test]
    fn poison_recovery_rebuilds_only_the_untrusted_shard() {
        let config = MemoryStoreConfig::new(3)
            .unwrap()
            .with_shard_count(2)
            .unwrap();
        let shards = BoundedShards::<TestEntry>::new(config);
        let healthy_key = counter_key(7);
        {
            let mut healthy = shards.shard(1).lock().unwrap();
            insert(&mut healthy, healthy_key, 60);
        }

        let panic_result = catch_unwind(AssertUnwindSafe(|| {
            let _poisoned = shards.shard(0).lock().unwrap();
            panic!("poison the test shard");
        }));
        assert!(panic_result.is_err());
        assert!(matches!(
            shards.lock_shard(0),
            Err(PoisonedShardError { shard_index: 0 })
        ));

        assert_eq!(shards.recover_poisoned(), 1);
        assert_eq!(shards.lock_shard(0).unwrap().capacity(), 2);
        let healthy = shards.lock_shard(1).unwrap();
        assert_eq!(healthy.capacity(), 1);
        assert_eq!(healthy.used(), 1);
        assert!(healthy.contains_key(healthy_key));
    }
}
