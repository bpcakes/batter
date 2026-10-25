#![allow(
    dead_code,
    reason = "shared by the online-index integration tests, which use different subsets"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use runledger_postgres::{
    MIGRATOR, MigrationTimeoutSetting, PgSessionProfile, RunledgerDatabase,
    SchemaCompatibilityError,
};
use runledger_test_support::{
    EphemeralDatabase, setup_unmigrated_ephemeral_pool, teardown_ephemeral_pool,
};
use sqlx::migrate::Migrate;
use sqlx::{Connection, PgPool, Row};

pub const LAST_VERSION_BEFORE_SNAPSHOT_COLUMNS: i64 = 202604100001;
pub const LAST_VERSION_BEFORE_CUTOVER: i64 = 202605180001;
pub const LAST_VERSION_BEFORE_SUMMARY_PAGINATION: i64 = 202608240002;
pub const SUMMARY_PAGINATION_VERSION: i64 = 202609050001;
pub const DECLARED_INDEXES: [&str; 5] = [
    "idx_job_queue_missing_enqueue_request_snapshot",
    "idx_workflow_runs_missing_enqueue_request_snapshot",
    "idx_job_queue_execution_resource_claim_order",
    "idx_job_queue_scope_created_id",
    "idx_job_queue_created_id",
];

pub struct Harness {
    pub pool: PgPool,
    pub database: EphemeralDatabase,
}

impl Harness {
    pub async fn fresh(prefix: &str) -> Self {
        let (pool, database) = setup_unmigrated_ephemeral_pool(prefix, 4).await;
        Self { pool, database }
    }

    pub async fn teardown(self) {
        teardown_ephemeral_pool(self.pool, self.database).await;
    }
}

pub async fn raw_apply_through(pool: &PgPool, latest_version: i64) {
    let mut conn = pool.acquire().await.expect("raw migration connection");
    (*conn)
        .ensure_migrations_table("_sqlx_migrations")
        .await
        .expect("create sqlx history");
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(&mut *conn)
        .await
        .expect("applied versions");
    for migration in MIGRATOR
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
        .filter(|migration| migration.version <= latest_version)
        .filter(|migration| !applied.contains(&migration.version))
    {
        (*conn)
            .apply("_sqlx_migrations", migration)
            .await
            .unwrap_or_else(|error| panic!("raw apply {}: {error}", migration.version));
    }
}

pub async fn index_catalog(pool: &PgPool) -> BTreeMap<String, (String, bool)> {
    sqlx::query(
        "SELECT c.relname, pg_catalog.pg_get_indexdef(i.indexrelid), i.indisvalid
         FROM pg_catalog.pg_index i
         JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'",
    )
    .fetch_all(pool)
    .await
    .expect("index catalog")
    .into_iter()
    .map(|row| {
        (
            row.get::<String, _>(0),
            (row.get::<String, _>(1), row.get::<bool, _>(2)),
        )
    })
    .collect()
}

