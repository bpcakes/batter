#![allow(
    clippy::future_not_send,
    reason = "like the existing atomic runner, lending async callbacks may be thread-local; concrete Send inputs produce Send futures, checked by the native consumer test"
)]

use super::{AcceptedIntentOutcome, RequiredIntentError};
use crate::{
    Error, RunledgerDatabase,
    jobs::{JobEnqueue, JobEnqueueIntent, JobEnqueueIntentOutcome, JobEnqueueOutcome},
};
use batter_sqlx::{PgFailurePolicy, PgPolicyScope, PgScopeRolledBack, PgScopedSql};

/// Run a workflow with one consumer error type for SQL and named operations.
///
/// All failure-policy methods are required, including uncertain completion.
/// Named methods use the consumer's `From<Error>` / `From<RequiredIntentError>`
/// implementations. The intent-to-queue transition remains consuming, and the
/// foundation runner retains loss causes independently of consumer error handling.
///
/// ```no_run
/// use runledger_postgres::{PgFailurePolicy, RequiredIntentError, RunledgerDatabase};
/// async fn submit<P: PgFailurePolicy<()>>(
///     db: &RunledgerDatabase, policy: &P,
///     intent: &runledger_postgres::jobs::JobEnqueueIntent<'_>,
/// ) -> Result<(), P::Error>
/// where P::Error: From<RequiredIntentError> + From<sqlx::Error> {
///     runledger_postgres::run_atomic_with(db, policy, async |mut scope| {
///         scope.sql(async |sql| {
///             sqlx::query("SELECT 1").execute(sql.executor()).await?;
///             Ok(())
///         }).await?;
///         scope.record_required_job_enqueue_intent(intent).await?;
///         Ok(())
///     }).await
/// }
/// ```
pub async fn run_atomic_with<T, P: PgFailurePolicy<T>>(
    database: &RunledgerDatabase,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyIntentScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error> {
    batter_sqlx::run_atomic_profiled_with(
        database.pool(),
        database.profile(),
        policy,
        async |inner| work(PgPolicyIntentScope { inner }).await,
    )
    .await
}

/// Run named intent/queue operations and SQL with fail-fast rollback behavior.
/// Ordinary rejection closes the whole transaction; explicit recoverable SQL
/// remains available through the phase's `recoverable_sql` method. The same
/// mandatory uncertainty policy and consuming phase transition apply.
///
/// ```no_run
/// use runledger_postgres::{PgFailurePolicy, PgScopeRolledBack, RequiredIntentError, RunledgerDatabase};
/// async fn submit<P: PgFailurePolicy<()>>(
///     db: &RunledgerDatabase, policy: &P,
///     intent: &runledger_postgres::jobs::JobEnqueueIntent<'_>,
/// ) -> Result<(), P::Error>
/// where P::Error: From<RequiredIntentError> + From<PgScopeRolledBack> {
///     runledger_postgres::run_atomic_fail_fast_with(db, policy, async |mut scope| {
///         scope.record_required_job_enqueue_intent(intent).await?;
///         Ok(())
///     }).await
/// }
/// ```
pub async fn run_atomic_fail_fast_with<T, P: PgFailurePolicy<T>>(
    database: &RunledgerDatabase,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyIntentScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error>
where
    P::Error: From<PgScopeRolledBack>,
{
    batter_sqlx::run_atomic_profiled_fail_fast_with(
        database.pool(),
        database.profile(),
        policy,
        async |inner| work(PgPolicyIntentScope { inner }).await,
    )
    .await
}

/// Initial intent phase with the transaction's consumer error type fixed.
/// Consume with [`Self::queue`] before performing named queue operations.
pub struct PgPolicyIntentScope<'a, T, P: PgFailurePolicy<T>> {
    inner: PgPolicyScope<'a, T, P>,
}

impl<'a, T, P: PgFailurePolicy<T>> PgPolicyIntentScope<'a, T, P> {
    /// Run application SQL with the runner-selected behavior and fixed error type.
    /// Arbitrary SQL must not bypass the named operations' lock ordering.
    pub async fn sql<U>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<U, P::Error>,
    ) -> Result<U, P::Error> {
        self.inner.sql(work).await
    }

    /// Fetch one native SQLx row; absence is `sqlx::Error::RowNotFound`.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_one<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Q::Output, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_one(query).await
    }

    /// Fetch an optional native SQLx row.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_optional<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Option<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_optional(query).await
    }

    /// Fetch all native SQLx rows into an in-memory vector.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_all<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Vec<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_all(query).await
    }

    /// Execute and discard rows/mappers, returning native affected-row information.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn execute<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<sqlx::postgres::PgQueryResult, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.execute(query).await
    }

    /// Recover this SQL operation with its own savepoint, even in a fail-fast
    /// workflow. A previous fail-fast rejection still refuses further work.
    pub async fn recoverable_sql<U>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<U, P::Error>,
    ) -> Result<U, P::Error> {
        self.inner.recoverable_sql(work).await
    }

    /// Record a required handoff. Known durable conflicts are consumer errors.
    pub async fn record_required_job_enqueue_intent(
        &mut self,
        intent: &JobEnqueueIntent<'_>,
    ) -> Result<AcceptedIntentOutcome, P::Error>
    where
        P::Error: From<RequiredIntentError>,
    {
        self.inner
            .sql(async |sql| {
                let outcome = crate::jobs::record_job_enqueue_intent_in_transaction(sql, intent)
                    .await
                    .map_err(RequiredIntentError::Storage)?;
                Ok(AcceptedIntentOutcome::require(outcome)?)
            })
            .await
    }

    /// Explicitly permit observing a conflict without requiring a handoff.
    pub async fn observe_job_enqueue_intent(
        &mut self,
        intent: &JobEnqueueIntent<'_>,
    ) -> Result<JobEnqueueIntentOutcome, P::Error>
    where
        P::Error: From<Error>,
    {
        self.inner
            .sql(async |sql| {
                Ok(crate::jobs::record_job_enqueue_intent_in_transaction(sql, intent).await?)
            })
            .await
    }

    /// Irreversibly end intent recording for this transaction.
    pub fn queue(self) -> PgPolicyQueueScope<'a, T, P> {
        PgPolicyQueueScope { inner: self.inner }
    }
}

