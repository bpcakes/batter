use super::fast::{self, RollbackSlot};
use super::{PgFailurePolicy, PgPolicyScope, run_atomic_profiled_with, run_atomic_with};
use crate::{
    PgProfiledPool,
    atomic_context::{retain, retain_with_fallback},
};
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
///
/// ```no_run
/// async fn append<P: batter_sqlx::PgFailurePolicy<i64>>(
///     database: &batter_sqlx::PgProfiledPool,
///     context: &batter_core::operation::OperationContext,
///     policy: &P,
/// ) -> Result<i64, batter_core::operation::OperationError<P::Error>>
/// where P::Error: From<sqlx::Error> {
///     batter_sqlx::run_atomic_profiled_with_in(
///         database, context, "audit.append", policy, async |mut scope| {
///             scope.sql(async |sql| {
///                 sqlx::query_scalar::<_, i64>(
///                     "INSERT INTO audit_events DEFAULT VALUES RETURNING id"
///                 ).fetch_one(sql.executor()).await.map_err(Into::into)
///             }).await
///         },
///     ).await
/// }
/// ```
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

/// Fail-fast counterpart of [`run_atomic_with_in`], preserving acknowledged
/// whole rollback and refusing success after a caught rejection. Uses the same
/// original operation budget and retained completion boundary.
pub async fn run_atomic_fail_fast_with_in<T, P: PgFailurePolicy<T>>(
    pool: &PgPool,
    context: &OperationContext,
    operation: &'static str,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, OperationError<P::Error>>
where
    P::Error: From<super::PgScopeRolledBack>,
{
    let rollback = RollbackSlot::default();
    retain_with_fallback(
        context,
        operation,
        fast::run(pool, None, policy, work, rollback.clone()),
        || rollback.get().map(|confirmed| Err(confirmed.into())),
    )
    .await
}

/// Profiled fail-fast workflow using the pool's immutable profile and the
/// parent's existing budget. Interruption still does not prove rollback.
pub async fn run_atomic_profiled_fail_fast_with_in<T, P: PgFailurePolicy<T>>(
    database: &PgProfiledPool,
    context: &OperationContext,
    operation: &'static str,
    policy: &P,
    work: impl AsyncFnOnce(PgPolicyScope<'_, T, P>) -> Result<T, P::Error>,
) -> Result<T, OperationError<P::Error>>
where
    P::Error: From<super::PgScopeRolledBack>,
{
    let rollback = RollbackSlot::default();
    retain_with_fallback(
        context,
        operation,
        fast::run(
            database.pool(),
            Some(database.profile()),
            policy,
            work,
            rollback.clone(),
        ),
        || rollback.get().map(|confirmed| Err(confirmed.into())),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgScopeFailure, PgScopeLoss, PgTransactionError};
    use batter_core::operation::{Interruption, OperationOwner};
    use std::time::Duration;

    struct BeginFailure(PgTransactionError);
    struct CancelOnBegin<'a>(&'a OperationOwner);

    impl PgFailurePolicy<()> for CancelOnBegin<'_> {
        type Error = BeginFailure;

        fn begin_failed(&self, cause: PgTransactionError) -> BeginFailure {
            self.0.cancel();
            BeginFailure(cause)
        }
        fn scope_lost(&self, _: PgScopeFailure<BeginFailure>) -> BeginFailure {
            panic!("closed pool must prevent scope execution")
        }
        fn commit_unconfirmed(&self, _: (), _: PgTransactionError) -> BeginFailure {
            panic!("closed pool must prevent commit")
        }
        fn rollback_unconfirmed(&self, _: BeginFailure, _: PgTransactionError) -> BeginFailure {
            panic!("closed pool must prevent rollback")
        }
        fn scope_lost_after_body(
            &self,
            _: Result<(), BeginFailure>,
            _: PgScopeLoss,
        ) -> BeginFailure {
            panic!("closed pool must prevent body execution")
        }
    }

    #[tokio::test]
    async fn mapped_begin_error_survives_completion_poll_cancellation() {
        let profile = crate::PgSessionProfile::new(
            "login",
            "login",
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap();
        let database = PgProfiledPool::connect_lazy(
            "postgres://login@127.0.0.1:1/unused".parse().unwrap(),
            profile,
            sqlx::postgres::PgPoolOptions::new().min_connections(0),
        )
        .unwrap();
        database.pool().close().await;

        for profiled in [false, true] {
            let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
            let context = owner.context().clone();
            let policy = CancelOnBegin(&owner);
            let result = if profiled {
                run_atomic_profiled_with_in(
                    &database,
                    &context,
                    "policy.profiled",
                    &policy,
                    async |_| panic!("closed pool must prevent body invocation"),
                )
                .await
            } else {
                run_atomic_with_in(
                    database.pool(),
                    &context,
                    "policy.unprofiled",
                    &policy,
                    async |_| panic!("closed pool must prevent body invocation"),
                )
                .await
            };
            assert_eq!(context.check(), Err(Interruption::Cancelled));
            let Err(OperationError::Failed(BeginFailure(PgTransactionError::Query(cause)))) =
                result
            else {
                panic!("mapped begin error replaced after cancellation; profiled={profiled}")
            };
            assert!(matches!(cause.native(), sqlx::Error::PoolClosed));
        }
    }

    struct NeverCalled;
    struct Unreachable;
    impl From<super::super::PgScopeRolledBack> for Unreachable {
        fn from(_: super::super::PgScopeRolledBack) -> Self {
            panic!("rollback attempted")
        }
    }
    impl PgFailurePolicy<()> for NeverCalled {
        type Error = Unreachable;
        fn begin_failed(&self, _: PgTransactionError) -> Unreachable {
            panic!("begin attempted")
        }
        fn scope_lost(&self, _: PgScopeFailure<Unreachable>) -> Unreachable {
            panic!("scope attempted")
        }
        fn commit_unconfirmed(&self, _: (), _: PgTransactionError) -> Unreachable {
            panic!("commit attempted")
        }
        fn rollback_unconfirmed(&self, _: Unreachable, _: PgTransactionError) -> Unreachable {
            panic!("rollback attempted")
        }
        fn scope_lost_after_body(&self, _: Result<(), Unreachable>, _: PgScopeLoss) -> Unreachable {
            panic!("body attempted")
        }
    }

    #[tokio::test]
    async fn budget_preflight_and_unpolled_work_do_not_acquire() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let owner = OperationOwner::new(Duration::from_secs(1)).unwrap();
        let context = owner.context().clone();
        drop(run_atomic_with_in(
            &pool,
            &context,
            "policy.unpolled",
            &NeverCalled,
            async |_| {
                panic!("unpolled body");
            },
        ));
        drop(run_atomic_fail_fast_with_in(
            &pool,
            &context,
            "fast.unpolled",
            &NeverCalled,
            async |_| {
                panic!("unpolled fast body");
            },
        ));
        owner.cancel();
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
        let result = run_atomic_fail_fast_with_in(
            &pool,
            &context,
            "fast.cancelled",
            &NeverCalled,
            async |_| {
                panic!("cancelled fast body");
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
