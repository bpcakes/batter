use super::policy::{Failure, Policy};
use super::*;
use batter_sqlx::{PgAtomicUncertainty, PgScopeFailure, PgScopeLoss, run_atomic_fail_fast_with};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_replaced_transactions_never_become_known_rejections() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_fast_loss_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id bigint)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        for control in ["COMMIT", "ROLLBACK"] {
            let error = run_atomic_fail_fast_with(&fixture.pool, &Policy, async |mut scope| {
                scope.sql(async |sql| {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                        "{control}; BEGIN; INSERT INTO {table} VALUES (1); COMMIT; BEGIN; SELECT 1/0"
                    ))).execute(sql.executor()).await?;
                    Ok(())
                }).await?;
                Ok(11)
            }).await.unwrap_err();
            let Failure::Completion(outcome) = error else { panic!("replacement is uncertain") };
            let PgAtomicUncertainty::ScopeLost {
                result: Err(Failure::Scope(failure)), cause: PgScopeLoss::Transaction(cause),
            } = *outcome else { panic!("keep body and loss cause") };
            let PgScopeFailure::Recovery { application: Failure::Sql(sqlx::Error::Database(_)), recovery } = *failure
                else { panic!("keep SQL error and missing guard") };
            assert!(Arc::ptr_eq(&cause, &recovery));
        }
        let committed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&mut fixture.observer).await?;
        assert_eq!(committed, 2, "explicit transaction control really committed replacement writes");
        Ok(())
    }.await;
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
        .execute(&mut fixture.observer)
        .await
        .map(|_| ())
        .map_err(Into::into);
    fixture.finish(support::combine(body, cleanup)).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_caught_validation_loss_keeps_first_cause() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let mut first = None;
        let error = run_atomic_fail_fast_with(&fixture.pool, &Policy, async |mut scope| {
            let error = scope
                .sql(async |sql| {
                    sqlx::raw_sql("COMMIT; BEGIN")
                        .execute(sql.executor())
                        .await?;
                    Ok(())
                })
                .await
                .unwrap_err();
            let Failure::Scope(failure) = error else {
                panic!("terminal validation loss")
            };
            let PgScopeFailure::Transaction(cause) = *failure else {
                panic!("native cause")
            };
            first = Some(cause);
            let error = scope
                .sql(async |_| {
                    panic!("lost scope must not run work");
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
                .unwrap_err();
            let Failure::Scope(failure) = error else {
                panic!("retained loss")
            };
            let PgScopeFailure::Transaction(cause) = *failure else {
                panic!("native cause")
            };
            assert!(Arc::ptr_eq(&cause, first.as_ref().unwrap()));
            Ok(17)
        })
        .await
        .unwrap_err();
        let Failure::Completion(outcome) = error else {
            panic!("caught loss cannot commit")
        };
        let PgAtomicUncertainty::ScopeLost {
            result: Ok(17),
            cause: PgScopeLoss::Transaction(cause),
        } = *outcome
        else {
            panic!("retain provisional output and loss")
        };
        assert!(Arc::ptr_eq(&cause, first.as_ref().unwrap()));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_abandoned_operation_never_claims_rollback() -> Result {
    let mut fixture = fixture().await?;
    let pool = fixture.pool.clone();
    let key = fixture.key;
    let body = async {
        let error = run_atomic_fail_fast_with(&pool, &Policy, async |mut scope| {
            let previous = scope.sql(async |sql| Ok(pid(sql).await?)).await?;
            fixture.retired.push(previous);
            let mut operation = Box::pin(scope.sql(async |sql| {
                sqlx::query("SELECT pg_advisory_lock($1)")
                    .bind(key)
                    .execute(sql.executor())
                    .await?;
                Ok(())
            }));
            tokio::select! {
                result = &mut operation => panic!("operation did not block: {result:?}"),
                result = fixture.blocked(previous) => result.expect("observed server lock"),
            }
            drop(operation);
            Ok(91)
        })
        .await
        .unwrap_err();
        let Failure::Completion(outcome) = error else {
            panic!("outer uncertainty")
        };
        assert!(matches!(
            *outcome,
            PgAtomicUncertainty::ScopeLost {
                result: Ok(91),
                cause: PgScopeLoss::OperationAbandoned,
            }
        ));
        fixture.replacement_and_close().await?;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_commit_and_cleanup_failures_remain_uncertain() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        let error = run_atomic_fail_fast_with(&fixture.pool, &Policy, async |mut scope| {
            scope.sql(async |sql| {
                sqlx::raw_sql("CREATE TEMP TABLE fast_deferred(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED); INSERT INTO fast_deferred VALUES (1),(1)")
                    .execute(sql.executor()).await?;
                Ok(())
            }).await?;
            Ok(73)
        }).await.unwrap_err();
        let Failure::Completion(outcome) = error else { panic!("commit uncertainty") };
        assert!(matches!(*outcome, PgAtomicUncertainty::CommitUnconfirmed { output: 73, .. }));
        let error = run_atomic_fail_fast_with(&fixture.pool, &Policy, async |mut scope| {
            scope.sql(async |sql| {
                let previous = pid(sql).await?;
                sqlx::query("SELECT pg_terminate_backend($1)").bind(previous)
                    .execute(&mut fixture.observer).await?;
                Err::<(), _>(Failure::Rejected("transport rejection"))
            }).await?;
            Ok(0)
        }).await.unwrap_err();
        let Failure::Completion(outcome) = error else { panic!("cleanup uncertainty") };
        let PgAtomicUncertainty::ScopeLost {
            result: Err(Failure::Scope(failure)), cause: PgScopeLoss::Transaction(cause),
        } = *outcome else { panic!("paired rejection and cleanup loss") };
        let PgScopeFailure::Recovery { application: Failure::Rejected("transport rejection"), recovery } = *failure
            else { panic!("original rejection and original cleanup failure") };
        assert!(Arc::ptr_eq(&cause, &recovery));
        Ok(())
    }.await;
    fixture.finish(body).await
}
