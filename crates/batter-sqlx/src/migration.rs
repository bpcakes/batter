use crate::{
    PgDeadlineError, PgLease, PgTransactionError, atomic::scope::normalize, deadline::until,
};
use std::{convert::Infallible, error::Error, fmt, time::Duration};
use tokio::time::Instant;

/// Nonzero PostgreSQL server budgets between 1 millisecond and 60 seconds.
/// The migration owner clamps them to its remaining absolute deadline.
#[derive(Clone, Copy, Debug)]
pub struct PgMigrationTimeouts {
    statement: Duration,
    lock: Duration,
    idle: Duration,
}
/// An invalid migration timeout. Formatting discloses no supplied values.
#[derive(Debug)]
pub struct PgMigrationTimeoutError;
impl fmt::Display for PgMigrationTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid PostgreSQL migration timeouts")
    }
}
impl Error for PgMigrationTimeoutError {}
impl PgMigrationTimeouts {
    /// Validate statement, lock and idle-in-transaction server limits before SQL.
    pub fn new(
        statement: Duration,
        lock: Duration,
        idle: Duration,
    ) -> Result<Self, PgMigrationTimeoutError> {
        if [statement, lock, idle]
            .iter()
            .any(|v| *v < Duration::from_millis(1) || *v > Duration::from_secs(60))
        {
            return Err(PgMigrationTimeoutError);
        }
        Ok(Self {
            statement,
            lock,
            idle,
        })
    }
}
impl PgLease {
    /// Run a native SQLx migrator with server limits and an absolute deadline.
    /// Reset inherited session state first, then install session-level limits:
    /// SQLx's advisory lock is outside a transaction and each migration starts
    /// its own transaction, so SET LOCAL alone cannot protect these phases.
    /// Limits inherit into those transactions. The same lease is always retired,
    /// including success, so no timeout settings or advisory locks enter the pool.
    /// Native SQLx still owns migration history, locking and checksums. A failure
    /// may follow a committed migration; no rollback, retry, or remote termination
    /// is inferred. SQL executed by a trusted migration can change server settings.
    ///
    /// ```no_run
    /// # async fn example(pool: &sqlx::PgPool, migrator: &sqlx::migrate::Migrator,
    /// # context: &batter_core::operation::OperationContext) -> Result<(), Box<dyn std::error::Error>> {
    /// use std::time::Duration;
    /// let lease = batter_sqlx::PgLease::acquire(pool, context).await?;
    /// let limits = batter_sqlx::PgMigrationTimeouts::new(Duration::from_secs(3),
    ///     Duration::from_secs(1), Duration::from_secs(3))?;
    /// lease.migrate_until(migrator, limits,
    ///     tokio::time::Instant::now() + Duration::from_secs(10)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn migrate_until(
        mut self,
        migrator: &sqlx::migrate::Migrator,
        limits: PgMigrationTimeouts,
        deadline: Instant,
    ) -> Result<(), PgDeadlineError<PgTransactionError, Infallible>> {
        until(deadline, async {
            normalize(self.connection_mut()).await?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let [statement, lock, idle] = [limits.statement, limits.lock, limits.idle]
                .map(|v| format!("{}ms", v.min(remaining).as_millis().max(1)));
            sqlx::query("SELECT set_config('statement_timeout',$1,false), set_config('lock_timeout',$2,false), set_config('idle_in_transaction_session_timeout',$3,false)")
                .bind(statement).bind(lock).bind(idle).execute(self.connection_mut()).await?;
            migrator.run_direct(None, self.connection_mut(), false).await
                .map_err(sqlx::Error::from).map_err(PgTransactionError::from)
        }).await.map_err(|_| PgDeadlineError::Elapsed)?.map_err(PgDeadlineError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_timeout_construction_rejects_disabling_or_unbounded_limits() {
        let valid = Duration::from_secs(1);
        for invalid in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_secs(61),
        ] {
            assert!(
                PgMigrationTimeouts::new(invalid, valid, valid).is_err(),
                "statement limit"
            );
            assert!(
                PgMigrationTimeouts::new(valid, invalid, valid).is_err(),
                "lock limit"
            );
            assert!(
                PgMigrationTimeouts::new(valid, valid, invalid).is_err(),
                "idle limit"
            );
        }
        assert!(
            PgMigrationTimeouts::new(Duration::from_millis(1), valid, Duration::from_secs(60))
                .is_ok(),
            "boundary values remain usable"
        );
    }
}
