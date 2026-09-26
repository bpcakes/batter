//! Pre-built metric keys, one write-once slot per possible series.
//!
//! Each cache is sized by the product of its closed label domains, so the
//! caches together hold at most [`super::MAX_SERIES`] keys. A key is built,
//! allocated and hashed once on its series' first observation; later
//! observations register the cached key with the current recorder without
//! allocating. Caching keys, not recorder handles, keeps recording correct when
//! the application installs or scopes a different recorder later.

use metrics::{Key, Label, Level, Metadata};
use std::sync::OnceLock;

static METADATA: Metadata<'static> =
    Metadata::new(module_path!(), Level::INFO, Some(module_path!()));

pub(super) struct KeyCache<const N: usize> {
    name: &'static str,
    keys: [OnceLock<Key>; N],
}

impl<const N: usize> KeyCache<N> {
    pub(super) const fn new(name: &'static str) -> Self {
        Self {
            name,
            keys: [const { OnceLock::new() }; N],
        }
    }

    /// The cached key for `index`, building it from static labels once.
    pub(super) fn key<const L: usize>(
        &self,
        index: usize,
        labels: [(&'static str, &'static str); L],
    ) -> &Key {
        self.keys[index].get_or_init(|| {
            let labels = labels.map(|(key, value)| Label::from_static_parts(key, value));
            Key::from_parts(self.name, labels.to_vec())
        })
    }
}

/// Position of `value` in a closed domain; values are the domain's own
/// `'static` literals, so a pointer match usually ends the scan.
pub(super) fn index_of(values: &[&str], value: &str) -> usize {
    values
        .iter()
        .position(|candidate| std::ptr::eq(*candidate, value) || *candidate == value)
        .expect("label belongs to its closed domain")
}

pub(super) fn increment(key: &Key, by: u64) {
    metrics::with_recorder(|recorder| recorder.register_counter(key, &METADATA).increment(by));
}

pub(super) fn sample(key: &Key, value: f64) {
    metrics::with_recorder(|recorder| recorder.register_histogram(key, &METADATA).record(value));
}
