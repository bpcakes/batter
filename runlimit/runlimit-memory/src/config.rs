use thiserror::Error;

const DEFAULT_SHARD_COUNT: usize = 1;
const DEFAULT_MAX_EXPIRED_REMOVALS: usize = 8;
const DEFAULT_MAX_BATCH_SIZE: usize = 32;

/// Configuration for a hard-bounded [`MemoryStore`](crate::MemoryStore).
///
/// With the `serde` feature, this is an object containing `max_keys`,
/// `shard_count`, `max_expired_removals_per_check`, and `max_batch_size`.
/// Only `max_keys` is required when deserializing; omitted bounds use the same
/// defaults as the constructors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryStoreConfig {
    max_keys: usize,
    shard_count: usize,
    max_expired_removals_per_check: usize,
    max_batch_size: usize,
}

impl MemoryStoreConfig {
    /// Creates a configuration with a global hard maximum of `max_keys` stored
    /// counters.
    ///
    /// The store uses one shard by default so the entire configured capacity is
    /// available regardless of key distribution. Increasing the shard count
    /// trades some capacity utilization under uneven key distribution for
    /// greater lock concurrency.
    ///
    /// # Errors
    ///
    /// Returns an error when `max_keys` is zero.
    pub fn new(max_keys: usize) -> Result<Self, MemoryStoreConfigError> {
        if max_keys == 0 {
            return Err(MemoryStoreConfigError::ZeroMaxKeys);
        }

        Ok(Self {
            max_keys,
            shard_count: DEFAULT_SHARD_COUNT.min(max_keys),
            max_expired_removals_per_check: DEFAULT_MAX_EXPIRED_REMOVALS,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
        })
    }

    /// Sets the number of independently locked shards.
    ///
    /// The global key capacity is divided into fixed per-shard capacities. A
    /// full shard denies a new key even when another shard has unused capacity.
    /// An atomic batch that targets more distinct keys at a shard than its
    /// fixed capacity is rejected as structurally unsatisfiable.
    ///
    /// # Errors
    ///
    /// Returns an error when the shard count is zero or greater than the
    /// configured global key capacity.
    pub fn with_shard_count(mut self, shard_count: usize) -> Result<Self, MemoryStoreConfigError> {
        if shard_count == 0 {
            return Err(MemoryStoreConfigError::ZeroShardCount);
        }
        if shard_count > self.max_keys {
            return Err(MemoryStoreConfigError::MoreShardsThanKeys {
                shard_count,
                max_keys: self.max_keys,
            });
        }
        self.shard_count = shard_count;
        Ok(self)
    }

    /// Sets the maximum expired entries removed from a shard per check that
    /// targets it.
    ///
    /// An atomic batch receives this cleanup allowance for each check in the
    /// batch, applied to that check's shard.
    ///
    /// # Errors
    ///
    /// Returns an error when `maximum` is zero.
    pub fn with_max_expired_removals_per_check(
        mut self,
        maximum: usize,
    ) -> Result<Self, MemoryStoreConfigError> {
        if maximum == 0 {
            return Err(MemoryStoreConfigError::ZeroExpiredRemovalLimit);
        }
        self.max_expired_removals_per_check = maximum;
        Ok(self)
    }

    /// Sets the largest atomic batch accepted by the store.
    ///
    /// # Errors
    ///
    /// Returns an error when `maximum` is zero.
    pub fn with_max_batch_size(mut self, maximum: usize) -> Result<Self, MemoryStoreConfigError> {
        if maximum == 0 {
            return Err(MemoryStoreConfigError::ZeroBatchSize);
        }
        self.max_batch_size = maximum;
        Ok(self)
    }

    /// Returns the global hard maximum number of stored keys.
    pub const fn max_keys(&self) -> usize {
        self.max_keys
    }

    /// Returns the number of independently locked shards.
    pub const fn shard_count(&self) -> usize {
        self.shard_count
    }

    /// Returns the cleanup work limit per check targeting a shard.
    pub const fn max_expired_removals_per_check(&self) -> usize {
        self.max_expired_removals_per_check
    }

    /// Returns the largest accepted atomic batch.
    pub const fn max_batch_size(&self) -> usize {
        self.max_batch_size
    }

