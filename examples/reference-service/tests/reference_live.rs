//! Explicit PostgreSQL 18 compatibility probes. Run through the root live runner.

#[path = "support/mod.rs"]
mod support;

use support::{ProbeResult, fixture_diagnostics::assert_probe, with_database};

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
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_template_reuse_and_isolation() {
    let result = support::fixtures::templates().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_one_slot_lock_operation() {
    let result = support::fixtures::lock_operation().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_finish_preserves_body_failure() {
    let result = support::fixtures::returned_body_failure().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_partial_acquisition_and_panic() {
    let result = support::fixture_failures::partial_acquisition_and_panic().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_close_order_and_resumable_wait() {
    let result = support::fixture_failures::close_order_and_resumable_wait().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_foreign_template_rejected() {
    let result = support::fixture_failures::foreign_template_rejected().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_body_and_cleanup_failures_retained() {
    let result = support::fixture_cleanup_failure::probe().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_observer_failure_preserves_body() {
    let result = support::fixture_failures::observer_failure_preserves_body().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_cancelled_creation_retains_producers() {
    let result = support::fixture_acquisition::cancelled_creation().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_abandoned_producer_failure_retained() {
    let result = support::fixture_acquisition::abandoned_failure().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_shared_harness_admission() {
    let result = support::fixture_acquisition::shared_admission().await;
    assert_probe(result);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_pool_error_is_pending_cleanup() {
    let result = support::fixture_failures::pending_pool_error().await;
    assert_probe(result);
}
