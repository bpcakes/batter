use std::time::Duration;

use runledger_postgres::{
    OnlineIndexProblem, PgSessionProfile, RunledgerDatabase, SchemaCompatibilityError,
    migrate_after_idempotency_cutover, prepare_online_indexes_for_external_migrator,
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
const CREATED_BUILD: &str =
    "CREATE INDEX CONCURRENTLY idx_job_queue_created_id ON job_queue(created_at DESC,id DESC)";
const SNAPSHOT_BUILD: &str = "CREATE INDEX CONCURRENTLY idx_job_queue_missing_enqueue_request_snapshot ON job_queue(id) WHERE idempotency_key IS NOT NULL AND enqueue_request IS NULL";

/// The canonical concurrent build for the index `invalid_index` leaves.
fn canonical_build(prepare: bool) -> &'static str {
    if prepare {
        SNAPSHOT_BUILD
    } else {
        CREATED_BUILD
    }
}

async fn invalid_index(pool: &PgPool, prepare: bool) -> &'static str {
    let index = if prepare {
        raw_apply_through(pool, LAST_VERSION_BEFORE_CUTOVER).await;
        SNAPSHOT
    } else {
        migrate_profiled(pool).await.expect("canonical schema");
        sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
            .execute(pool)
            .await
            .expect("remove declared index");
        CREATED
    };
    interrupt_build(pool, canonical_build(prepare)).await;
    index
}

/// Leave `sql`'s concurrent build INVALID: a held writer outlasts its timeout.
async fn interrupt_build(pool: &PgPool, sql: &'static str) {
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

async fn entry(database: RunledgerDatabase, prepare: bool) -> Result<(), SchemaCompatibilityError> {
    if prepare {
        prepare_online_indexes_for_external_migrator(&database)
            .await
            .map(|_| ())
    } else {
        migrate_after_idempotency_cutover(&database).await
    }
}

async fn index_valid(pool: &PgPool, oid: i64) -> bool {
    sqlx::query_scalar("SELECT indisvalid FROM pg_index WHERE indexrelid = $1::bigint::oid")
        .bind(oid)
        .fetch_one(pool)
        .await
        .expect("index state")
}

async fn relation_oid(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT to_regclass($1)::oid::bigint")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("relation oid")
}

/// A canonical INVALID index under the declared name is not the inspected one;
/// only the exact-OID recheck under the repair locks can tell them apart.
async fn refuses_changed_invalid_identity(prepare: bool) {
    let harness = Harness::fresh("online_repair_changed").await;
    let index = invalid_index(&harness.pool, prepare).await;
    let original = relation_oid(&harness.pool, index).await;
    let history = sqlx_history(&harness.pool).await;
    let mut relay = Relay::start(
        &harness.pool.connect_options(),
        Fault::HoldIndexInspection(index),
    )
    .await;
    let database = relayed_database(&harness.pool, &relay);
    let task = tokio::spawn(entry(database.clone(), prepare));
    relay.reached().await;
    let replacement = swap_in_invalid_replacement(&harness.pool, index, prepare, original).await;
    relay.release_response().await;
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("bounded repair")
        .expect("repair task");
    let retained = retained_identities(&harness.pool, index, replacement).await;
    let after = sqlx_history(&harness.pool).await;
    database.pool().close().await;
    relay.finish().await;
    harness.teardown().await;
    assert_changed_identity_refusal(&result);
    assert_eq!(
        retained,
        (replacement, original, false),
        "neither index was removed and the replacement is left INVALID"
    );
    assert_eq!(after, history);
}

/// Move the inspected index aside and leave a canonical INVALID build under
/// the declared name, returning the replacement's OID.
async fn swap_in_invalid_replacement(
    pool: &PgPool,
    index: &str,
    prepare: bool,
    original: i64,
) -> i64 {
    sqlx::raw_sql(AssertSqlSafe(format!(
        "ALTER INDEX {index} RENAME TO retained_original"
    )))
    .execute(pool)
    .await
    .expect("move the inspected INVALID index");
    interrupt_build(pool, canonical_build(prepare)).await;
    let replacement = relation_oid(pool, index).await;
    assert_ne!(replacement, original, "a second canonical INVALID index");
    replacement
}

/// The declared name's OID, the moved original's OID and the replacement's validity.
async fn retained_identities(pool: &PgPool, index: &str, replacement: i64) -> (i64, i64, bool) {
    (
        relation_oid(pool, index).await,
        relation_oid(pool, "retained_original").await,
        index_valid(pool, replacement).await,
    )
}

fn assert_changed_identity_refusal(result: &Result<(), SchemaCompatibilityError>) {
    let Err(SchemaCompatibilityError::OnlineIndexExecution(failure)) = result else {
        panic!("execution context: {result:?}")
    };
    assert!(
        matches!(
            failure.cause(),
            SchemaCompatibilityError::OnlineIndex {
                problem: OnlineIndexProblem::ChangedDuringRepair,
                ..
            }
        ),
        "unexpected refusal: {}",
        failure.cause()
    );
}

#[tokio::test]
async fn migration_refuses_a_changed_invalid_identity_after_inspection() {
    refuses_changed_invalid_identity(false).await;
}

#[tokio::test]
async fn preparation_refuses_a_changed_invalid_identity_after_inspection() {
    refuses_changed_invalid_identity(true).await;
}

#[path = "online_repair/lifecycle.rs"]
mod lifecycle;
