//! Audited PostgreSQL 18 control-plane storage for Batter flags.
//!
//! [`PooledStore`] owns transaction ordering, deadlines and connection retirement
//! through `batter-sqlx`. The six tables and migration ledger live in `flags`;
//! evaluation and publication remain in `batter-flags-core` and `batter-flags`.
//! No operation retries. Uncertain commits require application reconciliation.
//!
//! ```no_run
//! use batter_flags_core::{FlagKey, SwitchState, Version};
//! use batter_flags_postgres::{Actor, Change, ChangeReason, PooledStore,
//!     Precondition, StoreTiming, Write, WriteOutcome};
//! use std::time::Duration;
//! use tokio::time::Instant;
//! # async fn example(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
//! let timing = StoreTiming::new(Duration::from_secs(2), Duration::from_secs(2),
//!     Duration::from_secs(1), Duration::from_secs(2))?;
//! let store = PooledStore::new(pool, timing);
//! store.migrate(Instant::now() + Duration::from_secs(30)).await?;
//! let change = Change::new(Actor::new("operator")?, ChangeReason::new("contain failures")?,
//!     Precondition::UnchangedSince(Version::new(0)));
//! let outcome = store.write(Write::kill(FlagKey::new("outbound")?, SwitchState::Engaged, change),
//!     Instant::now() + Duration::from_secs(3)).await?;
//! match outcome {
//!     WriteOutcome::Written { .. } | WriteOutcome::WrittenUndeclared { .. } => {},
//!     WriteOutcome::Unchanged { .. } => {},
//!     WriteOutcome::RevisionMismatch { .. } => {},
//!     WriteOutcome::Refused(_) => {},
//!     WriteOutcome::Uncertain { .. } => { /* reconcile; never blindly replay */ },
//! }
//! # Ok(()) }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]
mod atomic;
mod catalogs;
mod errors;
mod fleet;
mod history;
mod listing;
mod load;
mod migrate;
mod outcome;
mod prepared;
mod presence;
mod prune;
mod relations;
mod request;
mod state;
mod store;
mod validate;
mod write;
pub use errors::{ErrorClass, StoreError, ValidationError};
pub use fleet::{Fleet, FleetProcess, ProcessState, Rollout};
pub use history::HistoryEntry;
pub use listing::{Coordination, Declaration, Expiry, KeyListing, Reference, Stored};
pub use load::Loaded;
pub use migrate::{CREATE_FLAGS_SQL, MIGRATOR, REQUIRED_SCHEMA_REVISION};
pub use outcome::{
    BlockingBuild, CommitUncertainty, Presence, Refusal, StoredState, UndeclaredBasis, WriteOutcome,
};
pub use presence::{
    Cadence, KeySample, PresenceReport, ProcessId, ProcessLabel, ProcessLabels, ProcessPresence,
    SnapshotReport, StopFailure, StopOutcome,
};
pub use prune::{PruneLimit, Pruned};
pub use relations::{Basis, Deferral, UnmetInvariant};
pub use request::{
    Actor, Archive, Change, ChangeReason, ExplicitValue, Precondition, Variant, Write,
};
pub use store::{PooledStore, StoreTiming};
pub use write::{FlipPolicy, InvariantPolicy, Request, RollbackFailure, WritePolicy};
#[cfg(test)]
extern crate self as batter_flags_postgres;
#[cfg(test)]
mod tests;
