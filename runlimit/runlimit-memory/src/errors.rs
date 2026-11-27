//! Failures both bounded stores share: a poisoned shard, and an atomic batch
//! the shards reject before any quota is evaluated.

use runlimit_core::BatchError;
use thiserror::Error;

/// A shard mutex was poisoned by a panic and remains unavailable.
///
/// This is the only failure a single check, [`MemoryStore::stats`], or
/// [`MemoryStore::clear`] can report. Every operation touching the shard
/// fails closed until [`MemoryStore::recover_poisoned`] rebuilds it.
///
/// [`MemoryStore::stats`]: crate::MemoryStore::stats
/// [`MemoryStore::clear`]: crate::MemoryStore::clear
/// [`MemoryStore::recover_poisoned`]: crate::MemoryStore::recover_poisoned
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("memory-store shard {shard_index} is poisoned and unavailable")]
pub struct PoisonedShardError {
    /// Poisoned shard index.
    pub shard_index: usize,
}

/// A fail-closed failure of an in-memory atomic batch.
///
/// Every variant is reachable from [`MemoryStore::check_all`]; failures that
/// only a batch can produce never appear on the single-check path.
///
/// [`MemoryStore::check_all`]: crate::MemoryStore::check_all
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MemoryBatchError {
    /// The atomic batch violated a backend-independent structural requirement.
    #[error(transparent)]
    InvalidBatch(#[from] BatchError),
    /// An atomic batch can never fit in one shard, regardless of expiry.
    #[error(
        "batch targets {key_count} distinct keys at memory-store shard {shard_index}, \
         but that shard can store at most {capacity}"
    )]
    BatchExceedsShardCapacity {
        /// Index of the shard that cannot hold all targeted keys.
        shard_index: usize,
        /// Number of distinct batch keys targeting the shard.
        key_count: usize,
        /// Hard key capacity of the shard.
        capacity: usize,
    },
    /// A touched shard was poisoned and remains unavailable.
    #[error(transparent)]
    PoisonedShard(#[from] PoisonedShardError),
}
