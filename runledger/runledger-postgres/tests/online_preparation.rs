//! Live evidence for the external-preparation path on its own: it refuses a
//! corrupted migration history before any build, drops and rebuilds an
//! `INVALID` adoptable leftover, and refuses a foreign index under an
//! adoptable name without dropping it.

use std::time::Duration;

use runledger_postgres::{
    OnlineIndexPhase, OnlineIndexProblem, SchemaCompatibilityError,
    prepare_online_indexes_for_external_migrator,
};
mod online_support;
mod support;
use online_support::{
    DECLARED_INDEXES, Harness, LAST_VERSION_BEFORE_CUTOVER, build_waits_for_writers,
    hold_writer_lock, index_catalog, raw_apply_through, sqlx_history, wait_for,
};
use sqlx::migrate::MigrateError;
use support::profiled_database;

const CUTOVER_VERSION: i64 = 202605220001;

async fn prepare(harness: &Harness) -> Result<Vec<&'static str>, SchemaCompatibilityError> {
    let database = profiled_database(&harness.pool).await;
    let result = prepare_online_indexes_for_external_migrator(&database).await;
    database.pool().close().await;
    result.map(|report| report.built().to_vec())
}

#[tokio::test]
async fn preparation_refuses_a_recorded_migration_with_a_foreign_checksum() {
    let harness = Harness::fresh("online_prep_checksum").await;
    raw_apply_through(&harness.pool, CUTOVER_VERSION).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
        .bind(vec![0_u8; 48])
        .bind(CUTOVER_VERSION)
        .execute(&harness.pool)
        .await
        .expect("tamper with the recorded checksum");
    let before = index_catalog(&harness.pool).await;

    let error = prepare(&harness)
        .await
        .expect_err("a foreign checksum is refused instead of treated as pending");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::Incompatible(MigrateError::VersionMismatch(CUTOVER_VERSION))
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        index_catalog(&harness.pool).await,
        before,
        "nothing is built under a refused history"
    );
    harness.teardown().await;
}

#[tokio::test]
async fn preparation_refuses_a_failed_recorded_migration() {
    let harness = Harness::fresh("online_prep_dirty").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = $1")
        .bind(LAST_VERSION_BEFORE_CUTOVER)
        .execute(&harness.pool)
        .await
        .expect("mark the last migration as failed");

    let error = prepare(&harness)
        .await
        .expect_err("a dirty row is refused before any build");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::Incompatible(MigrateError::Dirty(
                LAST_VERSION_BEFORE_CUTOVER
            ))
        ),
        "unexpected error: {error}"
    );
    let catalog = index_catalog(&harness.pool).await;
    for name in &DECLARED_INDEXES[..2] {
        assert!(
            !catalog.contains_key(*name),
            "{name} must not be built under a dirty history"
        );
    }
    harness.teardown().await;
}

#[tokio::test]
async fn preparation_rebuilds_an_invalid_adoptable_leftover() {
    let harness = Harness::fresh("online_prep_invalid").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    // Leave an INVALID index with the bundled definition: a concurrent build
    // cancelled while it waits for an open writer keeps its catalog entry.
    let holder = hold_writer_lock(&harness.pool).await;
    let mut builder = harness.pool.acquire().await.expect("builder connection");
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bound the build");
    sqlx::raw_sql(
        "CREATE INDEX CONCURRENTLY idx_job_queue_missing_enqueue_request_snapshot
         ON job_queue (id) WHERE idempotency_key IS NOT NULL AND enqueue_request IS NULL",
    )
    .execute(&mut *builder)
    .await
    .expect_err("the build is cancelled by statement_timeout");
    drop(builder);
    holder.rollback().await.expect("release writer lock");
    assert!(
        matches!(
            index_catalog(&harness.pool).await.get(DECLARED_INDEXES[0]),
            Some((_, false))
        ),
        "the cancelled build leaves an INVALID index"
    );

    let built = prepare(&harness)
        .await
        .expect("preparation drops and rebuilds the leftover while its migration is pending");
    assert_eq!(
        built,
        &DECLARED_INDEXES[..2],
        "the rebuilt index is reported as built"
    );
    let catalog = index_catalog(&harness.pool).await;
    for name in &DECLARED_INDEXES[..2] {
        assert!(
            matches!(catalog.get(*name), Some((_, true))),
            "{name} must be valid after preparation"
        );
    }
    harness.teardown().await;
}

