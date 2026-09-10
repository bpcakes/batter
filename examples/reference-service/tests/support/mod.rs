pub mod fixture_acquisition;
pub mod fixture_cleanup_failure;
pub mod fixture_diagnostics;
pub mod fixture_failures;
pub mod fixture_run;
pub mod fixtures;
pub mod leases;
pub mod migrations;
pub mod transactions;
pub mod worker;

use std::{future::Future, time::Duration};

use postgres_test_harness::{HarnessConfig, PostgresHarness};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub type ProbeResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

pub async fn harness() -> Result<PostgresHarness, Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("POSTGRES_TEST_ADMIN_URL")?;
    let config = HarnessConfig::new("batter_compat")?
        .with_admin_database_url(url)
        .with_cleanup_on_start(false)
        .with_connection_budget(16)?
        .with_connections_per_database(8)?
        .with_operation_timeout(Duration::from_secs(20))?;
    Ok(PostgresHarness::start(config).await?)
}

pub async fn pool(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
}

pub async fn with_database<F, Fut>(body: F)
where
    F: FnOnce(PgPool) -> Fut + Send + 'static,
    Fut: Future<Output = ProbeResult> + Send + 'static,
{
    let result = fixture_run::run(move |scope, _observer| {
        Box::pin(async move {
            let plan = batter_sqlx::test_support::ConnectionPlan::new(
                vec![
                    PgPoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(Duration::from_secs(10)),
                ],
                0,
            )?;
            let fixture = scope.empty(&plan).await?;
            body(fixture.pools()[0].clone()).await
        })
    })
    .await;
    fixture_diagnostics::assert_probe(result);
}
