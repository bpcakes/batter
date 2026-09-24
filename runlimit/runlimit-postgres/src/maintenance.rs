use std::{future::Future, time::Duration};

use sqlx::{Acquire, PgPool, Postgres, Transaction, pool::PoolConnection};
use tokio::time::{Instant, timeout, timeout_at};

use crate::{
    CleanupPhase, ConnectionOutcome, MaintenanceError,
    protocol::{
        CLEANUP_SQL, SET_LOCAL_TIMEOUTS_SQL, is_server_timeout, remaining_server_timeout_settings,
    },
};

pub(crate) async fn acquire_maintenance_connection(
    pool: &PgPool,
    acquire_timeout: Duration,
) -> Result<PoolConnection<Postgres>, MaintenanceError> {
    timeout(acquire_timeout, pool.acquire())
        .await
        .map_err(|_| MaintenanceError::TimedOutBeforeCommit {
            phase: CleanupPhase::AcquiringConnection,
        })?
        .map_err(MaintenanceError::Database)
}

pub(crate) async fn run_cleanup_transaction(
    connection: &mut PoolConnection<Postgres>,
    maximum_rows: u32,
    deadline: Instant,
) -> ConnectionOutcome<Result<u64, MaintenanceError>> {
    match run_cleanup_transaction_inner(connection, maximum_rows, deadline).await {
        Ok(rows) => ConnectionOutcome::Reusable(Ok(rows)),
        Err(outcome) => outcome.map(Err),
    }
}

async fn run_cleanup_transaction_inner(
    connection: &mut PoolConnection<Postgres>,
    maximum_rows: u32,
    deadline: Instant,
) -> Result<u64, ConnectionOutcome<MaintenanceError>> {
    let mut transaction = maintenance_before_commit(
        deadline,
        CleanupPhase::BeginningTransaction,
        connection.begin(),
    )
    .await?;
    set_maintenance_server_timeouts(
        &mut transaction,
        deadline,
        CleanupPhase::ConfiguringTimeouts,
    )
    .await?;
    // The imported cleanup SQL has unqualified count(*) calls. Put pg_catalog
    // first for this transaction so they resolve to the inspected built-in,
    // while preserving the caller's relation search path after it.
    let search_path: String = maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query_scalar("SHOW search_path").fetch_one(&mut *transaction),
    )
    .await?;
    maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query("SELECT pg_catalog.set_config('search_path', $1, true)")
            .bind(format!("pg_catalog, {search_path}"))
            .execute(&mut *transaction),
    )
    .await?;
    let result = maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query(CLEANUP_SQL)
            .bind(i64::from(maximum_rows))
            .execute(&mut *transaction),
    )
    .await?;
    let rows_affected = result.rows_affected();

    commit_maintenance(deadline, transaction).await?;
    Ok(rows_affected)
}

pub(crate) async fn maintenance_before_commit<T, F>(
    deadline: Instant,
    phase: CleanupPhase,
    future: F,
) -> Result<T, ConnectionOutcome<MaintenanceError>>
where
    F: Future<Output = Result<T, sqlx::Error>>,
{
    timeout_at(deadline, future)
        .await
        .map_err(|_| {
            ConnectionOutcome::MustClose(MaintenanceError::TimedOutBeforeCommit { phase })
        })?
        .map_err(|error| {
            let public = if is_server_timeout(&error) {
                MaintenanceError::TimedOutBeforeCommit { phase }
            } else {
                MaintenanceError::Database(error)
            };
            ConnectionOutcome::Reusable(public)
        })
}

pub(crate) async fn set_maintenance_server_timeouts(
    transaction: &mut Transaction<'_, Postgres>,
    deadline: Instant,
    phase: CleanupPhase,
) -> Result<(), ConnectionOutcome<MaintenanceError>> {
    let (statement_timeout, lock_timeout) = remaining_server_timeout_settings(deadline).ok_or(
        ConnectionOutcome::Reusable(MaintenanceError::TimedOutBeforeCommit { phase }),
    )?;
    maintenance_before_commit(
        deadline,
        phase,
        sqlx::query(SET_LOCAL_TIMEOUTS_SQL)
            .bind(statement_timeout)
            .bind(lock_timeout)
            .execute(&mut **transaction),
    )
    .await
    .map(|_| ())
}

pub(crate) async fn commit_maintenance(
    deadline: Instant,
    mut transaction: Transaction<'_, Postgres>,
) -> Result<(), ConnectionOutcome<MaintenanceError>> {
    if Instant::now() >= deadline {
        return Err(ConnectionOutcome::Reusable(
            MaintenanceError::TimedOutBeforeCommit {
                phase: CleanupPhase::StartingCommit,
            },
        ));
    }

    set_maintenance_server_timeouts(&mut transaction, deadline, CleanupPhase::PreparingCommit)
        .await?;

    timeout_at(deadline, transaction.commit())
        .await
        .map_err(|_| ConnectionOutcome::MustClose(MaintenanceError::CommitTimedOut))?
        .map_err(|error| ConnectionOutcome::Reusable(MaintenanceError::CommitOutcomeUnknown(error)))
}
