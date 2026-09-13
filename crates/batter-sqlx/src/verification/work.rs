//! Private collection boundaries for logical-work regression oracles.
//!
//! Native hashing is deliberately uncounted. Callers get indexed operations;
//! test-only traversal records each entry so scan mutations cannot look like
//! a single lookup. There is no raw-map or slice escape from these wrappers.

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub(super) trait Work {
    fn record(&self);
}

pub(super) struct WorkMap<K, V, W> {
    entries: HashMap<K, V>,
    work: W,
}

impl<K: Eq + Hash, V, W: Work> WorkMap<K, V, W> {
    pub(super) fn with_capacity(capacity: usize, work: W) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            work,
        }
    }

    // Source traversal already records index construction at the caller.
    pub(super) fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.entries.insert(key, value)
    }

    pub(super) fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.work.record();
        self.entries.get(key)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().inspect(|_| self.work.record())
    }
}

pub(super) struct WorkSet<K, W> {
    entries: HashSet<K>,
    work: W,
}

impl<K: Eq + Hash, W: Work> WorkSet<K, W> {
    pub(super) fn with_capacity(capacity: usize, work: W) -> Self {
        Self {
            entries: HashSet::with_capacity(capacity),
            work,
        }
    }

    pub(super) fn insert(&mut self, key: K) -> bool {
        self.entries.insert(key)
    }

    pub(super) fn contains<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.work.record();
        self.entries.contains(key)
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().inspect(|_| self.work.record())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    impl Work for &Cell<usize> {
        fn record(&self) {
            self.set(self.get() + 1);
        }
    }

    #[test]
    fn scanning_a_collection_counts_every_visited_entry() {
        let visits = Cell::new(0);
        let mut map = WorkMap::with_capacity(0, &visits);
        let mut set = WorkSet::with_capacity(0, &visits);
        for key in 0..1_000 {
            map.insert(key, key);
            set.insert(key);
        }
        assert_eq!(visits.get(), 0);
        assert!(map.get(&1_000).is_none());
        assert!(!set.contains(&1_000));
        assert_eq!(visits.get(), 2);
        assert!(!map.iter().any(|(key, _)| *key == 1_000));
        assert!(!set.iter().any(|key| *key == 1_000));
        assert_eq!(visits.get(), 2_002);
    }
}
