use std::time::Duration;

use batter_sqlx::test_support::{
    BodyFailure, CleanupPhase, ConnectionPlan, FixtureError, FixtureSuite, SessionObserver,
};
use sqlx::postgres::PgPoolOptions;

use super::{ProbeResult, fixture_completion::ObservedRun, fixture_run};

const BOUND: Duration = Duration::from_secs(5);

pub async fn pending_completion_recovers() -> ProbeResult {
    let harness = super::harness().await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    let broken = PgPoolOptions::new()
        .max_connections(1)
        .connect(harness.admin_database_url())
        .await?;
    broken.close().await;
    let observer = SessionObserver::new(broken.clone(), BOUND)?;
    let run = FixtureSuite::new(harness)
        .with_session_observer(observer.clone())
        .start(|scope| Box::pin(failing_body(scope)));
    super::fixture_sessions::phase(&run, CleanupPhase::AwaitingRetry).await;
    let failure = run.cleanup_progress()[0].observation_failures[0].clone();
    let error = ObservedRun::new(run, observer, broken, pool.clone())
        .finish(Duration::from_millis(30))
        .await
        .expect_err("bounded pending failure");
    assert!(error.to_string().contains("AwaitingRetry=1"));
    assert!(
        error
            .to_string()
            .contains("driver_joined=false; driver_failed=false")
    );
    assert!(error.to_string().contains("observation_failures=1"));
    let mut pending = *error
        .0
        .downcast::<ObservedRun<(), FixtureError>>()
        .expect("pending error retains the actual run and retry control");
    let name = pending.run.cleanup_progress()[0].database_name.clone();
    assert!(!pending.to_string().contains(&name));
    assert!(!pending.to_string().contains("division by zero"));
    let retained = fixture_run::present(&pool, &name).await;
    let replacement = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await?;
    pending.retry_with(replacement.clone());
    let report = pending.finish(BOUND).await?;
    assert!(replacement.is_closed());
    let absent = fixture_run::absent(&pool, &name).await;
    pool.close().await;
    absent?;
    assert_recovered(&report, &failure, retained?);
    Ok(())
}

async fn failing_body(
    scope: &mut batter_sqlx::test_support::FixtureScope,
) -> Result<(), FixtureError> {
    let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
    let db = scope.empty(&plan).await?;
    sqlx::query("SELECT 1 / 0")
        .execute(&db.pools()[0])
        .await
        .map_err(FixtureError::Observe)?;
    Ok(())
}

fn assert_recovered(
    report: &batter_sqlx::test_support::FixtureReport<(), FixtureError>,
    failure: &std::sync::Arc<FixtureError>,
    retained: bool,
) {
    assert!(retained && !report.is_ok());
    assert!(std::sync::Arc::ptr_eq(
        failure,
        &report.databases[0].observation_failures[0]
    ));
    assert!(
        matches!(report.body, Err(BodyFailure::Returned(FixtureError::Observe(sqlx::Error::Database(ref e)))) if e.code().as_deref() == Some("22012"))
    );
}

pub async fn pending_body_keeps_diagnostics() -> ProbeResult {
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let waiting = tokio::spawn(fixture_run::run_with_bound(
        Duration::from_secs(1),
        move |scope, diagnostics| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.empty(&plan).await?;
                let held = diagnostics.acquire().await?;
                entered.send(()).expect("test observer alive");
                released.await?;
                drop(held);
                // The pending return must not close the still-running body's pool.
                sqlx::query("SELECT 1").execute(&diagnostics).await?;
                Ok(())
            })
        },
    ));
    tokio::time::timeout(BOUND, started).await??;
    let error = tokio::time::timeout(BOUND, waiting)
        .await??
        .expect_err("body is pending");
    let error = error
        .downcast::<fixture_run::ProbeError>()
        .expect("redacted pending error");
    let pending = *error
        .0
        .downcast::<ObservedRun<(), fixture_run::ProbeError>>()
        .expect("outer helper returns the original owner");
    let diagnostics = pending.diagnostic_pool.clone();
    assert!(!diagnostics.is_closed());
    release.send(()).expect("body still alive");
    let report = pending.finish(BOUND).await?;
    let absent = fixture_run::absent(&diagnostics, &report.databases[0].database_name).await;
    diagnostics.close().await;
    absent?;
    assert!(report.is_ok());
    Ok(())
}
