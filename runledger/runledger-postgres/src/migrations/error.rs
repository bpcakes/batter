use std::fmt;

use sqlx::migrate::MigrateError;

use super::{OnlineIndexProblem, WorkflowJobLinkTriggerDiagnostic};

mod cleanup;
mod online;
mod rollback;
pub use cleanup::MigrationCleanupError;
pub use online::{
    MigrationLockTimeout, MigrationTimeoutSetting, OnlineIndexExecutionError, OnlineIndexPhase,
};
pub use rollback::OnlineIndexRollbackError;

#[derive(Debug)]
#[non_exhaustive]
pub enum SchemaCompatibilityError {
    Query(sqlx::Error),
    Snapshot(Box<batter_sqlx::PgSnapshotError<SchemaCompatibilityError>>),
    MissingMigrationHistory {
        required_first_migration_version: i64,
    },
    LegacyIdempotencySnapshotsMissing {
        job_count: i64,
        workflow_count: i64,
    },
    WorkflowJobLinkExpandInvalid {
        compatibility_trigger_count: i64,
        inconsistent_link_count: i64,
    },
    WorkflowJobLinkExpandTriggersInvalid {
        trigger_diagnostics: Vec<WorkflowJobLinkTriggerDiagnostic>,
        inconsistent_link_count: i64,
    },
    Incompatible(MigrateError),
    MigrationUnlock(MigrateError),
    /// Migration or preparation and its advisory-lock release both failed.
    MigrationCleanup(MigrationCleanupError),
    /// A local wait for the migration lock exhausted its profile bound.
    MigrationLockTimeout(MigrationLockTimeout),
    /// Online index execution failed; inspect the retained phase and causes.
    OnlineIndexExecution(OnlineIndexExecutionError),
    /// Guarded INVALID-index removal and its rollback both failed.
    OnlineIndexRollback(OnlineIndexRollbackError),
    /// Commit of guarded INVALID-index removal was not acknowledged. The
    /// removal may have committed; inspect the retained native cause explicitly.
    ///
    /// ```rust,no_run
    /// # fn inspect(cause: &runledger_postgres::SchemaCompatibilityError) {
    /// if let runledger_postgres::SchemaCompatibilityError::OnlineIndexRepairCommitUnconfirmed(error) = cause {
    ///     let native = error.sqlx_error();
    ///     // No subsequent build ran, but removal may have committed.
    /// }
    /// # }
    /// ```
    OnlineIndexRepairCommitUnconfirmed(crate::CommitUnconfirmed),
    /// A declared online index of an applied migration is absent, `INVALID`,
    /// or defined differently; see [`OnlineIndexProblem`].
    OnlineIndex {
        index: &'static str,
        problem: OnlineIndexProblem,
    },
}

impl fmt::Display for SchemaCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MigrationCleanup(error) => fmt::Display::fmt(error, f),
            Self::MigrationLockTimeout(error) => fmt::Display::fmt(error, f),
            Self::OnlineIndexExecution(error) => fmt::Display::fmt(error, f),
            Self::OnlineIndexRollback(error) => fmt::Display::fmt(error, f),
            Self::OnlineIndexRepairCommitUnconfirmed(error) => fmt::Display::fmt(error, f),
            Self::Snapshot(_) => f.write_str("Runledger schema snapshot failed"),
            Self::Query(error) => write!(
                f,
                "Runledger schema compatibility check could not query PostgreSQL state: {error}"
            ),
            Self::MissingMigrationHistory {
                required_first_migration_version,
            } => write!(
                f,
                "Runledger schema compatibility check requires the _sqlx_migrations table; apply or record Runledger migrations first (expected migration history starting at version {required_first_migration_version})"
            ),
            Self::LegacyIdempotencySnapshotsMissing {
                job_count,
                workflow_count,
            } => write!(
                f,
                "Runledger idempotency cutover requires enqueue_request snapshots for all keyed rows; found {job_count} legacy job rows and {workflow_count} legacy workflow rows"
            ),
            Self::WorkflowJobLinkExpandInvalid {
                compatibility_trigger_count,
                inconsistent_link_count,
            } => write!(
                f,
                "Runledger workflow-step/job expand schema requires all three expand-window triggers and empty reciprocal anti-joins before the contract migration; found {compatibility_trigger_count} valid triggers and {inconsistent_link_count} inconsistent relationships"
            ),
            Self::WorkflowJobLinkExpandTriggersInvalid {
                trigger_diagnostics,
                inconsistent_link_count,
            } => {
                f.write_str(
                    "Runledger workflow-step/job expand schema has invalid expand-window triggers: ",
                )?;
                for (index, diagnostic) in trigger_diagnostics.iter().enumerate() {
                    if index != 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{diagnostic}")?;
                }
                write!(
                    f,
                    "; found {inconsistent_link_count} inconsistent relationships"
                )
            }
            Self::OnlineIndex { index, problem } => {
                write!(f, "Runledger online index {index} {problem}")
            }
            Self::Incompatible(error) => write!(f, "{error}"),
            Self::MigrationUnlock(error) => {
                write!(
                    f,
                    "Runledger schema migration lock could not be released: {error}"
                )
            }
        }
    }
}

impl std::error::Error for SchemaCompatibilityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MigrationCleanup(error) => Some(error),
            Self::MigrationLockTimeout(error) => Some(error),
            Self::OnlineIndexExecution(error) => Some(error),
            Self::OnlineIndexRollback(error) => Some(error),
            Self::OnlineIndexRepairCommitUnconfirmed(error) => Some(error),
            Self::Query(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::MissingMigrationHistory { .. } | Self::OnlineIndex { .. } => None,
            Self::LegacyIdempotencySnapshotsMissing { .. } => None,
            Self::WorkflowJobLinkExpandInvalid { .. }
            | Self::WorkflowJobLinkExpandTriggersInvalid { .. } => None,
            Self::Incompatible(error) | Self::MigrationUnlock(error) => Some(error),
        }
    }
}

impl From<MigrateError> for SchemaCompatibilityError {
    fn from(error: MigrateError) -> Self {
        Self::Incompatible(error)
    }
}

impl From<sqlx::Error> for SchemaCompatibilityError {
    fn from(error: sqlx::Error) -> Self {
        Self::Query(error)
    }
}
