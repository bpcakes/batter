//! Read-only installation inspection, independent of host migration numbering.

use sqlx::{Acquire, PgConnection};
use thiserror::Error;
use tokio::time::{Instant, timeout, timeout_at};

use crate::{
    BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL, CAPACITY_SHARD_COUNT, ConnectionCancellationGuard,
    HARD_MAX_ROWS_PER_SHARD, MIGRATOR, PostgresLimiter,
    protocol::{SET_LOCAL_TIMEOUTS_SQL, is_server_timeout, remaining_server_timeout_settings},
};

const CATALOG_SQL: &str = include_str!("installation.sql");
const FUNCTION_NAMES: [&str; 3] = [
    "runlimit_fixed_windows_reject_storage_key_update",
    "runlimit_fixed_windows_capacity_after_insert",
    "runlimit_fixed_windows_capacity_after_delete",
];

/// Who owns the installed migration history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationHistory {
    /// Require Runlimit's published versions, successful status and checksums
    /// in the search-path-resolved `_sqlx_migrations` table. Unrelated versions
    /// are ignored. Schema and permission checks still run.
    Bundled,
    /// Inspect the installed schema and permissions without reading a migration
    /// ledger. Use when the application vendors the unchanged published SQL
    /// into its own migration stream with different versions or ledger names.
    ApplicationManaged,
}

/// An unmet installation requirement. Contains object names, never subjects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationIssue {
    object: String,
    requirement: String,
}

impl InstallationIssue {
    /// Table, column, constraint, function, or setting requiring attention.
    pub fn object(&self) -> &str {
        &self.object
    }

    /// The expected schema property or effective permission.
    pub fn requirement(&self) -> &str {
        &self.requirement
    }
}

/// Installation inspection failed. No variant implies quota consumption or
/// changes to schema, migration history, counters, or the capacity ledger.
#[derive(Debug, Error)]
pub enum InstallationError {
    /// The installed schema, permissions, or selected migration history do not
    /// meet Runlimit's requirements. Inspect every issue for remediation.
    #[error("Runlimit PostgreSQL installation is incompatible: {issues:?}")]
    Incompatible {
        /// Unmet requirements, ordered by object and requirement.
        issues: Vec<InstallationIssue>,
    },
    /// PostgreSQL or `SQLx` could not complete inspection. This is not evidence
    /// that the installation is compatible.
    #[error("could not inspect Runlimit PostgreSQL installation: {0}")]
    Database(#[from] sqlx::Error),
    /// The configured acquisition or database-operation budget elapsed.
    #[error("Runlimit PostgreSQL installation validation timed out while {operation}")]
    TimedOut {
        /// Phase that exhausted its budget.
        operation: &'static str,
    },
}

impl PostgresLimiter {
    /// Validates this connection pool's installed Runlimit schema and effective
    /// runtime permissions without applying migrations or consuming quota.
    ///
    /// Both history modes inspect tables, columns, constraints, indexes, the
    /// published capacity-trigger protocol, and all 256 ledger slots. Column
    /// grants and inherited grants are accepted. Extra privileges are allowed;
    /// this is not a least-privilege audit. The inspection does not scan counters
    /// or prove that ledger counts match them.
    ///
    /// Uses one read-only, repeatable-read transaction with this limiter's
    /// acquisition and operation budgets. Cancellation or unsuccessful rollback
    /// discards the connection. No test writes or trigger calls are performed.
    ///
    /// Configure every pool connection with the same role and search path.
    /// Success describes the inspected snapshot; later DDL, grants, role changes,
    /// or different session configuration can invalidate it. RLS, additional
    /// user triggers/rules/behavioral constraints, write-time extra columns or
    /// expression/partial/unique indexes, and modified published function bodies
    /// are unsupported. Application-managed migrations
    /// may renumber the published SQL but must preserve its object names and definitions.
    ///
    /// # Errors
    ///
    /// Returns [`InstallationError::Incompatible`] for unmet requirements, or a
    /// database/timeout error if inspection cannot finish. Every outcome is
    /// read-only and safe to retry.
    pub async fn validate_installation(
        &self,
        history: MigrationHistory,
    ) -> Result<(), InstallationError> {
        let connection = timeout(self.config.pool_acquire_timeout(), self.pool.acquire())
            .await
            .map_err(|_| InstallationError::TimedOut {
                operation: "acquiring database connection",
            })??;
        let mut guarded = ConnectionCancellationGuard::new(connection);
        let deadline = Instant::now() + self.config.operation_timeout();
        let result = timeout_at(deadline, async {
            let mut transaction = guarded.connection().begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
                .execute(&mut *transaction)
                .await?;
            let (statement, lock) =
                remaining_server_timeout_settings(deadline).ok_or(InstallationError::TimedOut {
                    operation: "starting installation inspection",
                })?;
            sqlx::query(SET_LOCAL_TIMEOUTS_SQL)
                .bind(statement)
                .bind(lock)
                .execute(&mut *transaction)
                .await?;
            let result = inspect(&mut transaction, history).await;
            transaction.rollback().await?;
            guarded.reuse();
            result
        })
        .await;
        result
            .unwrap_or(Err(InstallationError::TimedOut {
                operation: "inspecting installation",
            }))
            .map_err(|error| match error {
                InstallationError::Database(ref database) if is_server_timeout(database) => {
                    InstallationError::TimedOut {
                        operation: "inspecting installation",
                    }
                }
                other => other,
            })
    }
}

async fn inspect(
    connection: &mut PgConnection,
    history: MigrationHistory,
) -> Result<(), InstallationError> {
    // The three function bodies are embedded in immutable published SQL. Reuse
    // those bytes instead of maintaining another copy of the capacity protocol.
    let bodies = published_function_bodies();
    let rows: Vec<(String, String)> = sqlx::query_as(CATALOG_SQL)
        .bind(FUNCTION_NAMES.as_slice())
        .bind(bodies)
        .fetch_all(&mut *connection)
        .await?;
    let mut issues: Vec<_> = rows
        .into_iter()
        .map(|(object, requirement)| InstallationIssue {
            object,
            requirement,
        })
        .collect();
    // Only read installed tables after their shapes and permissions are known.
    // Catalog failures remain useful without triggering secondary SQL errors.
    if issues.is_empty() {
        inspect_ledger(connection, &mut issues).await?;
    }
    match history {
        MigrationHistory::Bundled => inspect_history(connection, &mut issues).await?,
        MigrationHistory::ApplicationManaged => {}
    }
    issues.sort_by(|left, right| {
        (&left.object, &left.requirement).cmp(&(&right.object, &right.requirement))
    });
    if issues.is_empty() {
        Ok(())
    } else {
        Err(InstallationError::Incompatible { issues })
    }
}

fn published_function_bodies() -> Vec<&'static str> {
    BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL
        .split("AS $$")
        .skip(1)
        .filter_map(|part| part.split_once("$$;").map(|(body, _)| body))
        .collect()
}

