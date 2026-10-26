//! Hidden progress from manual builders must not authorize interrupted-index repair.

use std::time::Duration;

use runledger_postgres::{
    OnlineIndexProblem, PgSessionProfile, RunledgerDatabase, SchemaCompatibilityError,
    ensure_schema_compatible_after_idempotency_cutover, migrate_after_idempotency_cutover,
    prepare_online_indexes_for_external_migrator,
};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::task::JoinHandle;

mod online_support;
use online_support::{
    Harness, LAST_VERSION_BEFORE_CUTOVER, hold_writer_lock, raw_apply_through, wait_for,
};

const CREATED_INDEX: &str = "idx_job_queue_created_id";
const CREATED_SQL: &str = "CREATE INDEX CONCURRENTLY idx_job_queue_created_id
    ON job_queue (created_at DESC, id DESC)";
const SNAPSHOT_INDEX: &str = "idx_job_queue_missing_enqueue_request_snapshot";
const SNAPSHOT_SQL: &str =
    "CREATE INDEX CONCURRENTLY idx_job_queue_missing_enqueue_request_snapshot
    ON job_queue (id) WHERE idempotency_key IS NOT NULL AND enqueue_request IS NULL";

struct RestrictedOwner {
    harness: Harness,
    database: RunledgerDatabase,
    role: String,
}

impl RestrictedOwner {
    async fn fresh() -> Self {
        let harness = Harness::fresh("online_hidden_progress").await;
        let role = format!("online_owner_{}", sqlx::types::Uuid::new_v4().simple());
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE ROLE {role}; GRANT USAGE, CREATE ON SCHEMA public TO {role}"
        )))
        .execute(&harness.pool)
        .await
        .expect("DDL-capable role without statistics privileges");
        let options = harness.pool.connect_options();
        let profile = PgSessionProfile::new(
            options.get_username(),
            &role,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .expect("restricted profile");
        let database = RunledgerDatabase::connect((*options).clone(), profile, 2)
            .await
            .expect("restricted database");
        Self {
            harness,
            database,
            role,
        }
    }

    async fn teardown(self) {
        self.database.pool().close().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP OWNED BY {0}; DROP ROLE {0}",
            self.role
        )))
        .execute(&self.harness.pool)
        .await
        .expect("remove test role and its objects");
        self.harness.teardown().await;
    }
}

async fn identity(pool: &PgPool, index: &str) -> (i64, bool) {
    sqlx::query_as(
        "SELECT indexrelid::bigint, indisvalid FROM pg_catalog.pg_index
         WHERE indexrelid = pg_catalog.to_regclass($1)",
    )
    .bind(index)
    .fetch_one(pool)
    .await
    .expect("actual index identity and validity")
}

async fn manual_builder(
    fixture: &RestrictedOwner,
    statement: &'static str,
    index: &str,
    table: &str,
) -> JoinHandle<Result<(), sqlx::Error>> {
    let pool = fixture.harness.pool.clone();
    let builder = tokio::spawn(async move {
        let mut conn = pool.acquire().await?;
        conn.close_on_drop();
        sqlx::raw_sql("SET statement_timeout = '30s'")
            .execute(&mut *conn)
            .await?;
        sqlx::raw_sql(statement).execute(&mut *conn).await?;
        Ok(())
    });
    wait_for("manual builder waiting for its writer", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
             WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
               AND index_relid = pg_catalog.to_regclass($1)
               AND command = 'CREATE INDEX CONCURRENTLY' AND phase LIKE 'waiting for%')",
        ).bind(index).fetch_one(&fixture.harness.pool).await.expect("privileged progress")
    }).await;
    let pid: i32 = sqlx::query_scalar(
        "SELECT pid FROM pg_catalog.pg_stat_progress_create_index
         WHERE datid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
           AND index_relid = pg_catalog.to_regclass($1)",
    )
    .bind(index)
    .fetch_one(&fixture.harness.pool)
    .await
    .expect("actual builder pid");
    let (hidden, relation_lock, no_advisory): (bool, bool, bool) = sqlx::query_as(
        "SELECT p.relid IS NULL AND p.index_relid IS NULL AND p.phase IS NULL,
            EXISTS (SELECT 1 FROM pg_catalog.pg_locks l
                    WHERE l.pid = p.pid AND l.database = p.datid AND l.locktype = 'relation'
                      AND l.relation = pg_catalog.to_regclass($2)
                      AND l.mode = 'ShareUpdateExclusiveLock' AND l.granted),
            NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks l
                        WHERE l.locktype = 'advisory' AND l.database = p.datid)
         FROM pg_catalog.pg_stat_progress_create_index p WHERE p.pid = $1",
    )
    .bind(pid)
    .bind(table)
    .fetch_one(fixture.database.pool())
    .await
    .expect("restricted observer sees only pid/database and relation locks");
    assert!(hidden && relation_lock && no_advisory);
    assert!(!identity(&fixture.harness.pool, index).await.1);
    builder
}