pub async fn sqlx_history(pool: &PgPool) -> Vec<(i64, Vec<u8>, bool)> {
    sqlx::query_as("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .expect("sqlx history")
}

pub async fn assert_declared_indexes_valid(pool: &PgPool, context: &str) {
    let catalog = index_catalog(pool).await;
    for name in DECLARED_INDEXES {
        let (_, valid) = catalog
            .get(name)
            .unwrap_or_else(|| panic!("{context}: {name} must exist"));
        assert!(valid, "{context}: {name} must be valid");
    }
}

/// Hold a writer-class lock on `job_queue` in an open transaction. A
/// concurrent build waits for it without queueing a table lock of its own; an
/// ordinary build queues a `SHARE` lock that stalls every later writer.
pub async fn hold_writer_lock(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut holder = pool.begin().await.expect("writer transaction");
    sqlx::raw_sql("LOCK TABLE job_queue IN ROW EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .expect("writer lock");
    holder
}

/// Try a writer-class lock acquisition under a bounded lock timeout.
pub async fn writer_can_lock(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut writer = pool.begin().await?;
    sqlx::raw_sql("SET LOCAL lock_timeout = '1500ms'; LOCK TABLE job_queue IN ROW EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await?;
    writer.commit().await
}

pub async fn wait_for<F, Fut>(description: &str, mut probe: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !probe().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Migrate through a profile with short statement and lock timeouts, as a
/// production pool would carry.
pub async fn migrate_with_short_timeouts(pool: &PgPool) -> Result<(), SchemaCompatibilityError> {
    migrate_with_profile_timeouts(pool, Duration::from_secs(2), Duration::from_millis(250)).await
}

/// Migrate through a profile with the given statement and lock timeouts.
pub async fn migrate_with_profile_timeouts(
    pool: &PgPool,
    statement_timeout: Duration,
    lock_timeout: Duration,
) -> Result<(), SchemaCompatibilityError> {
    let login = pool.connect_options().get_username().to_owned();
    let profile = PgSessionProfile::new(
        &login,
        &login,
        vec!["public".into()],
        statement_timeout,
        lock_timeout,
    )
    .expect("profile with explicit timeouts");
    migrate_through_profile(pool, profile).await
}

/// Migrate through a profile whose only nonzero bound is `transaction_timeout`,
/// which PostgreSQL 17 and later apply to each statement's implicit
/// transaction as well.
pub async fn migrate_with_transaction_timeout(
    pool: &PgPool,
    transaction_timeout: Duration,
) -> Result<(), SchemaCompatibilityError> {
    let login = pool.connect_options().get_username().to_owned();
    let profile = PgSessionProfile::with_timeouts(
        &login,
        &login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        transaction_timeout,
    )
    .expect("profile with only a transaction timeout");
    migrate_through_profile(pool, profile).await
}

async fn migrate_through_profile(
    pool: &PgPool,
    profile: PgSessionProfile,
) -> Result<(), SchemaCompatibilityError> {
    let options = pool.connect_options();
    let database = RunledgerDatabase::connect((*options).clone(), profile, 1)
        .await
        .expect("profiled database");
    let result = runledger_postgres::migrate_after_idempotency_cutover(&database).await;
    database.pool().close().await;
    result
}

/// Hold SQLx's own migration lock on a separate connection and require
/// `migrate` to give up with a typed setting and duration before release.
pub async fn assert_lock_wait_expires<F, Fut>(
    harness: &Harness,
    setting: MigrationTimeoutSetting,
    bound: Duration,
    migrate: F,
) where
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<(), SchemaCompatibilityError>> + Send + 'static,
{
    let mut holder = sqlx::PgConnection::connect(harness.database.url())
        .await
        .expect("holder connection");
    holder.lock().await.expect("SQLx migration lock");

    let started = tokio::time::Instant::now();
    let error = match tokio::time::timeout(
        Duration::from_secs(10),
        tokio::spawn(migrate(harness.pool.clone())),
    )
    .await
    .expect("the bounded wait must not hang")
    .expect("runner task")
    {
        Err(error) => error,
        Ok(()) => panic!("a nonzero {setting:?} must bound the wait for a held lock"),
    };
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "expiry must follow the profile bound, not the test deadline"
    );
    let SchemaCompatibilityError::MigrationLockTimeout(expiry) = error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(expiry.setting(), setting);
    assert_eq!(expiry.bound(), bound);
    assert!(started.elapsed() >= bound);

    holder.unlock().await.expect("release SQLx lock");
    holder.close().await.expect("close holder");
}

/// Whether a concurrent build on `job_queue` is currently waiting for writers
/// or older snapshots.
pub async fn build_waits_for_writers(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
            SELECT 1 FROM pg_catalog.pg_stat_progress_create_index
            WHERE command = 'CREATE INDEX CONCURRENTLY'
              AND relid = 'job_queue'::regclass
              AND phase LIKE 'waiting for%'
         )",
    )
    .fetch_one(pool)
    .await
    .expect("progress view")
}