#[tokio::test]
async fn preparation_refuses_a_foreign_index_under_an_adoptable_name() {
    let harness = Harness::fresh("online_prep_foreign").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    sqlx::raw_sql(
        "CREATE INDEX idx_workflow_runs_missing_enqueue_request_snapshot ON workflow_runs (id)",
    )
    .execute(&harness.pool)
    .await
    .expect("foreign index");
    let foreign = index_catalog(&harness.pool)
        .await
        .remove(DECLARED_INDEXES[1])
        .expect("foreign index exists");

    let error = prepare(&harness)
        .await
        .expect_err("a differently defined index under an adoptable name is refused");
    assert!(
        matches!(
            error,
            SchemaCompatibilityError::OnlineIndex {
                index: "idx_workflow_runs_missing_enqueue_request_snapshot",
                problem: OnlineIndexProblem::DefinitionMismatch
            }
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        index_catalog(&harness.pool).await.get(DECLARED_INDEXES[1]),
        Some(&foreign),
        "the foreign index is refused, never dropped or rebuilt"
    );

    sqlx::raw_sql("DROP INDEX idx_workflow_runs_missing_enqueue_request_snapshot")
        .execute(&harness.pool)
        .await
        .expect("remove the foreign index");
    let built = prepare(&harness)
        .await
        .expect("preparation proceeds once the name is free");
    assert!(built.contains(&DECLARED_INDEXES[1]));
    harness.teardown().await;
}

#[tokio::test]
async fn failed_preparation_retains_the_native_cause_without_recording_a_migration() {
    let harness = Harness::fresh("online_prep_build_error").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    let history = sqlx_history(&harness.pool).await;
    let writer = hold_writer_lock(&harness.pool).await;
    let pool = harness.pool.clone();
    let runner = tokio::spawn(async move {
        let database = profiled_database(&pool).await;
        let result = prepare_online_indexes_for_external_migrator(&database).await;
        database.pool().close().await;
        result
    });
    wait_for("preparation build waits on the writer", || {
        build_waits_for_writers(&harness.pool)
    })
    .await;
    let pid: i32 = sqlx::query_scalar(
        "SELECT pid FROM pg_catalog.pg_stat_progress_create_index
         WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
           AND index_relid = pg_catalog.to_regclass($1) AND phase LIKE 'waiting for%'",
    )
    .bind(DECLARED_INDEXES[0])
    .fetch_one(&harness.pool)
    .await
    .expect("the actual preparation builder");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_cancel_backend($1)")
            .bind(pid)
            .fetch_one(&harness.pool)
            .await
            .expect("cancel the preparation build")
    );
    let error = tokio::time::timeout(Duration::from_secs(10), runner)
        .await
        .expect("preparation settles while the writer remains held")
        .expect("preparation task")
        .expect_err("cancelled build");
    let rendered = format!("{error} {error:?}");
    assert!(!rendered.contains("canceling statement"));
    let SchemaCompatibilityError::OnlineIndexExecution(failure) = error else {
        panic!("missing preparation build context: {error}");
    };
    assert_eq!(failure.index(), DECLARED_INDEXES[0]);
    assert_eq!(failure.recorded_version(), None);
    assert_eq!(failure.phase(), OnlineIndexPhase::Build);
    assert!(failure.restoration_error().is_none());
    let SchemaCompatibilityError::Query(native) = failure.cause() else {
        panic!("native cancellation cause");
    };
    assert_eq!(
        native
            .as_database_error()
            .expect("server cancellation")
            .code()
            .as_deref(),
        Some("57014")
    );
    assert_eq!(sqlx_history(&harness.pool).await, history);
    assert!(!index_catalog(&harness.pool).await[DECLARED_INDEXES[0]].1);
    writer.rollback().await.expect("release writer");
    let built = prepare(&harness)
        .await
        .expect("repair the interrupted preparation");
    assert_eq!(built, &DECLARED_INDEXES[..2]);
    assert_eq!(sqlx_history(&harness.pool).await, history);
    harness.teardown().await;
}