async fn finish(builder: JoinHandle<Result<(), sqlx::Error>>) {
    tokio::time::timeout(Duration::from_secs(10), builder)
        .await
        .expect("manual build completes after writer release")
        .expect("builder task")
        .expect("manual build");
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
async fn hidden_manual_build_is_transient_for_verification_and_migration() {
    let fixture = RestrictedOwner::fresh().await;
    migrate_after_idempotency_cutover(&fixture.database)
        .await
        .expect("restricted role owns the migrated schema");
    sqlx::raw_sql("DROP INDEX idx_job_queue_created_id")
        .execute(fixture.database.pool())
        .await
        .expect("observer has actual index DDL authority");
    let writer = hold_writer_lock(&fixture.harness.pool).await;
    let builder = manual_builder(&fixture, CREATED_SQL, CREATED_INDEX, "job_queue").await;
    let before = identity(&fixture.harness.pool, CREATED_INDEX).await;
    let verification = ensure_schema_compatible_after_idempotency_cutover(&fixture.database).await;
    let migration = tokio::time::timeout(
        Duration::from_secs(5),
        migrate_after_idempotency_cutover(&fixture.database),
    )
    .await;
    let during = identity(&fixture.harness.pool, CREATED_INDEX).await;
    let waiting = !builder.is_finished();
    writer.rollback().await.expect("release writer");
    finish(builder).await;
    ensure_schema_compatible_after_idempotency_cutover(&fixture.database)
        .await
        .expect("completed manual build verifies");
    let after = identity(&fixture.harness.pool, CREATED_INDEX).await;
    fixture.teardown().await;

    assert_problem(
        verification.expect_err("active build is transient"),
        CREATED_INDEX,
        OnlineIndexProblem::BuildInProgress,
    );
    assert_problem(
        migration
            .expect("no attempt to drop a still-building index")
            .expect_err("active build"),
        CREATED_INDEX,
        OnlineIndexProblem::BuildInProgress,
    );
    assert!(waiting);
    assert_eq!(during, before);
    assert_eq!(after, (before.0, true));
}

#[tokio::test]
async fn hidden_manual_build_is_transient_for_external_preparation() {
    let fixture = RestrictedOwner::fresh().await;
    raw_apply_through(fixture.database.pool(), LAST_VERSION_BEFORE_CUTOVER).await;
    let writer = hold_writer_lock(&fixture.harness.pool).await;
    let builder = manual_builder(&fixture, SNAPSHOT_SQL, SNAPSHOT_INDEX, "job_queue").await;
    let before = identity(&fixture.harness.pool, SNAPSHOT_INDEX).await;
    let preparation = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_online_indexes_for_external_migrator(&fixture.database),
    )
    .await;
    assert_eq!(
        identity(&fixture.harness.pool, SNAPSHOT_INDEX).await,
        before
    );
    assert!(!builder.is_finished());
    writer.rollback().await.expect("release writer");
    finish(builder).await;
    let report = prepare_online_indexes_for_external_migrator(&fixture.database)
        .await
        .expect("completed manual build is adoptable");
    assert!(report.already_valid().contains(&SNAPSHOT_INDEX));
    migrate_after_idempotency_cutover(&fixture.database)
        .await
        .expect("remaining migrations");
    ensure_schema_compatible_after_idempotency_cutover(&fixture.database)
        .await
        .expect("adopted manual build verifies");
    assert_eq!(
        identity(&fixture.harness.pool, SNAPSHOT_INDEX).await,
        (before.0, true)
    );
    fixture.teardown().await;
    assert_problem(
        preparation
            .expect("no drop during manual build")
            .expect_err("active build"),
        SNAPSHOT_INDEX,
        OnlineIndexProblem::BuildInProgress,
    );
}

#[tokio::test]
async fn hidden_build_defers_other_indexes_only_on_its_own_table() {
    for same_table in [true, false] {
        assert_hidden_build_correlation(same_table).await;
    }
}

