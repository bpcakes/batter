//! Hard-bounded, process-local rate-limit storage.
//!
//! [`MemoryStore`] implements the same anchored fixed-window model as the
//! `PostgreSQL` backend. [`GcraStore`] implements a continuously replenished
//! generic-cell-rate-algorithm model for process-local use. Both stores
//! deliberately refuse to evict live entries. When a shard is full, a new
//! subject is denied until bounded cleanup frees space. An atomic batch that
//! can never fit in one shard returns a structural [`MemoryBatchError`]
//! (wrapped by [`GcraBatchError`] for GCRA admission) instead of a retryable
//! capacity denial. A single [`MemoryStore`] check can fail only because its
//! shard is poisoned, so its error type is the bare [`PoisonedShardError`]. A
//! single [`GcraStore`] check returns [`GcraCheckError`], which also names an
//! exact-arithmetic overflow. Both stores implement [`runlimit_core::Limiter`]
//! for async generic adapters while retaining their synchronous inherent check
//! methods.
//!
//! The optional `serde` feature enables validated [`MemoryStoreConfig`]
//! loading, read-only [`MemoryStoreStats`] serialization, and the corresponding
//! `runlimit-core` metadata feature.

pub mod attempts;
mod clock;
mod config;
mod gcra;
mod shards;
mod store;

pub use clock::{Clock, SystemClock};
pub use config::{MemoryStoreConfig, MemoryStoreConfigError};
pub use gcra::{GcraBatchError, GcraCheckError, GcraStore, GcraStoreBuilder};
pub use store::{
    MemoryBatchError, MemoryStore, MemoryStoreBuilder, MemoryStoreStats, PoisonedShardError,
};
