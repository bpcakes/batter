use super::*;
use batter_sqlx::{PgAtomicError, PgAtomicUncertainty, PgScopeError, PgScopeLoss, run_atomic};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn runner_releases_only_acknowledged_outputs() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_runner_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id integer)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        for commit in [false, true] {
            let result = run_atomic(&fixture.pool, async |scope| {
                let id = scope
                    .application(async |sql| {
                        sqlx::query_scalar::<_, i32>(sqlx::AssertSqlSafe(format!(
                            "INSERT INTO {table} VALUES (7) RETURNING id"
                        )))
                        .fetch_one(sql.executor())
                        .await
                    })
                    .await?;
                let visible: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table}"
                )))
                .fetch_one(&mut fixture.observer)
                .await
                .map_err(PgScopeError::Application)?;
                assert_eq!(visible, 0, "body output is still provisional");
                if commit {
                    Ok(id)
                } else {
                    Err(PgScopeError::Application(sqlx::Error::Protocol(
                        "reject".into(),
                    )))
                }
            })
            .await;
            if commit {
                assert_eq!(result?, 7);
            } else {
                assert!(matches!(
                    result,
                    Err(PgAtomicError::Rejected(PgScopeError::Application(
                        sqlx::Error::Protocol(_)
                    )))
                ));
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
async fn runner_uncertainty_retains_output_and_rejection() -> Result {
    let mut fixture = fixture().await?;
    let body = async {
        let error = run_atomic(&fixture.pool, async |scope| {
            scope.application(async |sql| {
                sqlx::raw_sql("CREATE TEMP TABLE runner_deferred(id integer UNIQUE DEFERRABLE INITIALLY DEFERRED); INSERT INTO runner_deferred VALUES (1),(1)")
                    .execute(sql.executor()).await?;
                Ok::<_, sqlx::Error>(())
            }).await?;
            Ok::<_, PgScopeError<sqlx::Error>>(73)
        }).await.unwrap_err();
        assert!(matches!(error, PgAtomicError::Uncertain(PgAtomicUncertainty::CommitUnconfirmed { output: 73, .. })));
        let error = run_atomic(&fixture.pool, async |scope| {
            let previous = scope.application(async |sql| pid(sql).await).await?;
            sqlx::query("SELECT pg_terminate_backend($1)").bind(previous)
                .execute(&mut fixture.observer).await.map_err(PgScopeError::Application)?;
            Err::<(), _>(PgScopeError::Application(sqlx::Error::Protocol("retained rejection".into())))
        }).await.unwrap_err();
        assert!(matches!(error, PgAtomicError::Uncertain(PgAtomicUncertainty::RollbackUnconfirmed {
            rejection: PgScopeError::Application(sqlx::Error::Protocol(ref message)), ..
        }) if message == "retained rejection"));
        assert!(!format!("{error:?}").contains("retained rejection"));
        Ok(())
    }.await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn runner_cannot_commit_after_caught_operation_cancellation() -> Result {
    let mut fixture = fixture().await?;
    let pool = fixture.pool.clone();
    let key = fixture.key;
    let body = async {
        let result = run_atomic(&pool, async |scope| {
            let previous = scope.application(async |sql| pid(sql).await).await?;
            fixture.retired.push(previous);
            let mut operation = Box::pin(scope.application(async |sql| {
                sqlx::query("SELECT pg_advisory_lock($1)")
                    .bind(key)
                    .execute(sql.executor())
                    .await
            }));
            tokio::select! {
                result = &mut operation => panic!("operation did not block: {result:?}"),
                result = fixture.blocked(previous) => result.expect("observed server lock"),
            }
            drop(operation);
            let mut invoked = false;
            let later = scope
                .application(async |_| {
                    invoked = true;
                    Ok::<(), sqlx::Error>(())
                })
                .await;
            assert!(!invoked, "abandoned scope invoked subsequent work");
            assert!(matches!(
                later,
                Err(PgScopeError::Terminal(
                    batter_sqlx::PgScopeFailure::OperationAbandoned
                ))
            ));
            // Even deliberately swallowing the cancellation cannot resurrect the owner.
            Ok::<_, PgScopeError<sqlx::Error>>(91)
        })
        .await;
        assert!(matches!(
            result,
            Err(PgAtomicError::Uncertain(PgAtomicUncertainty::ScopeLost {
                result: Ok(91),
                cause: PgScopeLoss::OperationAbandoned,
            }))
        ));
        fixture.replacement_and_close().await
    }
    .await;
    fixture.finish(body).await
}
