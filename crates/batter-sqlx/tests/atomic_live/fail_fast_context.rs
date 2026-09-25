use super::{
    fixture,
    policy::{Failure, Policy},
    support::Result,
};
use batter_core::operation::{OperationError, OperationOwner};
use batter_sqlx::{
    PgPolicyScope, PgProfiledPool, PgSessionProfile, run_atomic_fail_fast_with_in,
    run_atomic_profiled_fail_fast_with_in,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn fail_fast_rollback_acknowledgement_survives_caught_error_then_cancellation() -> Result {
    let fixture = fixture().await?;
    let body = async {
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
        for profiled in [false, true] {
            for completed in [false, true] {
                let owner = OperationOwner::new(Duration::from_secs(10))?;
                let context = owner.context().clone();
                let work = async |mut scope: PgPolicyScope<'_, i64, Policy>| {
                    let error = scope
                        .sql(async |sql| {
                            sqlx::query("SELECT 1 / 0").execute(sql.executor()).await?;
                            Ok(())
                        })
                        .await
                        .unwrap_err();
                    assert!(matches!(error, Failure::Sql(sqlx::Error::Database(_))));
                    owner.cancel();
                    if completed {
                        // A returned original error wins over fallback rollback evidence.
                        Err(error)
                    } else {
                        // Cancellation after acknowledged rollback must retain disposition,
                        // even if the callback catches the error and never finishes.
                        std::future::pending().await
                    }
                };
                let result = if profiled {
                    run_atomic_profiled_fail_fast_with_in(
                        &database,
                        &context,
                        "fast.rollback",
                        &Policy,
                        work,
                    )
                    .await
                } else {
                    run_atomic_fail_fast_with_in(
                        &fixture.pool,
                        &context,
                        "fast.rollback",
                        &Policy,
                        work,
                    )
                    .await
                };
                if completed {
                    assert!(matches!(
                        result,
                        Err(OperationError::Failed(Failure::Sql(sqlx::Error::Database(
                            _
                        ))))
                    ));
                } else {
                    assert!(matches!(
                        result,
                        Err(OperationError::Failed(Failure::RolledBack(_)))
                    ));
                }
            }
        }
        database.pool().close().await;
        Ok(())
    }
    .await;
    fixture.finish(body).await
}
