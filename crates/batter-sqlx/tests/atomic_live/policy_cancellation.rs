use super::*;
use batter_sqlx::{PgAtomicUncertainty, PgScopeFailure, PgScopeLoss, run_atomic_with};
use policy::{Failure, Policy};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_abandoned_operation_cannot_be_caught_into_success() -> Result {
    let mut fixture = fixture().await?;
    let pool = fixture.pool.clone();
    let key = fixture.key;
    let body = async {
        let error = run_atomic_with(&pool, &Policy, async |mut scope| {
            let previous = scope.sql(async |sql| Ok(pid(sql).await?)).await?;
            fixture.retired.push(previous);
            let mut operation = Box::pin(scope.sql(async |sql| {
                sqlx::query("SELECT pg_advisory_lock($1)").bind(key)
                    .execute(sql.executor()).await?;
                Ok(())
            }));
            tokio::select! {
                result = &mut operation => panic!("operation did not block: {result:?}"),
                result = fixture.blocked(previous) => result.expect("observed server lock"),
            }
            drop(operation);
            let later = scope.sql(async |_| {
                panic!("abandoned scope must not invoke more work");
                #[allow(unreachable_code)]
                Ok(())
            }).await.unwrap_err();
            assert!(matches!(later, Failure::Scope(failure) if matches!(*failure, PgScopeFailure::OperationAbandoned)));
            Ok(91)
        }).await.unwrap_err();
        let Failure::Completion(outcome) = error else { panic!("outer uncertainty") };
        assert!(matches!(*outcome, PgAtomicUncertainty::ScopeLost {
            result: Ok(91), cause: PgScopeLoss::OperationAbandoned,
        }));
        fixture.replacement_and_close().await?;
        Ok(())
    }.await;
    fixture.finish(body).await
}
