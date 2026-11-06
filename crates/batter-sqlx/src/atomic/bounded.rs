use super::{PgAtomicTransaction, PgScopeFailure, PgScopedSql, scope::ScopeSavepoint};
use crate::{PgDeadlineError, deadline::until};
use tokio::time::Instant;

impl PgAtomicTransaction {
    /// Run a savepoint operation under one absolute deadline, retaining an
    /// observed application rejection across bounded recovery. Unlike wrapping
    /// [`Self::operation`] in an outer timeout, recovery cannot erase that error.
    /// All failures retire; no owner returns after a deadline. External future
    /// cancellation still discards the result and proves no remote rollback.
    ///
    /// ```no_run
    /// # async fn example(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    /// let tx = batter_sqlx::low_level::PgAtomicTransaction::begin(pool).await?;
    /// let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    /// let (tx, result) = tx.operation_until(deadline, async |sql| {
    ///     sqlx::query("INSERT INTO audit_events DEFAULT VALUES")
    ///         .execute(sql.executor()).await
    /// }).await?;
    /// match result {
    ///     Ok(_) => { let _ = tx.commit().await?; }
    ///     Err(_) => { let _ = tx.rollback().await?; }
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn operation_until<T, E>(
        mut self,
        deadline: Instant,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<T, E>,
    ) -> Result<(Self, Result<T, E>), PgDeadlineError<PgScopeFailure<E>, E>> {
        let guard = until(deadline, async {
            if self.profile.is_some() {
                self.validate().await?;
            }
            ScopeSavepoint::begin(self.lease.connection_mut()).await
        })
        .await
        .map_err(|_| PgDeadlineError::Elapsed)?
        .map_err(|e| PgDeadlineError::Failed(e.into()))?;
        let result = until(
            deadline,
            work(&mut PgScopedSql {
                connection: self.lease.connection_mut(),
            }),
        )
        .await
        .map_err(|_| PgDeadlineError::Elapsed)?;
        match result {
            Ok(value) => {
                until(deadline, async {
                    self.validate().await?;
                    guard.release(self.lease.connection_mut()).await
                })
                .await
                .map_err(|_| PgDeadlineError::Elapsed)?
                .map_err(|e| PgDeadlineError::Failed(e.into()))?;
                Ok((self, Ok(value)))
            }
            Err(application) => {
                // Ownership of the rejection is outside the cancellable recovery.
                let recovery = until(deadline, async {
                    guard.rollback(self.lease.connection_mut()).await?;
                    self.validate().await
                })
                .await;
                match recovery {
                    Ok(Ok(())) => Ok((self, Err(application))),
                    Ok(Err(recovery)) => Err(PgDeadlineError::Failed(PgScopeFailure::Recovery {
                        application,
                        recovery: recovery.into(),
                    })),
                    Err(_) => Err(PgDeadlineError::RecoveryElapsed(application)),
                }
            }
        }
    }
}
