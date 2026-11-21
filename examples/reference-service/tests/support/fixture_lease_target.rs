//! An admin URL whose `dbname` parameter would retarget every lease is
//! refused on every fixture path before the harness provisions anything.
use std::{
    error::Error,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use batter::sqlx::test_support::{
    ConnectionPlan, FixtureError, FixtureSuite, HarnessEndpointReason, PlannedPool, template_spec,
};
use postgres_test_harness::{HarnessConfig, PostgresHarness, cleanup_stale_databases};
use sqlx::{PgPool, postgres::PgPoolOptions};

use super::ProbeResult;

type BoxError = Box<dyn Error + Send + Sync>;

// Harness project names allow at most 16 ASCII characters.
const PROJECT: &str = "batter_retarget";

/// A harness whose admin URL also names the admin database in a `dbname`
/// query parameter. Harness administration honours that parameter, but the
/// harness keeps the query in every lease URL, where it would override the
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

fn is_database_parameter(error: &FixtureError) -> bool {
    matches!(error, FixtureError::Endpoint(refusal)
        if refusal.reason() == HarnessEndpointReason::DatabaseParameter)
}

pub async fn database_parameter_refused() -> ProbeResult {
    let (harness, admin) = retargeting_harness().await?;
    // Reclaim only tagged, inactive residue of this probe's own project.
    cleanup_stale_databases(&admin, PROJECT, Duration::ZERO).await?;
    let observer = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin)
        .await?;
    let connections = Arc::new(AtomicUsize::new(0));
    // Both paths return what they saw; assertions wait until every lease,
    // template, observer and the harness itself have been released.
    let owned = owned_refusals(harness.clone(), &connections).await;
    let low_level = low_level_refusals(harness.clone(), &connections).await;
    let residue = inventory(&observer).await;
    observer.close().await;
    let shutdown = harness.shutdown().await;
    let owned = owned?;
    let low_level = low_level?;
    shutdown?;
    assert_eq!(
        owned.refusals,
        [true, true],
        "owned acquisition and template"
    );
    assert!(owned.body_and_drain_ok);
    assert_eq!(owned.acquisitions, 0, "a refused call starts no producer");
    assert_eq!(owned.databases, 0, "nothing was provisioned");
    assert_eq!(
        low_level,
        [true, true],
        "low-level acquisition and template"
    );
    // An eager pool connects as soon as it exists, so no path built one.
    assert_eq!(connections.load(Ordering::SeqCst), 0);
    assert_eq!(residue?, Vec::<String>::new());
    Ok(())
}

struct OwnedOutcome {
    refusals: Vec<bool>,
    body_and_drain_ok: bool,
    acquisitions: usize,
    databases: usize,
}

/// The owned run's acquisition and template calls; both must be refused
/// before a producer starts, so the report holds no database or acquisition.
async fn owned_refusals(
    harness: PostgresHarness,
    connections: &Arc<AtomicUsize>,
) -> Result<OwnedOutcome, BoxError> {
    let plan = counted_plan(connections)?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let body_seen = Arc::clone(&seen);
    let run = FixtureSuite::new(harness).start(move |scope| {
        Box::pin(async move {
            let acquired = scope.empty(&plan).await;
            let templated = scope
                .template(template_spec(&[], "retarget-owned-v1"), |_| async {
                    Err("initializer ran for a refused endpoint".into())
                })
                .await;
            let mut seen = body_seen.lock().expect("refusal slot");
            seen.push(
                acquired
                    .err()
                    .is_some_and(|error| is_database_parameter(&error)),
            );
            seen.push(
                templated
                    .err()
                    .is_some_and(|error| is_database_parameter(&error)),
            );
            Ok::<_, FixtureError>(())
        })
    });
    let report = tokio::time::timeout(Duration::from_secs(30), run.into_report()).await??;
    let refusals = seen.lock().expect("refusal slot").clone();
    Ok(OwnedOutcome {
        refusals,
        body_and_drain_ok: report.body.is_ok() && report.drain.is_ok(),
        acquisitions: report.acquisitions.len(),
        databases: report.databases.len(),
    })
}

/// The low-level acquisition and template calls. A regression that acquired
/// or initialized anyway still releases what it got before returning.
async fn low_level_refusals(
    harness: PostgresHarness,
    connections: &Arc<AtomicUsize>,
) -> Result<[bool; 2], BoxError> {
    let plan = counted_plan(connections)?;
    let mut suite = FixtureSuite::new(harness);
    let acquired = match suite.empty(&plan).await {
        Ok(fixture) => {
            fixture.finish(Ok::<(), FixtureError>(())).await?;
            false
        }
        Err(error) => is_database_parameter(&error),
    };
    let templated = suite
        .template(template_spec(&[], "retarget-low-level-v1"), |_| async {
            Err("initializer ran for a refused endpoint".into())
        })
        .await
        .err()
        .is_some_and(|error| is_database_parameter(&error));
    suite.drain().await?;
    Ok([acquired, templated])
}
