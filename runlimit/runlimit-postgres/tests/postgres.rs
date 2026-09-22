//! Opt-in integration tests against a real `PostgreSQL` database.

use std::{
    process,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use runlimit_core::{
    AdmissionOutcome, Allowance, BatchDecision, BatchDecisionView, Capacity, Check, CleanupOutcome,
    ConsumptionStatus, Decision, DecisionView, Denial, FixedWindowPolicy, Limiter, MAX_LIMIT,
    MAX_WINDOW, MAX_WINDOW_MILLIS, Observation, Observer, PolicyId, QuotaDenial, QuotaMode,
    ScopeId, SubjectKey,
};
use runlimit_memory::{Clock, MemoryStore, MemoryStoreConfig};
use runlimit_postgres::{
    BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL, CREATE_RUNLIMIT_FIXED_WINDOWS_SQL, CheckError,
    HARD_MAX_ROWS_PER_SHARD, MIGRATOR, MaintenanceError, PostgresConfig, PostgresLimiter,
    SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL,
};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, PgPool, migrate::Migrate, postgres::PgPoolOptions};
use tokio::{sync::Barrier, time::sleep};

const TEST_DATABASE_URL: &str = "RUNLIMIT_POSTGRES_TEST_DATABASE_URL";
const CLEANUP_SQL: &str = include_str!("../src/cleanup_expired.sql");
const TEST_HOST_MIGRATION_VERSION: i64 = 20_260_722_000_000;
const PUBLISHED_CREATE_MIGRATION_VERSION: i64 = 20_260_723_000_000;
const FILLFACTOR_MIGRATION_VERSION: i64 = 20_260_725_000_000;
const CARDINALITY_MIGRATION_VERSION: i64 = 20_260_726_000_000;
const ADVISORY_LOCK_DOMAIN: &[u8] = b"runlimit/postgres-advisory-lock/v1\0";

const PUBLISHED_CREATE_MIGRATION_SHA384: [u8; 48] = [
    0x31, 0xd9, 0x3f, 0xde, 0x98, 0xc2, 0x36, 0x4a, 0x33, 0x06, 0x2e, 0x37, 0x35, 0x08, 0xf5, 0x63,
    0xb4, 0x86, 0xdf, 0x28, 0x98, 0xf8, 0xe7, 0x73, 0x65, 0x91, 0x8e, 0x81, 0xda, 0x89, 0x80, 0xa8,
    0xe7, 0x4b, 0xf9, 0xc6, 0x30, 0x22, 0xd2, 0x51, 0xae, 0xb3, 0xd5, 0x80, 0x67, 0xe9, 0x80, 0xec,
];
static NEXT_POLICY: AtomicU64 = AtomicU64::new(0);

fn capacity(value: u64) -> Capacity {
    Capacity::new(value).expect("test capacity is valid")
}

fn allowance(capacity_value: u64, available: u64, replenishes_after: Duration) -> Allowance {
    Allowance::new(capacity(capacity_value), available, replenishes_after)
        .expect("test allowance is valid")
}

fn quota(capacity_value: u64, retry_after: Duration) -> QuotaDenial {
    QuotaDenial::new(capacity(capacity_value), retry_after)
}

fn storage_capacity() -> Denial {
    Denial::StorageCapacity { retry_after: None }
}

fn available(decision: &Decision) -> u64 {
    match decision.view() {
        DecisionView::Allowed { allowance } => allowance.available(),
        DecisionView::Denied { .. } | DecisionView::ShadowDenied { .. } => {
            panic!("expected an allowed decision, got {decision:?}")
        }
    }
}

/// Describes an outcome without its backend-measured durations.
fn shape(decision: &Decision) -> String {
    match decision.view() {
        DecisionView::Allowed { allowance } => format!(
            "allowed {} of {}",
            allowance.available(),
            allowance.capacity()
        ),
        DecisionView::Denied {
            denial: Denial::QuotaExceeded(quota),
        } => format!("quota denied at {}", quota.capacity()),
        DecisionView::Denied {
            denial: Denial::StorageCapacity { .. },
        } => "capacity denied".to_owned(),
        DecisionView::ShadowDenied { denial } => format!("shadow denied at {}", denial.capacity()),
    }
}

#[derive(Default)]
struct RecordingObserver {
    admissions: Mutex<Vec<(AdmissionOutcome, ConsumptionStatus)>>,
    cleanups: Mutex<Vec<(usize, CleanupOutcome)>>,
}

impl Observer for RecordingObserver {
    fn observe(&self, observation: &Observation<'_>) {
        match observation {
            Observation::Admission(admission) => self
                .admissions
                .lock()
                .unwrap()
                .push((admission.outcome(), admission.consumption())),
            Observation::Cleanup(cleanup) => self
                .cleanups
                .lock()
                .unwrap()
                .push((cleanup.requested(), cleanup.outcome())),
            // The PostgreSQL backend has no local capacity to report.
            Observation::Capacity(_) => {}
        }
    }
}

#[test]
fn published_create_migration_is_immutable_and_later_migrations_are_additive() {
    let migrations = MIGRATOR.iter().collect::<Vec<_>>();

    assert!(migrations.len() >= 3);
    assert_eq!(migrations[0].version, PUBLISHED_CREATE_MIGRATION_VERSION);
    assert_eq!(migrations[1].version, FILLFACTOR_MIGRATION_VERSION);
    assert_eq!(migrations[2].version, CARDINALITY_MIGRATION_VERSION);
    assert_eq!(
        migrations[0].checksum.as_ref(),
        PUBLISHED_CREATE_MIGRATION_SHA384
    );
    assert_eq!(
        migrations[0].sql.as_ref(),
        CREATE_RUNLIMIT_FIXED_WINDOWS_SQL
    );
    assert_eq!(
        migrations[1].sql.as_ref(),
        SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL
    );
    assert_eq!(
        migrations[2].sql.as_ref(),
        BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL
    );
    assert!(!CREATE_RUNLIMIT_FIXED_WINDOWS_SQL.contains("fillfactor"));
    assert!(SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL.contains("SET (fillfactor = 80)"));
    assert!(BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL.contains("GENERATED ALWAYS"));
    assert!(BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL.contains("65536"));
    assert!(
        BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL
            .contains("runlimit_fixed_windows_immutable_storage_key")
    );
}

