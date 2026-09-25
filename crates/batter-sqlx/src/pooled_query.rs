use crate::{PgLease, PgNativeQuery, atomic_context::retain_with_fallback};
use batter_core::{
    ConfigurationError,
    operation::{OperationContext, OperationError},
};
use sqlx::{PgPool, postgres::PgQueryResult};
use std::{sync::Mutex, time::Duration};

/// Native queries with one total operation budget and one consumer error mapping.
///
/// Construct per operation: acquisition, query and pool-return cleanup share an
/// absolute deadline inherited from the parent. Calls do not reset the budget.
/// Each call owns its lease. Failed/interrupted work retires that connection;
/// success attempts the ordinary idle-state handshake before pool return.
/// An observed native result survives cancellation during that cleanup, whose
/// interrupted connection is retired. Neither interruption nor a native error
/// proves that a write had no effect; there is no automatic replay.
///
/// This is single-query execution. Use atomic runners for multi-query transactions.
/// Cleanup resets transaction state, not arbitrary session settings or locks.
/// A profile-owned pool applies its existing native hooks at acquisition/return.
///
/// ```no_run
/// # async fn example(pool: &sqlx::PgPool, parent: &batter_core::operation::OperationContext)
/// # -> Result<i64, Box<dyn std::error::Error>> {
/// let queries = batter_sqlx::PgQueryHandle::within(
///     pool, parent, std::time::Duration::from_secs(2), "records.read",
///     |error| Box::<dyn std::error::Error>::from(error),
/// )?;
/// let value = queries.fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")).await?;
/// # Ok(value)
/// # }
/// ```
pub struct PgQueryHandle<'pool, MapError> {
    pool: &'pool PgPool,
    context: OperationContext,
    operation: &'static str,
    map_error: MapError,
}

impl<'pool, E, MapError> PgQueryHandle<'pool, MapError>
where
    MapError: Fn(OperationError<sqlx::Error>) -> E,
{
    /// Select a positive total budget, clamped to the parent's deadline, and
    /// convert query/acquisition errors and interruptions to one consumer type.
    /// Construction and unpolled helper futures acquire no connection.
    pub fn within(
        pool: &'pool PgPool,
        parent: &OperationContext,
        maximum: Duration,
        operation: &'static str,
        map_error: MapError,
    ) -> Result<Self, ConfigurationError> {
        Ok(Self {
            pool,
            context: parent.child(maximum)?.into_context(),
            operation,
            map_error,
        })
    }

    /// Fetch one native mapped row; absence returns SQLx's `RowNotFound`.
    pub async fn fetch_one<Q: PgNativeQuery>(&self, query: Q) -> Result<Q::Output, E> {
        self.run(async move |session| query.fetch_one_on(session).await)
            .await
    }

    /// Fetch one optional native mapped row.
    pub async fn fetch_optional<Q: PgNativeQuery>(&self, query: Q) -> Result<Option<Q::Output>, E> {
        self.run(async move |session| query.fetch_optional_on(session).await)
            .await
    }

    /// Fetch all native mapped rows into SQLx's ordinary in-memory vector.
    pub async fn fetch_all<Q: PgNativeQuery>(&self, query: Q) -> Result<Vec<Q::Output>, E> {
        self.run(async move |session| query.fetch_all_on(session).await)
            .await
    }

    /// Execute and return affected-row information, discarding rows and mappers
    /// as native `Executor::execute` does, including for mapped/scalar queries.
    pub async fn execute<Q: PgNativeQuery>(&self, query: Q) -> Result<PgQueryResult, E> {
        self.run(async move |session| query.execute_on(session).await)
            .await
    }

    async fn run<T>(
        &self,
        work: impl AsyncFnOnce(&mut sqlx::PgConnection) -> Result<T, sqlx::Error>,
    ) -> Result<T, E> {
        let observed = Mutex::new(None);
        let result = retain_with_fallback(
            &self.context,
            self.operation,
            async {
                let connection = self.pool.acquire().await?;
                let mut lease = PgLease {
                    connection: Some(connection),
                };
                let outcome = work(lease.connection_mut()).await;
                let success = outcome.is_ok();
                *observed.lock().expect("private query outcome slot") = Some(outcome);
                lease.finish_query(success).await;
                observed
                    .lock()
                    .expect("private query outcome slot")
                    .take()
                    .expect("completed query retained its outcome")
            },
            || observed.lock().expect("private query outcome slot").take(),
        )
        .await;
        result.map_err(&self.map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter_core::operation::{Interruption, OperationOwner};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(start_paused = true)]
    async fn query_budget_is_clamped_and_never_restarted_at_call_time() {
        for (parent_seconds, maximum_seconds) in [(1, 10), (10, 1)] {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused@127.0.0.1:1/unused")
                .unwrap();
            let parent_owner = OperationOwner::new(Duration::from_secs(parent_seconds)).unwrap();
            let parent = parent_owner.context().clone();
            let mapped = AtomicUsize::new(0);
            let queries = PgQueryHandle::within(
                &pool,
                &parent,
                Duration::from_secs(maximum_seconds),
                "queries.deadline",
                |error| {
                    mapped.fetch_add(1, Ordering::SeqCst);
                    error
                },
            )
            .unwrap();
            let deadline = queries.context.deadline();
            drop(queries.fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")));
            assert_eq!(mapped.load(Ordering::SeqCst), 0);
            assert_eq!(pool.size(), 0);
            tokio::time::advance(Duration::from_secs(2)).await;
            for _ in 0..2 {
                assert!(matches!(
                    queries
                        .fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint"))
                        .await,
                    Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
                ));
            }
            assert_eq!(queries.context.deadline(), deadline);
            assert_eq!(mapped.load(Ordering::SeqCst), 2);
            assert_eq!(pool.size(), 0);
            if parent_seconds == 10 {
                assert!(parent.check().is_ok());
            }
        }
    }

    #[tokio::test]
    async fn cancelled_parent_and_invalid_budget_never_acquire() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let parent_owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let parent = parent_owner.context().clone();
        assert!(
            PgQueryHandle::within(
                &pool,
                &parent,
                Duration::ZERO,
                "queries.invalid",
                std::convert::identity
            )
            .is_err()
        );
        parent_owner.cancel();
        let queries = PgQueryHandle::within(
            &pool,
            &parent,
            Duration::from_secs(1),
            "queries.cancelled",
            std::convert::identity,
        )
        .unwrap();
        assert!(matches!(
            queries.execute(sqlx::query("SELECT 1")).await,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        assert_eq!(pool.size(), 0);
    }
}
