use super::{
    fixture,
    policy::{Failure, Policy},
    support::Result,
    validation::counted,
};
use batter_sqlx::{
    PgPolicyScope, PgSessionProfile, run_atomic, run_atomic_fail_fast_with,
    run_atomic_profiled_fail_fast_with, run_atomic_profiled_with, run_atomic_with,
};
use std::time::Duration;

#[derive(Debug, PartialEq)]
struct Record {
    value: i64,
}

async fn macro_queries(
    scope: &mut PgPolicyScope<'_, i64, Policy>,
    statements: usize,
) -> std::result::Result<i64, Failure> {
    for value in [41_i64, 42] {
        assert_eq!(
            counted(
                statements,
                scope.fetch_one(sqlx::query_scalar!(
                    "SELECT $1::bigint AS \"value!\"",
                    value
                ))
            )
            .await?,
            value
        );
    }
    let text = String::from("bound value");
    let row = scope
        .fetch_one(sqlx::query!("SELECT $1::text AS \"value!\"", &text))
        .await?;
    assert_eq!(row.value, text);
    let row = scope
        .fetch_one(sqlx::query_as!(Record, "SELECT 42::bigint AS \"value!\""))
        .await?;
    assert_eq!(row, Record { value: 42 });
    macro_collections(scope).await?;
    macro_commands(scope).await?;
    Ok(42)
}

async fn macro_collections(
    scope: &mut PgPolicyScope<'_, i64, Policy>,
) -> std::result::Result<(), Failure> {
    assert!(
        scope
            .fetch_optional(sqlx::query!("SELECT 42::bigint AS \"value!\" WHERE false"))
            .await?
            .is_none()
    );
    assert!(
        scope
            .fetch_optional(sqlx::query_scalar!(
                "SELECT 42::bigint AS \"value!\" WHERE false"
            ))
            .await?
            .is_none()
    );
    let rows = scope
        .fetch_all(sqlx::query_as!(
            Record,
            "SELECT value AS \"value!\" FROM generate_series(1::bigint, 3) AS value ORDER BY value"
        ))
        .await?;
    assert_eq!(
        rows,
        vec![
            Record { value: 1 },
            Record { value: 2 },
            Record { value: 3 }
        ]
    );
    assert_eq!(scope.fetch_all(sqlx::query_scalar!(
        "SELECT value AS \"value!\" FROM generate_series(1::bigint, 3) AS value ORDER BY value")).await?, vec![1,2,3]);
    Ok(())
}

async fn macro_commands(
    scope: &mut PgPolicyScope<'_, i64, Policy>,
) -> std::result::Result<(), Failure> {
    // The no-result macro produces Query rather than Map/QueryScalar.
    assert_eq!(
        scope
            .execute(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .rows_affected(),
        0
    );
    assert!(
        scope
            .fetch_optional(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .is_none()
    );
    assert!(
        scope
            .fetch_all(sqlx::query!("DO $$ BEGIN NULL; END $$"))
            .await?
            .is_empty()
    );
    assert_eq!(
        scope
            .execute(sqlx::query!("SELECT 42::bigint AS \"value!\""))
            .await?
            .rows_affected(),
        1
    );
    assert_eq!(
        scope
            .execute(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
            .await?
            .rows_affected(),
        1
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn native_query_macros_preserve_scope_modes_and_statement_counts() -> Result {
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
            for fast in [false, true] {
                let statements = if fast { 2 } else { 4 } + usize::from(profiled);
                let work = async |mut scope: PgPolicyScope<'_, i64, Policy>| {
                    macro_queries(&mut scope, statements).await
                };
                let value = match (profiled, fast) {
                    (false, false) => run_atomic_with(&fixture.pool, &Policy, work).await?,
                    (false, true) => {
                        run_atomic_fail_fast_with(&fixture.pool, &Policy, work).await?
                    }
                    (true, false) => {
                        run_atomic_profiled_with(&fixture.pool, &profile, &Policy, work).await?
                    }
                    (true, true) => {
                        run_atomic_profiled_fail_fast_with(&fixture.pool, &profile, &Policy, work)
                            .await?
                    }
                };
                assert_eq!(value, 42);
            }
        }
        let value = run_atomic(&fixture.pool, async |scope| {
            scope
                .execute(sqlx::query!("DO $$ BEGIN NULL; END $$"))
                .await?;
            assert!(
                scope
                    .fetch_optional(sqlx::query_scalar!(
                        "SELECT 42::bigint AS \"value!\" WHERE false"
                    ))
                    .await?
                    .is_none()
            );
            assert_eq!(
                scope
                    .fetch_all(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
                    .await?,
                vec![42]
            );
            scope
                .fetch_one(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
                .await
        })
        .await?;
        assert_eq!(value, 42);
        Ok(())
    }
    .await;
    fixture.finish(body).await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn native_query_helpers_preserve_mapper_errors_and_refuse_closed_work() -> Result {
    let fixture = fixture().await?;
    let body = async {
        for fast in [false, true] {
            let work = async |mut scope: PgPolicyScope<'_, i64, Policy>| {
                let error = scope
                    .fetch_one(sqlx::query!("DO $$ BEGIN NULL; END $$"))
                    .await
                    .unwrap_err();
                assert!(matches!(error, Failure::Sql(sqlx::Error::RowNotFound)));
                let later = scope
                    .fetch_one(sqlx::query("SELECT 1").map(|_| {
                        assert!(!fast, "closed scope invoked mapper");
                        42_i64
                    }))
                    .await;
                if fast {
                    assert!(matches!(later, Err(Failure::RolledBack(_))));
                } else {
                    assert_eq!(later?, 42);
                }
                Ok(42)
            };
            if fast {
                assert!(matches!(
                    run_atomic_fail_fast_with(&fixture.pool, &Policy, work).await,
                    Err(Failure::RolledBack(_))
                ));
            } else {
                assert_eq!(run_atomic_with(&fixture.pool, &Policy, work).await?, 42);
            }
        }
        let value = run_atomic_with(&fixture.pool, &Policy, async |mut scope| {
            let error = scope
                .fetch_all(sqlx::query("SELECT 1").try_map(|_| {
                    Err::<i64, _>(sqlx::Error::Decode(
                        std::io::Error::other("mapper rejected").into(),
                    ))
                }))
                .await
                .unwrap_err();
            assert!(matches!(error, Failure::Sql(sqlx::Error::Decode(_))));
            let count = scope
                .execute(
                    sqlx::query("SELECT 1")
                        .map(|_| -> i64 { panic!("execute must discard the mapper") }),
                )
                .await?
                .rows_affected();
            assert_eq!(count, 1);
            assert_eq!(
                scope
                    .execute(sqlx::query_scalar::<_, String>("SELECT 1"))
                    .await?
                    .rows_affected(),
                1
            );
            scope
                .fetch_one(sqlx::query_scalar!("SELECT 42::bigint AS \"value!\""))
                .await
        })
        .await?;
        assert_eq!(value, 42);
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
