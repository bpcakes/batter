use std::time::Duration;

use batter_sqlx::test_support::{FixtureError, FixtureRun, FixtureSuite, SessionObserver};
use sqlx::postgres::PgPoolOptions;

use super::{ProbeResult, fixture_completion::ObservedRun};

const BOUND: Duration = Duration::from_secs(5);

pub async fn driver_error_closes_admin_pools() -> ProbeResult {
    let harness = super::harness().await?;
    for held in [false, true] {
        completion_case(harness.clone(), held).await?;
    }
    successful_driver_with_blocked_admin_close(harness.clone()).await?;
    replacement_closes_while_original_is_blocked(harness.clone()).await?;
    harness.drain_deferred_cleanup().await?;
    Ok(())
}

async fn replacement_closes_while_original_is_blocked(
    harness: postgres_test_harness::PostgresHarness,
) -> ProbeResult {
    let catalog = super::fixture_observation::admin().await?;
    let original = super::fixture_observation::admin().await?;
    let replacement = super::fixture_observation::admin().await?;
    let held = original.acquire().await?;
    let control = SessionObserver::new(original.clone(), BOUND)?;
    let mut run =
        FixtureSuite::new(harness).start(|_| Box::pin(async { Ok::<_, FixtureError>(()) }));
    assert!(tokio::time::timeout(BOUND, run.wait()).await?.is_ok());
    let mut completion = ObservedRun::new(run, control, original.clone(), catalog.clone());
    completion.retry_with(replacement.clone());
    let error = completion
        .finish(Duration::from_millis(40))
        .await
        .expect_err("original checkout blocks completion");
    let pending = *error.0.downcast::<ObservedRun<(), FixtureError>>().unwrap();
    let both_started = original.is_closed() && replacement.is_closed();
    tokio::time::timeout(BOUND, replacement.close()).await?;
    let replacement_finished = replacement.size() == 0;
    drop(held);
    let report = pending.finish(BOUND).await?;
    catalog.close().await;
    assert!(both_started && replacement_finished && report.is_ok());
    assert_eq!(original.size(), 0);
    Ok(())
}

async fn completion_case(
    harness: postgres_test_harness::PostgresHarness,
    held: bool,
) -> ProbeResult {
    let mut run = cancelled_run(harness.clone()).await?;
    let error = run.wait().await.expect_err("runtime destroyed the driver");
    assert!(error.is_cancelled());
    let task_id = error.id();
    let diagnostic = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let session = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let checkouts = if held {
        Some((diagnostic.acquire().await?, session.acquire().await?))
    } else {
        None
    };
    let observer = SessionObserver::new(session.clone(), BOUND)?;
    let completion = ObservedRun::new(run, observer, session.clone(), diagnostic.clone());
    let wait_bound = if held {
        Duration::from_millis(40)
    } else {
        BOUND
    };
    let error = tokio::time::timeout(BOUND, completion.finish(wait_bound))
        .await?
        .expect_err("driver failure cannot become success");
    let error = if held {
        assert!(error.to_string().contains("driver_joined=true"));
        assert!(error.to_string().contains("driver_failed=true"));
        let mut pending = *error
            .0
            .downcast::<ObservedRun<(), FixtureError>>()
            .expect("blocked admin close retains the completion owner");
        // Both closes started, but neither has completed while checked out.
        assert!(diagnostic.is_closed() && session.is_closed());
        assert_eq!(diagnostic.size(), 1);
        assert_eq!(session.size(), 1);
        assert_eq!(pending.run.wait().await.unwrap_err().id(), task_id);
        drop(checkouts);
        tokio::time::timeout(BOUND, pending.finish(BOUND))
            .await?
            .expect_err("resumed completion retains the original failure")
    } else {
        error
    };
    assert_closed_error(error, task_id, [&diagnostic, &session]);
    Ok(())
}

async fn successful_driver_with_blocked_admin_close(
    harness: postgres_test_harness::PostgresHarness,
) -> ProbeResult {
    let diagnostic = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let session = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let held = session.acquire().await?;
    let observer = SessionObserver::new(session.clone(), BOUND)?;
    let mut run =
        FixtureSuite::new(harness).start(|_| Box::pin(async { Ok::<_, FixtureError>(()) }));
    assert!(tokio::time::timeout(BOUND, run.wait()).await?.is_ok());
    let error = ObservedRun::new(run, observer, session.clone(), diagnostic.clone())
        .finish(Duration::from_millis(40))
        .await
        .expect_err("session checkout blocks admin close after successful join");
    let summary = error.to_string();
    assert!(summary.contains("driver_joined=true; driver_failed=false"));
    assert!(!diagnostic.is_closed());
    let pending = *error.0.downcast::<ObservedRun<(), FixtureError>>().unwrap();
    drop(held);
    let report = pending.finish(BOUND).await?;
    assert!(report.is_ok());
    assert!(session.is_closed() && session.size() == 0);
    let query = sqlx::query("SELECT 1").execute(&diagnostic).await;
    diagnostic.close().await;
    query?;
    Ok(())
}

fn assert_closed_error(
    error: super::fixture_diagnostics::ProbeError,
    task_id: tokio::task::Id,
    pools: [&sqlx::PgPool; 2],
) {
    let joined = error
        .0
        .downcast::<tokio::task::JoinError>()
        .expect("original typed driver failure survives cleanup");
    assert!(joined.is_cancelled());
    assert_eq!(joined.id(), task_id);
    for pool in pools {
        assert!(pool.is_closed());
        assert_eq!(pool.size(), 0);
    }
}

async fn cancelled_run(
    harness: postgres_test_harness::PostgresHarness,
) -> Result<FixtureRun<(), FixtureError>, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let run = {
            let _entered = runtime.enter();
            FixtureSuite::new(harness).start(|_| Box::pin(async { Ok(()) }))
        };
        // Real runtime cancellation before first poll, with no acquired lease or
        // test-only driver hook. Admin pools belong to the surviving test runtime.
        drop(runtime);
        run
    })
    .await
}
