use std::fmt;

use runlimit_core::{BatchError, ConsumptionStatus};
use thiserror::Error;

/// A malformed persisted value or query response that violated Runlimit's
/// storage protocol.
///
/// Construction is owned by the backend so callers cannot fabricate a
/// semantic invariant with a decode source, or discard a source from an
/// invariant that arose while decoding a response. Match
/// [`CheckError::StorageInvariant`] as the operation-level error and use
/// [`Self::detail`] for its stable classification.
#[derive(Debug, Error)]
#[error("{detail}")]
pub struct StorageInvariantError {
    detail: &'static str,
    #[source]
    source: Option<sqlx::Error>,
}

impl StorageInvariantError {
    const fn semantic(detail: &'static str) -> Self {
        Self {
            detail,
            source: None,
        }
    }

    fn decode(detail: &'static str, source: sqlx::Error) -> Self {
        Self {
            detail,
            source: Some(source),
        }
    }

    /// Returns the stable protocol-invariant classification.
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

/// The database phase a quota check was in when its deadline elapsed.
///
/// Every phase runs before commit starts, so a timeout in any of them means
/// the transaction did not commit and no quota was consumed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CheckPhase {
    /// Waiting for a pooled connection.
    AcquiringConnection,
    /// Opening the transaction.
    BeginningTransaction,
    /// Taking the advisory locks that cover the batch's logical keys.
    AcquiringLogicalKeyLocks,
    /// Locking the counter rows that already exist.
    AcquiringCounterRowLocks,
    /// Locking the capacity-ledger shards of keys that need a new row.
    AcquiringCapacityShardLocks,
    /// Sampling database time and finding the first quota denial.
    PreflightingCounters,
    /// Inserting or updating the allowed counters.
    UpdatingCounters,
    /// Refreshing server-side timeouts for the commit statement.
    PreparingCommit,
    /// Checking the remaining budget just before commit.
    StartingCommit,
}

impl fmt::Display for CheckPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AcquiringConnection => "acquiring database connection",
            Self::BeginningTransaction => "beginning transaction",
            Self::AcquiringLogicalKeyLocks => "acquiring logical key lock",
            Self::AcquiringCounterRowLocks => "acquiring counter row lock",
            Self::AcquiringCapacityShardLocks => "acquiring capacity shard lock",
            Self::PreflightingCounters => "preflighting counter batch",
            Self::UpdatingCounters => "updating counter batch",
            Self::PreparingCommit => "preparing commit",
            Self::StartingCommit => "starting commit",
        })
    }
}

/// The database phase an expired-window cleanup was in when its deadline
/// elapsed.
///
/// Every phase runs before commit starts, so a timeout in any of them means no
/// rows were removed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CleanupPhase {
    /// Waiting for a pooled connection.
    AcquiringConnection,
    /// Opening the cleanup transaction.
    BeginningTransaction,
    /// Setting transaction-local server timeouts.
    ConfiguringTimeouts,
    /// Deleting expired rows and releasing their ledger slots.
    DeletingExpiredWindows,
    /// Refreshing server-side timeouts for the commit statement.
    PreparingCommit,
    /// Checking the remaining budget just before commit.
    StartingCommit,
}

impl fmt::Display for CleanupPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AcquiringConnection => "acquiring database connection",
            Self::BeginningTransaction => "beginning cleanup transaction",
            Self::ConfiguringTimeouts => "configuring cleanup transaction timeouts",
            Self::DeletingExpiredWindows => "deleting expired windows",
            Self::PreparingCommit => "preparing cleanup commit",
            Self::StartingCommit => "starting expired-window cleanup commit",
        })
    }
}

