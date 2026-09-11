use std::time::Duration;

use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver,
};
use sqlx::postgres::PgPoolOptions;

use super::{
    ProbeResult,
    fixture_completion::ObservedRun,
    fixture_observation::{BOUND, admin},
    fixture_run,
};

pub async fn active_attempts() -> ProbeResult {
    let catalog = admin().await?;
    let mut blocker = catalog.acquire().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    // The key is scoped to this admin database and live blocker identity.
    sqlx::query("SELECT pg_advisory_lock($1::bigint)")
        .bind(i64::from(pid))
        .execute(&mut *blocker)
        .await?;
    let gated = PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .after_connect(move |conn, _| {
            Box::pin(async move {
                sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
                    .bind(i64::from(pid))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect_lazy_with(catalog.connect_options().as_ref().clone());
    let broken = admin().await?;
    broken.close().await;
    let control = SessionObserver::new(gated.clone(), Duration::from_secs(3))?;
    let run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(|scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.empty(&plan).await?;
                scope.empty(&plan).await?;
                Ok::<_, FixtureError>(())
            })
        });
    // An actual blocked query proves both attempts selected the old pool even
    // when the driver and observer run on different Tokio worker threads.
    tokio::time::timeout(BOUND, async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(pid)
            .fetch_one(&catalog)
            .await?;
            if count == 2 {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let mut completion = ObservedRun::new(run, control, gated, catalog.clone());
    completion.retry_with(broken);
    let error = completion
        .finish(Duration::from_millis(30))
        .await
        .expect_err("blocked or failed observations retain completion ownership");
    let mut completion = *error.0.downcast::<ObservedRun<(), FixtureError>>().unwrap();
    // One unread request authorizes one subsequent attempt for each database.
    tokio::time::timeout(BOUND, async {
        loop {
            let p = completion.run.cleanup_progress();
            if p.len() == 2
                && p.iter().all(|p| {
                    p.observation_failures.len() == 2 && p.phase == CleanupPhase::AwaitingRetry
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let before = completion.run.cleanup_progress();
    blocker.close().await?;
    completion.retry_with(admin().await?);
    let report = completion.finish(BOUND).await?;
    for database in &report.databases {
        fixture_run::absent(&catalog, &database.database_name).await?;
    }
    catalog.close().await;
    assert_report(&report, &before);
    Ok(())
}

fn assert_report(
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    before: &[batter_sqlx::test_support::DatabaseProgress],
) {
    assert!(!report.is_ok() && report.body.is_ok() && report.drain.is_ok());
    assert_eq!(report.databases.len(), 2);
    for (database, before) in report.databases.iter().zip(before) {
        assert!(database.result.is_ok());
        assert_eq!(database.observation_failures.len(), 2);
        assert!(matches!(
            *database.observation_failures[0],
            FixtureError::ObservationTimeout
        ));
        assert!(matches!(
            *database.observation_failures[1],
            FixtureError::Observe(sqlx::Error::PoolClosed)
        ));
        for (old, retained) in before
            .observation_failures
            .iter()
            .zip(&database.observation_failures)
        {
            assert!(std::sync::Arc::ptr_eq(old, retained));
        }
    }
}
