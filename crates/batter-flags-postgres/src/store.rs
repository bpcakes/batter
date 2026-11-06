use crate::{ErrorClass, StoreError, ValidationError};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::Instant;

/// Required nonzero limits, each at most 60 seconds.
#[derive(Clone, Copy, Debug)]
pub struct StoreTiming {
    pub(crate) acquire: Duration,
    pub(crate) statement: Duration,
    pub(crate) lock: Duration,
    pub(crate) idle: Duration,
}
impl StoreTiming {
    /// Validate all budgets before any connection can be acquired.
    pub fn new(
        acquire: Duration,
        statement: Duration,
        lock: Duration,
        idle: Duration,
    ) -> Result<Self, ValidationError> {
        if [acquire, statement, lock, idle]
            .iter()
            .any(|v| *v < Duration::from_millis(1) || *v > Duration::from_secs(60))
        {
            return Err(ValidationError);
        }
        Ok(Self {
            acquire,
            statement,
            lock,
            idle,
        })
    }
    pub(crate) fn settings(self, deadline: Instant) -> Result<[String; 3], StoreError> {
        let remaining = remaining(deadline)?;
        Ok([self.statement, self.lock, self.idle]
            .map(|d| format!("{}ms", d.min(remaining).as_millis().max(1))))
    }
}
/// Protected pool-backed flags store. Every method owns acquisition, transaction
/// completion and disposition; no connection or incomplete transaction escapes.
/// Atomic/read-only transactions and migrations retire even after success;
/// migration session timeout settings never return to the pool.
#[derive(Clone)]
pub struct PooledStore {
    pub(crate) pool: PgPool,
    pub(crate) timing: StoreTiming,
    #[cfg(test)]
    pub(crate) pause: Option<std::sync::Arc<super::tests::Pause>>,
}
impl std::fmt::Debug for PooledStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PooledStore")
    }
}
impl PooledStore {
    /// Bind an application-owned pool to validated timing. The application owns
    /// pool shutdown; individual operations own all checked-out connections.
    pub fn new(pool: PgPool, timing: StoreTiming) -> Self {
        Self {
            pool,
            timing,
            #[cfg(test)]
            pause: None,
        }
    }
}
pub(crate) fn remaining(deadline: Instant) -> Result<Duration, StoreError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or_else(|| StoreError::classified(ErrorClass::Deadline))
}
pub(crate) async fn bounded<T>(
    deadline: Instant,
    future: impl Future<Output = Result<T, StoreError>>,
) -> Result<T, StoreError> {
    remaining(deadline)?;
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| StoreError::classified(ErrorClass::Deadline))?
}
