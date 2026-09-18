//! Factory-owned native Runlimit admission under Batter operation deadlines.
//!
//! [`Quota::run`] performs one atomic native quota check before invoking work.
//! It retains quota truth separately from the application's concrete outcome.
//! No automatic replay, quota refund, detached-task supervision or external
//! effect rollback is implied. Unix only, like the foundation.
//!
//! Native policies, opaque keys, algorithms, storage, transactions and batches
//! stay in Runlimit. There are no default features. `memory` and `postgres`
//! supply native error-classification bridges; `axum` exposes `http` for
//! authenticated HTTP assembly. PostgreSQL pools, migrations and maintenance
//! remain application/native-owned; this adapter does not prepare their lifecycle.
//!
//! ```
//! # #[cfg(feature = "memory")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use batter::operation::OperationContext;
//! use batter_runlimit::{Checks, Quota, RunResult};
//! use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
//! use runlimit_memory::{MemoryStore, MemoryStoreConfig};
//! use std::{convert::Infallible, time::Duration};
//! let policy = FixedWindowPolicy::new(PolicyId::new("api.read")?, ScopeId::new("owner")?, 5, Duration::from_secs(60))?;
//! let hasher = KeyHasher::new([7; 32])?; // Test key; production loads its own secret.
//! let checks = [Check::new(&policy, hasher.hash_for(&policy, "owner-a"))];
//! let quota = Quota::new(MemoryStore::new(MemoryStoreConfig::new(100)?));
//! let context = OperationContext::new(Duration::from_secs(1))?;
//! let result = quota.run(&context, Checks::new(&checks)?, |_| async {
//!     Ok::<_, Infallible>(42)
//! }).await;
//! assert!(matches!(result, RunResult::Admitted { work: Ok(42), .. }));
//! # Ok(()) }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod quota;
pub use quota::{
    Admission, AllowedBatch, AllowedDecision, Checks, ConsumptionError, EmptyChecks,
    InterruptedCheck, Quota, RunResult,
};

/// Authenticated HTTP assembly, selected with the `axum` feature.
#[cfg(feature = "axum")]
pub mod http;
