use std::time::Duration;

use thiserror::Error;

use crate::HARD_MAX_ROWS_PER_SHARD;

/// Runtime bounds for `PostgreSQL` admission work.
///
/// With the `serde` feature, this is an object containing
/// `maximum_rows_per_shard`, `max_batch_size`, `pool_acquire_timeout`, and
/// `operation_timeout`. Durations use Serde's exact
/// `{ "secs": ..., "nanos": ... }` representation. Every field may be omitted
/// when deserializing to use the constructor defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresConfig {
    maximum_rows_per_shard: u32,
    max_batch_size: usize,
    pool_acquire_timeout: Duration,
    operation_timeout: Duration,
}

impl PostgresConfig {
    /// Default configured row limit for each of the 256 capacity shards.
    pub const DEFAULT_MAXIMUM_ROWS_PER_SHARD: u32 = 4_096;

    /// Default maximum number of checks in one atomic batch.
    pub const DEFAULT_MAX_BATCH_SIZE: usize = 32;

    /// Default budget for acquiring a connection from the pool.
    pub const DEFAULT_POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);

    /// Default deadline for database work after acquiring a connection.
    pub const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(3);

    /// Largest accepted pool-acquisition budget.
    pub const MAXIMUM_POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_mins(1);

    /// Largest accepted database-operation deadline.
    pub const MAXIMUM_OPERATION_TIMEOUT: Duration = Duration::from_mins(1);

    /// Returns the conservative defaults suitable for an online admission
    /// path.
    pub const fn new() -> Self {
        Self {
            maximum_rows_per_shard: Self::DEFAULT_MAXIMUM_ROWS_PER_SHARD,
            max_batch_size: Self::DEFAULT_MAX_BATCH_SIZE,
            pool_acquire_timeout: Self::DEFAULT_POOL_ACQUIRE_TIMEOUT,
            operation_timeout: Self::DEFAULT_OPERATION_TIMEOUT,
        }
    }

    /// Sets the configured admission limit for one capacity shard.
    ///
    /// The database migration independently enforces
    /// [`HARD_MAX_ROWS_PER_SHARD`] for compatibility with older replicas.
    /// Configuring a lower value makes this limiter deny new keys sooner.
    /// Existing keys remain usable, including when the ledger is already above
    /// the newly configured value.
    ///
    /// # Errors
    ///
    /// Returns an error if `maximum` is zero or exceeds the database-enforced
    /// hard maximum.
    pub fn with_maximum_rows_per_shard(
        mut self,
        maximum: u32,
    ) -> Result<Self, PostgresConfigError> {
        if maximum == 0 {
            return Err(PostgresConfigError::ZeroMaximumRowsPerShard);
        }
        if maximum > HARD_MAX_ROWS_PER_SHARD {
            return Err(PostgresConfigError::MaximumRowsPerShardTooLarge {
                actual: maximum,
                maximum: HARD_MAX_ROWS_PER_SHARD,
            });
        }
        self.maximum_rows_per_shard = maximum;
        Ok(self)
    }

    /// Sets the maximum number of checks that may retain locks in one batch.
    ///
    /// # Errors
    ///
    /// Returns an error if `maximum` is zero.
    pub fn with_max_batch_size(mut self, maximum: usize) -> Result<Self, PostgresConfigError> {
        if maximum == 0 {
            return Err(PostgresConfigError::ZeroBatchSize);
        }
        self.max_batch_size = maximum;
        Ok(self)
    }

    /// Sets the budget for acquiring a connection from the pool.
    ///
    /// A successful acquisition receives a fresh
    /// [`Self::operation_timeout`] budget. This timeout is intentionally not
    /// applied to schema migrations.
    ///
    /// # Errors
    ///
    /// Returns an error if `timeout` is zero or exceeds 60 seconds.
    pub fn with_pool_acquire_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, PostgresConfigError> {
        if timeout.is_zero() {
            return Err(PostgresConfigError::ZeroPoolAcquireTimeout);
        }
        if timeout > Self::MAXIMUM_POOL_ACQUIRE_TIMEOUT {
            return Err(PostgresConfigError::PoolAcquireTimeoutTooLong {
                actual: timeout,
                maximum: Self::MAXIMUM_POOL_ACQUIRE_TIMEOUT,
            });
        }
        self.pool_acquire_timeout = timeout;
        Ok(self)
    }

    /// Sets the deadline for database work after acquiring a connection.
    ///
    /// The fresh deadline covers transaction begin, lock waits, statements,
    /// rollback, and commit. Server-side timeouts are refreshed from the
    /// remaining budget before each statement and commit, so an earlier lock
    /// wait cannot give a later statement a fresh full-length timeout.
    /// It is intentionally not applied to pool
    /// acquisition or schema migrations.
    ///
    /// # Errors
    ///
    /// Returns an error if `timeout` is zero or exceeds 60 seconds.
    pub fn with_operation_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, PostgresConfigError> {
        if timeout.is_zero() {
            return Err(PostgresConfigError::ZeroOperationTimeout);
        }
        if timeout > Self::MAXIMUM_OPERATION_TIMEOUT {
            return Err(PostgresConfigError::OperationTimeoutTooLong {
                actual: timeout,
                maximum: Self::MAXIMUM_OPERATION_TIMEOUT,
            });
        }
        self.operation_timeout = timeout;
        Ok(self)
    }

    /// Returns the maximum accepted batch length.
    pub const fn max_batch_size(&self) -> usize {
        self.max_batch_size
    }

    /// Returns the configured admission limit for one capacity shard.
    pub const fn maximum_rows_per_shard(self) -> u32 {
        self.maximum_rows_per_shard
    }

    /// Returns the budget for acquiring a connection from the pool.
    pub const fn pool_acquire_timeout(self) -> Duration {
        self.pool_acquire_timeout
    }

    /// Returns the deadline applied after a connection is acquired.
    pub const fn operation_timeout(self) -> Duration {
        self.operation_timeout
    }
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
struct PostgresConfigRef {
    maximum_rows_per_shard: u32,
    max_batch_size: usize,
    pool_acquire_timeout: Duration,
    operation_timeout: Duration,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PostgresConfigWire {
    #[serde(default = "default_postgres_maximum_rows_per_shard")]
    maximum_rows_per_shard: u32,
    #[serde(default = "default_postgres_max_batch_size")]
    max_batch_size: usize,
    #[serde(default = "default_postgres_pool_acquire_timeout")]
    pool_acquire_timeout: Duration,
    #[serde(default = "default_postgres_operation_timeout")]
    operation_timeout: Duration,
}

#[cfg(feature = "serde")]
const fn default_postgres_maximum_rows_per_shard() -> u32 {
    PostgresConfig::DEFAULT_MAXIMUM_ROWS_PER_SHARD
}

#[cfg(feature = "serde")]
const fn default_postgres_max_batch_size() -> usize {
    PostgresConfig::DEFAULT_MAX_BATCH_SIZE
}

#[cfg(feature = "serde")]
const fn default_postgres_pool_acquire_timeout() -> Duration {
    PostgresConfig::DEFAULT_POOL_ACQUIRE_TIMEOUT
}

#[cfg(feature = "serde")]
const fn default_postgres_operation_timeout() -> Duration {
    PostgresConfig::DEFAULT_OPERATION_TIMEOUT
}

#[cfg(feature = "serde")]
impl serde::Serialize for PostgresConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(
            &PostgresConfigRef {
                maximum_rows_per_shard: self.maximum_rows_per_shard(),
                max_batch_size: self.max_batch_size(),
                pool_acquire_timeout: self.pool_acquire_timeout(),
                operation_timeout: self.operation_timeout(),
            },
            serializer,
        )
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for PostgresConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = <PostgresConfigWire as serde::Deserialize>::deserialize(deserializer)?;
        Self::new()
            .with_maximum_rows_per_shard(wire.maximum_rows_per_shard)
            .and_then(|config| config.with_max_batch_size(wire.max_batch_size))
            .and_then(|config| config.with_pool_acquire_timeout(wire.pool_acquire_timeout))
            .and_then(|config| config.with_operation_timeout(wire.operation_timeout))
            .map_err(serde::de::Error::custom)
    }
}

