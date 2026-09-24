use super::policy::{Failure, Policy};
use super::*;
use batter_sqlx::{
    PgSessionProfile, run_atomic_fail_fast_with, run_atomic_profiled_fail_fast_with,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_scopes_have_no_per_operation_savepoint() -> Result {
    let fixture = fixture().await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = PgSessionProfile::with_timeouts(
            &login,
            &login,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        )?;
        for profiled in [false, true] {
            let work = async |mut scope: batter_sqlx::PgPolicyScope<'_, i64, Policy>| {
                for _ in 0..2 {
                    let value = super::validation::counted(
                        if profiled { 3 } else { 2 },
                        scope.sql(async |sql| {
                            Ok(sqlx::query_scalar::<_, i64>("SELECT 42::bigint")
                                .fetch_one(sql.executor())
                                .await?)
                        }),
                    )
                    .await?;
                    assert_eq!(value, 42);
                }
                // Recovery is an explicit choice, including inside a fast workflow.
                let error = super::validation::counted(
                    if profiled { 6 } else { 5 },
                    scope.recoverable_sql(async |sql| {
                        sqlx::query("SELECT 1 / 0").execute(sql.executor()).await?;
                        Ok(())
                    }),
                )
                .await
                .unwrap_err();
                assert!(matches!(error, Failure::Sql(sqlx::Error::Database(_))));
                super::validation::counted(
                    if profiled { 3 } else { 2 },
                    scope.sql(async |sql| {
                        Ok(sqlx::query_scalar::<_, i64>("SELECT 43::bigint")
                            .fetch_one(sql.executor())
                            .await?)
                    }),
                )
                .await
            };
            let value = if profiled {
                run_atomic_profiled_fail_fast_with(&fixture.pool, &profile, &Policy, work).await?
            } else {
                run_atomic_fail_fast_with(&fixture.pool, &Policy, work).await?
            };
            assert_eq!(value, 43);
        }
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_rejection_rolls_back_prior_writes_and_refuses_caught_success() -> Result {
    let mut fixture = fixture().await?;
    let table = format!("batter_fast_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {table}(id bigint)"
    )))
    .execute(&mut fixture.observer)
    .await?;
    let body = async {
        for sql_error in [false, true] {
            for caught in [false, true] {
                let error = run_atomic_fail_fast_with(&fixture.pool, &Policy, async |mut scope| {
                    let original =
                        scope
                            .sql(async |sql| {
                                // Nested application savepoints may survive fast calls;
                                // the transaction guard still rolls back every write.
                                sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                                    "INSERT INTO {table} VALUES (1); SAVEPOINT application_guard"
                                )))
                                .execute(sql.executor())
                                .await?;
                                Ok(sqlx::query_scalar::<_, String>(
                                    "SELECT pg_current_xact_id()::text",
                                )
                                .fetch_one(sql.executor())
                                .await?)
                            })
                            .await?;
                    let result = scope
                        .sql(async |sql| {
                            sqlx::query(sqlx::AssertSqlSafe(format!(
                                "INSERT INTO {table} VALUES (2)"
                            )))
                            .execute(sql.executor())
                            .await?;
                            if sql_error {
                                sqlx::query("SELECT 1/0").execute(sql.executor()).await?;
                            }
                            Err::<(), _>(Failure::Rejected("reject"))
                        })
                        .await;
                    if !caught {
                        result?;
                    } else {
                        assert!(matches!(
                            result,
                            Err(Failure::Rejected("reject") | Failure::Sql(_))
                        ));
                        assert_rollback_observed(&mut fixture.observer, &original, &table).await?;
                        let refusal = scope
                            .sql(async |_| {
                                panic!("closed scope must not invoke SQL work");
                                #[allow(unreachable_code)]
                                Ok(())
                            })
                            .await
                            .unwrap_err();
                        assert!(matches!(refusal, Failure::RolledBack(_)));
                        let refusal = scope
                            .recoverable_sql(async |_| {
                                panic!("recovery cannot resurrect a closed scope");
                                #[allow(unreachable_code)]
                                Ok(())
                            })
                            .await
                            .unwrap_err();
                        assert!(matches!(refusal, Failure::RolledBack(_)));
                    }
                    Ok(99)
                })
                .await
                .unwrap_err();
                if caught {
                    let Failure::RolledBack(proof) = error else {
                        panic!("expected known rollback")
                    };
                    assert_eq!(
                        format!("{proof:?}"),
                        "PostgreSQL transaction already rolled back after scope rejection"
                    );
                } else if sql_error {
                    assert!(matches!(error, Failure::Sql(sqlx::Error::Database(_))));
                } else {
                    assert!(matches!(error, Failure::Rejected("reject")));
                }
                let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table}"
                )))
                .fetch_one(&mut fixture.observer)
                .await?;
                assert_eq!(count, 0);
            }
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

async fn assert_rollback_observed(
    observer: &mut sqlx::PgConnection,
    original: &str,
    table: &str,
) -> std::result::Result<(), sqlx::Error> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT pg_catalog.pg_xact_status($1::text::xid8)")
            .bind(original)
            .fetch_one(&mut *observer)
            .await?;
    assert_eq!(status.as_deref(), Some("aborted"));
    let remaining: i64 =
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(observer)
            .await?;
    assert_eq!(
        remaining, 0,
        "rollback is acknowledged before scope error returns"
    );
    Ok(())
}
