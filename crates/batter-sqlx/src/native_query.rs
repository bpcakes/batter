//! Sealed dispatch over native SQLx PostgreSQL query types.
mod scope;

use sqlx::postgres::{PgQueryResult, PgRow};
use sqlx::query::{Map, Query, QueryScalar};
use sqlx::{Executor, IntoArguments, PgConnection, Postgres};
use std::{future::Future, pin::Pin};

// SQLx also returns boxed executor futures. Keep Send explicit through lending
// scope callbacks; opaque futures hit the Rust limitation reported as #100013.
type QueryFuture<'e, T> = Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'e>>;
mod sealed {
    pub trait Sealed {}
    impl<A> Sealed for sqlx::query::Query<'_, sqlx::Postgres, A> {}
    impl<F, A> Sealed for sqlx::query::Map<'_, sqlx::Postgres, F, A> {}
    impl<O, A> Sealed for sqlx::query::QueryScalar<'_, sqlx::Postgres, O, A> {}
}
/// A native SQLx 0.9 PostgreSQL query accepted by scope and pooled helpers.
///
/// Implemented only for `Query`, `Map` and `QueryScalar`: the types returned by
/// SQLx's `query!`, `query_as!` and `query_scalar!` macros. Native row mapping,
/// decoding and bind behavior are preserved. Runtime query constructors return
/// the same types and also work; only SQLx macros provide compile-time SQL checks.
/// Consumers cannot implement this trait or use it to extract a library owner.
/// Prefer the methods on [`crate::PgPolicyScope`] or [`crate::PgQueryHandle`].
///
/// ```compile_fail,E0603
/// struct Forged;
/// impl batter_sqlx::native_query::sealed::Sealed for Forged {}
/// ```
pub trait PgNativeQuery: sealed::Sealed + Send + Sized {
    /// Native row, mapped output or scalar selected by the SQLx query.
    type Output: Send + Unpin;
    #[doc(hidden)]
    fn fetch_one_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Self::Output>
    where
        Self: 'e,
        Self::Output: 'e;
    #[doc(hidden)]
    fn fetch_optional_on<'e>(
        self,
        executor: &'e mut PgConnection,
    ) -> QueryFuture<'e, Option<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e;
    #[doc(hidden)]
    fn fetch_all_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Vec<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e;
    #[doc(hidden)]
    fn execute_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, PgQueryResult>
    where
        Self: 'e;
}
impl<'q, A> PgNativeQuery for Query<'q, Postgres, A>
where
    A: 'q + Send + IntoArguments<Postgres>,
{
    type Output = PgRow;
    fn fetch_one_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Self::Output>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_one(executor))
    }
    fn fetch_optional_on<'e>(
        self,
        executor: &'e mut PgConnection,
    ) -> QueryFuture<'e, Option<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_optional(executor))
    }
    fn fetch_all_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Vec<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_all(executor))
    }
    fn execute_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, PgQueryResult>
    where
        Self: 'e,
    {
        Box::pin(executor.execute(self))
    }
}
impl<'q, F, O, A> PgNativeQuery for Map<'q, Postgres, F, A>
where
    F: FnMut(PgRow) -> Result<O, sqlx::Error> + Send,
    O: Send + Unpin,
    A: 'q + Send + IntoArguments<Postgres>,
{
    type Output = O;
    fn fetch_one_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Self::Output>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_one(executor))
    }
    fn fetch_optional_on<'e>(
        self,
        executor: &'e mut PgConnection,
    ) -> QueryFuture<'e, Option<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_optional(executor))
    }
    fn fetch_all_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Vec<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_all(executor))
    }
    fn execute_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, PgQueryResult>
    where
        Self: 'e,
    {
        Box::pin(executor.execute(self))
    }
}
impl<'q, O, A> PgNativeQuery for QueryScalar<'q, Postgres, O, A>
where
    O: Send + Unpin,
    A: 'q + Send + IntoArguments<Postgres>,
    (O,): for<'r> sqlx::FromRow<'r, PgRow>,
{
    type Output = O;
    fn fetch_one_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Self::Output>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_one(executor))
    }
    fn fetch_optional_on<'e>(
        self,
        executor: &'e mut PgConnection,
    ) -> QueryFuture<'e, Option<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_optional(executor))
    }
    fn fetch_all_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, Vec<Self::Output>>
    where
        Self: 'e,
        Self::Output: 'e,
    {
        Box::pin(self.fetch_all(executor))
    }
    fn execute_on<'e>(self, executor: &'e mut PgConnection) -> QueryFuture<'e, PgQueryResult>
    where
        Self: 'e,
    {
        Box::pin(executor.execute(self))
    }
}