async fn interrupt_target(fixture: &RestrictedOwner) -> (i64, bool) {
    migrate_after_idempotency_cutover(&fixture.database)
        .await
        .expect("canonical schema");
    sqlx::raw_sql(
        "DROP INDEX idx_job_queue_created_id; CREATE TABLE unrelated_index_work (id int)",
    )
    .execute(fixture.database.pool())
    .await
    .expect("prepare independent relations");
    let writer = hold_writer_lock(&fixture.harness.pool).await;
    let mut interrupted = fixture
        .harness
        .pool
        .acquire()
        .await
        .expect("interrupted builder");
    interrupted.close_on_drop();
    sqlx::raw_sql("SET statement_timeout = '250ms'")
        .execute(&mut *interrupted)
        .await
        .expect("bounded interruption");
    let error = sqlx::raw_sql(CREATED_SQL)
        .execute(&mut *interrupted)
        .await
        .expect_err("leave actual INVALID index");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    drop(interrupted);
    writer.rollback().await.expect("release target writer");
    let before = identity(&fixture.harness.pool, CREATED_INDEX).await;
    assert!(!before.1);
    before
}

async fn hold_other_writer(
    fixture: &RestrictedOwner,
    same_table: bool,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    if same_table {
        hold_writer_lock(&fixture.harness.pool).await
    } else {
        let mut writer = fixture
            .harness
            .pool
            .begin()
            .await
            .expect("unrelated writer");
        sqlx::raw_sql("INSERT INTO unrelated_index_work VALUES (1)")
            .execute(&mut *writer)
            .await
            .expect("hold unrelated writer");
        writer
    }
}

async fn assert_hidden_build_correlation(same_table: bool) {
    let fixture = RestrictedOwner::fresh().await;
    let before = interrupt_target(&fixture).await;
    let other_writer = hold_other_writer(&fixture, same_table).await;
    let (statement, table, problem) = if same_table {
        (
            "CREATE INDEX CONCURRENTLY other_work_idx ON job_queue (id)",
            "job_queue",
            OnlineIndexProblem::BuildInProgress,
        )
    } else {
        (
            "CREATE INDEX CONCURRENTLY other_work_idx ON unrelated_index_work (id)",
            "unrelated_index_work",
            OnlineIndexProblem::Invalid,
        )
    };
    let builder = manual_builder(&fixture, statement, "other_work_idx", table).await;
    let other_before = identity(&fixture.harness.pool, "other_work_idx").await;
    assert_problem(
        ensure_schema_compatible_after_idempotency_cutover(&fixture.database)
            .await
            .expect_err("target remains interrupted"),
        CREATED_INDEX,
        problem,
    );
    let migration = tokio::time::timeout(
        Duration::from_secs(5),
        migrate_after_idempotency_cutover(&fixture.database),
    )
    .await
    .expect("classify without waiting for the other builder");
    if same_table {
        assert_problem(
            migration.expect_err("defer same-table build"),
            CREATED_INDEX,
            OnlineIndexProblem::BuildInProgress,
        );
        assert_eq!(identity(&fixture.harness.pool, CREATED_INDEX).await, before);
    } else {
        migration.expect("repair target while unrelated build is active");
    }
    assert!(!builder.is_finished());
    assert_eq!(
        identity(&fixture.harness.pool, "other_work_idx").await,
        other_before
    );
    other_writer.rollback().await.expect("release other writer");
    finish(builder).await;
    assert_target_repair_after_other_build(&fixture, same_table, before).await;
    fixture.teardown().await;
}

async fn assert_target_repair_after_other_build(
    fixture: &RestrictedOwner,
    same_table: bool,
    before: (i64, bool),
) {
    if same_table {
        assert_problem(
            ensure_schema_compatible_after_idempotency_cutover(&fixture.database)
                .await
                .expect_err("completed other build leaves the target interrupted"),
            CREATED_INDEX,
            OnlineIndexProblem::Invalid,
        );
        migrate_after_idempotency_cutover(&fixture.database)
            .await
            .expect("repair after other build");
    }
    let repaired = identity(&fixture.harness.pool, CREATED_INDEX).await;
    assert!(repaired.1);
    assert_ne!(repaired.0, before.0);
    ensure_schema_compatible_after_idempotency_cutover(&fixture.database)
        .await
        .expect("repaired target verifies");
}
