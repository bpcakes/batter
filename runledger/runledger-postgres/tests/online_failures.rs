use std::time::Duration;

use runledger_postgres::{
    MigrationTimeoutSetting, OnlineIndexPhase, OnlineIndexProblem, PgSessionProfile,
    RunledgerDatabase, SchemaCompatibilityError,
    ensure_schema_compatible_after_idempotency_cutover,
    prepare_online_indexes_for_external_migrator,
};
use sqlx::migrate::Migrate;
use sqlx::{AssertSqlSafe, Connection};

mod online_support;
mod support;

use online_support::{
    Harness, LAST_VERSION_BEFORE_SUMMARY_PAGINATION, SUMMARY_PAGINATION_VERSION,
    assert_declared_indexes_valid, assert_lock_wait_expires, build_waits_for_writers,
    hold_writer_lock, index_catalog, migrate_with_profile_timeouts, migrate_with_short_timeouts,
    migrate_with_transaction_timeout, raw_apply_through, sqlx_history, wait_for,
};
use support::{migrate_profiled, verify_profiled};

async fn waiting_builder(pool: &sqlx::PgPool) -> i32 {
    wait_for("concurrent build waiting on the writer", || {
        build_waits_for_writers(pool)
    })
    .await;
    sqlx::query_scalar(
        "SELECT pid FROM pg_catalog.pg_stat_progress_create_index
         WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
           AND relid = 'job_queue'::regclass AND phase LIKE 'waiting for%'",
    )
    .fetch_one(pool)
    .await
    .expect("builder pid")
}

async fn wait_for_backend_exit(pool: &sqlx::PgPool, pid: i32) {
    tokio::time::timeout(
        Duration::from_secs(10),
        wait_for("backend and locks gone", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid = $1)
                AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE pid = $1)",
            )
            .bind(pid)
            .fetch_one(pool)
            .await
            .expect("independent backend observation")
        }),
    )
    .await
    .expect("disconnect check must release the blocked backend");
}

#[tokio::test]
async fn cancelling_the_runner_releases_the_backend_and_lock_before_the_writer() {
    let harness = Harness::fresh("online_disconnect").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    let pid = waiting_builder(&harness.pool).await;
    runner.abort();
    assert!(runner.await.expect_err("runner cancelled").is_cancelled());
    // The writer remains held: termination must follow disconnect detection,
    // not eventual completion of the concurrent build or a profile timeout.
    wait_for_backend_exit(&harness.pool, pid).await;
    assert!(!index_catalog(&harness.pool).await["idx_job_queue_scope_created_id"].1);
    assert!(
        sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|row| row.0 == SUMMARY_PAGINATION_VERSION && row.2)
    );
    holder
        .commit()
        .await
        .expect("release writer after observed termination");
    migrate_profiled(&harness.pool)
        .await
        .expect("next startup repairs interruption");
    assert_declared_indexes_valid(&harness.pool, "repaired after disconnect").await;
    verify_profiled(&harness.pool)
        .await
        .expect("repaired schema");
    harness.teardown().await;
}

#[tokio::test]
async fn a_failed_build_retains_the_acknowledged_migration_and_native_cause() {
    let harness = Harness::fresh("online_build_error").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    let pid = waiting_builder(&harness.pool).await;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_cancel_backend($1)")
            .bind(pid)
            .fetch_one(&harness.pool)
            .await
            .expect("cancel build")
    );
    let error = tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .expect("runner returns after server cancellation")
        .expect("runner task")
        .expect_err("cancelled build");
    let SchemaCompatibilityError::OnlineIndexExecution(failure) = error else {
        panic!("missing build context: {error}");
    };
    assert_eq!(failure.index(), "idx_job_queue_scope_created_id");
    assert_eq!(failure.recorded_version(), Some(SUMMARY_PAGINATION_VERSION));
    assert_eq!(failure.phase(), OnlineIndexPhase::Build);
    assert!(failure.restoration_error().is_none());
    let SchemaCompatibilityError::Query(native) = failure.cause() else {
        panic!("native cause");
    };
    assert_eq!(
        native
            .as_database_error()
            .expect("server cancellation")
            .code()
            .as_deref(),
        Some("57014")
    );
    assert!(
        sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|row| row.0 == SUMMARY_PAGINATION_VERSION && row.2)
    );
    holder.commit().await.expect("release writer");
    migrate_profiled(&harness.pool)
        .await
        .expect("repair failed build");
    harness.teardown().await;
}

