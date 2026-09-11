use std::time::Duration;

use batter_sqlx::test_support::{
    CleanupPhase, ConnectionPlan, FixtureError, FixtureRun, FixtureSuite, SessionObserver,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;

use super::{ProbeResult, fixture_completion::ObservedRun, fixture_run, fixture_sessions::phase};

pub const BOUND: Duration = Duration::from_secs(10);
pub type Run = FixtureRun<(), FixtureError>;

pub async fn admin() -> Result<PgPool, Box<dyn std::error::Error + Send + Sync>> {
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("POSTGRES_TEST_ADMIN_URL")?)
        .await?)
}

pub async fn empty(
    observer: SessionObserver,
) -> Result<Run, Box<dyn std::error::Error + Send + Sync>> {
    Ok(FixtureSuite::new(super::harness().await?)
        .with_session_observer(observer)
        .start(|scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.empty(&plan).await?;
                Ok(())
            })
        }))
}

pub async fn wrong_server_retains_lease() -> ProbeResult {
    let catalog = admin().await?;
    let wrong = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("POSTGRES_TEST_OBSERVER_URL")?)
        .await?;
    let control = SessionObserver::new(wrong.clone(), BOUND)?;
    let mut run = empty(control.clone()).await?;
    phase(&run, CleanupPhase::AwaitingRetry).await;
    let before = run.cleanup_progress();
    let name = &before[0].database_name;
    let real_exists = fixture_run::present(&catalog, name).await?;
    let wrong_exists = fixture_run::present(&wrong, name).await?;
    let pending = run
        .wait_for(Duration::from_millis(30))
        .await
        .unwrap()
        .is_none();
    control.retry_with(catalog.clone());
    let report = tokio::time::timeout(BOUND, run.into_report()).await??;
    let absent = fixture_run::absent(&catalog, name).await;
    tokio::join!(catalog.close(), wrong.close());
    absent?;
    assert!(real_exists && !wrong_exists && pending);
    assert!(matches!(
        *before[0].observation_failures[0],
        FixtureError::ObservationTarget
    ));
    assert!(!report.is_ok() && report.body.is_ok() && report.drain.is_ok());
    assert!(report.databases[0].result.is_ok());
    assert!(std::sync::Arc::ptr_eq(
        &before[0].observation_failures[0],
        &report.databases[0].observation_failures[0]
    ));
    Ok(())
}

pub async fn shared_retry_and_close() -> ProbeResult {
    let catalog = admin().await?;
    let shared = admin().await?;
    shared.close().await;
    let control = SessionObserver::new(shared.clone(), BOUND)?;
    let left = empty(control.clone()).await?;
    let right = empty(control.clone()).await?;
    phase(&left, CleanupPhase::AwaitingRetry).await;
    phase(&right, CleanupPhase::AwaitingRetry).await;
    let pool = admin().await?;
    control.retry_with(pool.clone()); // One request must wake both independent runs.
    let (mut left, mut right) = (left, right);
    let (a, b) = tokio::join!(left.wait_for(BOUND), right.wait_for(BOUND));
    assert!(a.unwrap().is_some() && b.unwrap().is_some());
    // Both drivers finish before either consumer closes their shared pool.
    let a = ObservedRun::new(left, control.clone(), pool.clone(), catalog.clone())
        .finish(BOUND)
        .await?;
    let b = ObservedRun::new(right, control, pool.clone(), catalog.clone())
        .finish(BOUND)
        .await?;
    assert!(pool.is_closed());
    for report in [a, b] {
        fixture_run::absent(&catalog, &report.databases[0].database_name).await?;
        assert!(!report.is_ok() && report.databases[0].result.is_ok());
        assert_eq!(report.databases[0].observation_failures.len(), 1);
    }
    catalog.close().await;
    Ok(())
}

pub async fn replacement_before_first_attempt() -> ProbeResult {
    let original = admin().await?;
    original.close().await;
    let catalog = admin().await?;
    let replacement = admin().await?;
    let control = SessionObserver::new(original.clone(), BOUND)?;
    let (ready, entered) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let run = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.empty(&plan).await?;
                ready.send(()).unwrap();
                gate.await.unwrap();
                Ok::<_, FixtureError>(())
            })
        });
    tokio::time::timeout(BOUND, entered).await??;
    assert!(run.cleanup_progress().is_empty());
    control.retry_with(original); // Unread updates before cleanup coalesce.
    control.retry_with(replacement.clone());
    release.send(()).unwrap();
    let report = ObservedRun::new(run, control, replacement, catalog.clone())
        .finish(BOUND)
        .await?;
    let absent = fixture_run::absent(&catalog, &report.databases[0].database_name).await;
    catalog.close().await;
    absent?;
    assert!(report.is_ok() && report.databases[0].observation_failures.is_empty());
    Ok(())
}

pub async fn premature_shared_close_recovers() -> ProbeResult {
    let catalog = admin().await?;
    let pool = admin().await?;
    let control = SessionObserver::new(pool.clone(), BOUND)?;
    let first = empty(control.clone()).await?;
    let (ready, entered) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let second = FixtureSuite::new(super::harness().await?)
        .with_session_observer(control.clone())
        .start(move |scope| {
            Box::pin(async move {
                let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
                scope.empty(&plan).await?;
                ready.send(()).unwrap();
                gate.await.unwrap();
                Ok::<_, FixtureError>(())
            })
        });
    tokio::time::timeout(BOUND, entered).await??;
    let first = ObservedRun::new(first, control.clone(), pool.clone(), catalog.clone())
        .finish(BOUND)
        .await?;
    assert!(first.is_ok() && pool.is_closed());
    release.send(()).unwrap();
    phase(&second, CleanupPhase::AwaitingRetry).await;
    let before = second.cleanup_progress();
    let retained = fixture_run::present(&catalog, &before[0].database_name).await?;
    let mut second = ObservedRun::new(second, control, pool, catalog.clone());
    second.retry_with(admin().await?);
    let report = second.finish(BOUND).await?;
    fixture_run::absent(&catalog, &report.databases[0].database_name).await?;
    fixture_run::absent(&catalog, &first.databases[0].database_name).await?;
    catalog.close().await;
    assert!(retained && !report.is_ok() && report.databases[0].result.is_ok());
    assert!(matches!(
        *before[0].observation_failures[0],
        FixtureError::Observe(sqlx::Error::PoolClosed)
    ));
    Ok(())
}
