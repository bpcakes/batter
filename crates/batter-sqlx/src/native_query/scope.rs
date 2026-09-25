use crate::{PgAtomicScope, PgFailurePolicy, PgNativeQuery, PgPolicyScope, PgScopeError};
use sqlx::postgres::PgQueryResult;

impl<T, P: PgFailurePolicy<T>> PgPolicyScope<'_, T, P> {
    /// Fetch one native SQLx row; absence is `sqlx::Error::RowNotFound`.
    /// Uses the runner-selected scope behavior and fixed consumer error type.
    /// Returned values remain provisional until the atomic runner succeeds.
    ///
    /// ```no_run
    /// async fn read<P: batter_sqlx::PgFailurePolicy<i64>>(
    ///     scope: &mut batter_sqlx::PgPolicyScope<'_, i64, P>,
    /// ) -> Result<i64, P::Error> where P::Error: From<sqlx::Error> {
    ///     scope.execute(sqlx::query("SELECT 1")).await?;
    ///     let _optional = scope.fetch_optional(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")).await?;
    ///     let _all = scope.fetch_all(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")).await?;
    ///     scope.fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")).await
    /// }
    /// ```
    pub async fn fetch_one<Q: PgNativeQuery>(&mut self, query: Q) -> Result<Q::Output, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.sql(async move |sql| {
            query
                .fetch_one_on(sql.native_connection())
                .await
                .map_err(Into::into)
        })
        .await
    }

    /// Fetch an optional native SQLx row.
    /// Uses the runner-selected scope behavior and fixed consumer error type.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn fetch_optional<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Option<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.sql(async move |sql| {
            query
                .fetch_optional_on(sql.native_connection())
                .await
                .map_err(Into::into)
        })
        .await
    }

    /// Fetch all native SQLx rows into an in-memory vector.
    /// Uses the runner-selected scope behavior and fixed consumer error type.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn fetch_all<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Vec<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.sql(async move |sql| {
            query
                .fetch_all_on(sql.native_connection())
                .await
                .map_err(Into::into)
        })
        .await
    }

    /// Execute and discard rows/mappers, returning native affected-row information.
    /// Uses the runner-selected scope behavior and fixed consumer error type.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn execute<Q: PgNativeQuery>(&mut self, query: Q) -> Result<PgQueryResult, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.sql(async move |sql| {
            query
                .execute_on(sql.native_connection())
                .await
                .map_err(Into::into)
        })
        .await
    }
}

impl PgAtomicScope {
    /// Fetch one native SQLx row; absence is `sqlx::Error::RowNotFound`.
    /// Uses the same savepoint recovery as `Self::application`.
    /// Returned values remain provisional until the atomic runner succeeds.
    ///
    /// ```no_run
    /// # async fn example(pool: &sqlx::PgPool) -> Result<i64, Box<dyn std::error::Error>> {
    /// let value = batter_sqlx::run_atomic(pool, async |scope| {
    ///     scope.fetch_one(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")).await
    /// }).await?;
    /// # Ok(value)
    /// # }
    /// ```
    pub async fn fetch_one<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Q::Output, PgScopeError<sqlx::Error>> {
        self.application(async move |sql| query.fetch_one_on(sql.native_connection()).await)
            .await
    }

    /// Fetch an optional native SQLx row.
    /// Uses the same savepoint recovery as `Self::application`.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn fetch_optional<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Option<Q::Output>, PgScopeError<sqlx::Error>> {
        self.application(async move |sql| query.fetch_optional_on(sql.native_connection()).await)
            .await
    }

    /// Fetch all native SQLx rows into an in-memory vector.
    /// Uses the same savepoint recovery as `Self::application`.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn fetch_all<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Vec<Q::Output>, PgScopeError<sqlx::Error>> {
        self.application(async move |sql| query.fetch_all_on(sql.native_connection()).await)
            .await
    }

    /// Execute and discard rows/mappers, returning native affected-row information.
    /// Uses the same savepoint recovery as `Self::application`.
    /// Returned values remain provisional until the atomic runner succeeds.
    pub async fn execute<Q: PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<PgQueryResult, PgScopeError<sqlx::Error>> {
        self.application(async move |sql| query.execute_on(sql.native_connection()).await)
            .await
    }
}