#[tokio::test]
async fn restricted_verifier_recognizes_hidden_build_progress_and_settled_interruption() {
    let harness = Harness::fresh("online_restricted_progress").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let role = format!("online_reader_{}", sqlx::types::Uuid::new_v4().simple());
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE ROLE {role}; GRANT USAGE ON SCHEMA public TO {role};
         GRANT SELECT ON ALL TABLES IN SCHEMA public TO {role}"
    )))
    .execute(&harness.pool)
    .await
    .expect("restricted verification role");
    let options = harness.pool.connect_options();
    let profile = PgSessionProfile::new(
        options.get_username(),
        &role,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .expect("restricted profile");
    let verifier = RunledgerDatabase::connect((*options).clone(), profile, 1)
        .await
        .expect("verifier");
    let holder = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    let pid = waiting_builder(&harness.pool).await;
    let hidden: bool = sqlx::query_scalar(
        "SELECT relid IS NULL AND index_relid IS NULL AND phase IS NULL
         FROM pg_catalog.pg_stat_progress_create_index WHERE pid = $1",
    )
    .bind(pid)
    .fetch_one(verifier.pool())
    .await
    .expect("progress row without privileged details");
    assert!(
        hidden,
        "this regression must exercise the hidden-details path"
    );
    let error = ensure_schema_compatible_after_idempotency_cutover(&verifier)
        .await
        .expect_err("INVALID during an active build is transient");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_scope_created_id",
                problem: OnlineIndexProblem::BuildInProgress,
            }
        ),
        "unexpected refusal: {error}"
    );
    runner.abort();
    assert!(
        runner
            .await
            .expect_err("builder interrupted")
            .is_cancelled()
    );
    wait_for_backend_exit(&harness.pool, pid).await;
    assert!(!index_catalog(&harness.pool).await["idx_job_queue_scope_created_id"].1);
    let interrupted = ensure_schema_compatible_after_idempotency_cutover(&verifier)
        .await
        .expect_err("settled interruption is INVALID rather than still building");
    assert!(
        matches!(
            interrupted,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_scope_created_id",
                problem: OnlineIndexProblem::Invalid,
            }
        ),
        "unexpected interrupted state: {interrupted}"
    );
    holder
        .commit()
        .await
        .expect("release writer after settled interruption");
    migrate_profiled(&harness.pool)
        .await
        .expect("repair interrupted index");
    ensure_schema_compatible_after_idempotency_cutover(&verifier)
        .await
        .expect("valid after completion");
    verifier.pool().close().await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP OWNED BY {role}; DROP ROLE {role}"
    )))
    .execute(&harness.pool)
    .await
    .expect("remove restricted role");
    harness.teardown().await;
}

