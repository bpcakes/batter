use super::{
    fixture,
    policy::{Failure, Policy},
    support::Result,
};
use batter_core::operation::{Interruption, OperationError, OperationOwner};
use batter_sqlx::{
    PgAtomicUncertainty, PgFailurePolicy, PgPolicyScope, PgProfiledPool, PgScopeFailure,
    PgScopeLoss, PgSessionProfile, PgTransactionError, run_atomic_profiled_with_in,
    run_atomic_with_in,
};
use std::time::Duration;

struct CancelOnCommit<'a>(&'a OperationOwner);

impl PgFailurePolicy<i64> for CancelOnCommit<'_> {
    type Error = Failure;

    fn begin_failed(&self, cause: PgTransactionError) -> Failure {
        Policy.begin_failed(cause)
    }
    fn scope_lost(&self, failure: PgScopeFailure<Failure>) -> Failure {
        Policy.scope_lost(failure)
    }
    fn commit_unconfirmed(&self, output: i64, cause: PgTransactionError) -> Failure {
        self.0.cancel();
        Policy.commit_unconfirmed(output, cause)
    }
    fn rollback_unconfirmed(&self, rejection: Failure, cause: PgTransactionError) -> Failure {
        Policy.rollback_unconfirmed(rejection, cause)
    }
    fn scope_lost_after_body(
        &self,
        result: std::result::Result<i64, Failure>,
        cause: PgScopeLoss,
    ) -> Failure {
        Policy.scope_lost_after_body(result, cause)
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_commit_uncertainty_survives_completion_poll_cancellation() -> Result {
    let fixture = fixture().await?;
    let options = fixture.pool.connect_options();
    let login = options.get_username();
    let profile = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let database = PgProfiledPool::connect_lazy(
        (*options).clone(),
        profile,
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
    )?;
    let body = async {
        for profiled in [false, true] {
            let owner = OperationOwner::new(Duration::from_secs(5))?;
            let context = owner.context().clone();
            let policy = CancelOnCommit(&owner);
            let work = async |mut scope: PgPolicyScope<'_, i64, CancelOnCommit<'_>>| {
                scope.sql(async |sql| {
                    sqlx::raw_sql("CREATE TEMP TABLE policy_context_deferred(id integer CONSTRAINT policy_context_unique UNIQUE DEFERRABLE INITIALLY DEFERRED); INSERT INTO policy_context_deferred VALUES (1),(1)")
                        .execute(sql.executor()).await?;
                    Ok(())
                }).await?;
                Ok(73)
            };
            let result = if profiled {
                run_atomic_profiled_with_in(&database, &context, "policy.profiled", &policy, work)
                    .await
            } else {
                run_atomic_with_in(&fixture.pool, &context, "policy.unprofiled", &policy, work)
                    .await
            };
            assert_eq!(context.check(), Err(Interruption::Cancelled));
            let Err(OperationError::Failed(Failure::Completion(outcome))) = result else {
                panic!("mapped commit uncertainty replaced after cancellation; profiled={profiled}")
            };
            let PgAtomicUncertainty::CommitUnconfirmed {
                output: 73,
                cause: PgTransactionError::Query(cause),
            } = *outcome else {
                panic!("provisional output and native commit cause must survive mapping")
            };
            let native = cause.native().as_database_error().expect("native commit error");
            assert_eq!(native.code().as_deref(), Some("23505"));
            assert_eq!(native.constraint(), Some("policy_context_unique"));
        }
        Ok(())
    }.await;
    database.pool().close().await;
    fixture.finish(body).await
}
