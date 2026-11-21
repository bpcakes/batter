use super::{End, Inspection, PgReadOnlySnapshot, PgReadOnlySql, PgSnapshotError};
use crate::{PgDeadlineError, deadline::until};
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
        Self::inspect_bounded(pool, None, deadline, inspect).await
    }

    /// Inspect under the supplied session policy and one absolute deadline.
    /// The policy is established before BEGIN and verified before completion.
    ///
    /// ```no_run
    /// # async fn example(pool: &batter_sqlx::PgProfiledPool, deadline: tokio::time::Instant) -> Result<(), Box<dyn std::error::Error>> {
    /// batter_sqlx::PgReadOnlySnapshot::inspect_profiled_until(pool.pool(), pool.profile(), deadline, async |sql| {
    ///     sqlx::query("SELECT 1").execute(sql.executor()).await
    /// }).await?;
    /// # Ok(()) }
    /// ```
    pub async fn inspect_profiled_until<T, E>(
        pool: &PgPool,
        profile: &crate::PgSessionProfile,
        deadline: Instant,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
    ) -> Result<T, PgDeadlineError<PgSnapshotError<E>, E>> {
        Self::inspect_bounded(pool, Some(profile), deadline, inspect).await
    }

    async fn inspect_bounded<T, E>(
        pool: &PgPool,
        profile: Option<&crate::PgSessionProfile>,
        deadline: Instant,
        inspect: impl AsyncFnOnce(&mut PgReadOnlySql<'_>) -> Result<T, E>,
    ) -> Result<T, PgDeadlineError<PgSnapshotError<E>, E>> {
        let mut session = until(deadline, Inspection::begin(pool, profile))
            .await
            .map_err(|_| PgDeadlineError::Elapsed)?
            .map_err(|e| PgDeadlineError::Failed(PgSnapshotError::Transaction(e)))?;
        let result = until(deadline, inspect(&mut session.sql()))
            .await
            .map_err(|_| PgDeadlineError::Elapsed)?;
        match result {
            Ok(value) => {
                until(deadline, session.finish(End::Accepted))
                    .await
                    .map_err(|_| PgDeadlineError::Elapsed)?
                    .map_err(|e| PgDeadlineError::Failed(PgSnapshotError::Transaction(e)))?;
                Ok(value)
            }
            Err(inspection) => {
                let cleanup = until(deadline, session.finish(End::Rejected)).await;
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
