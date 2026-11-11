//! One READ COMMITTED transaction for the unaudited operations: presence,
//! stop and prune. The audited write keeps its own completion, because its
//! uncertain commit is a [`WriteOutcome`](crate::WriteOutcome), not an error.
use crate::{
    ErrorClass, PooledStore, StoreError,
    errors::ErrorKind,
    store::{bounded, remaining},
    write::{RollbackFailure, known_commit_rejection},
};
use batter_sqlx::{PgDeadlineError, PgScopeFailure, PgScopedSql, low_level::PgAtomicTransaction};
use tokio::time::Instant;

impl PooledStore {
    /// Runs `work` in one transaction with the store's timeouts set locally,
    /// commits it, and rolls it back on any error. A commit whose outcome is
    /// unknown is [`ErrorClass::CommitUnknown`]; nothing here is replayed.
    pub(crate) async fn atomically<T>(
        &self,
        deadline: Instant,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let acquire_deadline = deadline.min(Instant::now() + self.timing.acquire);
        let tx = bounded(acquire_deadline, async {
            PgAtomicTransaction::begin(&self.pool)
                .await
                .map_err(StoreError::from)
        })
        .await?;
        let (tx, result) = tx
            .operation_until(deadline, async |sql| {
                self.limit(sql, deadline).await?;
                work(sql).await
            })
            .await
            .map_err(scope_error)?;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                return match bounded(deadline, async {
                    tx.rollback().await.map_err(StoreError::from)
                })
                .await
                {
                    Ok(_) => Err(error),
                    Err(cleanup) => Err(RollbackFailure::both(error, cleanup)),
                };
            }
        };
        remaining(deadline)?;
        match tokio::time::timeout_at(deadline, tx.commit()).await {
            Ok(Ok(_)) => Ok(value),
            Ok(Err(cause)) if known_commit_rejection(cause.cause()) => {
                Err(cause.into_cause().into())
            }
            Ok(Err(cause)) => Err(StoreError(ErrorKind::Unconfirmed(cause.into_cause()))),
            Err(_) => Err(StoreError::classified(ErrorClass::CommitUnknown)),
        }
    }

    /// Sets the statement, lock and idle timeouts for this transaction only,
    /// each capped by what is left of the deadline.
    pub(crate) async fn limit(
        &self,
        sql: &mut PgScopedSql<'_>,
        deadline: Instant,
    ) -> Result<(), StoreError> {
        let [statement, lock, idle] = self.timing.settings(deadline)?;
        sqlx::query("SELECT set_config('statement_timeout',$1,true), set_config('lock_timeout',$2,true), set_config('idle_in_transaction_session_timeout',$3,true)")
            .bind(statement).bind(lock).bind(idle).execute(sql.executor()).await?;
        Ok(())
    }
}

/// How a transaction body's deadline or scope failure reads as a store error.
pub(crate) fn scope_error(
    error: PgDeadlineError<PgScopeFailure<StoreError>, StoreError>,
) -> StoreError {
    match error {
        PgDeadlineError::Elapsed => StoreError::classified(ErrorClass::Deadline),
        PgDeadlineError::Failed(error) => StoreError(ErrorKind::Scope(Box::new(error))),
        PgDeadlineError::RecoveryElapsed(error) => RollbackFailure::deadline(error),
    }
}
