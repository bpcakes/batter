//! A lease URL that selects another database is refused before any pool
//! exists, and both fixture paths retain that refusal.
use std::{
    error::Error,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter::sqlx::test_support::{
    ConnectionPlan, FixtureError, FixtureReport, FixtureSuite, LeaseDatabaseMismatch, PlannedPool,
};
use postgres_test_harness::{HarnessConfig, PostgresHarness, cleanup_stale_databases};
use sqlx::{PgPool, postgres::PgPoolOptions};

use super::{ProbeResult, fixture_run};

type BoxError = Box<dyn Error + Send + Sync>;

// Harness project names allow at most 16 ASCII characters.
const PROJECT: &str = "batter_retarget";

/// A harness whose admin URL also names the admin database in a `dbname`
/// query parameter. Harness administration honours that parameter, but the
/// harness keeps the query in every lease URL, where SQLx applies it over the
/// lease path. Returns the harness and the unmodified admin URL.
async fn retargeting_harness() -> Result<(PostgresHarness, String), BoxError> {
    let (url, options) = super::live_endpoint::from_process()?;
    let admin = url.expose_secret().to_owned();
    let database = options
        .get_database()
        .ok_or("the live endpoint names its administrative database")?;
    if !database
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("this probe needs an unquoted administrative database name".into());
    }
    let separator = if admin.contains('?') { '&' } else { '?' };
    let retargeted = format!("{admin}{separator}dbname={database}");
    let harness = PostgresHarness::start(
        HarnessConfig::new(PROJECT)?
            .with_admin_database_url(retargeted)
            .with_cleanup_on_start(false)
            .with_connection_budget(8)?
            .with_connections_per_database(4)?
            .with_operation_timeout(Duration::from_secs(20))?,
    )
    .await?;
    Ok((harness, admin))
}

/// Eager, so a constructed pool connects on its own without any check.
fn counted_plan(connections: &Arc<AtomicUsize>) -> Result<ConnectionPlan, FixtureError> {
    let counter = Arc::clone(connections);
    let mut plan = ConnectionPlan::declare(0);
    plan.pool(PlannedPool::lease_login(
        PgPoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .acquire_timeout(Duration::from_secs(10))
            .after_connect(move |_, _| {
                counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(()) })
            }),
    ));
    plan.finish()
}

async fn inventory(observer: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT datname::text FROM pg_database WHERE starts_with(datname::text, $1) ORDER BY 1",
    )
    .bind(format!("pgh_{PROJECT}_"))
    .fetch_all(observer)
    .await
}

fn is_mismatch(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Configuration(source) if source.is::<LeaseDatabaseMismatch>())
}

pub async fn mismatch_retained() -> ProbeResult {
    let (harness, admin) = retargeting_harness().await?;
    // Reclaim only tagged, inactive residue of this probe's own project.
    cleanup_stale_databases(&admin, PROJECT, Duration::ZERO).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin)
        .await?;
    let owned_connections = Arc::new(AtomicUsize::new(0));
    let owned = owned_refusal(harness.clone(), &owned_connections).await;
    let owned_present = match &owned {
        Ok((report, _)) => match report.databases.first() {
            Some(database) => Some(fixture_run::present(&observer, &database.database_name).await),
            None => None,
        },
        Err(_) => None,
    };
    let low_level_connections = Arc::new(AtomicUsize::new(0));
    let low_level = low_level_refusal(harness.clone(), &low_level_connections).await;
    let residue = inventory(&observer).await;
    observer.close().await;
    let shutdown = harness.shutdown().await;
    let (report, delivered) = owned?;
    let (result, drained) = low_level?;
    shutdown?;
    assert_owned(&report, &delivered);
    assert!(
        !owned_present.ok_or("owned report names its database")??,
        "the refused owned lease must be cleaned"
    );
    assert!(
        matches!(&result, Err(FixtureError::Connect { source, cleanup: None }) if is_mismatch(source)),
        "low-level acquisition must report the refusal after awaited cleanup"
    );
    drained?;
    // An eager pool connects as soon as it exists, so neither path built one.
    assert_eq!(owned_connections.load(Ordering::SeqCst), 0);
    assert_eq!(low_level_connections.load(Ordering::SeqCst), 0);
    assert_eq!(residue?, Vec::<String>::new());
    Ok(())
}

type OwnedOutcome = (FixtureReport<(), FixtureError>, Arc<sqlx::Error>);

/// The body handles the delivered refusal; the report must still retain it.
async fn owned_refusal(
    harness: PostgresHarness,
    connections: &Arc<AtomicUsize>,
) -> Result<OwnedOutcome, BoxError> {
    let plan = counted_plan(connections)?;
    let delivered = Arc::new(Mutex::new(None));
    let body_delivered = Arc::clone(&delivered);
    let run = FixtureSuite::new(harness).start(move |scope| {
        Box::pin(async move {
            let Err(FixtureError::PoolAcquire(error)) = scope.empty(&plan).await else {
                panic!("a lease URL selecting another database must be refused");
            };
            *body_delivered.lock().expect("delivery slot") = Some(error);
            Ok::<_, FixtureError>(())
        })
    });
    let report = tokio::time::timeout(Duration::from_secs(30), run.into_report()).await??;
    let delivered = delivered
        .lock()
        .expect("delivery slot")
        .take()
        .ok_or("the body must observe the refusal")?;
    Ok((report, delivered))
}

fn assert_owned(report: &FixtureReport<(), FixtureError>, delivered: &Arc<sqlx::Error>) {
    assert!(is_mismatch(delivered));
    assert!(report.body.is_ok() && !report.is_ok());
    assert!(report.acquisitions.iter().all(Result::is_ok) && report.drain.is_ok());
    assert_eq!(report.databases.len(), 1);
    let database = &report.databases[0];
    assert!(database.result.is_ok() && database.observation_failures.is_empty());
    assert_eq!(database.pool_failures.len(), 1);
    assert!(Arc::ptr_eq(delivered, &database.pool_failures[0]));
    assert!(
        report
            .to_string()
            .contains("database_failures=0, pool_failures=1")
    );
}

type LowLevelOutcome = (
    Result<batter::sqlx::test_support::DatabaseFixture, FixtureError>,
    Result<(), FixtureError>,
);

async fn low_level_refusal(
    harness: PostgresHarness,
    connections: &Arc<AtomicUsize>,
) -> Result<LowLevelOutcome, BoxError> {
    let plan = counted_plan(connections)?;
    let suite = FixtureSuite::new(harness);
    let result = suite.empty(&plan).await;
    let result = match result {
        // A regression that acquired anyway still releases its lease here.
        Ok(fixture) => {
            fixture.finish(Ok::<(), FixtureError>(())).await?;
            return Err("low-level acquisition must refuse the retargeted lease".into());
        }
        Err(error) => Err(error),
    };
    let drained = suite.drain().await;
    Ok((result, drained))
}
