//! Index predicates and progress must retain their literal/database identity.

use std::time::Duration;

use runledger_postgres::{OnlineIndexProblem, SchemaCompatibilityError};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, postgres::PgPoolOptions};

mod online_support;
mod support;
use online_support::{Harness, hold_writer_lock, index_catalog, sqlx_history, wait_for};
use support::{migrate_profiled, verify_profiled};

const CLAIM_INDEX: &str = "idx_job_queue_execution_resource_claim_order";
const FOREIGN_PREDICATE: &str =
    "CREATE INDEX idx_job_queue_execution_resource_claim_order ON job_queue
     (priority DESC, next_run_at, created_at, id) INCLUDE (execution_resource_key, job_type)
     WHERE status = 'public.PENDING' AND execution_resource_key IS NOT NULL";
const CREATED_INDEX: &str = "idx_job_queue_created_id";

async fn oid(pool: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT pg_catalog.to_regclass($1)::oid::bigint")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("index identity")
}

fn assert_problem(error: SchemaCompatibilityError, index: &str, problem: OnlineIndexProblem) {
    let SchemaCompatibilityError::OnlineIndex {
        index: actual,
        problem: actual_problem,
    } = error
    else {
        panic!("unexpected failure: {error}")
    };
    assert_eq!(actual, index);
    assert_eq!(actual_problem, problem);
}

#[tokio::test]
async fn foreign_predicate_literals_are_refused_and_preserved_valid_or_invalid() {
    for (invalid, quote_all) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut harness = Harness::fresh("online_literal_identity").await;
        if quote_all {
            sqlx::raw_sql(AssertSqlSafe(format!(
                "ALTER DATABASE \"{}\" SET quote_all_identifiers = on",
                harness.database.name().replace('"', "\"\"")
            )))
            .execute(&harness.pool)
            .await
            .expect("new profiled sessions inherit identifier quoting");
            // Reconnect the observer too: a mixture of old/new defaults would
            // compare different textual renderings of an unchanged catalog.
            let options = (*harness.pool.connect_options()).clone();
            harness.pool.close().await;
            harness.pool = PgPoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await
                .expect("observer inherits the same quoting default");
        }
        migrate_profiled(&harness.pool)
            .await
            .expect("canonical schema");
        sqlx::raw_sql("ALTER TYPE job_status ADD VALUE 'public.PENDING'")
            .execute(&harness.pool)
            .await
            .expect("distinct foreign enum label");
        sqlx::raw_sql("DROP INDEX idx_job_queue_execution_resource_claim_order")
            .execute(&harness.pool)
            .await
            .expect("remove canonical index");
        if invalid {
            interrupt_build(
                &harness.pool,
                &FOREIGN_PREDICATE.replace("CREATE INDEX", "CREATE INDEX CONCURRENTLY"),
            )
            .await;
        } else {
            sqlx::raw_sql(FOREIGN_PREDICATE)
                .execute(&harness.pool)
                .await
                .expect("foreign index");
        }
        let before = index_catalog(&harness.pool).await;
        assert_eq!(before[CLAIM_INDEX].1, !invalid);
        assert!(before[CLAIM_INDEX].0.contains("'public.PENDING'"));
        assert_eq!(
            before[CLAIM_INDEX].0.starts_with("CREATE INDEX \""),
            quote_all
        );
        let identity = oid(&harness.pool, CLAIM_INDEX).await;
        let history = sqlx_history(&harness.pool).await;
        assert_problem(
            verify_profiled(&harness.pool)
                .await
                .expect_err("foreign predicate"),
            CLAIM_INDEX,
            OnlineIndexProblem::DefinitionMismatch,
        );
        assert_problem(
            migrate_profiled(&harness.pool)
                .await
                .expect_err("never repair foreign predicate"),
            CLAIM_INDEX,
            OnlineIndexProblem::DefinitionMismatch,
        );
        assert_eq!(index_catalog(&harness.pool).await, before);
        assert_eq!(oid(&harness.pool, CLAIM_INDEX).await, identity);
        assert_eq!(sqlx_history(&harness.pool).await, history);
        harness.teardown().await;
    }
}

async fn interrupt_build(pool: &PgPool, statement: &str) {
    let writer = hold_writer_lock(pool).await;
    let mut builder = pool.acquire().await.expect("builder");
    builder.close_on_drop();
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bounded build");
    let error = sqlx::raw_sql(AssertSqlSafe(statement.to_owned()))
        .execute(&mut *builder)
        .await
        .expect_err("interrupt while waiting for writer");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    drop(builder);
    writer.rollback().await.expect("release writer");
}

