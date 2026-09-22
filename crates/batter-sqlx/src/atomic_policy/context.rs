use super::{PgFailurePolicy, PgPolicyScope, run_atomic_profiled_with, run_atomic_with};
use crate::{PgProfiledPool, atomic_context::retain};
use batter_core::operation::{OperationContext, OperationError};
use sqlx::PgPool;

/// Policy-bound atomic work within the parent's existing operation budget.
///
/// Retains acknowledged completion before resolving deadline/cancellation, just
/// like [`crate::run_atomic_in`]. Interruption remains distinct from the policy's
/// consumer error and does not prove rollback. Scope SQL still returns `P::Error`
/// directly. No timeout, cancellation, or uncertain outcome authorizes replay.
///
/// ```no_run
/// async fn example<P: batter_sqlx::PgFailurePolicy<()>>(
///     pool: &sqlx::PgPool, context: &batter_core::operation::OperationContext,
///     policy: &P,
/// ) where P::Error: From<sqlx::Error> {
///     let result = batter_sqlx::run_atomic_with_in(
///         pool, context, "audit.append", policy, async |mut scope| {
///             scope.sql(async |sql| {
///                 sqlx::query("SELECT 1").execute(sql.executor()).await?;
///                 Ok(())
///             }).await
///         },
///     ).await;
///     // OperationError::Failed contains the consumer's concrete error.
///     // OperationError::Interrupted has no acknowledged disposition.
///     let _ = result;
/// }
/// ```
pub async fn run_atomic_with_in<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    context: &OperationContext,
    operation: &'static str,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, OperationError<P::Error>> {
    retain(context, operation, run_atomic_with(pool, policy, work)).await
}

/// Policy-bound profiled work within one operation budget. The immutable pool
/// profile establishes authority; completion retention matches
/// [`run_atomic_with_in`] and [`crate::run_atomic_profiled_in`].
pub async fn run_atomic_profiled_with_in<T, P: PgFailurePolicy<T>>(
    database: &PgProfiledPool,
    context: &OperationContext,
    operation: &'static str,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, OperationError<P::Error>> {
    retain(
        context,
        operation,
        run_atomic_profiled_with(database.pool(), database.profile(), policy, work),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgScopeFailure, PgScopeLoss, PgTransactionError};
    use batter_core::operation::Interruption;

    struct NeverCalled;
    impl PgFailurePolicy<()> for NeverCalled {
        type Error = ();
        fn begin_failed(&self, _: PgTransactionError) {
            panic!("begin attempted")
        }
        fn scope_lost(&self, _: PgScopeFailure<()>) {
            panic!("scope attempted")
        }
        fn commit_unconfirmed(&self, _: (), _: PgTransactionError) {
            panic!("commit attempted")
        }
        fn rollback_unconfirmed(&self, _: (), _: PgTransactionError) {
            panic!("rollback attempted")
        }
        fn scope_lost_after_body(&self, _: Result<(), ()>, _: PgScopeLoss) {
            panic!("body attempted")
        }
    }

    #[tokio::test]
    async fn budget_preflight_and_unpolled_work_do_not_acquire() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let context = OperationContext::new(std::time::Duration::from_secs(1)).unwrap();
        drop(run_atomic_with_in(
            &pool,
            &context,
            "policy.unpolled",
            &NeverCalled,
            async |_| {
                panic!("unpolled body");
            },
        ));
        context.cancel();
        let result = run_atomic_with_in(
            &pool,
            &context,
            "policy.cancelled",
            &NeverCalled,
            async |_| {
                panic!("cancelled body");
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::Cancelled))
        ));
        assert_eq!(pool.size(), 0);
    }
}
