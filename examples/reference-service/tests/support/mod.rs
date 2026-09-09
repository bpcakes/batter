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
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = ProbeResult> + Send + 'static,
{
    let harness_result = harness().await;
    assert!(
        harness_result.is_ok(),
        "external harness prerequisite failed"
    );
    let harness = harness_result.unwrap_or_else(|_| unreachable!());
    let lease_result = harness.empty_database().await;
    assert!(lease_result.is_ok(), "database lease acquisition failed");
    let lease = lease_result.unwrap_or_else(|_| unreachable!());
    let connection = pool(lease.database_url()).await;
    let body_result = match connection {
        Ok(pool) => {
            // Retain pool and lease outside the body task, including on panic.
            let result = tokio::spawn(body(pool.clone())).await;
            pool.close().await;
            result
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
                .and_then(|result| result)
        }
        Err(error) => Err(Box::new(error) as Box<dyn std::error::Error + Send + Sync>),
    };
    let cleanup = lease.cleanup().await;
    let drained = harness.drain_deferred_cleanup().await;
    let shutdown = harness.shutdown().await;
    let cleanup = batter_test_support::finish(cleanup, drained);
    let cleanup = batter_test_support::finish(cleanup, shutdown);
    let result = batter_test_support::finish(body_result, cleanup);
    assert!(
        result.is_ok(),
        "body or awaited lease cleanup failed; causes retained"
    );
}
