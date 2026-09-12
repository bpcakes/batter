//! Explicit PostgreSQL 18 compatibility probes. Run through the root live runner.

#[path = "support/mod.rs"]
mod support;

use support::{ProbeResult, fixture_diagnostics::assert_probe, with_database};

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_preserves_history_and_disables_old_catalog() {
    with_database(support::retirement::history_and_disable).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_rejects_wrong_identity() {
    with_database(support::retirement::wrong_identity).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_rejects_hidden_sessions() {
    with_database(support::retirement::restricted_visibility).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_observes_late_enqueue() {
    with_database(support::retirement::pending_transaction).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL 18 with max_prepared_transactions greater than zero"]
async fn retirement_rejects_prepared_enqueue() {
    with_database(support::retirement::prepared_transaction).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_retains_commit_error_through_readback_cancellation() {
    with_database(support::retirement::commit_error_and_readback).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn retirement_retains_lost_commit_acknowledgement() {
    with_database(support::retirement::lost_commit_acknowledgement).await;
}

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
async fn isolated_durable_execution_and_shutdown() {
    with_database(support::worker::probe).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn native_initialization_without_queue_writes() {
    with_database(support::native_hosted::queue_independent_initialization).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn native_in_flight_finishes_after_drain() {
    with_database(support::native_hosted::in_flight_drain).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn native_owner_drop_retains_settlement() {
    with_database(support::native_hosted::owner_drop).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn native_business_failure_preserves_process() {
    with_database(support::native_hosted::business_failure).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn native_unjoined_callback_blocks_dependency_cleanup() {
    with_database(support::native_descendants::callback_outlives_wrapper).await;
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

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_detached_sessions_retained() {
    assert_probe(support::fixture_sessions::detached_sessions().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_session_observer_failure_resumes() {
    assert_probe(support::fixture_sessions::observer_failure().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_waiter_loss_keeps_cleanup_driven() {
    assert_probe(support::fixture_owner_loss::waiter_loss().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_runtime_loss_exposes_native_drop() {
    assert_probe(support::fixture_owner_loss::runtime_loss().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_distinct_deferred_and_consuming_failures() {
    assert_probe(support::fixture_cleanup_failure::deferred_and_consuming().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_handled_pool_error_retained() {
    assert_probe(support::fixture_failures::handled_pool_failure().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_assertion_and_script_failures_retained() {
    assert_probe(support::fixture_failures::assertion_and_script_failure().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_pending_completion_recovers() {
    assert_probe(support::fixture_retry::pending_completion_recovers().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_retry_during_active_attempts() {
    assert_probe(support::fixture_retry_gate::active_attempts().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_pending_body_keeps_diagnostics() {
    assert_probe(support::fixture_retry::pending_body_keeps_diagnostics().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn fixture_driver_error_closes_admin_pools() {
    assert_probe(support::fixture_completion_failure::driver_error_closes_admin_pools().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_wrong_server_retains_lease() {
    assert_probe(support::fixture_observation::wrong_server_retains_lease().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_shared_retry_and_close() {
    assert_probe(support::fixture_observation::shared_retry_and_close().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_replacement_before_first_attempt() {
    assert_probe(support::fixture_observation::replacement_before_first_attempt().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_premature_shared_close_recovers() {
    assert_probe(support::fixture_observation::premature_shared_close_recovers().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_restricted_observer_sees_other_role() {
    assert_probe(support::fixture_observer_role::restricted_observer_sees_other_role().await);
}

#[tokio::test]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_template_observation_recovers() {
    assert_probe(support::fixture_template_observer::template_recovery().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicitly selected disposable PostgreSQL 18 endpoints"]
async fn fixture_retry_during_active_attempts_multithread() {
    assert_probe(support::fixture_retry_gate::active_attempts().await);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18 with SCRAM and autovacuum_naptime <= 5s"]
async fn fixture_startup_is_not_a_connection_fence() {
    assert_probe(support::fixture_startup_session::startup_is_not_a_connection_fence().await);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18 with SCRAM and autovacuum_naptime <= 5s"]
async fn fixture_autovacuum_retains_lease() {
    assert_probe(support::fixture_background_session::autovacuum_retains_lease().await);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18 with superuser and SCRAM"]
async fn fixture_role_cleanup_after_assertion() {
    assert_probe(support::fixture_observer_role::role_cleanup_after_assertion().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn configured_pool_capacity_and_acquire_timeout() {
    assert_probe(support::configuration::one_slot_pool().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn configured_startup_pool_close_before_lease() {
    assert_probe(support::configuration::failed_startup_closes_pool_before_lease().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn configured_worker_concurrency() {
    support::with_database(support::configured_worker::probe).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn reference_delivery_command_and_reconciliation() {
    support::with_database(support::delivery::command_and_reconciliation).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn configured_command_root_bounds() {
    support::with_database(support::delivery::configured_root_bounds).await;
}

#[test]
fn child_fixture() {
    if let Some(stage) = support::startup_process::launch::scenario() {
        support::startup_process::child(&stage);
    }
}

#[tokio::test]
async fn startup_signal_during_pool_acquisition() {
    assert_probe(support::startup_signals::during_pool_acquisition().await);
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn startup_signal_during_schema_initialization() {
    with_database(support::startup_signals::during_database_initialization).await;
}

#[tokio::test]
#[ignore = "requires an explicitly selected disposable PostgreSQL 18 endpoint"]
async fn production_root_withholds_readiness_without_control_jobs() {
    with_database(support::production_readiness).await;
}
