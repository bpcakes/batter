//! Explicit PostgreSQL 18 compatibility probes. Run through the root live runner.

#[path = "support/mod.rs"]
mod support;

use support::{ProbeResult, with_database};

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn migrations_and_transactional_enqueue() {
    with_database(support::transactions::probe).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn initialized_schema_upgrade() {
    with_database(support::migrations::upgrade_probe).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn worker_startup_witness_and_shutdown() {
    with_database(support::worker::probe).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn lease_cleanup_defer_and_drop() {
    let result: ProbeResult = support::leases::probe().await;
    assert!(
        result.is_ok(),
        "lease ownership probe failed; error retained in result"
    );
}