async fn test_pool(maximum_connections: u32) -> PgPool {
    let database_url = std::env::var(TEST_DATABASE_URL)
        .expect("RUNLIMIT_POSTGRES_TEST_DATABASE_URL is required for ignored integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(maximum_connections)
        .connect(&database_url)
        .await
        .expect("connect to integration-test PostgreSQL");
    PostgresLimiter::new(pool.clone())
        .migrate()
        .await
        .expect("apply Runlimit migrations");
    pool
}

// Dynamic SQL below interpolates only fixture-generated schema identifiers and
// checked-in SQL. Values continue to use bind parameters. AssertSqlSafe marks
// these audited statements for SQLx.
struct IsolatedSchema {
    admin_pool: PgPool,
    primary_pool: PgPool,
    schema: String,
    additional_pools: Vec<PgPool>,
}

impl IsolatedSchema {
    async fn for_migration_test(maximum_connections: u32) -> Self {
        let admin_pool = Self::admin_pool(2).await;
        Self::create(admin_pool, "migration", maximum_connections).await
    }

    async fn for_cleanup_test(maximum_connections: u32) -> Self {
        // Keep applying the bundled migrations in the default schema: these
        // tests also cover coexistence with an already-migrated application.
        let admin_pool = test_pool(1).await;
        let fixture = Self::create(admin_pool, "cleanup", maximum_connections).await;
        sqlx::raw_sql(include_str!(
            "../migrations/20260723000000_create_runlimit_fixed_windows.sql"
        ))
        .execute(&fixture.primary_pool)
        .await
        .expect("create isolated cleanup-test table");
        sqlx::raw_sql(BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL)
            .execute(&fixture.primary_pool)
            .await
            .expect("install isolated cardinality ledger");
        fixture
    }

    async fn admin_pool(maximum_connections: u32) -> PgPool {
        let database_url = std::env::var(TEST_DATABASE_URL).expect(
            "RUNLIMIT_POSTGRES_TEST_DATABASE_URL is required for ignored integration tests",
        );
        PgPoolOptions::new()
            .max_connections(maximum_connections)
            .connect(&database_url)
            .await
            .expect("connect isolated-schema admin pool")
    }

    async fn create(admin_pool: PgPool, label: &str, maximum_connections: u32) -> Self {
        let schema_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let schema = format!("runlimit_{label}_{}_{}", process::id(), schema_suffix);
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create isolated test schema");
        let primary_pool = Self::schema_bound_pool(&schema, maximum_connections).await;

        Self {
            admin_pool,
            primary_pool,
            schema,
            additional_pools: Vec::new(),
        }
    }

    async fn schema_bound_pool(schema: &str, maximum_connections: u32) -> PgPool {
        let database_url = std::env::var(TEST_DATABASE_URL).expect(
            "RUNLIMIT_POSTGRES_TEST_DATABASE_URL is required for ignored integration tests",
        );
        let connection_schema = schema.to_owned();
        let pool = PgPoolOptions::new()
            .max_connections(maximum_connections)
            .after_connect(move |connection, _metadata| {
                let search_path_sql = format!("SET search_path = {connection_schema}, pg_catalog");
                Box::pin(async move {
                    sqlx::query(AssertSqlSafe(search_path_sql))
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("connect isolated-schema pool");
        let search_path = sqlx::query_scalar::<_, Vec<String>>("SELECT current_schemas(false)")
            .fetch_one(&pool)
            .await
            .expect("read isolated-schema search path");
        assert_eq!(search_path, [schema, "pg_catalog"]);
        pool
    }

    async fn additional_pool(&mut self, maximum_connections: u32) -> PgPool {
        let pool = Self::schema_bound_pool(&self.schema, maximum_connections).await;
        self.additional_pools.push(pool.clone());
        pool
    }

    async fn teardown(self) {
        for pool in self.additional_pools {
            pool.close().await;
            assert!(
                pool.is_closed(),
                "additional pool must close before schema drop"
            );
        }
        self.primary_pool.close().await;
        assert!(
            self.primary_pool.is_closed(),
            "primary pool must close before schema drop"
        );
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.admin_pool)
        .await
        .expect("drop isolated test schema");
        self.admin_pool.close().await;
        assert!(self.admin_pool.is_closed(), "admin pool must close last");
    }
}

async fn run_test_host_migrations(pool: &PgPool) {
    let mut migrator = sqlx::migrate!("./tests/host-migrations");
    migrator.set_ignore_missing(true);
    migrator
        .run(pool)
        .await
        .expect("apply host migrations configured to ignore unrelated versions");
}

async fn counter_table_options(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        r"
SELECT COALESCE(reloptions, ARRAY[]::TEXT[])
FROM pg_catalog.pg_class
WHERE oid = 'runlimit_fixed_windows'::regclass
",
    )
    .fetch_one(pool)
    .await
    .expect("read counter table storage options")
}

fn unique_policy(label: &str, limit: u64, window: Duration) -> FixedWindowPolicy {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let sequence = NEXT_POLICY.fetch_add(1, Ordering::Relaxed);
    FixedWindowPolicy::new(
        PolicyId::new(format!(
            "test.{label}.{}.{}.{}",
            process::id(),
            timestamp,
            sequence
        ))
        .expect("generated policy identifier is valid"),
        ScopeId::new("subject").expect("static scope identifier is valid"),
        limit,
        window,
    )
    .expect("generated fixed-window policy is valid")
}

fn key(value: u8) -> SubjectKey {
    SubjectKey::from_digest([value; 32])
}

fn key_in_capacity_shard(
    policy: &FixedWindowPolicy,
    capacity_shard: u8,
    discriminator: u8,
) -> SubjectKey {
    let mut digest = [discriminator; 32];
    digest[0] = policy.fingerprint().as_bytes()[0] ^ capacity_shard;
    SubjectKey::from_digest(digest)
}

fn capacity_shard_for(policy: &FixedWindowPolicy, subject: SubjectKey) -> i16 {
    i16::from(policy.fingerprint().as_bytes()[0] ^ subject.as_bytes()[0])
}

async fn capacity_row_count(pool: &PgPool, capacity_shard: i16) -> i64 {
    sqlx::query_scalar(
        r"
SELECT row_count
FROM runlimit_capacity_shards
WHERE capacity_shard = $1
",
    )
    .bind(capacity_shard)
    .fetch_one(pool)
    .await
    .expect("read capacity ledger row")
}

fn advisory_lock_id(check: &Check<'_>) -> i64 {
    let mut digest = Sha256::new();
    digest.update(ADVISORY_LOCK_DOMAIN);
    digest.update(check.counter_key().to_bytes());
    let digest = digest.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(id_bytes)
}

fn test_config() -> PostgresConfig {
    PostgresConfig::new()
        .with_operation_timeout(Duration::from_secs(10))
        .expect("test timeout is valid")
}

#[derive(Clone, Default)]
struct ConformanceClock {
    now_millis: Arc<AtomicU64>,
}

impl ConformanceClock {
    fn set(&self, now_millis: u64) {
        self.now_millis.store(now_millis, Ordering::Relaxed);
    }
}

impl Clock for ConformanceClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.now_millis.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug)]
struct TransitionStep {
    name: &'static str,
    at_millis: u64,
    cost: u64,
    expected: Decision,
}

fn fixed_window_transitions() -> [TransitionStep; 6] {
    [
        TransitionStep {
            name: "new window",
            at_millis: 0,
            cost: 2,
            expected: Decision::allowed(allowance(5, 3, Duration::from_secs(10))),
        },
        TransitionStep {
            name: "live increment",
            at_millis: 3_000,
            cost: 2,
            expected: Decision::allowed(allowance(5, 1, Duration::from_secs(7))),
        },
        TransitionStep {
            name: "live denial",
            at_millis: 3_000,
            cost: 2,
            expected: Decision::denied(quota(5, Duration::from_secs(7))),
        },
        TransitionStep {
            name: "exact fill after denial",
            at_millis: 9_000,
            cost: 1,
            expected: Decision::allowed(allowance(5, 0, Duration::from_secs(1))),
        },
        TransitionStep {
            name: "full-window denial",
            at_millis: 9_000,
            cost: 1,
            expected: Decision::denied(quota(5, Duration::from_secs(1))),
        },
        TransitionStep {
            name: "exact-expiry renewal",
            at_millis: 10_000,
            cost: 3,
            expected: Decision::allowed(allowance(5, 2, Duration::from_secs(10))),
        },
    ]
}

async fn delete_counter(pool: &PgPool, policy: &FixedWindowPolicy, subject: SubjectKey) -> u64 {
    sqlx::query(
        r"
DELETE FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .execute(pool)
    .await
    .expect("delete test counter")
    .rows_affected()
}

async fn stored_counter_usage(
    pool: &PgPool,
    policy: &FixedWindowPolicy,
    subject: SubjectKey,
) -> i64 {
    sqlx::query_scalar(
        r"
SELECT used
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(pool)
    .await
    .expect("read test counter usage")
}

async fn wait_for_advisory_waiters(pool: &PgPool, query_fragment: &str, minimum: i64) {
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            r"
SELECT count(*)
FROM pg_stat_activity
WHERE
    datname = current_database()
    AND state = 'active'
    AND wait_event_type = 'Lock'
    AND wait_event = 'advisory'
    AND position($1 IN query) > 0
",
        )
        .bind(query_fragment)
        .fetch_one(pool)
        .await
        .expect("inspect advisory-lock waiters");
        if waiting >= minimum {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("did not observe {minimum} advisory-lock waiter(s) running {query_fragment:?}");
}

async fn insert_counter_window(
    pool: &PgPool,
    policy: &FixedWindowPolicy,
    subject: SubjectKey,
    starts_after_seconds: i64,
    expires_after_seconds: i64,
) {
    sqlx::query(
        r"
INSERT INTO runlimit_fixed_windows (
    policy_id,
    scope_id,
    config_fingerprint,
    subject_key,
    window_started_at,
    window_expires_at,
    used
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    clock_timestamp() + $5::BIGINT * INTERVAL '1 second',
    clock_timestamp() + $6::BIGINT * INTERVAL '1 second',
    1
)
",
    )
    .bind(policy.id().as_str())
    .bind(policy.scope().as_str())
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .bind(starts_after_seconds)
    .bind(expires_after_seconds)
    .execute(pool)
    .await
    .expect("insert counter with a controlled window");
}

async fn counter_exists(pool: &PgPool, policy: &FixedWindowPolicy, subject: SubjectKey) -> bool {
    sqlx::query_scalar(
        r"
SELECT EXISTS (
    SELECT 1
    FROM runlimit_fixed_windows
    WHERE
        config_fingerprint = $1
        AND subject_key = $2
)
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(pool)
    .await
    .expect("check whether the test counter exists")
}

async fn create_pool_budget_delay_triggers(setup_pool: &PgPool, schema: &str) {
    let trigger_sql = format!(
        r"
CREATE FUNCTION {schema}.sleep_during_admission()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    PERFORM pg_catalog.pg_sleep(0.1);
    RETURN NEW;
END
$function$;

CREATE TRIGGER sleep_during_admission
BEFORE INSERT ON {schema}.runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION {schema}.sleep_during_admission();

CREATE FUNCTION {schema}.sleep_during_cleanup()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    PERFORM pg_catalog.pg_sleep(0.1);
    RETURN OLD;
END
$function$;

CREATE TRIGGER sleep_during_cleanup
BEFORE DELETE ON {schema}.runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION {schema}.sleep_during_cleanup();
"
    );
    sqlx::raw_sql(AssertSqlSafe(trigger_sql))
        .execute(setup_pool)
        .await
        .expect("create delayed admission and cleanup triggers");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn cleanup_uses_an_indexable_cutoff_and_skips_locked_rows() {
    let fixture = IsolatedSchema::for_cleanup_test(4).await;
    let pool = fixture.primary_pool.clone();
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let locked_policy = unique_policy("cleanup-locked-expired", 1, Duration::from_secs(1));
    let expired_policy = unique_policy("cleanup-expired", 1, Duration::from_secs(1));
    let active_policy = unique_policy("cleanup-active", 1, Duration::from_hours(1));
    let locked_subject = key(151);
    let expired_subject = key(152);
    let active_subject = key(153);

    let mut plan_transaction = pool.begin().await.expect("begin plan transaction");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *plan_transaction)
        .await
        .expect("make the cleanup access path deterministic");
    let plan_sql = format!("EXPLAIN (COSTS OFF) {CLEANUP_SQL}");
    let plan = sqlx::query_scalar::<_, String>(AssertSqlSafe(plan_sql))
        .bind(1_i64)
        .fetch_all(&mut *plan_transaction)
        .await
        .expect("explain the production cleanup statement");
    plan_transaction
        .rollback()
        .await
        .expect("finish plan transaction");
    let plan = plan.join("\n");
    assert!(
        plan.contains("Index Scan using runlimit_fixed_windows_expiry_idx"),
        "cleanup did not use the expiry index:\n{plan}"
    );
    assert!(
        plan.lines()
            .any(|line| line.contains("Index Cond:") && line.contains("window_expires_at")),
        "cleanup did not use expiry as an index condition:\n{plan}"
    );

    insert_counter_window(&pool, &locked_policy, locked_subject, -7_200, -3_600).await;
    insert_counter_window(&pool, &expired_policy, expired_subject, -3_600, -1_800).await;
    insert_counter_window(&pool, &active_policy, active_subject, 0, 3_600).await;

    let mut blocker = pool.begin().await.expect("begin cleanup blocker");
    sqlx::query(
        r"
SELECT 1
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
FOR UPDATE
",
    )
    .bind(locked_policy.fingerprint().as_bytes().as_slice())
    .bind(locked_subject.as_bytes().as_slice())
    .fetch_one(&mut *blocker)
    .await
    .expect("lock the oldest expired counter");

    let first_cleanup = limiter
        .cleanup_expired(1)
        .await
        .expect("cleanup skips the locked row");
    assert_eq!(first_cleanup, 1);
    assert!(counter_exists(&pool, &locked_policy, locked_subject).await);
    assert!(!counter_exists(&pool, &expired_policy, expired_subject).await);
    assert!(counter_exists(&pool, &active_policy, active_subject).await);

    let active_only = limiter
        .cleanup_expired(1)
        .await
        .expect("cleanup ignores active rows");
    assert_eq!(active_only, 0);

    blocker.commit().await.expect("unlock expired counter");
    let unlocked_cleanup = limiter
        .cleanup_expired(1)
        .await
        .expect("cleanup removes the unlocked expired row");
    assert_eq!(unlocked_cleanup, 1);
    assert!(!counter_exists(&pool, &locked_policy, locked_subject).await);
    assert!(counter_exists(&pool, &active_policy, active_subject).await);

    let deleted_active = delete_counter(&pool, &active_policy, active_subject).await;
    drop(limiter);
    fixture.teardown().await;
    assert_eq!(deleted_active, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn cleanup_timeout_cannot_late_commit_and_releases_pool_slot() {
    let fixture = IsolatedSchema::for_cleanup_test(1).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("cleanup-timeout", 1, Duration::from_secs(1));
    let subject = key(154);
    insert_counter_window(&pool, &policy, subject, -7_200, -3_600).await;

    let trigger_sql = format!(
        r"
CREATE FUNCTION {schema}.sleep_before_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    PERFORM pg_catalog.pg_sleep(1);
    RETURN OLD;
END
$function$;

CREATE TRIGGER sleep_before_delete
BEFORE DELETE ON {schema}.runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION {schema}.sleep_before_delete();
"
    );
    sqlx::raw_sql(AssertSqlSafe(trigger_sql))
        .execute(&setup_pool)
        .await
        .expect("create delayed cleanup trigger");

    let short_config = PostgresConfig::new()
        .with_operation_timeout(Duration::from_millis(100))
        .expect("short nonzero timeout is valid");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(short_config);
    let error = limiter
        .cleanup_expired(1)
        .await
        .expect_err("delayed cleanup exceeds its operation deadline");
    assert!(matches!(
        error,
        MaintenanceError::TimedOutBeforeCommit { .. }
    ));
    assert!(!error.may_have_removed_rows());

    let probe: i32 = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_scalar("SELECT 1").fetch_one(&pool),
    )
    .await
    .expect("timed-out cleanup releases the only pool slot")
    .expect("replacement pool query succeeds");
    assert_eq!(probe, 1);

    let cancellation_config = PostgresConfig::new()
        .with_operation_timeout(Duration::from_secs(5))
        .expect("long cleanup timeout is valid");
    let cancellable_limiter = PostgresLimiter::new(pool.clone()).with_config(cancellation_config);
    let cancelled = tokio::time::timeout(
        Duration::from_millis(100),
        cancellable_limiter.cleanup_expired(1),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "outer deadline must cancel cleanup before its configured deadline"
    );

    let probe_after_cancellation: i32 = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_scalar("SELECT 1").fetch_one(&pool),
    )
    .await
    .expect("cancelled cleanup releases the only pool slot")
    .expect("pool query after cancellation succeeds");
    assert_eq!(probe_after_cancellation, 1);

    // Observe through an independent connection after the injected statement
    // would have completed if either deadline had only dropped its query
    // future.
    sleep(Duration::from_millis(1_100)).await;
    let row_exists_sql = format!(
        r"
SELECT EXISTS (
    SELECT 1
    FROM {schema}.runlimit_fixed_windows
    WHERE
        config_fingerprint = $1
        AND subject_key = $2
)
"
    );
    let row_exists: bool = sqlx::query_scalar(AssertSqlSafe(row_exists_sql))
        .bind(policy.fingerprint().as_bytes().as_slice())
        .bind(subject.as_bytes().as_slice())
        .fetch_one(&setup_pool)
        .await
        .expect("observe cleanup row through an independent connection");
    assert!(row_exists, "timed-out cleanup must not commit later");

    drop(limiter);
    drop(cancellable_limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn pool_wait_does_not_spend_admission_or_cleanup_operation_budget() {
    let fixture = IsolatedSchema::for_cleanup_test(1).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("pool-budget-survivor", 1, Duration::from_mins(1));
    let subject = key(155);
    create_pool_budget_delay_triggers(&setup_pool, &schema).await;

    let config = PostgresConfig::new()
        .with_pool_acquire_timeout(Duration::from_secs(1))
        .expect("one-second pool budget is valid")
        .with_operation_timeout(Duration::from_millis(250))
        .expect("250-millisecond operation budget is valid");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(config);

    let held_connection = pool
        .acquire()
        .await
        .expect("hold the only pool connection before admission");
    let admission_barrier = Arc::new(Barrier::new(2));
    let waiting_barrier = Arc::clone(&admission_barrier);
    let waiting_limiter = limiter.clone();
    let waiting_policy = policy.clone();
    let admission = tokio::spawn(async move {
        waiting_barrier.wait().await;
        waiting_limiter
            .check(&Check::new(subject.bind(&waiting_policy)))
            .await
    });
    admission_barrier.wait().await;
    sleep(Duration::from_millis(200)).await;
    drop(held_connection);

    let decision = admission
        .await
        .expect("admission task does not panic")
        .expect("admission receives a fresh operation budget after pool wait");
    assert!(decision.permits_request());
    assert_eq!(available(&decision), 0);

    sqlx::query(
        r"
UPDATE runlimit_fixed_windows
SET
    window_started_at = pg_catalog.clock_timestamp() - INTERVAL '2 seconds',
    window_expires_at = pg_catalog.clock_timestamp() - INTERVAL '1 second'
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("expire the admitted counter before cleanup");

    let held_connection = pool
        .acquire()
        .await
        .expect("hold the only pool connection before cleanup");
    let cleanup_barrier = Arc::new(Barrier::new(2));
    let waiting_barrier = Arc::clone(&cleanup_barrier);
    let waiting_limiter = limiter.clone();
    let cleanup = tokio::spawn(async move {
        waiting_barrier.wait().await;
        waiting_limiter.cleanup_expired(1).await
    });
    cleanup_barrier.wait().await;
    sleep(Duration::from_millis(200)).await;
    drop(held_connection);

    let removed = cleanup
        .await
        .expect("cleanup task does not panic")
        .expect("cleanup receives a fresh operation budget after pool wait");
    assert_eq!(removed, 1);
    assert!(!counter_exists(&pool, &policy, subject).await);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn migration_coexists_with_an_ignore_missing_host_and_discards_errors() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    let pool = fixture.primary_pool.clone();
    let limiter = PostgresLimiter::new(pool.clone());

    // Both migrators must opt in to a shared SQLx history. Exercise both run
    // orders twice so each migrator sees the other's already-applied version.
    run_test_host_migrations(&pool).await;
    limiter
        .migrate()
        .await
        .expect("Runlimit accepts the host migration version");
    run_test_host_migrations(&pool).await;
    limiter
        .migrate()
        .await
        .expect("Runlimit migration remains repeatable");

    let mut expected_versions = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .collect::<Vec<_>>();
    let applied_versions =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("read shared migration history");
    expected_versions.push(TEST_HOST_MIGRATION_VERSION);
    expected_versions.sort_unstable();
    assert_eq!(applied_versions, expected_versions);

    let tables_exist = sqlx::query_as::<_, (bool, bool)>(
        r"
SELECT
    to_regclass('runlimit_test_host_marker') IS NOT NULL,
    to_regclass('runlimit_fixed_windows') IS NOT NULL
",
    )
    .fetch_one(&pool)
    .await
    .expect("check both migration products");
    assert_eq!(tables_exist, (true, true));

    // A checksum error occurs after SQLx takes its session advisory lock. The
    // failed migration connection must be closed instead of returned locked to
    // this single-connection pool.
    sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
        .bind(vec![0_u8; 48])
        .bind(PUBLISHED_CREATE_MIGRATION_VERSION)
        .execute(&pool)
        .await
        .expect("corrupt Runlimit checksum for the failure-path test");
    let locked_backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .expect("read migration backend PID");

    let error = limiter
        .migrate()
        .await
        .expect_err("modified migration checksum must fail");
    assert!(matches!(
        error,
        sqlx::migrate::MigrateError::VersionMismatch(version)
            if version == PUBLISHED_CREATE_MIGRATION_VERSION
    ));

    let replacement_backend: i32 = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool),
    )
    .await
    .expect("failed migration releases the only pool slot")
    .expect("replacement connection is usable");
    assert_ne!(
        replacement_backend, locked_backend,
        "the possibly locked migration session must not return to the pool"
    );

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn migration_upgrades_the_published_0_1_table_additively() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    let pool = fixture.primary_pool.clone();
    let existing_policy = unique_policy("migration-capacity-backfill", 2, Duration::from_mins(1));
    let existing_subject = key(154);
    let published_create_migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == PUBLISHED_CREATE_MIGRATION_VERSION)
        .expect("published create migration remains bundled");

    let mut connection = pool
        .acquire()
        .await
        .expect("acquire published-migration setup connection");
    connection
        .ensure_migrations_table("_sqlx_migrations")
        .await
        .expect("create published migration history");
    connection
        .apply("_sqlx_migrations", published_create_migration)
        .await
        .expect("apply published create migration");
    drop(connection);
    insert_counter_window(&pool, &existing_policy, existing_subject, -1, 60).await;

    assert!(
        !counter_table_options(&pool)
            .await
            .iter()
            .any(|option| option.starts_with("fillfactor=")),
        "the published 0.1.0 migration unexpectedly contains storage tuning"
    );

    let limiter = PostgresLimiter::new(pool.clone());
    limiter
        .migrate()
        .await
        .expect("apply additive migrations to the published schema");

    let applied_versions =
        sqlx::query_scalar::<_, i64>("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("read upgraded migration history");
    let expected_versions = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .collect::<Vec<_>>();
    assert_eq!(applied_versions, expected_versions);
    let table_options = counter_table_options(&pool).await;
    assert!(
        table_options.iter().any(|option| option == "fillfactor=80"),
        "upgraded table options do not reserve space for HOT updates: {table_options:?}"
    );
    let expected_shard = capacity_shard_for(&existing_policy, existing_subject);
    let stored_shard: i16 = sqlx::query_scalar(
        r"
SELECT capacity_shard
FROM runlimit_fixed_windows
WHERE config_fingerprint = $1 AND subject_key = $2
",
    )
    .bind(existing_policy.fingerprint().as_bytes().as_slice())
    .bind(existing_subject.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("read generated shard for pre-migration row");
    assert_eq!(stored_shard, expected_shard);
    assert_eq!(capacity_row_count(&pool, expected_shard).await, 1);
    assert_eq!(
        delete_counter(&pool, &existing_policy, existing_subject).await,
        1
    );
    assert_eq!(capacity_row_count(&pool, expected_shard).await, 0);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn cancelling_migration_discards_its_session_lock() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    run_test_host_migrations(&pool).await;
    let migration_backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .expect("read migration backend PID");

    // Block history inspection after SQLx has acquired its session-scoped
    // migration lock, then observe that lock before cancelling the future.
    let mut blocker = setup_pool
        .begin()
        .await
        .expect("begin migration metadata blocker");
    sqlx::query(AssertSqlSafe(format!(
        "LOCK TABLE {schema}._sqlx_migrations IN ACCESS EXCLUSIVE MODE"
    )))
    .execute(&mut *blocker)
    .await
    .expect("block migration metadata access");

    let limiter = PostgresLimiter::new(pool.clone());
    let migration_task = tokio::spawn({
        let limiter = limiter.clone();
        async move { limiter.migrate().await }
    });

    let mut observed_session_lock = false;
    for _ in 0..100 {
        observed_session_lock = sqlx::query_scalar(
            r"
SELECT EXISTS (
    SELECT 1
    FROM pg_catalog.pg_locks
    WHERE
        pid = $1
        AND locktype = 'advisory'
        AND granted
)
",
        )
        .bind(migration_backend)
        .fetch_one(&setup_pool)
        .await
        .expect("inspect migration advisory lock");
        if observed_session_lock {
            break;
        }
        assert!(
            !migration_task.is_finished(),
            "migration unexpectedly finished while metadata was locked"
        );
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        observed_session_lock,
        "migration never reached its session advisory lock"
    );

    migration_task.abort();
    let cancelled = migration_task
        .await
        .expect_err("aborted migration task must be cancelled");
    assert!(cancelled.is_cancelled());
    blocker
        .rollback()
        .await
        .expect("release migration metadata blocker");

    let replacement_backend: i32 = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool),
    )
    .await
    .expect("cancelled migration releases the only pool slot")
    .expect("replacement connection is usable");
    assert_ne!(
        replacement_backend, migration_backend,
        "the cancelled migration session must not return to the pool"
    );
    tokio::time::timeout(Duration::from_secs(2), limiter.migrate())
        .await
        .expect("the cancelled session lock was released")
        .expect("Runlimit migration succeeds after cancellation");

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn migration_sets_counter_key_fillfactor_and_retains_metadata() {
    let pool = test_pool(1).await;
    let primary_key_columns = sqlx::query_scalar::<_, String>(
        r"
SELECT attributes.attname::TEXT
FROM pg_catalog.pg_constraint AS constraints
CROSS JOIN LATERAL unnest(constraints.conkey)
    WITH ORDINALITY AS key_columns(attnum, position)
INNER JOIN pg_catalog.pg_attribute AS attributes
    ON attributes.attrelid = constraints.conrelid
    AND attributes.attnum = key_columns.attnum
WHERE
    constraints.conrelid = 'runlimit_fixed_windows'::regclass
    AND constraints.contype = 'p'
ORDER BY key_columns.position
",
    )
    .fetch_all(&pool)
    .await
    .expect("read migrated primary-key columns");
    assert_eq!(primary_key_columns, ["config_fingerprint", "subject_key"]);

    let table_options = counter_table_options(&pool).await;
    assert!(
        table_options.iter().any(|option| option == "fillfactor=80"),
        "migrated table options do not reserve space for HOT updates: {table_options:?}"
    );

    let policy = unique_policy("counter-key-metadata", 3, Duration::from_mins(1));
    let subject = key(155);
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    assert!(
        limiter
            .check(&Check::new(subject.bind(&policy)))
            .await
            .expect("admit metadata test check")
            .permits_request()
    );
    let metadata = sqlx::query_as::<_, (String, String)>(
        r"
SELECT policy_id, scope_id
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("read retained policy metadata by counter key");

    let deleted_rows = delete_counter(&pool, &policy, subject).await;
    drop(limiter);
    pool.close().await;

    assert_eq!(metadata.0, policy.id().as_str());
    assert_eq!(metadata.1, policy.scope().as_str());
    assert_eq!(deleted_rows, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn configured_capacity_denies_only_new_keys_in_the_full_shard() {
    let fixture = IsolatedSchema::for_cleanup_test(4).await;
    let pool = fixture.primary_pool.clone();
    let policy = unique_policy("configured-capacity", 10, Duration::from_mins(1));
    let shard = 17;
    let first_subject = key_in_capacity_shard(&policy, shard, 1);
    let second_subject = key_in_capacity_shard(&policy, shard, 2);
    let denied_subject = key_in_capacity_shard(&policy, shard, 3);
    let other_subject = key_in_capacity_shard(&policy, shard + 1, 4);
    let config = test_config()
        .with_maximum_rows_per_shard(2)
        .expect("two rows per shard is valid");
    let observer = Arc::new(RecordingObserver::default());
    let limiter = PostgresLimiter::new(pool.clone())
        .with_config(config)
        .with_observer(observer.clone());

    let first = limiter
        .check(&Check::new(first_subject.bind(&policy)))
        .await
        .expect("first key is admitted");
    let second = limiter
        .check(&Check::new(second_subject.bind(&policy)))
        .await
        .expect("second key is admitted");
    let denied = limiter
        .check(&Check::new(denied_subject.bind(&policy)))
        .await
        .expect("capacity exhaustion is a decision");
    let existing = limiter
        .check(&Check::new(first_subject.bind(&policy)))
        .await
        .expect("an existing key remains usable");
    let other = limiter
        .check(&Check::new(other_subject.bind(&policy)))
        .await
        .expect("another shard retains capacity");

    assert!(first.permits_request());
    assert!(second.permits_request());
    assert_eq!(denied, Decision::denied(storage_capacity()));
    assert!(existing.permits_request());
    assert!(other.permits_request());
    assert_eq!(
        observer.admissions.lock().unwrap().as_slice(),
        [
            (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed),
            (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed),
            (
                AdmissionOutcome::CapacityDenied,
                ConsumptionStatus::NotConsumed,
            ),
            (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed),
            (AdmissionOutcome::Allowed, ConsumptionStatus::Consumed),
        ]
    );
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 2);
    assert_eq!(capacity_row_count(&pool, i16::from(shard + 1)).await, 1);

    assert_eq!(delete_counter(&pool, &policy, first_subject).await, 1);
    assert_eq!(delete_counter(&pool, &policy, second_subject).await, 1);
    assert_eq!(delete_counter(&pool, &policy, denied_subject).await, 0);
    assert_eq!(delete_counter(&pool, &policy, other_subject).await, 1);
    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn expired_rows_hold_capacity_until_cleanup_commits() {
    let fixture = IsolatedSchema::for_cleanup_test(3).await;
    let pool = fixture.primary_pool.clone();
    let policy = unique_policy("expired-capacity", 10, Duration::from_mins(1));
    let shard = 23;
    let expired_subject = key_in_capacity_shard(&policy, shard, 1);
    let replacement_subject = key_in_capacity_shard(&policy, shard, 2);
    insert_counter_window(&pool, &policy, expired_subject, -120, -60).await;
    let config = test_config()
        .with_maximum_rows_per_shard(1)
        .expect("one row per shard is valid");
    let observer = Arc::new(RecordingObserver::default());
    let limiter = PostgresLimiter::new(pool.clone())
        .with_config(config)
        .with_observer(observer.clone());

    let full = limiter
        .check(&Check::new(replacement_subject.bind(&policy)))
        .await
        .expect("an expired stored row still occupies capacity");
    assert_eq!(full, Decision::denied(storage_capacity()));
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 1);

    assert_eq!(
        limiter.cleanup_expired(1).await.expect("cleanup succeeds"),
        1
    );
    assert_eq!(
        observer.cleanups.lock().unwrap().as_slice(),
        [(1, CleanupOutcome::Confirmed { removed: 1 })]
    );
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);

    let replacement = limiter
        .check(&Check::new(replacement_subject.bind(&policy)))
        .await
        .expect("cleanup releases the shard slot");
    assert!(replacement.permits_request());
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 1);
    assert_eq!(delete_counter(&pool, &policy, replacement_subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn capacity_denied_batch_rolls_back_and_remains_enforced_in_shadow_mode() {
    let fixture = IsolatedSchema::for_cleanup_test(3).await;
    let pool = fixture.primary_pool.clone();
    let policy = unique_policy("batch-capacity", 10, Duration::from_mins(1))
        .with_quota_mode(QuotaMode::Shadow);
    let shard = 29;
    let first_subject = key_in_capacity_shard(&policy, shard, 1);
    let second_subject = key_in_capacity_shard(&policy, shard, 2);
    let config = test_config()
        .with_maximum_rows_per_shard(1)
        .expect("one row per shard is valid");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(config);

    let result = limiter
        .check_all(&[
            Check::new(first_subject.bind(&policy)),
            Check::new(second_subject.bind(&policy)),
        ])
        .await
        .expect("capacity exhaustion is a batch decision");
    assert_eq!(
        result,
        BatchDecision::denied(1, 2, storage_capacity()).unwrap()
    );
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);
    assert!(!counter_exists(&pool, &policy, first_subject).await);
    assert!(!counter_exists(&pool, &policy, second_subject).await);

    let admitted = limiter
        .check(&Check::new(first_subject.bind(&policy)))
        .await
        .expect("rolled-back capacity remains available");
    assert!(admitted.permits_request());
    assert_eq!(delete_counter(&pool, &policy, first_subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn shadow_quota_denial_is_reported_without_consuming_more_quota() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let pool = fixture.primary_pool.clone();
    let policy =
        unique_policy("shadow-quota", 1, Duration::from_mins(1)).with_quota_mode(QuotaMode::Shadow);
    let earlier_subject = key_in_capacity_shard(&policy, 30, 1);
    let exhausted_subject = key_in_capacity_shard(&policy, 30, 2);
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());

    let allowed = limiter
        .check(&Check::new(exhausted_subject.bind(&policy)))
        .await
        .expect("first shadow-policy check is consumed");
    let single_shadow_denial = limiter
        .check(&Check::new(exhausted_subject.bind(&policy)))
        .await
        .expect("single shadow quota exhaustion is a decision");
    assert!(allowed.permits_request());
    assert!(single_shadow_denial.permits_request());
    assert!(matches!(
        single_shadow_denial.view(),
        DecisionView::ShadowDenied { denial } if denial.capacity().get() == 1
    ));
    assert_eq!(
        stored_counter_usage(&pool, &policy, exhausted_subject).await,
        1
    );

    let batch_shadow_denial = limiter
        .check_all(&[
            Check::new(earlier_subject.bind(&policy)),
            Check::new(exhausted_subject.bind(&policy)),
        ])
        .await
        .expect("shadow quota exhaustion is a batch decision");
    assert!(batch_shadow_denial.permits_request());
    assert!(matches!(
        batch_shadow_denial.view(),
        BatchDecisionView::ShadowDenied {
            index: 1,
            batch_size,
            denial,
        } if batch_size.get() == 2 && denial.capacity().get() == 1
    ));
    assert!(!counter_exists(&pool, &policy, earlier_subject).await);
    assert_eq!(
        stored_counter_usage(&pool, &policy, exhausted_subject).await,
        1
    );
    assert_eq!(delete_counter(&pool, &policy, exhausted_subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn database_trigger_hard_cap_blocks_old_writer_insertions() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let pool = fixture.primary_pool.clone();
    let policy = unique_policy("trigger-hard-cap", 10, Duration::from_mins(1));
    let shard = 31;
    let subject = key_in_capacity_shard(&policy, shard, 1);
    let mut transaction = pool.begin().await.expect("begin old-writer transaction");
    sqlx::query(
        r"
UPDATE runlimit_capacity_shards
SET row_count = $2
WHERE capacity_shard = $1
",
    )
    .bind(i16::from(shard))
    .bind(i64::from(HARD_MAX_ROWS_PER_SHARD))
    .execute(&mut *transaction)
    .await
    .expect("simulate a shard at the database hard maximum");

    let error = sqlx::query(
        r"
INSERT INTO runlimit_fixed_windows (
    policy_id,
    scope_id,
    config_fingerprint,
    subject_key,
    window_started_at,
    window_expires_at,
    used
)
VALUES ($1, $2, $3, $4, pg_catalog.clock_timestamp(),
        pg_catalog.clock_timestamp() + INTERVAL '1 minute', 1)
",
    )
    .bind(policy.id().as_str())
    .bind(policy.scope().as_str())
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .execute(&mut *transaction)
    .await
    .expect_err("the migration trigger rejects an old writer above the hard cap");
    let sqlx::Error::Database(database_error) = error else {
        panic!("hard-cap violation returned a non-database error");
    };
    assert_eq!(database_error.code().as_deref(), Some("23514"));
    assert_eq!(
        database_error.constraint(),
        Some("runlimit_capacity_shards_hard_max")
    );
    transaction
        .rollback()
        .await
        .expect("roll back simulated ledger state");

    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);
    assert!(!counter_exists(&pool, &policy, subject).await);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn database_trigger_rejects_storage_key_updates_without_ledger_drift() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let pool = fixture.primary_pool.clone();
    let policy = unique_policy("immutable-storage-key", 10, Duration::from_mins(1));
    let original_shard = 33;
    let replacement_shard = 34;
    let original_subject = key_in_capacity_shard(&policy, original_shard, 1);
    let replacement_subject = key_in_capacity_shard(&policy, replacement_shard, 2);
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());

    let admitted = limiter
        .check(&Check::new(original_subject.bind(&policy)))
        .await
        .expect("create the original counter");
    assert!(admitted.permits_request());
    assert_eq!(
        capacity_row_count(&pool, i16::from(original_shard)).await,
        1
    );
    assert_eq!(
        capacity_row_count(&pool, i16::from(replacement_shard)).await,
        0
    );

    let error = sqlx::query(
        r"
UPDATE runlimit_fixed_windows
SET subject_key = $3
WHERE config_fingerprint = $1 AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(original_subject.as_bytes().as_slice())
    .bind(replacement_subject.as_bytes().as_slice())
    .execute(&pool)
    .await
    .expect_err("raw storage-key mutation must be rejected");
    let sqlx::Error::Database(database_error) = error else {
        panic!("storage-key mutation returned a non-database error");
    };
    assert_eq!(database_error.code().as_deref(), Some("23514"));
    assert_eq!(
        database_error.constraint(),
        Some("runlimit_fixed_windows_immutable_storage_key")
    );

    assert!(counter_exists(&pool, &policy, original_subject).await);
    assert!(!counter_exists(&pool, &policy, replacement_subject).await);
    assert_eq!(
        capacity_row_count(&pool, i16::from(original_shard)).await,
        1
    );
    assert_eq!(
        capacity_row_count(&pool, i16::from(replacement_shard)).await,
        0
    );
    assert_eq!(delete_counter(&pool, &policy, original_subject).await, 1);
    assert_eq!(
        capacity_row_count(&pool, i16::from(original_shard)).await,
        0
    );

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn malformed_upsert_response_is_rejected_before_commit() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("malformed-response-rollback", 10, Duration::from_mins(1));
    let shard = 41;
    let subject = key_in_capacity_shard(&policy, shard, 1);
    let corruption_sql = format!(
        r"
CREATE FUNCTION {schema}.corrupt_returned_usage()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    NEW.used := 11;
    RETURN NEW;
END
$function$;

CREATE TRIGGER corrupt_returned_usage
BEFORE INSERT ON {schema}.runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION {schema}.corrupt_returned_usage();
"
    );
    sqlx::raw_sql(AssertSqlSafe(corruption_sql))
        .execute(&setup_pool)
        .await
        .expect("install malformed-response trigger");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());

    let error = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect_err("usage above the policy limit is a storage invariant failure");
    let CheckError::StorageInvariant(invariant) = &error else {
        panic!("expected a storage invariant");
    };
    assert_eq!(invariant.detail(), "stored usage exceeds its policy limit");
    assert!(std::error::Error::source(invariant).is_none());
    assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
    assert!(!counter_exists(&pool, &policy, subject).await);
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);

    sqlx::query(AssertSqlSafe(format!(
        "DROP TRIGGER corrupt_returned_usage ON {schema}.runlimit_fixed_windows"
    )))
    .execute(&setup_pool)
    .await
    .expect("drop malformed-response trigger");
    sqlx::query(AssertSqlSafe(format!(
        "DROP FUNCTION {schema}.corrupt_returned_usage()"
    )))
    .execute(&setup_pool)
    .await
    .expect("drop malformed-response function");

    let admitted = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect("the rolled-back malformed response leaves quota unconsumed");
    assert!(matches!(
        admitted.view(),
        DecisionView::Allowed { allowance } if allowance.available() == 9
    ));
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 1);
    assert_eq!(delete_counter(&pool, &policy, subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn undecodable_upsert_field_is_storage_invariant_before_commit() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("undecodable-response-rollback", 10, Duration::from_mins(1));
    let shard = 43;
    let subject = key_in_capacity_shard(&policy, shard, 1);

    // Keep the SQL operation valid while changing the returned field's wire
    // type. The update reaches RETURNING, but shaping it as an i64 must fail
    // before commit and roll back both the counter and capacity reservation.
    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.runlimit_fixed_windows ALTER COLUMN used TYPE NUMERIC"
    )))
    .execute(&setup_pool)
    .await
    .expect("make returned usage undecodable as an i64");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());

    let error = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect_err("an undecodable returned field is a storage invariant failure");
    let CheckError::StorageInvariant(invariant) = &error else {
        panic!("expected a storage invariant");
    };
    assert_eq!(
        invariant.detail(),
        "batch update returned undecodable usage"
    );
    assert!(
        std::error::Error::source(invariant)
            .is_some_and(|source| source.downcast_ref::<sqlx::Error>().is_some())
    );
    assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
    assert!(!counter_exists(&pool, &policy, subject).await);
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);

    sqlx::query(AssertSqlSafe(format!(
        "ALTER TABLE {schema}.runlimit_fixed_windows ALTER COLUMN used TYPE BIGINT USING used::BIGINT"
    )))
    .execute(&setup_pool)
    .await
    .expect("restore the migration-defined usage type");

    let admitted = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect("the rolled-back undecodable response leaves quota unconsumed");
    assert!(matches!(
        admitted.view(),
        DecisionView::Allowed { allowance } if allowance.available() == 9
    ));
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 1);
    assert_eq!(delete_counter(&pool, &policy, subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn failed_insert_rolls_back_capacity_trigger_accounting() {
    let fixture = IsolatedSchema::for_cleanup_test(2).await;
    let setup_pool = fixture.admin_pool.clone();
    let pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("capacity-rollback", 10, Duration::from_mins(1));
    let shard = 37;
    let subject = key_in_capacity_shard(&policy, shard, 1);
    let failure_sql = format!(
        r"
CREATE FUNCTION {schema}.fail_after_capacity_accounting()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    RAISE EXCEPTION 'injected failure after capacity accounting';
END
$function$;

CREATE TRIGGER zz_fail_after_capacity_accounting
AFTER INSERT ON {schema}.runlimit_fixed_windows
FOR EACH STATEMENT
EXECUTE FUNCTION {schema}.fail_after_capacity_accounting();
"
    );
    sqlx::raw_sql(AssertSqlSafe(failure_sql))
        .execute(&setup_pool)
        .await
        .expect("install post-accounting failure trigger");
    let config = test_config()
        .with_maximum_rows_per_shard(1)
        .expect("one row per shard is valid");
    let limiter = PostgresLimiter::new(pool.clone()).with_config(config);

    let failed = limiter.check(&Check::new(subject.bind(&policy))).await;
    assert!(matches!(failed, Err(CheckError::DefinitelyNotConsumed(_))));
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 0);
    assert!(!counter_exists(&pool, &policy, subject).await);

    sqlx::query(AssertSqlSafe(format!(
        "DROP TRIGGER zz_fail_after_capacity_accounting ON {schema}.runlimit_fixed_windows"
    )))
    .execute(&setup_pool)
    .await
    .expect("drop post-accounting failure trigger");
    sqlx::query(AssertSqlSafe(format!(
        "DROP FUNCTION {schema}.fail_after_capacity_accounting()"
    )))
    .execute(&setup_pool)
    .await
    .expect("drop post-accounting failure function");

    let admitted = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect("rolled-back trigger accounting releases the slot");
    assert!(admitted.permits_request());
    assert_eq!(capacity_row_count(&pool, i16::from(shard)).await, 1);
    assert_eq!(delete_counter(&pool, &policy, subject).await, 1);

    drop(limiter);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn concurrent_replicas_never_exceed_configured_shard_capacity() {
    const CAPACITY: u32 = 4;
    const ATTEMPTS: u8 = 24;

    let mut fixture = IsolatedSchema::for_cleanup_test(12).await;
    let first_pool = fixture.primary_pool.clone();
    let second_pool = fixture.additional_pool(12).await;
    let policy = Arc::new(unique_policy(
        "concurrent-capacity",
        10,
        Duration::from_mins(1),
    ));
    let shard = 41;
    let config = test_config()
        .with_maximum_rows_per_shard(CAPACITY)
        .expect("test shard capacity is valid");
    let first = PostgresLimiter::new(first_pool.clone()).with_config(config);
    let second = PostgresLimiter::new(second_pool.clone()).with_config(config);
    let barrier = Arc::new(Barrier::new(usize::from(ATTEMPTS) + 1));
    let mut tasks = Vec::with_capacity(usize::from(ATTEMPTS));

    for discriminator in 1..=ATTEMPTS {
        let limiter = if discriminator.is_multiple_of(2) {
            first.clone()
        } else {
            second.clone()
        };
        let policy = Arc::clone(&policy);
        let barrier = Arc::clone(&barrier);
        let subject = key_in_capacity_shard(&policy, shard, discriminator);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            (
                subject,
                limiter
                    .check(&Check::new(subject.bind(policy.as_ref())))
                    .await,
            )
        }));
    }
    barrier.wait().await;

    let mut allowed = 0_u32;
    let mut capacity_denied = 0_u32;
    for task in tasks {
        let (_subject, result) = task.await.expect("capacity task does not panic");
        let decision = result.expect("capacity contention returns a decision");
        if decision.permits_request() {
            allowed += 1;
        } else {
            assert_eq!(decision, Decision::denied(storage_capacity()));
            capacity_denied += 1;
        }
    }

    let stored_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM runlimit_fixed_windows")
        .fetch_one(&first_pool)
        .await
        .expect("count concurrent capacity rows");
    assert_eq!(allowed, CAPACITY);
    assert_eq!(capacity_denied, u32::from(ATTEMPTS) - CAPACITY);
    assert_eq!(stored_rows, i64::from(CAPACITY));
    assert_eq!(
        capacity_row_count(&first_pool, i16::from(shard)).await,
        i64::from(CAPACITY)
    );

    assert_eq!(
        sqlx::query("DELETE FROM runlimit_fixed_windows")
            .execute(&first_pool)
            .await
            .expect("delete concurrent capacity rows")
            .rows_affected(),
        u64::from(CAPACITY)
    );
    assert_eq!(capacity_row_count(&first_pool, i16::from(shard)).await, 0);

    drop(first);
    drop(second);
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn single_quota_denial_and_anchored_reset() {
    let pool = test_pool(4).await;
    let limiter = PostgresLimiter::new(pool).with_config(test_config());
    let policy = unique_policy("single", 2, Duration::from_millis(250));
    let check = Check::new(key(1).bind(&policy));

    let first = limiter.check(&check).await.expect("first check succeeds");
    assert!(first.permits_request());
    assert_eq!(available(&first), 1);
    assert!(matches!(
        first.view(),
        DecisionView::Allowed { allowance } if !allowance.replenishes_after().duration().is_zero()
    ));

    let second = limiter.check(&check).await.expect("second check succeeds");
    assert!(second.permits_request());
    assert_eq!(available(&second), 0);

    let denied = limiter.check(&check).await.expect("denial is a decision");
    let DecisionView::Denied {
        denial: Denial::QuotaExceeded(quota),
    } = denied.view()
    else {
        panic!("expected a quota denial, got {denied:?}")
    };
    let retry_after = quota.retry_after().duration();
    assert!(!retry_after.is_zero());
    assert!(retry_after <= policy.window().duration());

    sleep(retry_after + Duration::from_millis(20)).await;
    let reset = limiter
        .check(&check)
        .await
        .expect("check after expiry succeeds");
    assert!(reset.permits_request());
    assert_eq!(available(&reset), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn portable_policy_upper_bounds_are_stored_exactly() {
    let pool = test_pool(1).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let policy = unique_policy("portable-maximum", MAX_LIMIT, MAX_WINDOW);
    let subject = key(7);
    let check = Check::new(subject.bind(&policy))
        .with_cost(MAX_LIMIT)
        .unwrap();

    let allowed = limiter.check(&check).await.expect("maximum check succeeds");

    let (stored_used, stored_window_millis): (i64, i64) = sqlx::query_as(
        r"
SELECT
    used,
    (EXTRACT(EPOCH FROM (window_expires_at - window_started_at)) * 1000)::BIGINT
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("read maximum counter");

    let denied = limiter
        .check(&check)
        .await
        .expect("maximum quota denial succeeds");

    let deleted_rows = delete_counter(&pool, &policy, subject).await;
    drop(limiter);
    pool.close().await;

    assert!(matches!(
        allowed.view(),
        DecisionView::Allowed { allowance }
            if allowance.capacity().get() == MAX_LIMIT && allowance.available() == 0
    ));
    assert_eq!(stored_used, i64::MAX);
    assert_eq!(
        u64::try_from(stored_window_millis).unwrap(),
        MAX_WINDOW_MILLIS
    );
    assert!(matches!(
        denied.view(),
        DecisionView::Denied {
            denial: Denial::QuotaExceeded(quota),
        } if quota.capacity().get() == MAX_LIMIT
    ));
    assert_eq!(deleted_rows, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn denied_batch_rolls_back_every_counter() {
    let pool = test_pool(4).await;
    let limiter = PostgresLimiter::new(pool).with_config(test_config());
    let first_policy = unique_policy("batch-first", 2, Duration::from_secs(5));
    let saturated_policy = unique_policy("batch-saturated", 1, Duration::from_secs(5));
    let first_check = Check::new(key(2).bind(&first_policy));
    let saturated_check = Check::new(key(3).bind(&saturated_policy));

    assert!(
        limiter
            .check(&saturated_check)
            .await
            .expect("initial saturation succeeds")
            .permits_request()
    );

    let batch = limiter
        .check_all(&[first_check, saturated_check])
        .await
        .expect("denied batch is a decision");
    assert!(matches!(
        batch.view(),
        BatchDecisionView::Denied { index: 1, .. }
    ));

    let after_rollback = limiter
        .check(&first_check)
        .await
        .expect("counter remains usable");
    assert_eq!(available(&after_rollback), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn earliest_denial_skips_a_later_failing_statement() {
    let pool = test_pool(4).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let denied_policy = unique_policy("batch-a-denied", 1, Duration::from_secs(5));
    let failing_policy = unique_policy("batch-z-errors", 10, Duration::from_secs(5));
    let denied_subject = key(8);
    let failing_subject = key(9);
    let denied_check = Check::new(denied_subject.bind(&denied_policy));
    let failing_check = Check::new(failing_subject.bind(&failing_policy));

    let saturated = limiter
        .check(&denied_check)
        .await
        .expect("saturating check succeeds");
    assert!(saturated.permits_request());

    sqlx::query(
        r"
CREATE OR REPLACE FUNCTION runlimit_test_fail_later_batch_check()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.policy_id LIKE 'test.batch-z-errors.%' THEN
        RAISE EXCEPTION 'injected failure for a later batch check';
    END IF;
    RETURN NEW;
END
$$
",
    )
    .execute(&pool)
    .await
    .expect("create scoped failure trigger function");
    sqlx::query(
        r"
DROP TRIGGER IF EXISTS runlimit_test_fail_later_batch_check
ON runlimit_fixed_windows
",
    )
    .execute(&pool)
    .await
    .expect("drop stale scoped failure trigger");
    sqlx::query(
        r"
CREATE TRIGGER runlimit_test_fail_later_batch_check
BEFORE INSERT OR UPDATE ON runlimit_fixed_windows
FOR EACH ROW
EXECUTE FUNCTION runlimit_test_fail_later_batch_check()
",
    )
    .execute(&pool)
    .await
    .expect("create scoped failure trigger");

    let injected_error = limiter.check(&failing_check).await;
    let batch = limiter.check_all(&[denied_check, failing_check]).await;

    sqlx::query(
        r"
DROP TRIGGER runlimit_test_fail_later_batch_check
ON runlimit_fixed_windows
",
    )
    .execute(&pool)
    .await
    .expect("drop scoped failure trigger");
    sqlx::query("DROP FUNCTION runlimit_test_fail_later_batch_check()")
        .execute(&pool)
        .await
        .expect("drop scoped failure trigger function");
    let deleted_rows = delete_counter(&pool, &denied_policy, denied_subject).await;

    assert!(matches!(
        injected_error,
        Err(CheckError::DefinitelyNotConsumed(_))
    ));
    let batch = batch.expect("later failing statement must not replace a known denial");
    assert!(matches!(
        batch.view(),
        BatchDecisionView::Denied { index: 0, .. }
    ));
    assert_eq!(deleted_rows, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn search_path_clock_shadow_cannot_hijack_admission_or_cleanup_time() {
    let mut fixture = IsolatedSchema::for_cleanup_test(1).await;
    let setup_pool = fixture.admin_pool.clone();
    let table_pool = fixture.primary_pool.clone();
    let schema = fixture.schema.clone();
    let policy = unique_policy("clock-shadow", 1, Duration::from_hours(1));
    let subject = key(10);
    let check = Check::new(subject.bind(&policy));
    let setup_limiter = PostgresLimiter::new(table_pool.clone()).with_config(test_config());
    let first = setup_limiter
        .check(&check)
        .await
        .expect("initialize an active window");
    assert!(first.permits_request());

    sqlx::query(AssertSqlSafe(format!(
        r"
CREATE FUNCTION {schema}.clock_timestamp()
RETURNS TIMESTAMPTZ
LANGUAGE sql
IMMUTABLE
AS $function$
    SELECT TIMESTAMPTZ '2100-01-01 00:00:00+00'
$function$
"
    )))
    .execute(&setup_pool)
    .await
    .expect("create malicious clock shadow");

    let shadowed_pool = fixture.additional_pool(2).await;
    let limiter = PostgresLimiter::new(shadowed_pool.clone()).with_config(test_config());

    let denied = limiter
        .check(&check)
        .await
        .expect("authoritative clock remains callable");
    let removed = limiter
        .cleanup_expired(10)
        .await
        .expect("cleanup uses authoritative clock");
    let stored_used = stored_counter_usage(&shadowed_pool, &policy, subject).await;

    drop(limiter);
    drop(setup_limiter);
    let deleted_rows = delete_counter(&table_pool, &policy, subject).await;
    fixture.teardown().await;

    assert!(
        !denied.permits_request(),
        "a search_path-resolved future clock would incorrectly renew the active window"
    );
    assert_eq!(
        removed, 0,
        "a search_path-resolved future clock would incorrectly delete the active window"
    );
    assert_eq!(stored_used, 1);
    assert_eq!(deleted_rows, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn memory_and_postgres_share_fixed_window_transition_semantics() {
    let postgres_pool = test_pool(2).await;
    let postgres = PostgresLimiter::new(postgres_pool.clone()).with_config(test_config());
    let memory_clock = ConformanceClock::default();
    let memory =
        MemoryStore::builder(MemoryStoreConfig::new(1).expect("test memory capacity is valid"))
            .with_clock(memory_clock.clone())
            .build();
    let policy = unique_policy("fixed-window-conformance", 5, Duration::from_secs(10));
    let subject = key(11);

    let observations = async {
        let transitions = fixed_window_transitions();
        let mut observations = Vec::with_capacity(transitions.len());
        for step in transitions {
            memory_clock.set(step.at_millis);
            if step.at_millis == 10_000 {
                sqlx::query(
                    r"
UPDATE runlimit_fixed_windows
SET window_expires_at = pg_catalog.clock_timestamp()
WHERE
    config_fingerprint = $1
    AND subject_key = $2
",
                )
                .bind(policy.fingerprint().as_bytes().as_slice())
                .bind(subject.as_bytes().as_slice())
                .execute(&postgres_pool)
                .await?;
            }

            let check = Check::new(subject.bind(&policy)).with_cost(step.cost)?;
            let memory_decision = memory.check(&check)?;
            let postgres_decision = postgres.check(&check).await?;
            observations.push((step, memory_decision, postgres_decision));
        }
        Ok::<_, Box<dyn std::error::Error>>(observations)
    }
    .await;

    let deleted_rows = delete_counter(&postgres_pool, &policy, subject).await;
    drop(postgres);
    postgres_pool.close().await;

    let observations = observations.expect("both backends replay the transition sequence");
    for (step, memory_decision, postgres_decision) in observations {
        assert_eq!(
            memory_decision.permits_request(),
            postgres_decision.permits_request(),
            "backends disagreed on admission at the '{}' transition",
            step.name
        );
        assert_eq!(
            shape(&memory_decision),
            shape(&postgres_decision),
            "backends disagreed on the outcome at the '{}' transition",
            step.name
        );
        assert_eq!(
            memory_decision, step.expected,
            "memory returned the wrong '{}' transition",
            step.name
        );
        let postgres_duration = match postgres_decision.view() {
            DecisionView::Allowed { allowance } => allowance.replenishes_after().duration(),
            DecisionView::Denied {
                denial: Denial::QuotaExceeded(quota),
            }
            | DecisionView::ShadowDenied { denial: quota } => quota.retry_after().duration(),
            DecisionView::Denied {
                denial: Denial::StorageCapacity { .. },
            } => panic!("the transition sequence never exhausts storage capacity"),
        };
        assert!(
            postgres_duration <= policy.window().duration(),
            "PostgreSQL returned an overlong duration at the '{}' transition",
            step.name
        );
    }
    assert_eq!(deleted_rows, 1);
}

/// The `Limiter` contract requires that an unpolled future performs no work.
/// This is the PostgreSQL counterpart of the memory crate's laziness tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn unpolled_limiter_futures_consume_no_quota() {
    let pool = test_pool(2).await;
    let observer = Arc::new(RecordingObserver::default());
    let limiter = PostgresLimiter::new(pool.clone())
        .with_config(test_config())
        .with_observer(observer.clone());
    let policy = unique_policy("lazy-limiter", 1, Duration::from_mins(1));
    let subject = key(12);
    let check = Check::new(subject.bind(&policy));
    let checks = [check];

    // Go through the trait, not the inherent methods, because the contract
    // under test belongs to `Limiter`.
    drop(Limiter::check(&limiter, &check));
    drop(Limiter::check_all(&limiter, &checks));

    assert!(
        !counter_exists(&pool, &policy, subject).await,
        "dropping unpolled futures must not create a counter row"
    );
    assert!(
        observer.admissions.lock().unwrap().is_empty(),
        "dropping unpolled futures must not emit admission observations"
    );

    let first = Limiter::check(&limiter, &check)
        .await
        .expect("the first polled check succeeds");
    assert_eq!(
        available(&first),
        0,
        "the first polled check consumes the only quota unit, proving the unpolled futures consumed nothing"
    );
    assert_eq!(stored_counter_usage(&pool, &policy, subject).await, 1);

    let batch = Limiter::check_all(&limiter, &checks)
        .await
        .expect("the polled batch is a decision");
    assert!(matches!(
        batch.view(),
        BatchDecisionView::Denied { index: 0, .. }
    ));
    assert_eq!(observer.admissions.lock().unwrap().len(), 2);
    assert_eq!(delete_counter(&pool, &policy, subject).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn set_based_batch_preserves_caller_order_and_array_alignment() {
    let pool = test_pool(4).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let policies = (0_u64..32)
        .map(|index| {
            unique_policy(
                &format!("set-array-{index:02}"),
                50 + index * 3,
                Duration::from_millis(1_000 + index),
            )
        })
        .collect::<Vec<_>>();
    let checks = policies
        .iter()
        .enumerate()
        .rev()
        .map(|(index, policy)| {
            Check::new(key(100 + u8::try_from(index).unwrap()).bind(policy))
                .with_cost(u64::try_from(index % 7 + 1).unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>();

    let result = limiter.check_all(&checks).await;

    let fingerprints = checks
        .iter()
        .map(|check| check.policy().fingerprint().as_bytes().to_vec())
        .collect::<Vec<_>>();
    let subjects = checks
        .iter()
        .map(|check| check.subject().as_bytes().to_vec())
        .collect::<Vec<_>>();
    let stored_usage = sqlx::query_as::<_, (i64, i64, i64)>(
        r"
WITH input AS (
    SELECT *
    FROM unnest($1::BYTEA[], $2::BYTEA[])
        WITH ORDINALITY AS keys(config_fingerprint, subject_key, input_position)
)
SELECT
    input.input_position,
    windows.used,
    (EXTRACT(EPOCH FROM (
        windows.window_expires_at - windows.window_started_at
    )) * 1000)::BIGINT
FROM input
INNER JOIN runlimit_fixed_windows AS windows
    ON windows.config_fingerprint = input.config_fingerprint
    AND windows.subject_key = input.subject_key
ORDER BY input.input_position
",
    )
    .bind(fingerprints)
    .bind(subjects)
    .fetch_all(&pool)
    .await
    .expect("read set-based batch counters in caller order");

    let mut deleted_rows = 0;
    for check in &checks {
        deleted_rows += delete_counter(&pool, check.policy(), check.subject()).await;
    }

    let batch_decision = result.expect("set-based batch succeeds");
    let BatchDecisionView::Allowed { allowances } = batch_decision.view() else {
        panic!("fresh counters must all be allowed: {batch_decision:?}");
    };
    assert_eq!(allowances.len(), checks.len());
    for (allowance, check) in allowances.iter().zip(&checks) {
        assert_eq!(allowance.capacity(), check.policy().limit());
        assert_eq!(
            allowance.available(),
            check.policy().limit().get() - check.cost()
        );
    }

    assert_eq!(stored_usage.len(), checks.len());
    for (caller_index, ((input_position, used, window_millis), check)) in
        stored_usage.iter().zip(&checks).enumerate()
    {
        assert_eq!(usize::try_from(*input_position).unwrap(), caller_index + 1);
        assert_eq!(u64::try_from(*used).unwrap(), check.cost());
        assert_eq!(
            u64::try_from(*window_millis).unwrap(),
            check.policy().window().millis()
        );
    }
    assert_eq!(deleted_rows, u64::try_from(checks.len()).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn opposite_order_batches_across_pools_do_not_deadlock_or_over_admit() {
    const ROUNDS: u64 = 8;

    let first_pool = test_pool(4).await;
    let database_url =
        std::env::var(TEST_DATABASE_URL).expect("test URL remains available during test");
    let second_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect second replica pool");
    let first_limiter = PostgresLimiter::new(first_pool.clone()).with_config(test_config());
    let second_limiter = PostgresLimiter::new(second_pool).with_config(test_config());
    let policy_a = Arc::new(unique_policy(
        "opposite-order-a",
        ROUNDS * 2,
        Duration::from_secs(10),
    ));
    let policy_b = Arc::new(unique_policy(
        "opposite-order-b",
        ROUNDS * 2,
        Duration::from_secs(10),
    ));
    let subject_a = key(140);
    let subject_b = key(141);
    let mut outcomes = Vec::with_capacity(usize::try_from(ROUNDS).unwrap());

    for _ in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(3));

        let first_task_limiter = first_limiter.clone();
        let first_task_barrier = Arc::clone(&barrier);
        let first_task_a = Arc::clone(&policy_a);
        let first_task_b = Arc::clone(&policy_b);
        let first_task = tokio::spawn(async move {
            let checks = [
                Check::new(subject_a.bind(first_task_a.as_ref())),
                Check::new(subject_b.bind(first_task_b.as_ref())),
            ];
            first_task_barrier.wait().await;
            first_task_limiter.check_all(&checks).await
        });

        let second_task_limiter = second_limiter.clone();
        let second_task_barrier = Arc::clone(&barrier);
        let second_task_a = Arc::clone(&policy_a);
        let second_task_b = Arc::clone(&policy_b);
        let second_task = tokio::spawn(async move {
            let checks = [
                Check::new(subject_b.bind(second_task_b.as_ref())),
                Check::new(subject_a.bind(second_task_a.as_ref())),
            ];
            second_task_barrier.wait().await;
            second_task_limiter.check_all(&checks).await
        });

        barrier.wait().await;
        outcomes.push((first_task.await, second_task.await));
    }

    let stored_a = stored_counter_usage(&first_pool, &policy_a, subject_a).await;
    let stored_b = stored_counter_usage(&first_pool, &policy_b, subject_b).await;
    let after_capacity = first_limiter
        .check_all(&[
            Check::new(subject_a.bind(&policy_a)),
            Check::new(subject_b.bind(&policy_b)),
        ])
        .await;

    let deleted_a = delete_counter(&first_pool, &policy_a, subject_a).await;
    let deleted_b = delete_counter(&first_pool, &policy_b, subject_b).await;

    for (first, second) in outcomes {
        for outcome in [first, second] {
            let outcome = outcome
                .expect("contending batch task does not panic")
                .expect("opposite-order batch completes before its deadline");
            let BatchDecisionView::Allowed { allowances } = outcome.view() else {
                panic!("capacity permits every contending batch: {outcome:?}");
            };
            assert_eq!(allowances.len(), 2);
        }
    }
    assert_eq!(u64::try_from(stored_a).unwrap(), ROUNDS * 2);
    assert_eq!(u64::try_from(stored_b).unwrap(), ROUNDS * 2);
    let after_capacity = after_capacity.expect("post-capacity batch is a decision");
    assert!(matches!(
        after_capacity.view(),
        BatchDecisionView::Denied { index: 0, .. }
    ));
    assert_eq!(deleted_a, 1);
    assert_eq!(deleted_b, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn concurrent_pools_never_over_admit() {
    let first_pool = test_pool(24).await;
    let database_url =
        std::env::var(TEST_DATABASE_URL).expect("test URL remains available during test");
    let second_pool = PgPoolOptions::new()
        .max_connections(24)
        .connect(&database_url)
        .await
        .expect("connect second replica pool");
    let first_limiter = PostgresLimiter::new(first_pool).with_config(test_config());
    let second_limiter = PostgresLimiter::new(second_pool).with_config(test_config());
    let policy = Arc::new(unique_policy("concurrent", 12, Duration::from_secs(10)));
    let barrier = Arc::new(Barrier::new(49));
    let mut tasks = Vec::with_capacity(48);

    for index in 0_usize..48 {
        let limiter = if index.is_multiple_of(2) {
            first_limiter.clone()
        } else {
            second_limiter.clone()
        };
        let policy = Arc::clone(&policy);
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            limiter
                .check(&Check::new(key(4).bind(&policy)))
                .await
                .expect("concurrent check completes")
                .permits_request()
        }));
    }

    barrier.wait().await;
    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await.expect("check task does not panic"));
    }

    assert_eq!(admitted, 12);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn concurrent_fresh_single_checks_advance_the_waiters_snapshot() {
    let pool = test_pool(6).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let policy = Arc::new(unique_policy(
        "concurrent-fresh-single",
        1,
        Duration::from_secs(10),
    ));
    let subject = key(156);
    let check = Check::new(subject.bind(policy.as_ref()));
    let logical_lock_id = advisory_lock_id(&check);

    let mut blocker = pool.begin().await.expect("begin logical-lock blocker");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(logical_lock_id)
        .execute(&mut *blocker)
        .await
        .expect("hold the fresh key's logical lock");

    let barrier = Arc::new(Barrier::new(3));
    let mut tasks = Vec::with_capacity(2);
    for _ in 0..2 {
        let task_limiter = limiter.clone();
        let task_policy = Arc::clone(&policy);
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_limiter
                .check(&Check::new(subject.bind(task_policy.as_ref())))
                .await
        }));
    }
    barrier.wait().await;

    wait_for_advisory_waiters(&pool, "WITH RECURSIVE acquired(position, locked)", 2).await;

    blocker.commit().await.expect("release fresh-key lock");

    let mut allowed = 0;
    let mut denied = 0;
    for task in tasks {
        let decision = task
            .await
            .expect("fresh-key check task does not panic")
            .expect("snapshot fallback returns a decision");
        allowed += usize::from(decision.permits_request());
        denied += usize::from(!decision.permits_request());
    }
    let stored = stored_counter_usage(&pool, &policy, subject).await;
    let deleted = delete_counter(&pool, &policy, subject).await;

    assert_eq!(allowed, 1);
    assert_eq!(denied, 1);
    assert_eq!(stored, 1);
    assert_eq!(deleted, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn fresh_single_waiting_behind_a_batch_advances_its_snapshot() {
    let pool = test_pool(8).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let target_policy = Arc::new(unique_policy(
        "mixed-fresh-target",
        1,
        Duration::from_secs(10),
    ));
    let companion_policy = Arc::new(unique_policy(
        "mixed-fresh-companion",
        1,
        Duration::from_secs(10),
    ));
    let target_subject = key(157);
    let companion_subject = key(158);
    let target_check = Check::new(target_subject.bind(target_policy.as_ref()));
    let logical_lock_id = advisory_lock_id(&target_check);

    let mut blocker = pool.begin().await.expect("begin mixed-path lock blocker");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(logical_lock_id)
        .execute(&mut *blocker)
        .await
        .expect("hold the mixed-path target's logical lock");

    let batch_limiter = limiter.clone();
    let batch_target_policy = Arc::clone(&target_policy);
    let batch_companion_policy = Arc::clone(&companion_policy);
    let batch = tokio::spawn(async move {
        batch_limiter
            .check_all(&[
                Check::new(target_subject.bind(batch_target_policy.as_ref())),
                Check::new(companion_subject.bind(batch_companion_policy.as_ref())),
            ])
            .await
    });
    wait_for_advisory_waiters(&pool, "WITH RECURSIVE acquired(position, locked)", 1).await;

    let single_limiter = limiter.clone();
    let single_policy = Arc::clone(&target_policy);
    let single = tokio::spawn(async move {
        single_limiter
            .check(&Check::new(target_subject.bind(single_policy.as_ref())))
            .await
    });
    wait_for_advisory_waiters(&pool, "WITH RECURSIVE acquired(position, locked)", 2).await;

    blocker.commit().await.expect("release mixed-path lock");

    let batch = batch
        .await
        .expect("fresh-key batch task does not panic")
        .expect("fresh-key batch succeeds");
    let single = single
        .await
        .expect("fresh-key single task does not panic")
        .expect("fresh-key single returns a decision");
    let target_stored = stored_counter_usage(&pool, &target_policy, target_subject).await;
    let companion_stored = stored_counter_usage(&pool, &companion_policy, companion_subject).await;
    let target_deleted = delete_counter(&pool, &target_policy, target_subject).await;
    let companion_deleted = delete_counter(&pool, &companion_policy, companion_subject).await;

    assert!(matches!(
        batch.view(),
        BatchDecisionView::Allowed { allowances } if allowances.len() == 2
    ));
    assert!(!single.permits_request());
    assert_eq!(target_stored, 1);
    assert_eq!(companion_stored, 1);
    assert_eq!(target_deleted, 1);
    assert_eq!(companion_deleted, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn contended_check_samples_time_after_row_lock() {
    let pool = test_pool(6).await;
    let limiter = PostgresLimiter::new(pool.clone()).with_config(test_config());
    let policy = Arc::new(unique_policy(
        "lock-crosses-expiry",
        1,
        Duration::from_millis(200),
    ));
    let subject = key(5);

    let first = limiter
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect("first check succeeds");
    assert!(first.permits_request());

    let mut blocker = pool.begin().await.expect("begin blocker transaction");
    sqlx::query(
        r"
SELECT 1
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
FOR UPDATE
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(&mut *blocker)
    .await
    .expect("lock existing counter row");

    sleep(Duration::from_millis(25)).await;
    let waiting_limiter = limiter.clone();
    let waiting_policy = Arc::clone(&policy);
    let waiting = tokio::spawn(async move {
        waiting_limiter
            .check(&Check::new(subject.bind(waiting_policy.as_ref())))
            .await
    });

    // The check starts while the original window is active, then remains
    // blocked until well after it expires.
    sleep(Duration::from_millis(250)).await;
    blocker.commit().await.expect("release counter row");

    let decision = waiting
        .await
        .expect("waiting task does not panic")
        .expect("waiting check succeeds");
    assert!(
        decision.permits_request(),
        "a clock sampled before the row-lock wait would deny against the expired window"
    );
    assert_eq!(available(&decision), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn successive_lock_waits_share_the_remaining_server_deadline() {
    let mut fixture = IsolatedSchema::for_cleanup_test(1).await;
    let tested_pool = fixture.primary_pool.clone();
    let blockers = fixture.additional_pool(2).await;
    let observer = fixture.additional_pool(1).await;
    let policy = unique_policy("successive-locks", 2, Duration::from_secs(10));
    let subject = key(97);
    let normal = PostgresLimiter::new(tested_pool.clone()).with_config(test_config());
    normal
        .check(&Check::new(subject.bind(&policy)))
        .await
        .expect("seed row");
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&tested_pool)
        .await
        .expect("tested backend");
    let mut logical = blockers.begin().await.expect("logical blocker");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(advisory_lock_id(&Check::new(subject.bind(&policy))))
        .execute(&mut *logical)
        .await
        .expect("hold logical lock");
    let mut row = blockers.begin().await.expect("row blocker");
    sqlx::query("SELECT 1 FROM runlimit_fixed_windows WHERE config_fingerprint = $1 AND subject_key = $2 FOR UPDATE")
        .bind(policy.fingerprint().as_bytes().as_slice()).bind(subject.as_bytes().as_slice())
        .execute(&mut *row).await.expect("hold row lock");
    let config = PostgresConfig::new()
        .with_operation_timeout(Duration::from_secs(1))
        .unwrap();
    let limiter = PostgresLimiter::new(tested_pool.clone()).with_config(config);
    let owned_policy = policy.clone();
    let request = tokio::spawn(async move {
        limiter
            .check(&Check::new(subject.bind(&owned_policy)))
            .await
    });
    wait_for_advisory_waiters(&observer, "WITH RECURSIVE acquired", 1).await;
    // Spend most of the operation budget on the first lock. The next statement
    // must receive only the remainder, not a fresh one-second server timeout.
    sleep(Duration::from_millis(600)).await;
    logical.rollback().await.expect("release logical lock");
    let error = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .expect("bounded check")
        .expect("request task")
        .expect_err("row remains locked");
    assert!(matches!(error, CheckError::TimedOutBeforeCommit { .. }));
    assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
    tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(backend).fetch_one(&observer).await.expect("observe tested backend");
            if !waiting { break; }
            sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("the server must not keep the expired check waiting on its second lock");
    row.rollback().await.expect("release row lock");
    assert_eq!(
        stored_counter_usage(&tested_pool, &policy, subject).await,
        1
    );
    fixture.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires RUNLIMIT_POSTGRES_TEST_DATABASE_URL"]
async fn row_lock_timeout_releases_single_connection_pool_slot_without_consuming_quota() {
    let blocker_pool = test_pool(1).await;
    let tested_pool = test_pool(1).await;
    let normal_limiter = PostgresLimiter::new(blocker_pool.clone()).with_config(test_config());
    let policy = unique_policy("lock-timeout", 2, Duration::from_secs(5));
    let subject = key(6);
    let check = Check::new(subject.bind(&policy));

    let first = normal_limiter
        .check(&check)
        .await
        .expect("first check succeeds");
    assert_eq!(available(&first), 1);

    let mut blocker = blocker_pool
        .begin()
        .await
        .expect("begin blocker transaction");
    sqlx::query(
        r"
SELECT 1
FROM runlimit_fixed_windows
WHERE
    config_fingerprint = $1
    AND subject_key = $2
FOR UPDATE
",
    )
    .bind(policy.fingerprint().as_bytes().as_slice())
    .bind(subject.as_bytes().as_slice())
    .fetch_one(&mut *blocker)
    .await
    .expect("lock existing counter row");

    let short_config = PostgresConfig::new()
        .with_operation_timeout(Duration::from_millis(75))
        .expect("short nonzero timeout is valid");
    let short_limiter = PostgresLimiter::new(tested_pool.clone()).with_config(short_config);
    let error = short_limiter
        .check(&check)
        .await
        .expect_err("row lock must outlive the operation deadline");
    assert!(matches!(error, CheckError::TimedOutBeforeCommit { .. }));
    assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);

    let probe: i32 = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_scalar("SELECT 1").fetch_one(&tested_pool),
    )
    .await
    .expect("timed-out check releases the only tested-pool slot")
    .expect("replacement tested-pool query succeeds");
    assert_eq!(probe, 1);

    let cancellable_limiter = PostgresLimiter::new(tested_pool.clone()).with_config(test_config());
    let cancelled =
        tokio::time::timeout(Duration::from_millis(75), cancellable_limiter.check(&check)).await;
    assert!(
        cancelled.is_err(),
        "outer deadline must cancel the check before its configured deadline"
    );

    let probe_after_cancellation: i32 = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_scalar("SELECT 1").fetch_one(&tested_pool),
    )
    .await
    .expect("cancelled check releases the only tested-pool slot")
    .expect("tested-pool query after cancellation succeeds");
    assert_eq!(probe_after_cancellation, 1);

    blocker.commit().await.expect("release counter row");
    let after_timeout = normal_limiter
        .check(&check)
        .await
        .expect("counter remains usable");
    assert!(after_timeout.permits_request());
    assert_eq!(available(&after_timeout), 0);

    let deleted_rows = delete_counter(&blocker_pool, &policy, subject).await;
    drop(short_limiter);
    drop(cancellable_limiter);
    drop(normal_limiter);
    tested_pool.close().await;
    blocker_pool.close().await;
    assert_eq!(deleted_rows, 1);
}