#[tokio::test]
async fn local_build_progress_is_detected_without_an_advisory_migration_lock() {
    let harness = Harness::fresh("online_local_progress").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("canonical schema");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&harness.pool)
        .await
        .expect("remove index for manual rebuild");
    let writer = hold_writer_lock(&harness.pool).await;
    let builder_pool = harness.pool.clone();
    let builder = tokio::spawn(async move {
        let mut conn = builder_pool.acquire().await.expect("manual builder");
        conn.close_on_drop();
        sqlx::raw_sql("SET statement_timeout = '30s'")
            .execute(&mut *conn)
            .await
            .expect("bound manual build");
        sqlx::raw_sql(
            "CREATE INDEX CONCURRENTLY idx_job_queue_created_id
             ON job_queue (created_at DESC, id DESC)",
        )
        .execute(&mut *conn)
        .await
    });
    wait_for("local manual index progress", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
             WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
               AND index_relid = pg_catalog.to_regclass('idx_job_queue_created_id')
               AND command = 'CREATE INDEX CONCURRENTLY' AND phase LIKE 'waiting for%')",
        ).fetch_one(&harness.pool).await.expect("observe actual local progress")
    }).await;
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks
         WHERE locktype = 'advisory'
           AND database = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()))",
    ).fetch_one(&harness.pool).await.expect("no migration-lock fallback"));
    let identity = oid(&harness.pool, CREATED_INDEX).await;
    assert!(!index_catalog(&harness.pool).await[CREATED_INDEX].1);
    assert_problem(
        verify_profiled(&harness.pool)
            .await
            .expect_err("active local build"),
        CREATED_INDEX,
        OnlineIndexProblem::BuildInProgress,
    );
    assert_problem(
        tokio::time::timeout(Duration::from_secs(5), migrate_profiled(&harness.pool))
            .await
            .expect("refuse without attempting to drop the active index")
            .expect_err("active manual build cannot be repaired concurrently"),
        CREATED_INDEX,
        OnlineIndexProblem::BuildInProgress,
    );
    assert_eq!(oid(&harness.pool, CREATED_INDEX).await, identity);
    assert!(
        !builder.is_finished(),
        "manual build still waits for its writer"
    );
    writer.rollback().await.expect("release writer");
    tokio::time::timeout(Duration::from_secs(10), builder)
        .await
        .expect("manual build completes")
        .expect("builder task")
        .expect("manual index build");
    verify_profiled(&harness.pool)
        .await
        .expect("verification after manual build");
    assert_eq!(oid(&harness.pool, CREATED_INDEX).await, identity);
    harness.teardown().await;
}

#[tokio::test]
async fn progress_in_a_cloned_database_does_not_prevent_local_invalid_index_repair() {
    let mut local = Harness::fresh("online_progress_local").await;
    migrate_profiled(&local.pool)
        .await
        .expect("canonical schema");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(&local.pool)
        .await
        .expect("remove local index");
    interrupt_build(&local.pool,
        "CREATE INDEX CONCURRENTLY idx_job_queue_created_id ON job_queue (created_at DESC, id DESC)").await;
    assert!(!index_catalog(&local.pool).await[CREATED_INDEX].1);
    let identity = oid(&local.pool, CREATED_INDEX).await;

    // Clone through the existing fixture's database identities and lifecycle.
    // All template connections must be closed; no catalog OIDs are fabricated.
    let mut other = Harness::fresh("online_progress_other").await;
    let options = local.pool.connect_options();
    let mut admin = PgConnection::connect_with(&(*options).clone().database("postgres"))
        .await
        .expect("fixture administrator");
    local.pool.close().await;
    other.pool.close().await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP DATABASE \"{}\"",
        other.database.name()
    )))
    .execute(&mut admin)
    .await
    .expect("replace empty fixture with clone");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE DATABASE \"{}\" TEMPLATE \"{}\"",
        other.database.name(),
        local.database.name()
    )))
    .execute(&mut admin)
    .await
    .expect("clone database including index OIDs");
    admin.close().await.expect("close administrator");
    local.pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(local.database.url())
        .await
        .expect("reopen local fixture");
    other.pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(other.database.url())
        .await
        .expect("open cloned fixture");
    assert_eq!(oid(&other.pool, CREATED_INDEX).await, identity);

    // REINDEX TABLE reports the existing index OID before taking its exclusive
    // index lock. A held reloption change blocks that index without blocking
    // the table's ShareLock or any local-database operation.
    let mut holder = other.pool.begin().await.expect("index lock holder");
    sqlx::raw_sql("ALTER INDEX idx_job_queue_created_id SET (fillfactor = 90)")
        .execute(&mut *holder)
        .await
        .expect("hold cloned index lock");
    let rebuild_pool = other.pool.clone();
    let rebuild = tokio::spawn(async move {
        sqlx::raw_sql("REINDEX TABLE job_queue")
            .execute(&rebuild_pool)
            .await
    });
    wait_for("unrelated progress with the same index OID", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
             WHERE index_relid::bigint = $1 AND datname = $2 AND command = 'REINDEX')",
        )
        .bind(identity)
        .bind(other.database.name())
        .fetch_one(&local.pool)
        .await
        .expect("cluster progress visible from local database")
    })
    .await;
    assert!(!rebuild.is_finished());
    assert_problem(
        verify_profiled(&local.pool)
            .await
            .expect_err("local interrupted build"),
        CREATED_INDEX,
        OnlineIndexProblem::Invalid,
    );
    tokio::time::timeout(Duration::from_secs(20), migrate_profiled(&local.pool))
        .await
        .expect("local repair independent of remote rebuild")
        .expect("repair local index");
    verify_profiled(&local.pool)
        .await
        .expect("local repaired schema");
    assert_ne!(oid(&local.pool, CREATED_INDEX).await, identity);
    assert!(
        !rebuild.is_finished(),
        "remote index remains locked throughout local repair"
    );
    holder.rollback().await.expect("release remote index");
    tokio::time::timeout(Duration::from_secs(20), rebuild)
        .await
        .expect("remote rebuild completes")
        .expect("rebuild task")
        .expect("remote reindex");
    other.teardown().await;
    local.teardown().await;
}