    pub(crate) fn shard_capacity(&self, shard_index: usize) -> usize {
        let base = self.max_keys / self.shard_count;
        let extra = usize::from(shard_index < self.max_keys % self.shard_count);
        base + extra
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct MemoryStoreConfigRef {
    max_keys: usize,
    shard_count: usize,
    max_expired_removals_per_check: usize,
    max_batch_size: usize,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryStoreConfigWire {
    max_keys: usize,
    #[serde(default = "default_shard_count")]
    shard_count: usize,
    #[serde(default = "default_max_expired_removals")]
    max_expired_removals_per_check: usize,
    #[serde(default = "default_max_batch_size")]
    max_batch_size: usize,
}

#[cfg(feature = "serde")]
const fn default_shard_count() -> usize {
    DEFAULT_SHARD_COUNT
}

#[cfg(feature = "serde")]
const fn default_max_expired_removals() -> usize {
    DEFAULT_MAX_EXPIRED_REMOVALS
}

#[cfg(feature = "serde")]
const fn default_max_batch_size() -> usize {
    DEFAULT_MAX_BATCH_SIZE
}

#[cfg(feature = "serde")]
impl serde::Serialize for MemoryStoreConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &MemoryStoreConfigRef {
                max_keys: self.max_keys(),
                shard_count: self.shard_count(),
                max_expired_removals_per_check: self.max_expired_removals_per_check(),
                max_batch_size: self.max_batch_size(),
            },
            serializer,
        )
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MemoryStoreConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <MemoryStoreConfigWire as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(wire.max_keys)
            .and_then(|config| config.with_shard_count(wire.shard_count))
            .and_then(|config| {
                config.with_max_expired_removals_per_check(wire.max_expired_removals_per_check)
            })
            .and_then(|config| config.with_max_batch_size(wire.max_batch_size))
            .map_err(serde::de::Error::custom)
    }
}

/// Invalid in-memory store configuration.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MemoryStoreConfigError {
    /// The global key capacity was zero.
    #[error("max_keys must be greater than zero")]
    ZeroMaxKeys,
    /// The shard count was zero.
    #[error("shard_count must be greater than zero")]
    ZeroShardCount,
    /// A shard count greater than the global capacity was requested.
    #[error("shard_count ({shard_count}) cannot exceed max_keys ({max_keys})")]
    MoreShardsThanKeys {
        /// Requested shard count.
        shard_count: usize,
        /// Configured global key capacity.
        max_keys: usize,
    },
    /// The bounded cleanup work limit was zero.
    #[error("max_expired_removals_per_check must be greater than zero")]
    ZeroExpiredRemovalLimit,
    /// The accepted batch size was zero.
    #[error("max_batch_size must be greater than zero")]
    ZeroBatchSize,
}

#[cfg(test)]
mod tests {
    use super::{MemoryStoreConfig, MemoryStoreConfigError};

    #[test]
    fn defaults_to_one_shard_so_all_capacity_is_available() {
        for max_keys in [1, 64, 65, 50_000] {
            let config = MemoryStoreConfig::new(max_keys).unwrap();

            assert_eq!(config.shard_count(), 1);
            assert_eq!(config.shard_capacity(0), max_keys);
        }
    }

    #[test]
    fn distributes_capacity_without_exceeding_global_maximum() {
        let config = MemoryStoreConfig::new(10)
            .unwrap()
            .with_shard_count(3)
            .unwrap();

        assert_eq!(config.shard_capacity(0), 4);
        assert_eq!(config.shard_capacity(1), 3);
        assert_eq!(config.shard_capacity(2), 3);
        assert_eq!(
            (0..config.shard_count())
                .map(|index| config.shard_capacity(index))
                .sum::<usize>(),
            config.max_keys()
        );
    }

    #[test]
    fn rejects_zero_capacity() {
        assert_eq!(
            MemoryStoreConfig::new(0),
            Err(MemoryStoreConfigError::ZeroMaxKeys)
        );
    }

    #[test]
    fn rejects_more_shards_than_keys() {
        assert_eq!(
            MemoryStoreConfig::new(2).unwrap().with_shard_count(3),
            Err(MemoryStoreConfigError::MoreShardsThanKeys {
                shard_count: 3,
                max_keys: 2,
            })
        );
    }
}
