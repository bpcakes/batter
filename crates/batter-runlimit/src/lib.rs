//! Factory-owned native Runlimit admission under Batter operation deadlines.
//!
//! [`Quota::run`] performs one atomic native quota check before invoking work.
//! It retains quota truth separately from the application's concrete outcome.
//! No automatic replay, quota refund, detached-task supervision or external
//! effect rollback is implied. Unix only, like the foundation.
//!
//! Native policies, opaque keys, algorithms, storage, transactions and batches
//! stay in Runlimit. There are no default features. `memory` and `postgres`
//! supply native error-classification bridges and re-export their native
//! packages as `memory` and `postgres`; `axum` exposes `http` for
//! authenticated HTTP assembly. PostgreSQL pools, migrations and maintenance
//! remain application/native-owned; this adapter does not prepare their lifecycle.
//!
//! The separate `native-http` and `native-axum` features expose the native
//! transport packages under `native_transport`. They are low-level and are not
//! interchangeable with `http`; see that module for their caller obligations.
//!
//! ```
//! # #[cfg(feature = "memory")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use batter_core::operation::OperationContext;
//! use batter_runlimit::{Checks, Quota, RunResult};
//! use runlimit_core::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
//! use runlimit_memory::{MemoryStore, MemoryStoreConfig};
//! use std::{convert::Infallible, time::Duration};
//! let policy = FixedWindowPolicy::new(PolicyId::new("api.read")?, ScopeId::new("owner")?, 5, Duration::from_secs(60))?;
//! let hasher = KeyHasher::new([7; 32])?; // Test key; production loads its own secret.
//! let checks = [Check::new(hasher.hash_for(&policy, "owner-a"))];
//! let quota = Quota::new(MemoryStore::new(MemoryStoreConfig::new(100)?));
//! let context = batter_core::operation::OperationOwner::new(Duration::from_secs(1))?.into_context();
//! let result = quota.run(&context, Checks::new(&checks)?, |_| async {
//!     Ok::<_, Infallible>(42)
//! }).await;
//! assert!(matches!(result, RunResult::Admitted { work: Ok(42), .. }));
//! # Ok(()) }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Outcome-aware PostgreSQL attempts under one operation and transaction owner.
#[cfg(feature = "postgres")]
pub mod attempts;
pub mod quota;

/// The exact native policy and decision types used by this adapter.
pub use runlimit_core as native;

pub use quota::{
    Admission, AllowedBatch, Checks, ConsumptionError, EmptyChecks, InterruptedCheck, Quota,
    RunResult,
};
/// The selected native PostgreSQL backend and its explicit migrations.
#[cfg(feature = "postgres")]
pub use runlimit_postgres as postgres;

/// The selected native process-local backend, selected with the `memory` feature.
///
/// This is the native package itself, so its stores have the same identity
/// through the direct and facade paths. Bounded-capacity behaviour, eviction
/// refusal and shard poisoning remain Runlimit's; this adapter only executes
/// checks before work through [`Quota`].
///
/// ```
/// use batter_runlimit::memory::{MemoryStore, MemoryStoreConfig};
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store: MemoryStore = MemoryStore::new(MemoryStoreConfig::new(64)?);
/// let _quota = batter_runlimit::Quota::new(store);
/// # Ok(())
/// # }
/// # example().unwrap();
/// ```
#[cfg(feature = "memory")]
pub use runlimit_memory as memory;

/// Authenticated HTTP assembly, selected with the `axum` feature.
#[cfg(feature = "axum")]
pub mod http;

/// The native Runlimit transport packages, selected with `native-http` and
/// `native-axum`. They are named apart from `http` on purpose.
///
/// `http`, above, is this adapter's protected assembly: it composes quota
/// admission before body reading under an authenticated boundary Batter owns.
/// Nothing in `native_transport` does that. These are the native packages
/// themselves, with the same type identity as a direct dependency, and every
/// obligation they document stays with the caller:
///
/// - `axum::RateLimitLayer` evaluates one native check per layer and hands the
///   application both trust-sensitive decisions — deriving the subject key from
///   a request, and mapping extraction failures, enforced denials and backend
///   failures to a response. It interprets no forwarding header, connection
///   metadata or identity, selects no status code or body, and is outside
///   Batter's operation deadlines, telemetry and admission ordering.
/// - `http::draft_11` serializes caller-selected policy and decision metadata
///   into `draft-ietf-httpapi-ratelimit-headers-11` fields. That Internet-Draft
///   is an unstable wire contract, not a published RFC, and the versioned module
///   path is where that instability is disclosed.
///
/// Selecting either feature adds no Batter-owned behaviour and does not enable
/// `http`, the `axum` feature or the Batter Axum adapter.
#[cfg(any(feature = "native-http", feature = "native-axum"))]
pub mod native_transport {
    /// The native caller-controlled Axum admission layer.
    ///
    /// The application supplies the subject-key extractor and the rejection
    /// mapper; this layer selects no status code, body or header.
    ///
    /// ```
    /// use batter_runlimit::native_transport::axum::RateLimitRejection;
    /// use runlimit_core::Denial;
    ///
    /// fn status<K, B>(rejection: RateLimitRejection<K, B>) -> u16 {
    ///     match rejection {
    ///         RateLimitRejection::Key(_) => 400,
    ///         RateLimitRejection::Denied(Denial::QuotaExceeded(_)) => 429,
    ///         RateLimitRejection::Denied(Denial::StorageCapacity { .. }) => 503,
    ///         RateLimitRejection::Backend(_) => 503,
    ///     }
    /// }
    /// let _: fn(RateLimitRejection<(), ()>) -> u16 = status;
    /// ```
    #[cfg(feature = "native-axum")]
    pub use runlimit_axum as axum;
    /// Framework-neutral native response metadata for Runlimit decisions.
    ///
    /// The application chooses the disclosed policy name and the response this
    /// metadata accompanies.
    ///
    /// ```
    /// use batter_runlimit::native_transport::http::draft_11;
    /// use runlimit_core::{FixedWindowPolicy, PolicyId, ScopeId};
    /// use std::time::Duration;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let policy = FixedWindowPolicy::new(
    ///     PolicyId::new("example.read")?,
    ///     ScopeId::new("owner")?,
    ///     5,
    ///     Duration::from_secs(60),
    /// )?;
    /// let (name, value) = draft_11::quota_policy("reads", &policy)?;
    /// assert_eq!(name.as_str(), "ratelimit-policy");
    /// assert_eq!(value.to_str()?, "\"reads\";q=5;w=60");
    /// # Ok(())
    /// # }
    /// # example().unwrap();
    /// ```
    #[cfg(feature = "native-http")]
    pub use runlimit_http as http;
}
