use std::time::Duration;

use runledger_postgres::{
    PgSessionProfile, RunledgerDatabase, migrate_after_idempotency_cutover,
    prepare_online_indexes_for_external_migrator,
};
use sqlx::{AssertSqlSafe, PgPool};

mod online_support;
#[path = "online_transport_failures/wire.rs"]
mod online_wire;
mod support;

use online_support::{
    Harness, LAST_VERSION_BEFORE_CUTOVER, hold_writer_lock, raw_apply_through, sqlx_history,
};
use online_wire::{Fault, Relay};
use support::migrate_profiled;

const CREATED: &str = "idx_job_queue_created_id";
const SNAPSHOT: &str = "idx_job_queue_missing_enqueue_request_snapshot";

async fn invalid_index(pool: &PgPool, prepare: bool) -> &'static str {
    let (index, sql) = if prepare {
        raw_apply_through(pool, LAST_VERSION_BEFORE_CUTOVER).await;
        (
            SNAPSHOT,
            "CREATE INDEX CONCURRENTLY idx_job_queue_missing_enqueue_request_snapshot ON job_queue(id) WHERE idempotency_key IS NOT NULL AND enqueue_request IS NULL",
        )
    } else {
        migrate_profiled(pool).await.expect("canonical schema");
        sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
            .execute(pool)
            .await
            .expect("remove declared index");
        (
            CREATED,
            "CREATE INDEX CONCURRENTLY idx_job_queue_created_id ON job_queue(created_at DESC,id DESC)",
        )
    };
    let writer = hold_writer_lock(pool).await;
    let mut builder = pool.acquire().await.expect("builder");
    builder.close_on_drop();
    sqlx::raw_sql("SET statement_timeout='200ms'")
        .execute(&mut *builder)
        .await
        .expect("bound interrupted build");
    let error = sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(&mut *builder)
        .await
        .expect_err("writer interrupts build");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    builder.close().await.expect("retire builder");
    writer.rollback().await.expect("release writer");
    index
}

fn relayed_database(pool: &PgPool, relay: &Relay) -> RunledgerDatabase {
    let options = pool.connect_options();
    let login = options.get_username();
    let profile = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .expect("profile");
    RunledgerDatabase::connect_lazy(
        relay.options.clone(),
        profile,
        sqlx::postgres::PgPoolOptions::new().max_connections(1),
    )
    .expect("relayed database")
}

async fn preserves_replacement(prepare: bool) {
    let harness = Harness::fresh("online_repair_swap").await;
    let index = invalid_index(&harness.pool, prepare).await;
    let history = sqlx_history(&harness.pool).await;
    sqlx::raw_sql("CREATE TABLE unrelated_records(value integer); CREATE UNIQUE INDEX foreign_unique ON unrelated_records(value); INSERT INTO unrelated_records VALUES (1)").execute(&harness.pool).await.expect("foreign unique index");
    let foreign: i64 = sqlx::query_scalar("SELECT 'foreign_unique'::regclass::oid::bigint")
        .fetch_one(&harness.pool)
        .await
        .expect("foreign oid");
    let mut relay = Relay::start(
        &harness.pool.connect_options(),
        Fault::HoldIndexInspection(index),
    )
    .await;
    let database = relayed_database(&harness.pool, &relay);
    let task_database = database.clone();
    let task = tokio::spawn(async move {
        if prepare {
            prepare_online_indexes_for_external_migrator(&task_database)
                .await
                .map(|_| ())
        } else {
            migrate_after_idempotency_cutover(&task_database).await
        }
    });
    relay.reached().await;
    sqlx::raw_sql(AssertSqlSafe(format!("BEGIN; ALTER INDEX {index} RENAME TO retained_original; ALTER INDEX foreign_unique RENAME TO {index}; COMMIT"))).execute(&harness.pool).await.expect("swap after completed inspection");
    relay.release_response().await;
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("bounded repair")
        .expect("repair task");
    let preserved: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_class WHERE oid=$1::bigint::oid)")
            .bind(foreign)
            .fetch_one(&harness.pool)
            .await
            .expect("foreign identity");
    let definition: Option<String> = sqlx::query_scalar("SELECT pg_get_indexdef($1::bigint::oid)")
        .bind(foreign)
        .fetch_one(&harness.pool)
        .await
        .expect("retained definition");
    let duplicate = sqlx::raw_sql("INSERT INTO unrelated_records VALUES (1)")
        .execute(&harness.pool)
        .await;
    let after = sqlx_history(&harness.pool).await;
    database.pool().close().await;
    relay.finish().await;
    harness.teardown().await;
    assert_foreign_refusal(preserved, result, definition, index, duplicate);
    assert_eq!(after, history);
}

fn assert_foreign_refusal(
    preserved: bool,
    result: Result<(), runledger_postgres::SchemaCompatibilityError>,
    definition: Option<String>,
    index: &str,
    duplicate: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
) {
    assert!(
        preserved,
        "repair must not delete the swapped-in foreign UNIQUE index"
    );
    assert!(result.is_err(), "foreign replacement must be refused");
    assert_eq!(
        definition.as_deref(),
        Some(
            format!("CREATE UNIQUE INDEX {index} ON public.unrelated_records USING btree (value)")
                .as_str()
        )
    );
    assert_eq!(
        duplicate
            .expect_err("unique enforcement survives")
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("23505")
    );
}

#[tokio::test]
async fn migration_preserves_foreign_index_swapped_in_after_inspection() {
    preserves_replacement(false).await;
}

#[tokio::test]
async fn preparation_preserves_foreign_index_swapped_in_after_inspection() {
    preserves_replacement(true).await;
}

#[path = "online_repair/lifecycle.rs"]
mod lifecycle;
