//! Consumer error policy over the single owned atomic runner.
mod context;
use crate::{
    PgAtomicError, PgAtomicScope, PgAtomicUncertainty, PgScopeError, PgScopeFailure, PgScopeLoss,
    PgScopedSql, PgSessionProfile, PgTransactionError, run_atomic, run_atomic_profiled,
};
pub use context::{run_atomic_profiled_with_in, run_atomic_with_in};
use sqlx::PgPool;
use std::marker::PhantomData;

/// Translate atomic outcomes into one concrete consumer error type.
///
/// Implement this once for each transaction output type `T` (a generic impl is
/// also possible). Output is part of the policy so an uncertain commit can
/// retain its concrete provisional value. Every failure method is required:
/// uncertainty must never silently become an ordinary rejection or authorize
/// replay. The policy decides how to retain/reconcile these values; Rust cannot
/// prove the meaning of an application error or prevent deliberate discarding.
/// Formatting and logging of the resulting error belong to the consumer.
///
/// Ordinary application errors pass through unchanged after recovery or whole
/// transaction rollback. Native `?` needs the consumer's usual `From<sqlx::Error>`
/// implementation; the policy does not erase native errors.
///
/// ```compile_fail,E0046
/// struct Policy;
/// impl batter_sqlx::PgFailurePolicy<()> for Policy {
///     type Error = std::io::Error;
///     fn begin_failed(&self, _: batter_sqlx::PgTransactionError) -> Self::Error {
///         std::io::Error::other("begin failed")
///     }
///     fn scope_lost(&self, _: batter_sqlx::PgScopeFailure<Self::Error>) -> Self::Error {
///         std::io::Error::other("scope lost")
///     }
///     // Completion uncertainty and scope loss after the body have no defaults.
/// }
/// ```
pub trait PgFailurePolicy<T> {
    /// Error shared by the transaction body and every SQL scope.
    type Error;

    /// Acquisition/setup failed before the body ran.
    fn begin_failed(&self, cause: PgTransactionError) -> Self::Error;

    /// A SQL scope became unusable. Recovery failures include the original
    /// application rejection as well as the native cleanup cause. The runner
    /// independently retains the first loss cause even if this error is caught.
    fn scope_lost(&self, failure: PgScopeFailure<Self::Error>) -> Self::Error;

    /// Commit was not acknowledged; `output` remains provisional.
    fn commit_unconfirmed(&self, output: T, cause: PgTransactionError) -> Self::Error;

    /// Rollback was not acknowledged; preserve the rejection and native cause.
    fn rollback_unconfirmed(
        &self,
        rejection: Self::Error,
        cause: PgTransactionError,
    ) -> Self::Error;

    /// The body ended after loss or abandonment of a scope. Neither its success
    /// nor rejection proves disposition. Includes the independently retained
    /// first cause, even when the body caught the mapped scope error.
    fn scope_lost_after_body(
        &self,
        result: Result<T, Self::Error>,
        cause: PgScopeLoss,
    ) -> Self::Error;
}

/// Run an atomic workflow with its consumer error type selected by `policy`.
///
/// Uses the same state machine and disposition guarantees as [`run_atomic`].
/// The policy cannot restore a poisoned owner or release uncommitted output as
/// success. Scope closures infer `P::Error`, including with native SQLx `?`.
/// See [`PgFailurePolicy`] for required uncertainty handling.
///
/// ```no_run
/// use batter_sqlx::PgFailurePolicy;
/// async fn append<P: PgFailurePolicy<i64>>(
///     pool: &sqlx::PgPool, policy: &P,
/// ) -> Result<i64, P::Error>
/// where P::Error: From<sqlx::Error> {
///     batter_sqlx::run_atomic_with(pool, policy, async |mut scope| {
///         let id = scope.sql(async |sql| {
///             let id = sqlx::query_scalar::<_, i64>(
///                 "INSERT INTO audit_events DEFAULT VALUES RETURNING id"
///             ).fetch_one(sql.executor()).await?;
///             Ok(id) // Error type comes from the policy, even with `?` above.
///         }).await?;
///         Ok(id)
///     }).await
/// }
/// ```
/// A policy-bound scope cannot escape its transaction:
/// ```compile_fail
/// async fn escape<P: batter_sqlx::PgFailurePolicy<()>>(pool: &sqlx::PgPool, policy: &P) {
///     let mut escaped = None;
///     batter_sqlx::run_atomic_with(pool, policy, async |scope| {
///         escaped = Some(scope);
///         Ok(())
///     }).await;
/// }
/// ```
pub async fn run_atomic_with<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error> {
    resolve(
        policy,
        run_atomic(pool, async |inner| {
            work(PgPolicyScope::new(inner, policy)).await
        })
        .await,
    )
}

