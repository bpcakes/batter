use std::collections::BTreeMap;
use std::time::Duration;

use runledger_postgres::{
    MIGRATOR, OnlineIndexProblem, PgSessionProfile, RunledgerDatabase, SchemaCompatibilityError,
    ensure_schema_compatible_after_idempotency_cutover, migrate_after_idempotency_cutover,
    prepare_online_indexes_for_external_migrator,
};
use sqlx::{AssertSqlSafe, PgPool};

mod online_support;
mod support;
use online_support::{
    DECLARED_INDEXES, Harness, LAST_VERSION_BEFORE_CUTOVER, hold_writer_lock, index_catalog,
    raw_apply_through, sqlx_history,
};

async fn index_oids(pool: &PgPool, schema: &str) -> BTreeMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT c.relname, c.oid::bigint FROM pg_catalog.pg_class c
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = ANY($2)",
    )
    .bind(schema)
    .bind(&DECLARED_INDEXES[..2])
    .fetch_all(pool)
    .await
    .expect("prepared index identities")
    .into_iter()
    .collect()
}

#[tokio::test]
async fn protected_runner_adopts_prepared_pending_indexes_without_rebuilding() {
    let harness = Harness::fresh("online_pending_adoption").await;
    raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
    let database = support::profiled_database(&harness.pool).await;
    let report = prepare_online_indexes_for_external_migrator(&database)
        .await
        .expect("prepare pair");
    assert_eq!(report.built(), &DECLARED_INDEXES[..2]);
    let before = index_oids(&harness.pool, "public").await;
    assert_eq!(before.len(), 2);
    assert!(
        !sqlx_history(&harness.pool)
            .await
            .iter()
            .any(|row| row.0 == 202605220001)
    );
    migrate_after_idempotency_cutover(&database)
        .await
        .expect("adopt pending prepared pair");
    assert_eq!(index_oids(&harness.pool, "public").await, before);
    let expected: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .map(|m| (m.version, m.checksum.to_vec(), true))
        .collect();
    assert_eq!(sqlx_history(&harness.pool).await, expected);
    ensure_schema_compatible_after_idempotency_cutover(&database)
        .await
        .expect("adopted schema");
    database.pool().close().await;
    harness.teardown().await;
}

#[tokio::test]
async fn quoted_schema_delimiters_and_keywords_preserve_canonical_definitions() {
    for (schema, quote_all) in [
        ("tenant USING data", false),
        ("tenant USING \"data\"", false),
        ("select", false),
        ("public", true),
        ("tenant USING \"data\"", true),
        ("select", true),
    ] {
        let harness = Harness::fresh("online_quoted_schema").await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE SCHEMA IF NOT EXISTS \"{}\"",
            schema.replace('"', "\"\"")
        )))
        .execute(&harness.pool)
        .await
        .expect("quoted schema");
        let options = harness.pool.connect_options();
        let login = options.get_username();
        let profile = PgSessionProfile::new(
            login,
            login,
            vec![schema.into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .expect("supported schema identifier");
        let options = (*options).clone().options([(
            "quote_all_identifiers",
            if quote_all { "on" } else { "off" },
        )]);
        let database = RunledgerDatabase::connect(options, profile, 1)
            .await
            .expect("profiled schema");
        assert_quoting(database.pool(), quote_all).await;
        raw_apply_through(database.pool(), LAST_VERSION_BEFORE_CUTOVER).await;
        let report = prepare_online_indexes_for_external_migrator(&database)
            .await
            .expect("prepare in quoted schema");
        assert_eq!(report.built(), &DECLARED_INDEXES[..2]);
        let before = index_oids(database.pool(), schema).await;
        let repeated = prepare_online_indexes_for_external_migrator(&database)
            .await
            .expect("match prepared definitions");
        assert_eq!(repeated.already_valid(), &DECLARED_INDEXES[..2]);
        migrate_after_idempotency_cutover(&database)
            .await
            .expect("migrate quoted schema");
        assert_eq!(index_oids(database.pool(), schema).await, before);
        ensure_schema_compatible_after_idempotency_cutover(&database)
            .await
            .expect("verify quoted schema");
        migrate_after_idempotency_cutover(&database)
            .await
            .expect("repeated protected startup");
        if quote_all && schema == "public" {
            repair_interrupted_quoted_index(&database, &harness.pool).await;
        }
        assert_quoting(database.pool(), quote_all).await;
        database.pool().close().await;
        harness.teardown().await;
    }
}

async fn repair_interrupted_quoted_index(database: &RunledgerDatabase, pool: &PgPool) {
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(pool)
        .await
        .expect("remove canonical index");
    let writer = hold_writer_lock(pool).await;
    let mut builder = pool.acquire().await.expect("interrupted builder");
    builder.close_on_drop();
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bound concurrent build");
    let error = sqlx::raw_sql(
        "CREATE INDEX CONCURRENTLY idx_job_queue_created_id
         ON job_queue (created_at DESC, id DESC)",
    )
    .execute(&mut *builder)
    .await
    .expect_err("writer interrupts build");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    drop(builder);
    writer.rollback().await.expect("release writer");
    let error = ensure_schema_compatible_after_idempotency_cutover(database)
        .await
        .expect_err("canonical interrupted definition stays repairable");
    assert!(matches!(
        error,
        SchemaCompatibilityError::OnlineIndex {
            index: "idx_job_queue_created_id",
            problem: OnlineIndexProblem::Invalid
        }
    ));
    migrate_after_idempotency_cutover(database)
        .await
        .expect("repair quoted interrupted index");
    ensure_schema_compatible_after_idempotency_cutover(database)
        .await
        .expect("verify repaired quoted index");
}

async fn assert_quoting(pool: &PgPool, enabled: bool) {
    let actual: bool =
        sqlx::query_scalar("SELECT current_setting('quote_all_identifiers')::boolean")
            .fetch_one(pool)
            .await
            .expect("inherited quoting setting");
    assert_eq!(actual, enabled);
}

#[tokio::test]
async fn preparation_rejects_a_newer_history_fence_before_building() {
    for existing_history in [false, true] {
        let harness = Harness::fresh("online_newer_fence").await;
        if existing_history {
            raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
        }
        let newer = MIGRATOR
            .iter()
            .map(|m| m.version)
            .max()
            .expect("bundled versions")
            + 1;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS runledger_migration_history (version BIGINT PRIMARY KEY)",
        )
        .execute(&harness.pool)
        .await
        .expect("compatibility fence");
        sqlx::query("INSERT INTO runledger_migration_history (version) VALUES ($1)")
            .bind(newer)
            .execute(&harness.pool)
            .await
            .expect("newer incompatible version");
        let before = index_catalog(&harness.pool).await;
        let database = support::profiled_database(&harness.pool).await;
        let error = match prepare_online_indexes_for_external_migrator(&database).await {
            Err(error) => error,
            Ok(_) => panic!("newer fence must fail preparation"),
        };
        assert!(
            matches!(error, SchemaCompatibilityError::Incompatible(sqlx::migrate::MigrateError::VersionMissing(v)) if v == newer)
        );
        assert_eq!(
            index_catalog(&harness.pool).await,
            before,
            "refusal precedes any builds"
        );
        database.pool().close().await;
        harness.teardown().await;
    }
}
