use super::policy::{Failure, Policy};
use super::*;
use batter_sqlx::{
    PgAtomicUncertainty, PgScopeLoss, PgSessionProfile, run_atomic_profiled_fail_fast_with,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_profile_checks_before_work_and_restores_rejected_changes() -> Result {
    let mut fixture = fixture().await?;
    let schema = format!("fast_profile_{}", fixture.key);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut fixture.observer)
        .await?;
    let body = async {
        let login = fixture.pool.connect_options().get_username().to_owned();
        let profile = PgSessionProfile::with_timeouts(
            &login,
            &login,
            vec!["public".into(), schema.clone()],
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        )?
        .with_setting("app.tenant", "original")?;
        for rejection in [true, false] {
            let error = run_atomic_profiled_fail_fast_with(
                &fixture.pool,
                &profile,
                &Policy,
                async |mut scope| {
                    scope
                        .sql(async |sql| {
                            sqlx::query("SELECT set_config('app.tenant', 'changed', true)")
                                .execute(sql.executor())
                                .await?;
                            if rejection {
                                Err(Failure::Rejected("restore settings"))
                            } else {
                                Ok(())
                            }
                        })
                        .await?;
                    Ok(1)
                },
            )
            .await
            .unwrap_err();
            if rejection {
                assert!(matches!(error, Failure::Rejected("restore settings")));
            } else {
                let Failure::Completion(outcome) = error else {
                    panic!("profile drift is terminal")
                };
                assert!(matches!(
                    *outcome,
                    PgAtomicUncertainty::ScopeLost {
                        cause: PgScopeLoss::Transaction(_),
                        ..
                    }
                ));
            }
        }
        let error = run_atomic_profiled_fail_fast_with(
            &fixture.pool,
            &profile,
            &Policy,
            async |mut scope| {
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema}")))
                    .execute(&mut fixture.observer)
                    .await?;
                super::validation::counted(
                    1,
                    scope.sql(async |_| {
                        panic!("external profile drift must prevent work");
                        #[allow(unreachable_code)]
                        Ok(())
                    }),
                )
                .await?;
                Ok(2)
            },
        )
        .await
        .unwrap_err();
        let Failure::Completion(outcome) = error else {
            panic!("profile loss")
        };
        assert!(matches!(
            *outcome,
            PgAtomicUncertainty::ScopeLost {
                cause: PgScopeLoss::Transaction(_),
                ..
            }
        ));
        Ok(())
    }
    .await;
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema}"
    )))
    .execute(&mut fixture.observer)
    .await
    .map(|_| ())
    .map_err(Into::into);
    fixture.finish(support::combine(body, cleanup)).await
}