/// Invalid `PostgreSQL` backend configuration.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum PostgresConfigError {
    /// A zero per-shard maximum would reject every new storage key.
    #[error("maximum_rows_per_shard must be greater than zero")]
    ZeroMaximumRowsPerShard,
    /// The configured bound exceeds the ceiling enforced by the database
    /// migration.
    #[error("maximum_rows_per_shard ({actual}) exceeds database-enforced maximum ({maximum})")]
    MaximumRowsPerShardTooLarge {
        /// Supplied per-shard row bound.
        actual: u32,
        /// Largest database-enforced bound.
        maximum: u32,
    },
    /// A zero maximum batch size would reject every nonempty batch.
    #[error("max_batch_size must be greater than zero")]
    ZeroBatchSize,
    /// A zero acquisition budget cannot permit waiting for a pool connection.
    #[error("PostgreSQL pool acquisition timeout must be greater than zero")]
    ZeroPoolAcquireTimeout,
    /// A pool-acquisition budget longer than one minute is not operationally
    /// bounded enough for this backend.
    #[error("PostgreSQL pool acquisition timeout {actual:?} exceeds maximum {maximum:?}")]
    PoolAcquireTimeoutTooLong {
        /// Supplied timeout.
        actual: Duration,
        /// Largest accepted timeout.
        maximum: Duration,
    },
    /// A zero deadline cannot permit database work.
    #[error("PostgreSQL rate-limit operation timeout must be greater than zero")]
    ZeroOperationTimeout,
    /// An admission deadline longer than one minute is not operationally
    /// bounded enough for this backend.
    #[error("PostgreSQL rate-limit operation timeout {actual:?} exceeds maximum {maximum:?}")]
    OperationTimeoutTooLong {
        /// Supplied timeout.
        actual: Duration,
        /// Largest accepted timeout.
        maximum: Duration,
    },
}