#[tokio::test]
async fn verification_uses_the_migration_lock_for_invalid_indexes_without_build_progress() {
    let harness = Harness::fresh("online_invalid_under_lock").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    let writer = hold_writer_lock(&harness.pool).await;
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_profiled(&runner_pool).await });
    let pid = waiting_builder(&harness.pool).await;
    runner.abort();
    assert!(runner.await.expect_err("runner cancelled").is_cancelled());
    wait_for_backend_exit(&harness.pool, pid).await;
    writer.commit().await.expect("release writer");

    let catalog = index_catalog(&harness.pool).await;
    assert!(!catalog["idx_job_queue_scope_created_id"].1);
    let history = sqlx_history(&harness.pool).await;
    let identity_sql = "SELECT 'public.idx_job_queue_scope_created_id'::regclass::oid::bigint";
    let oid: i64 = sqlx::query_scalar(identity_sql)
        .fetch_one(&harness.pool)
        .await
        .expect("interrupted index identity");
    // No visible or hidden CREATE progress can satisfy the classifier here.
    // This isolates the advisory fallback used, for example, while protected
    // repair inspects and transactionally drops the INVALID index before its
    // replacement build, or during a cooperating manual DROP INDEX CONCURRENTLY
    // under the migration lock.
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT NOT EXISTS (
                SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
                WHERE datid = (
                    SELECT oid FROM pg_catalog.pg_database
                    WHERE datname = pg_catalog.current_database()
                )
             )",
        )
        .fetch_one(&harness.pool)
        .await
        .expect("no build progress in this database")
    );
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("migration lock connection");
    holder.lock().await.expect("SQLx migration lock");
    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("INVALID under a migration lock is transient without build progress");
    assert!(matches!(
        error,
        SchemaCompatibilityError::OnlineIndex {
            index: "idx_job_queue_scope_created_id",
            problem: OnlineIndexProblem::BuildInProgress,
        }
    ));
    holder.unlock().await.expect("release migration lock");
    holder.close().await.expect("close lock connection");
    let error = verify_profiled(&harness.pool)
        .await
        .expect_err("without the lock the interrupted index is persistent");
    assert!(matches!(
        error,
        SchemaCompatibilityError::OnlineIndex {
            index: "idx_job_queue_scope_created_id",
            problem: OnlineIndexProblem::Invalid,
        }
    ));
    assert_eq!(index_catalog(&harness.pool).await, catalog);
    assert_eq!(sqlx_history(&harness.pool).await, history);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(identity_sql)
            .fetch_one(&harness.pool)
            .await
            .expect("index identity after verification"),
        oid,
        "verification must not replace the INVALID index"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn verifier_refuses_a_valid_wrong_definition_without_changing_history_or_catalog() {
    let harness = Harness::fresh("online_valid_foreign").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("applied bundle");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id; CREATE INDEX idx_job_queue_created_id ON job_queue(id)")
        .execute(&harness.pool).await.expect("valid foreign definition");
    assert_definition_refused_unchanged(&harness.pool).await;
    harness.teardown().await;
}

#[tokio::test]
async fn explicit_default_fillfactor_is_refused_until_the_option_is_reset() {
    let harness = Harness::fresh("online_index_options").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("applied bundle");
    sqlx::raw_sql("ALTER INDEX idx_job_queue_created_id SET (fillfactor = 90)")
        .execute(&harness.pool)
        .await
        .expect("explicit default option");
    assert_definition_refused_unchanged(&harness.pool).await;
    sqlx::raw_sql("ALTER INDEX idx_job_queue_created_id RESET (fillfactor)")
        .execute(&harness.pool)
        .await
        .expect("restore implicit default");
    verify_profiled(&harness.pool)
        .await
        .expect("canonical definition restored");
    harness.teardown().await;
}

async fn assert_definition_refused_unchanged(pool: &sqlx::PgPool) {
    let catalog = index_catalog(pool).await;
    let history = sqlx_history(pool).await;
    assert!(
        catalog["idx_job_queue_created_id"].1,
        "wrong definition must be VALID"
    );
    let error = verify_profiled(pool)
        .await
        .expect_err("strict definition mismatch");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_job_queue_created_id",
                problem: OnlineIndexProblem::DefinitionMismatch,
            }
        ),
        "unexpected refusal: {error}"
    );
    assert_eq!(index_catalog(pool).await, catalog);
    assert_eq!(sqlx_history(pool).await, history);
}

#[tokio::test]
async fn external_preparation_exposes_the_same_typed_lock_wait_expiry() {
    let harness = Harness::fresh("online_preparation_lock_bound").await;
    assert_lock_wait_expires(
        &harness,
        MigrationTimeoutSetting::Lock,
        Duration::from_millis(250),
        |pool| async move {
            let options = pool.connect_options();
            let login = options.get_username();
            let profile = PgSessionProfile::new(
                login,
                login,
                vec!["public".into()],
                Duration::from_secs(2),
                Duration::from_millis(250),
            )
            .expect("bounded profile");
            let database = RunledgerDatabase::connect((*options).clone(), profile, 1)
                .await
                .expect("profiled database");
            let result = prepare_online_indexes_for_external_migrator(&database)
                .await
                .map(|_| ());
            database.pool().close().await;
            result
        },
    )
    .await;
    harness.teardown().await;
}

