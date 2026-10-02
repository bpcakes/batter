use crate::{
    PgLease, PgNativeQuery, SendFuture, atomic_context::retain_with_fallback,
    native_query::QueryFuture,
};
use batter_core::{
    ConfigurationError,
    operation::{OperationContext, OperationError},
};
use sqlx::{PgConnection, PgPool, postgres::PgQueryResult};
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
/// Each call allocates its bounded operation once, on first poll, so the
/// returned future stays small and shallow inside consumer handlers.
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
        self.run(query, FetchOne).await
    }

    /// Fetch one optional native mapped row.
    pub async fn fetch_optional<Q: PgNativeQuery>(&self, query: Q) -> Result<Option<Q::Output>, E> {
        self.run(query, FetchOptional).await
    }

    /// Fetch all native mapped rows into SQLx's ordinary in-memory vector.
    pub async fn fetch_all<Q: PgNativeQuery>(&self, query: Q) -> Result<Vec<Q::Output>, E> {
        self.run(query, FetchAll).await
    }

    /// Execute and return affected-row information, discarding rows and mappers
    /// as native `Executor::execute` does, including for mapped/scalar queries.
    pub async fn execute<Q: PgNativeQuery>(&self, query: Q) -> Result<PgQueryResult, E> {
        self.run(query, Execute).await
    }

    async fn run<Q: PgNativeQuery, C: NativeCall<Q>>(
        &self,
        query: Q,
        call: C,
    ) -> Result<C::Output, E> {
        // Only Send library values enter the erased operation. The consumer's
        // mapper runs after it completes, so its bounds are unchanged.
        let operation: SendFuture<'_, _> = Box::pin(pooled(
            self.pool,
            &self.context,
            self.operation,
            query,
            call,
        ));
        operation.await.map_err(&self.map_error)
    }
}

async fn pooled<Q: PgNativeQuery, C: NativeCall<Q>>(
    pool: &PgPool,
    context: &OperationContext,
    operation: &'static str,
    query: Q,
    call: C,
) -> Result<C::Output, OperationError<sqlx::Error>> {
    let observed = &Mutex::new(None);
    retain_with_fallback(
        context,
        operation,
        move || async move {
            let connection = crate::acquire(pool).await?;
            let mut lease = PgLease {
                connection: Some(connection),
            };
            let outcome = call.call(query, lease.connection_mut()).await;
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
    .await
}

// Selects the native helper without an async closure: the erased operation must
// prove `Send`, which cannot be expressed for the future of an `AsyncFnOnce`.
trait NativeCall<Q: PgNativeQuery>: Send {
    type Output: Send;
    fn call<'e>(self, query: Q, connection: &'e mut PgConnection) -> QueryFuture<'e, Self::Output>
    where
        Q: 'e,
        Self::Output: 'e;
}

struct FetchOne;
struct FetchOptional;
struct FetchAll;
struct Execute;

impl<Q: PgNativeQuery> NativeCall<Q> for FetchOne {
    type Output = Q::Output;
    fn call<'e>(self, query: Q, connection: &'e mut PgConnection) -> QueryFuture<'e, Q::Output>
    where
        Q: 'e,
        Q::Output: 'e,
    {
        query.fetch_one_on(connection)
    }
}

impl<Q: PgNativeQuery> NativeCall<Q> for FetchOptional {
    type Output = Option<Q::Output>;
    fn call<'e>(
        self,
        query: Q,
        connection: &'e mut PgConnection,
    ) -> QueryFuture<'e, Option<Q::Output>>
    where
        Q: 'e,
        Q::Output: 'e,
    {
        query.fetch_optional_on(connection)
    }
}

impl<Q: PgNativeQuery> NativeCall<Q> for FetchAll {
    type Output = Vec<Q::Output>;
    fn call<'e>(self, query: Q, connection: &'e mut PgConnection) -> QueryFuture<'e, Vec<Q::Output>>
    where
        Q: 'e,
        Q::Output: 'e,
    {
        query.fetch_all_on(connection)
    }
}

impl<Q: PgNativeQuery> NativeCall<Q> for Execute {
    type Output = PgQueryResult;
    fn call<'e>(self, query: Q, connection: &'e mut PgConnection) -> QueryFuture<'e, PgQueryResult>
    where
        Q: 'e,
    {
        query.execute_on(connection)
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
