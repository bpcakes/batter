//! Concurrent reindex reports its replacement OID before the original is valid.

use std::time::Duration;

use runledger_postgres::{
    OnlineIndexProblem, SchemaCompatibilityError, prepare_online_indexes_for_external_migrator,
};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::task::JoinHandle;

mod online_support;
mod support;
use online_support::{
    Harness, LAST_VERSION_BEFORE_CUTOVER, hold_writer_lock, raw_apply_through, sqlx_history,
    wait_for,
};
use support::{migrate_profiled, profiled_database, verify_profiled};

const CREATED: &str = "idx_job_queue_created_id";
const CREATED_SQL: &str = "CREATE INDEX CONCURRENTLY idx_job_queue_created_id
    ON job_queue (created_at DESC, id DESC)";
const SNAPSHOT: &str = "idx_job_queue_missing_enqueue_request_snapshot";
const SNAPSHOT_SQL: &str =
    "CREATE INDEX CONCURRENTLY idx_job_queue_missing_enqueue_request_snapshot
    ON job_queue (id) WHERE idempotency_key IS NOT NULL AND enqueue_request IS NULL";

async fn identity(pool: &PgPool, index: &str) -> (i64, bool) {
    sqlx::query_as(
        "SELECT indexrelid::bigint, indisvalid FROM pg_catalog.pg_index
         WHERE indexrelid = pg_catalog.to_regclass($1)",
    )
    .bind(index)
    .fetch_one(pool)
    .await
    .expect("index identity")
}

async fn interrupt_build(pool: &PgPool, index: &str, statement: &'static str) {
    sqlx::raw_sql(AssertSqlSafe(format!("DROP INDEX IF EXISTS {index}")))
        .execute(pool)
        .await
        .expect("remove existing index");
    let writer = hold_writer_lock(pool).await;
    let mut builder = pool.acquire().await.expect("builder");
    builder.close_on_drop();
    sqlx::raw_sql("SET statement_timeout = '300ms'")
        .execute(&mut *builder)
        .await
        .expect("bound interrupted build");
    let error = sqlx::raw_sql(statement)
        .execute(&mut *builder)
        .await
        .expect_err("writer prevents concurrent build completion");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("57014")
    );
    builder.close().await.expect("retire interrupted builder");
    writer.rollback().await.expect("release writer");
    assert!(!identity(pool, index).await.1);
}

struct Reindex {
    snapshot: Transaction<'static, Postgres>,
    task: JoinHandle<Result<(), sqlx::Error>>,
    original_oid: i64,
    replacement_oid: i64,
}

impl Reindex {
    async fn start(pool: &PgPool, index: &'static str) -> Self {
        let original_oid = identity(pool, index).await.0;
        let mut snapshot = pool.begin().await.expect("older snapshot");
        sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *snapshot)
            .await
            .expect("stable old snapshot");
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_queue")
            .fetch_one(&mut *snapshot)
            .await
            .expect("pin a snapshot before reindex validation");
        let builder_pool = pool.clone();
        let task = tokio::spawn(async move {
            let mut conn = builder_pool.acquire().await?;
            conn.close_on_drop();
            sqlx::raw_sql("SET statement_timeout = '30s'")
                .execute(&mut *conn)
                .await?;
            sqlx::raw_sql(AssertSqlSafe(format!("REINDEX INDEX CONCURRENTLY {index}")))
                .execute(&mut *conn)
                .await?;
            Ok(())
        });
        // The first writer wait still reports the original OID. Wait past the
        // actual build and validation, when progress names the replacement.
        let progress = "SELECT p.index_relid::bigint FROM pg_catalog.pg_stat_progress_create_index p
            WHERE p.datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
              AND p.command = 'REINDEX CONCURRENTLY' AND p.phase = 'waiting for old snapshots'
              AND p.index_relid <> $1::bigint::oid
              AND EXISTS (SELECT 1 FROM pg_catalog.pg_locks l
                          WHERE l.pid = p.pid AND l.database = p.datid
                            AND l.locktype = 'relation' AND l.relation = $1::bigint::oid
                            AND l.mode = 'ShareUpdateExclusiveLock' AND l.granted)
              AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks l
                              WHERE l.pid = p.pid AND l.locktype = 'advisory')";
        wait_for(
            "reindex replacement waiting for the old snapshot",
            || async {
                sqlx::query_scalar::<_, i64>(progress)
                    .bind(original_oid)
                    .fetch_optional(pool)
                    .await
                    .expect("visible reindex progress and original-index lock")
                    .is_some()
            },
        )
        .await;
        let replacement_oid = sqlx::query_scalar(progress)
            .bind(original_oid)
            .fetch_one(pool)
            .await
            .expect("replacement OID");
        assert_ne!(original_oid, replacement_oid);
        Self {
            snapshot,
            task,
            original_oid,
            replacement_oid,
        }
    }

    async fn finish(self, pool: &PgPool, index: &str) {
        assert!(
            !self.task.is_finished(),
            "reindex still waits for the held snapshot"
        );
        assert_eq!(identity(pool, index).await.0, self.original_oid);
        self.snapshot
            .rollback()
            .await
            .expect("release older snapshot");
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("reindex finishes after snapshot release")
            .expect("reindex task")
            .expect("manual reindex succeeds");
        assert_eq!(identity(pool, index).await, (self.replacement_oid, true));
    }
}

