//! An INVALID observation is not authority to drop a later name occupant.
//! Keep object validation and removal in one bounded metadata transaction.
//! A refused lock leaves nothing changed, so the whole attempt is retried
//! briefly instead of queuing an exclusive lock behind queue traffic.

use std::time::Duration;

use sqlx::{AssertSqlSafe, Connection, PgConnection};

use super::{IndexState, OnlineIndex, OnlineIndexProblem, check_valid, index_state, problem};
use crate::{OnlineIndexRollbackError, RunledgerDatabase, SchemaCompatibilityError};

/// Time after the first attempt within which a refused attempt is retried.
const LOCK_RETRY_BUDGET: Duration = Duration::from_secs(5);
const FIRST_RETRY_DELAY: Duration = Duration::from_millis(10);
const MAX_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Returns false when the locked object has become a valid canonical index.
/// The connection is already marked close-on-drop by the owning entrypoint.
/// Only native `55P03` with an acknowledged rollback is retried; each attempt
/// rechecks identity under its own locks. Other outcomes return at once.
pub(super) async fn remove_invalid(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    expected_oid: i64,
) -> Result<bool, SchemaCompatibilityError> {
    let deadline = tokio::time::Instant::now() + LOCK_RETRY_BUDGET;
    let mut delay = FIRST_RETRY_DELAY;
    loop {
        match attempt(conn, database, index, expected_oid).await {
            Err(error)
                if lock_refused(&error) && tokio::time::Instant::now() + delay < deadline =>
            {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RETRY_DELAY);
            }
            result => return result,
        }
    }
}

/// A rolled-back attempt whose index or table lock was not available.
fn lock_refused(error: &SchemaCompatibilityError) -> bool {
    matches!(
        error,
        SchemaCompatibilityError::Query(cause)
            if cause.as_database_error().and_then(|e| e.code()).as_deref() == Some("55P03")
    )
}

async fn attempt(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    expected_oid: i64,
) -> Result<bool, SchemaCompatibilityError> {
    let mut tx = conn.begin().await?;
    let result = remove_locked(&mut tx, database, index, expected_oid).await;
    match result {
        Ok(true) => {
            tx.commit().await.map_err(|cause| {
                // Retain the same native unconfirmed-commit classification as
                // other owned Runledger transactions; no SQLSTATE grants replay.
                let crate::Error::CommitUnconfirmed(failure) =
                    crate::Error::commit_unconfirmed("remove interrupted online index", cause)
                else {
                    unreachable!("commit_unconfirmed constructs its named variant")
                };
                SchemaCompatibilityError::OnlineIndexRepairCommitUnconfirmed(failure)
            })?;
            Ok(true)
        }
        Ok(false) => {
            tx.rollback().await?;
            Ok(false)
        }
        Err(cause) => match tx.rollback().await {
            Ok(()) => Err(cause),
            Err(rollback) => Err(SchemaCompatibilityError::OnlineIndexRollback(
                OnlineIndexRollbackError::new(cause, rollback),
            )),
        },
    }
}

async fn remove_locked(
    conn: &mut PgConnection,
    database: &RunledgerDatabase,
    index: &OnlineIndex,
    expected_oid: i64,
) -> Result<bool, SchemaCompatibilityError> {
    // The outer concurrent-build session has lifted its timeouts. Bound this
    // short metadata transaction separately, including time between requests.
    // SET LOCAL disappears on both acknowledged commit and rollback.
    sqlx::raw_sql(
        "SET LOCAL lock_timeout = '100ms';
         SET LOCAL statement_timeout = '1s';
         SET LOCAL idle_in_transaction_session_timeout = '1s';
         SELECT CASE WHEN pg_catalog.current_setting('transaction_timeout', true) IS NOT NULL
                     THEN pg_catalog.set_config('transaction_timeout', '1s', true) END",
    )
    .execute(&mut *conn)
    .await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await?;
    let temporary = format!("_runledger_repair_{pid}_{expected_oid}");
    // ALTER INDEX RENAME locks the actual index (not merely its table) until
    // transaction completion. Restore its name inside the same transaction,
    // then inspect under that lock. A collision fails and rolls back; nothing
    // is ever committed under the temporary name. Even a swapped-in foreign
    // relation is restored on refusal. No statement here drops an object yet.
    sqlx::raw_sql(AssertSqlSafe(format!(
        "ALTER INDEX {} RENAME TO \"{temporary}\";
         ALTER INDEX {} RENAME TO \"{}\"",
        database.relation(index.name),
        database.relation(&temporary),
        index.name,
    )))
    .execute(&mut *conn)
    .await?;
    // Never queue ACCESS EXCLUSIVE behind an active reader/writer; the caller
    // retries a refusal. Once held, it also protects table identity through
    // the final inspection and DROP.
    sqlx::raw_sql(AssertSqlSafe(format!(
        "LOCK TABLE ONLY {} IN ACCESS EXCLUSIVE MODE NOWAIT",
        database.relation(index.table),
    )))
    .execute(&mut *conn)
    .await?;
    match index_state(conn, database, index).await? {
        IndexState::Invalid(oid) if oid == expected_oid => {}
        IndexState::Valid(definition) => {
            check_valid(index, &definition, database.schema())?;
            return Ok(false);
        }
        IndexState::Foreign => return Err(problem(index, OnlineIndexProblem::DefinitionMismatch)),
        IndexState::Building => return Err(problem(index, OnlineIndexProblem::BuildInProgress)),
        IndexState::Missing | IndexState::Invalid(_) => {
            return Err(problem(index, OnlineIndexProblem::ChangedDuringRepair));
        }
    }
    // Regular DROP is transactional. Concurrent DROP cannot retain our locks
    // through its internal commits. Only metadata removal holds the table lock;
    // the long CREATE INDEX CONCURRENTLY starts after this transaction commits.
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP INDEX {}",
        database.relation(index.name)
    )))
    .execute(&mut *conn)
    .await?;
    Ok(true)
}