async fn inspect_ledger(
    connection: &mut PgConnection,
    issues: &mut Vec<InstallationIssue>,
) -> Result<(), sqlx::Error> {
    let rows: Vec<(i16, i64)> = sqlx::query_as(
        "SELECT capacity_shard, row_count FROM runlimit_capacity_shards \
         ORDER BY capacity_shard LIMIT 257",
    )
    .fetch_all(connection)
    .await?;
    let valid = rows.len() == CAPACITY_SHARD_COUNT
        && rows.iter().enumerate().all(|(index, &(shard, count))| {
            usize::try_from(shard) == Ok(index)
                && (0..=i64::from(HARD_MAX_ROWS_PER_SHARD)).contains(&count)
        });
    if !valid {
        issues.push(InstallationIssue {
            object: "runlimit_capacity_shards".into(),
            requirement: "all 256 shard rows, numbered 0..255, with counts in 0..65536".into(),
        });
    }
    Ok(())
}

async fn inspect_history(
    connection: &mut PgConnection,
    issues: &mut Vec<InstallationIssue>,
) -> Result<(), sqlx::Error> {
    let readable: bool = sqlx::query_scalar(
        "SELECT COALESCE(pg_catalog.has_table_privilege(\
         pg_catalog.to_regclass('_sqlx_migrations'), 'SELECT'), false)",
    )
    .fetch_one(&mut *connection)
    .await?;
    if !readable {
        issues.push(InstallationIssue {
            object: "_sqlx_migrations".into(),
            requirement: "bundled SQLx history table with SELECT privilege".into(),
        });
        return Ok(());
    }
    for migration in MIGRATOR.iter() {
        let row: Option<(bool, Vec<u8>)> =
            sqlx::query_as("SELECT success, checksum FROM _sqlx_migrations WHERE version = $1")
                .bind(migration.version)
                .fetch_optional(&mut *connection)
                .await?;
        if !row
            .is_some_and(|(success, checksum)| success && checksum == migration.checksum.as_ref())
        {
            issues.push(InstallationIssue {
                object: format!("_sqlx_migrations[{}]", migration.version),
                requirement: "successful bundled migration with its published checksum".into(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_trigger_bodies_follow_the_named_functions() {
        let bodies = published_function_bodies();
        assert_eq!(bodies.len(), FUNCTION_NAMES.len());
        for (name, body) in FUNCTION_NAMES.iter().zip(bodies) {
            let definition = format!("CREATE FUNCTION {name}()");
            let (_, rest) = BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL
                .split_once(&definition)
                .unwrap();
            assert_eq!(
                rest.split_once("AS $$")
                    .unwrap()
                    .1
                    .split_once("$$;")
                    .unwrap()
                    .0,
                body
            );
        }
    }
}