/// Queue phase with the fixed consumer error type; no intent-recording methods.
///
/// ```compile_fail,E0599
/// async fn invalid<P: runledger_postgres::PgFailurePolicy<()>>(
///     db: &runledger_postgres::RunledgerDatabase, policy: &P,
///     intent: &runledger_postgres::jobs::JobEnqueueIntent<'_>,
/// ) {
///     runledger_postgres::run_atomic_with(db, policy, async |scope| {
///         let mut queue = scope.queue();
///         queue.record_required_job_enqueue_intent(intent).await?;
///         Ok(())
///     }).await;
/// }
/// ```
pub struct PgPolicyQueueScope<'a, T, P: PgFailurePolicy<T>> {
    inner: PgPolicyScope<'a, T, P>,
}

impl<T, P: PgFailurePolicy<T>> PgPolicyQueueScope<'_, T, P> {
    /// Run application SQL with the runner-selected behavior and fixed error type.
    pub async fn sql<U>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<U, P::Error>,
    ) -> Result<U, P::Error> {
        self.inner.sql(work).await
    }

    /// Fetch one native SQLx row; absence is `sqlx::Error::RowNotFound`.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_one<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Q::Output, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_one(query).await
    }

    /// Fetch an optional native SQLx row.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_optional<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Option<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_optional(query).await
    }

    /// Fetch all native SQLx rows into an in-memory vector.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn fetch_all<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<Vec<Q::Output>, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.fetch_all(query).await
    }

    /// Execute and discard rows/mappers, returning native affected-row information.
    /// Keeps the selected failure policy and phase ordering; output is provisional.
    pub async fn execute<Q: batter_sqlx::PgNativeQuery>(
        &mut self,
        query: Q,
    ) -> Result<sqlx::postgres::PgQueryResult, P::Error>
    where
        P::Error: From<sqlx::Error>,
    {
        self.inner.execute(query).await
    }

    /// Recover this SQL operation with its own savepoint, even in a fail-fast
    /// workflow. A previous fail-fast rejection still refuses further work.
    pub async fn recoverable_sql<U>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<U, P::Error>,
    ) -> Result<U, P::Error> {
        self.inner.recoverable_sql(work).await
    }

    /// Enqueue after leaving the intent phase. Output remains provisional.
    pub async fn enqueue_job(
        &mut self,
        request: &JobEnqueue<'_>,
    ) -> Result<JobEnqueueOutcome, P::Error>
    where
        P::Error: From<Error>,
    {
        self.inner
            .sql(async |sql| {
                Ok(crate::jobs::enqueue_job_with_outcome_in_transaction(sql, request).await?)
            })
            .await
    }

    /// Enqueue with an execution resource after leaving the intent phase.
    pub async fn enqueue_job_with_execution_resource(
        &mut self,
        request: &JobEnqueue<'_>,
        resource: &str,
    ) -> Result<JobEnqueueOutcome, P::Error>
    where
        P::Error: From<Error>,
    {
        self.inner
            .sql(async |sql| {
                Ok(
                    crate::jobs::enqueue_job_with_execution_resource_in_transaction(
                        sql, request, resource,
                    )
                    .await?,
                )
            })
            .await
    }
}
