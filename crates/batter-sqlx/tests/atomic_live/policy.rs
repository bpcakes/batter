use super::*;
use batter_sqlx::{
    PgAtomicUncertainty, PgFailurePolicy, PgScopeFailure, PgScopeLoss, PgTransactionError,
    run_atomic_with,
};
use std::{fmt, sync::Arc};

#[derive(Debug)]
pub(super) enum Failure {
    Sql(sqlx::Error),
    Begin(PgTransactionError),
    Scope(Box<PgScopeFailure<Self>>),
    Completion(Box<PgAtomicUncertainty<i64, Self>>),
    Rejected(&'static str),
    RolledBack(batter_sqlx::PgScopeRolledBack),
}
impl From<batter_sqlx::PgScopeRolledBack> for Failure {
    fn from(value: batter_sqlx::PgScopeRolledBack) -> Self {
        Self::RolledBack(value)
    }
}
impl From<sqlx::Error> for Failure {
    fn from(value: sqlx::Error) -> Self {
        Self::Sql(value)
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("consumer failure")
    }
}
impl std::error::Error for Failure {}
pub(super) struct Policy;
impl PgFailurePolicy<i64> for Policy {
    type Error = Failure;
    fn begin_failed(&self, cause: PgTransactionError) -> Failure {
        Failure::Begin(cause)
    }
    fn scope_lost(&self, failure: PgScopeFailure<Failure>) -> Failure {
        Failure::Scope(Box::new(failure))
    }
    fn commit_unconfirmed(&self, output: i64, cause: PgTransactionError) -> Failure {
        Failure::Completion(Box::new(PgAtomicUncertainty::CommitUnconfirmed {
            output,
            cause,
        }))
    }
    fn rollback_unconfirmed(&self, rejection: Failure, cause: PgTransactionError) -> Failure {
        Failure::Completion(Box::new(PgAtomicUncertainty::RollbackUnconfirmed {
            rejection,
            cause,
        }))
    }
    fn scope_lost_after_body(
        &self,
        result: std::result::Result<i64, Failure>,
        cause: PgScopeLoss,
    ) -> Failure {
        Failure::Completion(Box::new(PgAtomicUncertainty::ScopeLost { result, cause }))
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_infers_errors_and_preserves_recovery_and_rejection() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_policy_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id bigint)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        for commit in [false, true] {
            let result = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
                let error = scope
                    .sql(async |sql| {
                        sqlx::query("SELECT 1 / 0").execute(sql.executor()).await?;
                        Ok(())
                    })
                    .await
                    .unwrap_err();
                assert!(matches!(error, Failure::Sql(sqlx::Error::Database(_))));
                let id = scope
                    .sql(async |sql| {
                        let value = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                            "INSERT INTO {table} VALUES (17) RETURNING id"
                        )))
                        .fetch_one(sql.executor())
                        .await?;
                        Ok(value)
                    })
                    .await?;
                if commit {
                    Ok(id)
                } else {
                    Err(Failure::Rejected("reject"))
                }
            })
            .await;
            if commit {
                assert_eq!(result?, 17);
            } else {
                assert!(matches!(result, Err(Failure::Rejected("reject"))));
            }
            let count: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                    .fetch_one(&mut fixture.observer)
                    .await?;
            assert_eq!(count, i64::from(commit));
        }
        Ok(())
    }
    .await;
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TABLE {table}")))
        .execute(&mut fixture.observer)
        .await
        .map(|_| ())
        .map_err(Into::into);
    fixture.finish(support::combine(body, cleanup)).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_caught_loss_cannot_run_more_work_or_commit() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let mut first = None;
        let error = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
            let error = scope
                .sql(async |sql| {
                    sqlx::raw_sql("ROLLBACK").execute(sql.executor()).await?;
                    Ok(())
                })
                .await
                .unwrap_err();
            let Failure::Scope(failure) = error else {
                panic!("scope loss")
            };
            let PgScopeFailure::Transaction(cause) = *failure else {
                panic!("transaction loss")
            };
            first = Some(cause);
            let later = scope
                .sql(async |_| {
                    panic!("poison must prevent subsequent invocation");
                    #[allow(unreachable_code)]
                    Ok(())
                })
                .await
                .unwrap_err();
            let Failure::Scope(failure) = later else {
                panic!("scope loss")
            };
            let PgScopeFailure::Transaction(cause) = *failure else {
                panic!("transaction loss")
            };
            assert!(Arc::ptr_eq(&cause, first.as_ref().unwrap()));
            Ok(29)
        })
        .await
        .unwrap_err();
        let Failure::Completion(outcome) = error else {
            panic!("outer uncertainty")
        };
        let PgAtomicUncertainty::ScopeLost {
            result: Ok(29),
            cause: PgScopeLoss::Transaction(cause),
        } = *outcome
        else {
            panic!("provisional output and original cause")
        };
        assert!(Arc::ptr_eq(&cause, first.as_ref().unwrap()));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_recovery_loss_keeps_rejection_and_native_cause() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let error = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
            scope
                .sql(async |sql| {
                    sqlx::raw_sql("ROLLBACK").execute(sql.executor()).await?;
                    Err::<(), _>(Failure::Rejected("inner rejection"))
                })
                .await?;
            Ok(31)
        })
        .await
        .unwrap_err();
        let Failure::Completion(outcome) = error else {
            panic!("outer uncertainty")
        };
        let PgAtomicUncertainty::ScopeLost {
            result: Err(Failure::Scope(failure)),
            cause: PgScopeLoss::Transaction(cause),
        } = *outcome
        else {
            panic!("rejection and original cause")
        };
        let PgScopeFailure::Recovery {
            application: Failure::Rejected("inner rejection"),
            recovery,
        } = *failure
        else {
            panic!("paired application and recovery errors")
        };
        assert!(Arc::ptr_eq(&cause, &recovery));
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_completion_uncertainty_keeps_output_and_rejection() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        let error = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
            scope.sql(async |sql| {
                sqlx::raw_sql("CREATE TEMP TABLE policy_deferred(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED); INSERT INTO policy_deferred VALUES (1),(1)")
                    .execute(sql.executor()).await?;
                Ok(())
            }).await?;
            Ok(73)
        }).await.unwrap_err();
        let Failure::Completion(outcome) = error else { panic!("commit uncertainty") };
        assert!(matches!(*outcome, PgAtomicUncertainty::CommitUnconfirmed { output: 73, cause: PgTransactionError::Query(_) }));
        let error = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
            let previous = scope.sql(async |sql| Ok(pid(sql).await?)).await?;
            sqlx::query("SELECT pg_terminate_backend($1)").bind(previous)
                .execute(&mut fixture.observer).await?;
            Err(Failure::Rejected("outer rejection"))
        }).await.unwrap_err();
        let Failure::Completion(outcome) = error else { panic!("rollback uncertainty") };
        assert!(matches!(*outcome, PgAtomicUncertainty::RollbackUnconfirmed {
            rejection: Failure::Rejected("outer rejection"), cause: PgTransactionError::Query(_),
        }));
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn policy_begin_failure_does_not_invoke_body() -> Result {
    let fixture = fixture().await?;
    fixture.pool.close().await;
    let result = run_atomic_with(&fixture.pool, &Policy, async |_| {
        panic!("begin failure must prevent body invocation");
        #[allow(unreachable_code)]
        Ok(1)
    })
    .await;
    assert!(matches!(
        result,
        Err(Failure::Begin(PgTransactionError::Query(_)))
    ));
    fixture.finish(Ok(())).await
}
