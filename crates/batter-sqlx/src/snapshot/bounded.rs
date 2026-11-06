use super::{PgReadOnlySnapshot, PgReadOnlySql, PgSnapshotError, validate};
use crate::{
    PgDeadlineError, PgLease,
    atomic::scope::{ScopeSavepoint, command, normalize},
    deadline::until,
};
use sqlx::PgPool;
use tokio::time::Instant;

impl PgReadOnlySnapshot {
    /// Inspect under one absolute deadline, retaining an observed inspection
    /// error if rollback cannot acknowledge in time. The complete acquisition,
    /// setup, inspection and cleanup share the budget; every path retires.
    /// No successful read escapes before rollback acknowledgment. Dropping the
    /// outer future loses its result and does not confirm remote termination.
    ///
    /// ```no_run
    /// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    /// let count = batter_sqlx::PgReadOnlySnapshot::inspect_until(pool, deadline, async |sql| {
    ///     sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_catalog.pg_class")
    ///         .fetch_one(sql.executor()).await
    /// }).await?;
    /// # let _ = count; Ok(()) }
    /// ```
    pub async fn inspect_until<T, E>(
        pool: &PgPool,
        deadline: Instant,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
    ) -> Result<T, PgDeadlineError<PgSnapshotError<E>, E>> {
        let (mut lease, guard) = until(deadline, async {
            let mut lease = PgLease {
                connection: Some(crate::acquire(pool).await?),
            };
            normalize(lease.connection_mut()).await?;
            command(
                lease.connection_mut(),
                "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            )
            .await?;
            command(
                lease.connection_mut(),
                "SET LOCAL search_path = pg_catalog, pg_temp",
            )
            .await?;
            let guard = ScopeSavepoint::begin(lease.connection_mut()).await?;
            Ok::<_, crate::PgTransactionError>((lease, guard))
        })
        .await
        .map_err(|_| PgDeadlineError::Elapsed)?
        .map_err(|e| PgDeadlineError::Failed(PgSnapshotError::Transaction(e)))?;
        let result = until(
            deadline,
            inspect(&mut PgReadOnlySql {
                connection: lease.connection_mut(),
            }),
        )
        .await
        .map_err(|_| PgDeadlineError::Elapsed)?;
        match result {
            Ok(value) => {
                until(deadline, async {
                    guard.release(lease.connection_mut()).await?;
                    validate(lease.connection_mut()).await?;
                    command(lease.connection_mut(), "ROLLBACK").await
                })
                .await
                .map_err(|_| PgDeadlineError::Elapsed)?
                .map_err(|e| PgDeadlineError::Failed(PgSnapshotError::Transaction(e)))?;
                Ok(value)
            }
            Err(inspection) => {
                let cleanup = until(deadline, async {
                    guard.rollback(lease.connection_mut()).await?;
                    validate(lease.connection_mut()).await?;
                    command(lease.connection_mut(), "ROLLBACK").await
                })
                .await;
                match cleanup {
                    Ok(Ok(())) => Err(PgDeadlineError::Failed(PgSnapshotError::Inspection(
                        inspection,
                    ))),
                    Ok(Err(cleanup)) => Err(PgDeadlineError::Failed(PgSnapshotError::Cleanup {
                        inspection,
                        cleanup,
                    })),
                    Err(_) => Err(PgDeadlineError::RecoveryElapsed(inspection)),
                }
            }
        }
    }
}
