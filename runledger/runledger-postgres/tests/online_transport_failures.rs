use std::time::Duration;

use runledger_postgres::{
    MigrationTimeoutSetting, OnlineIndexPhase, PgSessionProfile, RunledgerDatabase,
    SchemaCompatibilityError, migrate_after_idempotency_cutover,
    prepare_online_indexes_for_external_migrator,
};

mod online_support;
#[path = "online_transport_failures/wire.rs"]
mod online_wire;
mod support;

use online_support::{
    Harness, LAST_VERSION_BEFORE_SUMMARY_PAGINATION, SUMMARY_PAGINATION_VERSION,
    build_waits_for_writers, hold_writer_lock, index_catalog, raw_apply_through, sqlx_history,
    wait_for,
};
use online_wire::{Fault, Relay};

fn relayed_database(harness: &Harness, relay: &Relay, bound: Duration) -> RunledgerDatabase {
    let options = harness.pool.connect_options();
    let login = options.get_username();
    let profile = PgSessionProfile::new(login, login, vec!["public".into()], Duration::ZERO, bound)
        .expect("profile");
    // The eager constructor probes and closes a separate connection first;
    // this one-session relay targets the lazy pool's actual migration session.
    RunledgerDatabase::connect_lazy(
        relay.options.clone(),
        profile,
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
    )
    .expect("relayed database")
}

async fn poll_response_expires(prepare: bool, deliver_late: bool) {
    let harness = Harness::fresh("online_poll_response").await;
    let mut relay = Relay::start(&harness.pool.connect_options(), Fault::HoldPollResponse).await;
    let bound = Duration::from_millis(250);
    let database = relayed_database(&harness, &relay, bound);
    let run = async {
        if prepare {
            prepare_online_indexes_for_external_migrator(&database)
                .await
                .map(|_| ())
        } else {
            migrate_after_idempotency_cutover(&database).await
        }
    };
    let mut run = std::pin::pin!(run);
    tokio::select! {
        _ = relay.reached() => {}
        result = &mut run => panic!("runner must await the held response: {result:?}"),
    }
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks
         WHERE locktype = 'advisory' AND granted
         AND database = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()))"
    ).fetch_one(&harness.pool).await.expect("server acquired the lock"));
    if deliver_late {
        // Deliver the complete successful response, but do not poll the runner
        // until after expiry. The active runtime processes socket readiness.
        // Tokio's timeout polls the inner future first: the post-response check
        // must still refuse this already-ready success.
        relay.release_response().await;
        tokio::time::sleep(bound + Duration::from_millis(100)).await;
    }
    let error = tokio::time::timeout(Duration::from_secs(2), &mut run)
        .await
        .expect("local deadline expires without needing a response")
        .expect_err("response cannot grant admission after the deadline");
    let SchemaCompatibilityError::MigrationLockTimeout(expiry) = error else {
        panic!("expected typed local expiry: {error}");
    };
    assert_eq!(expiry.setting(), MigrationTimeoutSetting::Lock);
    assert_eq!(expiry.bound(), bound);
    database.pool().close().await;
    relay.finish().await;
    harness.teardown().await;
}

#[tokio::test]
async fn both_entrypoints_expire_with_a_poll_response_still_unavailable() {
    for prepare in [false, true] {
        poll_response_expires(prepare, false).await;
    }
}

#[tokio::test]
async fn both_entrypoints_reject_an_already_ready_success_after_expiry() {
    for prepare in [false, true] {
        poll_response_expires(prepare, true).await;
    }
}

async fn restore_disconnect(cancel_build: bool) {
    let harness = Harness::fresh("online_restore_disconnect").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = if cancel_build {
        Some(hold_writer_lock(&harness.pool).await)
    } else {
        None
    };
    let mut relay = Relay::start(&harness.pool.connect_options(), Fault::DisconnectOnRestore).await;
    let database = relayed_database(&harness, &relay, Duration::ZERO);
    let runner = tokio::spawn(async move {
        let result = migrate_after_idempotency_cutover(&database).await;
        database.pool().close().await;
        result
    });
    if cancel_build {
        cancel_waiting_build(&harness.pool).await;
    }
    relay.reached().await;
    let error = tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .expect("runner settles after disconnect")
        .expect("runner task")
        .expect_err("restore failed");
    assert_failure_context(error, cancel_build);
    assert_eq!(
        index_catalog(&harness.pool).await["idx_job_queue_scope_created_id"].1,
        !cancel_build
    );
    assert!(
        sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|row| row.0 == SUMMARY_PAGINATION_VERSION && row.2)
    );
    relay.finish().await;
    if let Some(holder) = holder {
        holder.rollback().await.expect("release writer");
    }
    support::migrate_profiled(&harness.pool)
        .await
        .expect("recover after failed restoration");
    support::verify_profiled(&harness.pool)
        .await
        .expect("recovered schema");
    harness.teardown().await;
}

async fn cancel_waiting_build(pool: &sqlx::PgPool) {
    wait_for("blocked build before cancellation", || {
        build_waits_for_writers(pool)
    })
    .await;
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT pg_catalog.pg_cancel_backend(pid) FROM pg_catalog.pg_stat_progress_create_index
         WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
           AND relid = 'job_queue'::regclass"
        )
        .fetch_one(pool)
        .await
        .expect("cancel actual build")
    );
}

fn assert_failure_context(error: SchemaCompatibilityError, cancel_build: bool) {
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains("canceling statement"));
    assert!(!rendered.contains("unexpected end"));
    let SchemaCompatibilityError::MigrationCleanup(cleanup) = error else {
        panic!("disconnected session must retain the unlock failure: {error}");
    };
    assert!(matches!(
        cleanup.unlock_error(),
        sqlx::migrate::MigrateError::Execute(_)
    ));
    let SchemaCompatibilityError::OnlineIndexExecution(failure) = cleanup.operation_error() else {
        panic!("retained execution context required");
    };
    assert_eq!(failure.index(), "idx_job_queue_scope_created_id");
    assert_eq!(failure.recorded_version(), Some(SUMMARY_PAGINATION_VERSION));
    let SchemaCompatibilityError::Query(cause) = failure.cause() else {
        panic!("native cause")
    };
    if cancel_build {
        assert_eq!(failure.phase(), OnlineIndexPhase::Build);
        assert_eq!(
            cause
                .as_database_error()
                .expect("cancellation cause")
                .code()
                .as_deref(),
            Some("57014")
        );
        assert!(matches!(
            failure.restoration_error(),
            Some(sqlx::Error::Io(_))
        ));
    } else {
        assert_eq!(failure.phase(), OnlineIndexPhase::Restore);
        assert!(matches!(cause, sqlx::Error::Io(_)));
        assert!(failure.restoration_error().is_none());
    }
}

#[tokio::test]
async fn successful_build_retains_a_restoration_only_failure() {
    restore_disconnect(false).await;
}

#[tokio::test]
async fn failed_build_retains_its_cancellation_and_restoration_failure() {
    restore_disconnect(true).await;
}