/// Failure of a quota check against `PostgreSQL`.
///
/// Every variant can occur for a single check. Batch-only failures live in
/// [`BatchCheckError`]. Match [`CheckError::consumption`] to learn whether the
/// failed operation may already have consumed quota.
#[derive(Debug, Error)]
pub enum CheckError {
    /// `PostgreSQL` did not commit the transaction, so no quota was consumed.
    #[error("PostgreSQL rate-limit check failed before commit; quota was not consumed")]
    DefinitelyNotConsumed(#[source] sqlx::Error),

    /// Commit confirmation was lost, so quota may or may not have been consumed.
    #[error("PostgreSQL rate-limit commit outcome is unknown; quota may have been consumed")]
    CommitOutcomeUnknown(#[source] sqlx::Error),

    /// The pool-acquisition budget or operation deadline elapsed before commit
    /// started.
    #[error("PostgreSQL rate-limit check timed out while {phase}; quota was not consumed")]
    TimedOutBeforeCommit {
        /// Database phase that exhausted the deadline.
        phase: CheckPhase,
    },

    /// The operation deadline elapsed after commit started.
    #[error("PostgreSQL rate-limit commit timed out; quota may have been consumed")]
    CommitTimedOut,

    /// Persisted state or a query response violated an invariant guaranteed
    /// by the migration and SQL protocol. The transaction was rolled back.
    #[error("PostgreSQL rate-limit storage invariant failed: {0}")]
    StorageInvariant(#[source] StorageInvariantError),
}

impl CheckError {
    pub(crate) const fn storage_invariant(detail: &'static str) -> Self {
        Self::StorageInvariant(StorageInvariantError::semantic(detail))
    }

    pub(crate) fn storage_decode_invariant(detail: &'static str, source: sqlx::Error) -> Self {
        Self::StorageInvariant(StorageInvariantError::decode(detail, source))
    }

    /// Reports what is known about quota consumption after this failure.
    ///
    /// A failure before commit definitely consumed nothing. A lost commit
    /// confirmation or a commit timeout may have consumed quota. No failure
    /// reports a definite consumption, because a confirmed commit always
    /// returns a decision.
    pub const fn consumption(&self) -> ConsumptionStatus {
        match self {
            Self::CommitOutcomeUnknown(_) | Self::CommitTimedOut => {
                ConsumptionStatus::PossiblyConsumed
            }
            Self::DefinitelyNotConsumed(_)
            | Self::TimedOutBeforeCommit { .. }
            | Self::StorageInvariant(_) => ConsumptionStatus::NotConsumed,
        }
    }
}

/// Failure of an atomic batch check against `PostgreSQL`.
///
/// A batch can fail structural validation before any database work, or fail
/// in the database like a single check.
#[derive(Debug, Error)]
pub enum BatchCheckError {
    /// The atomic batch violated a backend-independent structural requirement.
    /// No connection was acquired and no quota was consumed.
    #[error(transparent)]
    InvalidBatch(#[from] BatchError),

    /// The database operation failed.
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl BatchCheckError {
    /// Reports what is known about quota consumption after this failure.
    pub const fn consumption(&self) -> ConsumptionStatus {
        match self {
            Self::InvalidBatch(_) => ConsumptionStatus::NotConsumed,
            Self::Check(error) => error.consumption(),
        }
    }
}

/// Failure from bounded expired-window cleanup.
#[derive(Debug, Error)]
pub enum MaintenanceError {
    /// `PostgreSQL` did not commit the cleanup transaction, so no rows were
    /// removed.
    #[error("PostgreSQL expired-window cleanup failed before commit; no rows were removed")]
    Database(#[source] sqlx::Error),
    /// The pool-acquisition budget or operation deadline elapsed before cleanup
    /// commit started.
    #[error("PostgreSQL expired-window cleanup timed out while {phase}; no rows were removed")]
    TimedOutBeforeCommit {
        /// Database phase that exhausted the deadline.
        phase: CleanupPhase,
    },
    /// Commit confirmation was lost, so rows may or may not have been removed.
    #[error(
        "PostgreSQL expired-window cleanup commit outcome is unknown; rows may have been removed"
    )]
    CommitOutcomeUnknown(#[source] sqlx::Error),
    /// The operation deadline elapsed after cleanup commit started.
    #[error("PostgreSQL expired-window cleanup commit timed out; rows may have been removed")]
    CommitTimedOut,
}

impl MaintenanceError {
    /// Reports whether the failed cleanup may have committed row removals.
    ///
    /// Cleanup has exactly two failure states, definitely no effect and
    /// unknown, so a boolean describes them without loss.
    pub const fn may_have_removed_rows(&self) -> bool {
        matches!(self, Self::CommitOutcomeUnknown(_) | Self::CommitTimedOut)
    }
}