/// Profiled counterpart of [`run_atomic_with`]. Setup and revalidation use the
/// declared profile, with the same guarantees as [`run_atomic_profiled`].
///
/// ```no_run
/// async fn append<P: batter_sqlx::PgFailurePolicy<i64>>(
///     pool: &sqlx::PgPool,
///     profile: &batter_sqlx::PgSessionProfile,
///     policy: &P,
/// ) -> Result<i64, P::Error>
/// where P::Error: From<sqlx::Error> {
///     batter_sqlx::run_atomic_profiled_with(pool, profile, policy, async |mut scope| {
///         scope.sql(async |sql| {
///             sqlx::query_scalar::<_, i64>(
///                 "INSERT INTO audit_events DEFAULT VALUES RETURNING id"
///             ).fetch_one(sql.executor()).await.map_err(Into::into)
///         }).await
///     }).await
/// }
/// ```
pub async fn run_atomic_profiled_with<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    profile: &PgSessionProfile,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, P::Error> {
    resolve(
        policy,
        run_atomic_profiled(pool, profile, async |inner| {
            work(PgPolicyScope::new(inner, policy)).await
        })
        .await,
    )
}

/// A scope whose error type is fixed for the transaction. There is no public
/// constructor, owner extraction, or completion method. `T` is the final body
/// output, independent of the output of any individual SQL operation.
pub struct PgPolicyScope<'a, T, P> {
    inner: &'a mut PgAtomicScope,
    policy: &'a P,
    output: PhantomData<fn() -> T>,
}

impl<'a, T, P: PgFailurePolicy<T>> PgPolicyScope<'a, T, P> {
    fn new(inner: &'a mut PgAtomicScope, policy: &'a P) -> Self {
        Self {
            inner,
            policy,
            output: PhantomData,
        }
    }

    /// Run SQL in a recoverable private savepoint. An ordinary rejection rolls
    /// back this scope and returns the consumer error unchanged. A lost scope
    /// maps through the required policy and permanently prevents further work.
    /// Results inside the transaction remain provisional until runner success.
    pub async fn sql<U>(
        &mut self,
        work: impl AsyncFnOnce(&mut PgScopedSql<'_>) -> Result<U, P::Error>,
    ) -> Result<U, P::Error> {
        self.inner
            .application(work)
            .await
            .map_err(|error| match error {
                PgScopeError::Application(error) => error,
                PgScopeError::Terminal(failure) => self.policy.scope_lost(failure),
            })
    }
}

fn resolve<T, P: PgFailurePolicy<T>>(
    policy: &P,
    result: Result<T, PgAtomicError<T, P::Error>>,
) -> Result<T, P::Error> {
    result.map_err(|error| match error {
        PgAtomicError::Begin(cause) => policy.begin_failed(cause),
        PgAtomicError::Rejected(rejection) => rejection,
        PgAtomicError::Uncertain(uncertainty) => match uncertainty {
            PgAtomicUncertainty::CommitUnconfirmed { output, cause } => {
                policy.commit_unconfirmed(output, cause)
            }
            PgAtomicUncertainty::RollbackUnconfirmed { rejection, cause } => {
                policy.rollback_unconfirmed(rejection, cause)
            }
            PgAtomicUncertainty::ScopeLost { result, cause } => {
                policy.scope_lost_after_body(result, cause)
            }
        },
    })
}