#[tokio::test]
async fn lock_wait_honors_the_profile_lock_timeout() {
    let harness = Harness::fresh("online_idx_lock_timeout").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    // A 250 ms lock_timeout under a 2 s statement_timeout.
    assert_lock_wait_expires(
        &harness,
        MigrationTimeoutSetting::Lock,
        Duration::from_millis(250),
        |pool| async move { migrate_with_short_timeouts(&pool).await },
    )
    .await;
    migrate_profiled(&harness.pool)
        .await
        .expect("startup proceeds once the lock is free");
    harness.teardown().await;
}

#[tokio::test]
async fn lock_wait_honors_the_profile_statement_timeout_when_lock_timeout_is_zero() {
    let harness = Harness::fresh("online_idx_statement_timeout").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    assert_lock_wait_expires(
        &harness,
        MigrationTimeoutSetting::Statement,
        Duration::from_millis(500),
        |pool| async move {
            migrate_with_profile_timeouts(&pool, Duration::from_millis(500), Duration::ZERO).await
        },
    )
    .await;
    migrate_profiled(&harness.pool)
        .await
        .expect("startup proceeds once the lock is free");
    harness.teardown().await;
}

#[tokio::test]
async fn lock_wait_honors_the_profile_transaction_timeout_when_the_others_are_zero() {
    let harness = Harness::fresh("online_idx_transaction_timeout").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_SUMMARY_PAGINATION).await;
    // Before the polling lock, a 500 ms transaction_timeout terminated the
    // blocking pg_advisory_lock session; the poll must keep that bound.
    assert_lock_wait_expires(&harness, MigrationTimeoutSetting::Transaction, Duration::from_millis(500), |pool| async move {
        migrate_with_transaction_timeout(&pool, Duration::from_millis(500)).await
    })
    .await;
    migrate_profiled(&harness.pool)
        .await
        .expect("startup proceeds once the lock is free");
    harness.teardown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_lock_released_after_the_deadline_cannot_be_admitted_on_delayed_resumption() {
    use sqlx::Connection;
    use sqlx::migrate::Migrate;

    let harness = Harness::fresh("online_lock_late_release").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("initial migration");
    let url = harness.database.url().to_owned();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (released_tx, released_rx) = std::sync::mpsc::channel();
    // The holder owns a separate runtime so it can release while this test's
    // current-thread runtime is deliberately stalled past the wait bound.
    let holder = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("holder runtime")
            .block_on(async {
                let mut conn = sqlx::PgConnection::connect(&url)
                    .await
                    .expect("holder connection");
                conn.lock().await.expect("hold SQLx lock");
                ready_tx.send(()).expect("holder ready");
                release_rx
                    .recv_timeout(Duration::from_secs(30))
                    .expect("release requested");
                conn.unlock().await.expect("release lock independently");
                conn.close().await.expect("close holder");
                released_tx.send(()).expect("release acknowledged");
            });
    });
    ready_rx.await.expect("holder acquired lock");
    let runner_pool = harness.pool.clone();
    let runner = tokio::spawn(async move { migrate_with_short_timeouts(&runner_pool).await });
    wait_for(
        "the runner to complete its first unsuccessful poll",
        || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_stat_activity
                WHERE datname = current_database() AND pid <> pg_backend_pid()
                  AND state = 'idle' AND query LIKE '%pg_try_advisory_lock%'
             )",
            )
            .fetch_one(&harness.pool)
            .await
            .expect("observe idle lock poll")
        },
    )
    .await;
    // No task on this runtime can advance during the stall. Only after the
    // 250 ms total bound has elapsed do we let the independent holder release.
    std::thread::sleep(Duration::from_millis(400));
    release_tx.send(()).expect("release after expiry");
    released_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("lock is free before resumption");
    let error = tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .expect("runner settles")
        .expect("runner task")
        .expect_err("late success cannot extend the lock-wait budget");
    let SchemaCompatibilityError::MigrationLockTimeout(expiry) = error else {
        panic!("unexpected late-result error: {error}");
    };
    assert_eq!(expiry.setting(), MigrationTimeoutSetting::Lock);
    assert_eq!(expiry.bound(), Duration::from_millis(250));
    holder.join().expect("holder thread");
    migrate_profiled(&harness.pool)
        .await
        .expect("later independent startup");
    harness.teardown().await;
}