enum Entry {
    Verify,
    Migrate,
    Prepare,
}

impl Entry {
    async fn seed_interrupted_index(&self, pool: &PgPool) -> &'static str {
        let (index, statement) = match self {
            Self::Prepare => {
                raw_apply_through(pool, LAST_VERSION_BEFORE_CUTOVER).await;
                (SNAPSHOT, SNAPSHOT_SQL)
            }
            Self::Verify | Self::Migrate => {
                migrate_profiled(pool).await.expect("canonical schema");
                (CREATED, CREATED_SQL)
            }
        };
        interrupt_build(pool, index, statement).await;
        index
    }

    async fn run(&self, pool: &PgPool) -> Result<(), SchemaCompatibilityError> {
        match self {
            Self::Verify => verify_profiled(pool).await.map(|_| ()),
            Self::Migrate => migrate_profiled(pool).await,
            Self::Prepare => {
                let database = profiled_database(pool).await;
                let result = prepare_online_indexes_for_external_migrator(&database).await;
                database.pool().close().await;
                result.map(|_| ())
            }
        }
    }
}

fn assert_problem(
    result: Result<(), SchemaCompatibilityError>,
    index: &str,
    expected: OnlineIndexProblem,
) {
    let SchemaCompatibilityError::OnlineIndex {
        index: actual_index,
        problem,
    } = result.expect_err("index problem")
    else {
        panic!("expected an online index classification")
    };
    assert_eq!(actual_index, index);
    assert_eq!(problem, expected);
}

async fn assert_active_reindex(entry: Entry) {
    let harness = Harness::fresh("online_reindex_active").await;
    let index = entry.seed_interrupted_index(&harness.pool).await;
    let history = sqlx_history(&harness.pool).await;
    let reindex = Reindex::start(&harness.pool, index).await;
    let replacement_oid = reindex.replacement_oid;
    let before = identity(&harness.pool, index).await;
    let outcome = tokio::time::timeout(Duration::from_secs(5), entry.run(&harness.pool)).await;
    assert_eq!(identity(&harness.pool, index).await, before);
    assert_eq!(sqlx_history(&harness.pool).await, history);
    reindex.finish(&harness.pool, index).await;
    entry
        .run(&harness.pool)
        .await
        .expect("settled index is adopted");
    assert_eq!(
        identity(&harness.pool, index).await,
        (replacement_oid, true)
    );
    assert_eq!(sqlx_history(&harness.pool).await, history);
    harness.teardown().await;
    assert_problem(
        outcome.expect("active reindex is refused without a competing DROP"),
        index,
        OnlineIndexProblem::BuildInProgress,
    );
}

#[tokio::test]
async fn verifier_defers_visible_concurrent_reindex_after_replacement_creation() {
    assert_active_reindex(Entry::Verify).await;
}

#[tokio::test]
async fn migration_defers_visible_concurrent_reindex_after_replacement_creation() {
    assert_active_reindex(Entry::Migrate).await;
}

#[tokio::test]
async fn preparation_defers_visible_concurrent_reindex_after_replacement_creation() {
    assert_active_reindex(Entry::Prepare).await;
}

#[tokio::test]
async fn visible_reindex_of_another_index_on_the_same_table_is_not_evidence() {
    let harness = Harness::fresh("online_reindex_unrelated").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("canonical schema");
    interrupt_build(&harness.pool, CREATED, CREATED_SQL).await;
    sqlx::raw_sql("CREATE INDEX unrelated_index ON job_queue (id)")
        .execute(&harness.pool)
        .await
        .expect("unrelated index on the same table");
    let history = sqlx_history(&harness.pool).await;
    let before = identity(&harness.pool, CREATED).await;
    let reindex = Reindex::start(&harness.pool, "unrelated_index").await;
    let result = Entry::Verify.run(&harness.pool).await;
    reindex.finish(&harness.pool, "unrelated_index").await;
    assert_eq!(identity(&harness.pool, CREATED).await, before);
    assert_eq!(sqlx_history(&harness.pool).await, history);
    harness.teardown().await;
    assert_problem(result, CREATED, OnlineIndexProblem::Invalid);
}

#[tokio::test]
async fn visible_reindex_of_a_foreign_definition_remains_foreign() {
    let harness = Harness::fresh("online_reindex_foreign").await;
    migrate_profiled(&harness.pool)
        .await
        .expect("canonical schema");
    interrupt_build(
        &harness.pool,
        CREATED,
        "CREATE INDEX CONCURRENTLY idx_job_queue_created_id ON job_queue (id)",
    )
    .await;
    let history = sqlx_history(&harness.pool).await;
    let reindex = Reindex::start(&harness.pool, CREATED).await;
    let result = Entry::Verify.run(&harness.pool).await;
    reindex.finish(&harness.pool, CREATED).await;
    assert_eq!(sqlx_history(&harness.pool).await, history);
    harness.teardown().await;
    assert_problem(result, CREATED, OnlineIndexProblem::DefinitionMismatch);
}
