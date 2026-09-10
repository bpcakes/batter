use std::time::Duration;

use batter_sqlx::test_support::{BodyFailure, ConnectionPlan, FixtureError, FixtureSuite};
use postgres_test_harness::{HarnessConfig, PostgresHarness, cleanup_stale_databases};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::oneshot;

use super::{ProbeResult, fixture_run};

pub async fn probe() -> ProbeResult {
    // Exclusive disposable endpoint: this acknowledged global catalog lock must
    // not race another live case. The runner selects --test-threads=1.
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let config = HarnessConfig::new("batter_faults")?
        .with_admin_database_url(url.clone())
        .with_cleanup_on_start(false)
        .with_connection_budget(8)?
        .with_connections_per_database(4)?
        .with_operation_timeout(Duration::from_secs(1))?;
    let harness = PostgresHarness::start(config).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let (ready_sender, ready_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();
    let mut run = FixtureSuite::new(harness).start(move |scope| {
        Box::pin(async move {
            let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
            let fixture = scope.empty(&plan).await?;
            ready_sender
                .send(fixture.database_name().to_owned())
                .expect("test observer alive");
            release_receiver
                .await
                .expect("catalog lock acknowledgement");
            sqlx::query("SELECT 1 / 0")
                .execute(&fixture.pools()[0])
                .await
                .map_err(FixtureError::Observe)?;
            Ok::<_, FixtureError>(())
        })
    });
    let name = ready_receiver.await?;
    let mut lock = observer.begin().await?;
    sqlx::query("LOCK TABLE pg_catalog.pg_shdescription IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    release_sender
        .send(())
        .expect("body waits for acknowledged lock");
    let completed_under_lock = tokio::time::timeout(Duration::from_secs(10), run.wait())
        .await
        .is_ok();
    let unlocked = lock.rollback().await;
    let report = run.into_report().await?;
    // The failed consuming cleanup retains a native error, not a reusable lease.
    // Reclaim the injected residual through upstream's owner-aware public API.
    let recovered = cleanup_stale_databases(&url, "batter_faults", Duration::ZERO).await;
    let absence = fixture_run::absent(&observer, &name).await;
    observer.close().await;
    unlocked?;
    let recovered = recovered?;
    absence?;
    assert!(completed_under_lock);
    assert!(recovered.dropped_test_databases >= 1);
    assert_both(&report);
    low_level_both().await?;
    Ok(())
}

fn assert_both(report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>) {
    assert!(
        matches!(&report.body, Err(BodyFailure::Returned(FixtureError::Observe(sqlx::Error::Database(error)))) if error.code().as_deref() == Some("22012"))
    );
    assert_eq!(report.databases.len(), 1);
    assert!(matches!(
        &report.databases[0].result,
        Err(FixtureError::Harness(
            postgres_test_harness::Error::Postgres {
                operation: "drop disposable PostgreSQL database",
                ..
            }
        ))
    ));
    assert!(
        report.drain.is_ok(),
        "a delivered consuming-cleanup error is not repeated by drain"
    );
    assert!(report.acquisitions.iter().all(Result::is_ok));
}

async fn low_level_both() -> ProbeResult {
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let config = HarnessConfig::new("batter_faults")?
        .with_admin_database_url(url.clone())
        .with_cleanup_on_start(false)
        .with_operation_timeout(Duration::from_secs(1))?;
    let harness = PostgresHarness::start(config).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let suite = FixtureSuite::new(harness);
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    let fixture = suite.empty(&plan).await?;
    let name = fixture.database_name().to_owned();
    let body = sqlx::query("SELECT 1 / 0")
        .execute(&fixture.pools()[0])
        .await;
    let mut lock = observer.begin().await?;
    sqlx::query("LOCK TABLE pg_catalog.pg_shdescription IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await?;
    let result = fixture.finish(body).await;
    let unlocked = lock.rollback().await;
    let drained = suite.drain().await;
    drop(suite);
    let recovered = cleanup_stale_databases(&url, "batter_faults", Duration::ZERO).await;
    let absence = fixture_run::absent(&observer, &name).await;
    observer.close().await;
    unlocked?;
    drained?;
    recovered?;
    absence?;
    assert!(
        matches!(result, Err(batter_test_support::TestFailure::Both { body: sqlx::Error::Database(error), cleanup: FixtureError::Harness(_) }) if error.code().as_deref() == Some("22012"))
    );
    Ok(())
}
