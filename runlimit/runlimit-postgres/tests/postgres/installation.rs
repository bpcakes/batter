use super::*;
use runlimit_postgres::{InstallationError, MigrationHistory};

async fn installed() -> IsolatedSchema {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    PostgresLimiter::new(fixture.primary_pool.clone())
        .migrate()
        .await
        .unwrap();
    fixture
}

fn assert_issue(error: InstallationError, object: &str, requirement: &str) {
    let InstallationError::Incompatible { issues } = error else {
        panic!("expected incompatibility for {object}: {error:?}");
    };
    assert!(
        issues
            .iter()
            .any(|issue| issue.object() == object && issue.requirement().contains(requirement)),
        "missing {object}/{requirement}: {issues:?}"
    );
}

async fn snapshot(pool: &PgPool) -> (String, String, String) {
    sqlx::query_as(
        "SELECT \
         (SELECT COALESCE(jsonb_agg(to_jsonb(w) ORDER BY config_fingerprint, subject_key)::text, '') \
          FROM runlimit_fixed_windows w), \
         (SELECT jsonb_agg(to_jsonb(c) ORDER BY capacity_shard)::text FROM runlimit_capacity_shards c), \
         (SELECT jsonb_agg(to_jsonb(m) ORDER BY version)::text FROM _sqlx_migrations m)",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_bundled_validation_is_read_only_and_preserves_quota() {
    let fixture = installed().await;
    let pool = &fixture.primary_pool;
    run_test_host_migrations(pool).await;
    let limiter = PostgresLimiter::new(pool.clone());
    let policy = FixedWindowPolicy::new(
        PolicyId::new("installation").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([51; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    let before = snapshot(pool).await;
    // Runs even when the database refuses writes by default. No rollback-only
    // test INSERT/UPDATE can pass this check.
    sqlx::query("SET default_transaction_read_only = on")
        .execute(pool)
        .await
        .unwrap();
    for mode in [
        MigrationHistory::Bundled,
        MigrationHistory::ApplicationManaged,
    ] {
        limiter.validate_installation(mode).await.unwrap();
    }
    assert_eq!(snapshot(pool).await, before);
    let read_only: String = sqlx::query_scalar("SHOW default_transaction_read_only")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(read_only, "on", "validation preserves session settings");
    sqlx::query("SET default_transaction_read_only = off")
        .execute(pool)
        .await
        .unwrap();
    assert!(limiter.check(&check).await.unwrap().permits_request());
    assert!(!limiter.check(&check).await.unwrap().permits_request());
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_application_migrations_need_no_upstream_ledger() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    for sql in [
        CREATE_RUNLIMIT_FIXED_WINDOWS_SQL,
        SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL,
        BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL,
    ] {
        let mut transaction = fixture.primary_pool.begin().await.unwrap();
        sqlx::raw_sql(sql).execute(&mut *transaction).await.unwrap();
        transaction.commit().await.unwrap();
    }
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    limiter
        .validate_installation(MigrationHistory::ApplicationManaged)
        .await
        .unwrap();
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "_sqlx_migrations",
        "history table",
    );
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert!(!exists, "validation must not install migration history");
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_missing_tables_are_actionable_and_not_created() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::ApplicationManaged)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows",
        "table",
    );
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('runlimit_fixed_windows') IS NOT NULL")
            .fetch_one(&fixture.primary_pool)
            .await
            .unwrap();
    assert!(!exists);
    fixture.teardown().await;
}

const SCHEMA_DRIFT_CASES: &[(&str, &str, &str)] = &[
    (
        "CREATE UNIQUE INDEX extra_unique ON runlimit_fixed_windows (policy_id)",
        "runlimit_fixed_windows.extra_unique",
        "unique indexes",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ALTER COLUMN used TYPE numeric",
        "runlimit_fixed_windows.used",
        "int8",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ALTER COLUMN policy_id DROP NOT NULL",
        "runlimit_fixed_windows.policy_id",
        "NOT NULL",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows DROP CONSTRAINT runlimit_fixed_windows_positive_usage",
        "runlimit_fixed_windows.runlimit_fixed_windows_positive_usage",
        "CHECK",
    ),
    (
        "ALTER TABLE runlimit_capacity_shards DROP CONSTRAINT runlimit_capacity_shards_hard_max; ALTER TABLE runlimit_capacity_shards ADD CONSTRAINT runlimit_capacity_shards_hard_max CHECK (row_count <= 999999)",
        "runlimit_capacity_shards.runlimit_capacity_shards_hard_max",
        "65536",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows DISABLE TRIGGER runlimit_fixed_windows_capacity_insert",
        "runlimit_fixed_windows_capacity_insert",
        "enabled",
    ),
    (
        "ALTER FUNCTION runlimit_fixed_windows_capacity_after_delete() SECURITY INVOKER",
        "runlimit_fixed_windows_capacity_after_delete",
        "security-definer",
    ),
    (
        "CREATE OR REPLACE FUNCTION runlimit_fixed_windows_capacity_after_insert() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$BEGIN RETURN NULL; END;$$",
        "runlimit_fixed_windows_capacity_after_insert",
        "published",
    ),
    (
        "DROP INDEX runlimit_fixed_windows_expiry_idx",
        "runlimit_fixed_windows_expiry_idx",
        "index",
    ),
    (
        "DROP INDEX runlimit_fixed_windows_expiry_idx; CREATE INDEX runlimit_fixed_windows_expiry_idx ON runlimit_fixed_windows (used)",
        "runlimit_fixed_windows_expiry_idx",
        "window_expires_at",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ENABLE ROW LEVEL SECURITY",
        "runlimit_fixed_windows",
        "row-level security",
    ),
    (
        "DELETE FROM runlimit_capacity_shards WHERE capacity_shard = 99",
        "runlimit_capacity_shards",
        "256 shard",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ALTER COLUMN capacity_shard DROP EXPRESSION",
        "runlimit_fixed_windows.capacity_shard",
        "generation",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ADD COLUMN extra text NOT NULL",
        "runlimit_fixed_windows.extra",
        "additional required",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ADD COLUMN extra text DEFAULT 'value'",
        "runlimit_fixed_windows.extra",
        "additional defaults",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows ADD COLUMN extra bigint GENERATED BY DEFAULT AS IDENTITY",
        "runlimit_fixed_windows.extra",
        "identity columns",
    ),
    (
        "CREATE DOMAIN extra_domain AS text NOT NULL; ALTER TABLE runlimit_fixed_windows ADD COLUMN extra extra_domain",
        "runlimit_fixed_windows.extra",
        "domains",
    ),
    (
        "CREATE INDEX extra_expression ON runlimit_fixed_windows ((1::bigint / (used - 1)))",
        "runlimit_fixed_windows.extra_expression",
        "expression indexes",
    ),
    (
        "CREATE INDEX extra_partial ON runlimit_fixed_windows (policy_id) WHERE used > 1",
        "runlimit_fixed_windows.extra_partial",
        "partial indexes",
    ),
    (
        "ALTER TABLE runlimit_fixed_windows DROP CONSTRAINT runlimit_fixed_windows_capacity_shard_fkey",
        "runlimit_fixed_windows.runlimit_fixed_windows_capacity_shard_fkey",
        "FOREIGN KEY",
    ),
    (
        "ALTER FUNCTION runlimit_fixed_windows_capacity_after_insert() SET search_path = public",
        "runlimit_fixed_windows_capacity_after_insert",
        "fixed search_path",
    ),
    (
        "CREATE TRIGGER extra_trigger BEFORE INSERT ON runlimit_fixed_windows FOR EACH ROW EXECUTE FUNCTION runlimit_fixed_windows_reject_storage_key_update()",
        "runlimit_fixed_windows.extra_trigger",
        "additional user triggers",
    ),
    (
        "CREATE FUNCTION get_byte(bytea, integer) RETURNS integer LANGUAGE sql IMMUTABLE AS 'SELECT 0'; ALTER TABLE runlimit_fixed_windows DROP CONSTRAINT runlimit_fixed_windows_capacity_shard_fkey; ALTER TABLE runlimit_fixed_windows DROP COLUMN capacity_shard; ALTER TABLE runlimit_fixed_windows ADD COLUMN capacity_shard smallint GENERATED ALWAYS AS ((get_byte(config_fingerprint, 0) # get_byte(subject_key, 0))::smallint) STORED NOT NULL",
        "runlimit_fixed_windows.capacity_shard",
        "published stored XOR shard expression",
    ),
    (
        "CREATE FUNCTION octet_length(text) RETURNS integer LANGUAGE sql IMMUTABLE AS 'SELECT 1'; ALTER TABLE runlimit_fixed_windows DROP CONSTRAINT runlimit_fixed_windows_policy_id_size; ALTER TABLE runlimit_fixed_windows ADD CONSTRAINT runlimit_fixed_windows_policy_id_size CHECK (octet_length(policy_id) BETWEEN 1 AND 128)",
        "runlimit_fixed_windows.runlimit_fixed_windows_policy_id_size",
        "validated",
    ),
];

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_detects_schema_drift_even_with_valid_migration_history() {
    for &(mutation, object, requirement) in SCHEMA_DRIFT_CASES {
        let fixture = installed().await;
        sqlx::raw_sql(mutation)
            .execute(&fixture.primary_pool)
            .await
            .unwrap();
        let before = snapshot(&fixture.primary_pool).await;
        let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
        assert_issue(
            limiter
                .validate_installation(MigrationHistory::Bundled)
                .await
                .unwrap_err(),
            object,
            requirement,
        );
        assert_eq!(snapshot(&fixture.primary_pool).await, before);
        fixture.teardown().await;
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_rejects_generated_column_that_breaks_admission() {
    let fixture = installed().await;
    sqlx::query(
        "ALTER TABLE runlimit_fixed_windows ADD COLUMN extra bigint \
         GENERATED ALWAYS AS (1::bigint / (used - 1)) STORED",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows.extra",
        "additional generated",
    );
    let policy = FixedWindowPolicy::new(
        PolicyId::new("generated").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([81; 32]).bind(&policy));
    match limiter.check(&check).await.unwrap_err() {
        CheckError::DefinitelyNotConsumed(sqlx::Error::Database(database)) => {
            assert_eq!(database.code().as_deref(), Some("22012"));
        }
        other => panic!("expected generated-column write failure: {other:?}"),
    }
    sqlx::query("ALTER TABLE runlimit_fixed_windows DROP COLUMN extra")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE runlimit_fixed_windows ADD COLUMN extra text")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    assert!(limiter.check(&check).await.unwrap().permits_request());
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_rejects_extra_default_that_denies_runtime_admission() {
    let fixture = installed().await;
    sqlx::raw_sql(
        "CREATE SEQUENCE private_sequence; \
         ALTER TABLE runlimit_fixed_windows ADD COLUMN extra bigint DEFAULT nextval('private_sequence')",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let runtime = RuntimeRole::new(&fixture).await;
    let limiter = PostgresLimiter::new(runtime.pool.clone());
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows.extra",
        "additional defaults",
    );
    let policy = FixedWindowPolicy::new(
        PolicyId::new("default").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([83; 32]).bind(&policy));
    match limiter.check(&check).await.unwrap_err() {
        CheckError::DefinitelyNotConsumed(sqlx::Error::Database(database)) => {
            assert_eq!(database.code().as_deref(), Some("42501"));
        }
        other => panic!("expected denied default execution: {other:?}"),
    }
    runtime.teardown(&fixture).await;
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_rejects_nonunique_expression_index_that_breaks_admission() {
    let fixture = installed().await;
    sqlx::query(
        "CREATE INDEX extra_expression ON runlimit_fixed_windows ((1::bigint / (used - 1)))",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows.extra_expression",
        "expression indexes",
    );
    let policy = FixedWindowPolicy::new(
        PolicyId::new("index").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([84; 32]).bind(&policy));
    match limiter.check(&check).await.unwrap_err() {
        CheckError::DefinitelyNotConsumed(sqlx::Error::Database(database)) => {
            assert_eq!(database.code().as_deref(), Some("22012"));
        }
        other => panic!("expected expression-index write failure: {other:?}"),
    }
    sqlx::raw_sql("DROP INDEX extra_expression; CREATE INDEX extra_plain ON runlimit_fixed_windows (policy_id)")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    assert!(limiter.check(&check).await.unwrap().permits_request());
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_and_admission_use_qualified_builtins() {
    let fixture = installed().await;
    sqlx::raw_sql(
        "CREATE FUNCTION pg_advisory_xact_lock(bigint) RETURNS void \
         LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'shadowed lock'; END$$; \
         CREATE FUNCTION set_config(text, text, boolean) RETURNS text \
         LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'shadowed setting'; END$$; \
         CREATE FUNCTION octet_length(text) RETURNS integer \
         LANGUAGE sql IMMUTABLE AS 'SELECT 0'; \
         CREATE FUNCTION octet_length(bytea) RETURNS integer \
         LANGUAGE sql IMMUTABLE AS 'SELECT 0'; \
         CREATE FUNCTION get_byte(bytea, integer) RETURNS integer \
         LANGUAGE sql IMMUTABLE AS 'SELECT 0'",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    assert!(
        sqlx::query("SELECT pg_advisory_xact_lock(1::bigint)")
            .execute(&fixture.primary_pool)
            .await
            .is_err(),
        "fixture must resolve the application-schema function first"
    );
    let shadowed: i32 = sqlx::query_scalar("SELECT octet_length('x'::text)")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert_eq!(shadowed, 0, "fixture must shadow the inspected built-in");
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("shadow").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([82; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_bundled_history_requires_successful_published_checksums() {
    for mutation in [
        "DELETE FROM _sqlx_migrations WHERE version = 20260726000000",
        "UPDATE _sqlx_migrations SET success = false WHERE version = 20260726000000",
        "UPDATE _sqlx_migrations SET checksum = decode('00', 'hex') WHERE version = 20260726000000",
    ] {
        let fixture = installed().await;
        sqlx::query(mutation)
            .execute(&fixture.primary_pool)
            .await
            .unwrap();
        let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
        assert_issue(
            limiter
                .validate_installation(MigrationHistory::Bundled)
                .await
                .unwrap_err(),
            "_sqlx_migrations[20260726000000]",
            "published checksum",
        );
        limiter
            .validate_installation(MigrationHistory::ApplicationManaged)
            .await
            .unwrap();
        fixture.teardown().await;
    }
}

struct RuntimeRole {
    name: String,
    pool: PgPool,
}

impl RuntimeRole {
    async fn new(fixture: &IsolatedSchema) -> Self {
        let name = format!(
            "rl_validate_{}_{}",
            process::id(),
            NEXT_POLICY.fetch_add(1, Ordering::Relaxed)
        );
        let schema = &fixture.schema;
        // Identifiers are generated by the fixture, never external input.
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE ROLE {name}; \
             REVOKE ALL ON FUNCTION {schema}.runlimit_fixed_windows_reject_storage_key_update(), \
                 {schema}.runlimit_fixed_windows_capacity_after_insert(), \
                 {schema}.runlimit_fixed_windows_capacity_after_delete() FROM PUBLIC; \
             GRANT USAGE ON SCHEMA {schema} TO {name}; \
             GRANT SELECT, INSERT, DELETE ON {schema}.runlimit_fixed_windows TO {name}; \
             GRANT UPDATE (policy_id, scope_id, window_started_at, window_expires_at, used) \
                 ON {schema}.runlimit_fixed_windows TO {name}; \
             GRANT SELECT ON {schema}.runlimit_capacity_shards, {schema}._sqlx_migrations TO {name}; \
             GRANT UPDATE (capacity_shard) ON {schema}.runlimit_capacity_shards TO {name}"
        ))).execute(&fixture.admin_pool).await.unwrap();
        let setup = format!("SET ROLE {name}; SET search_path = {schema}, pg_catalog");
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |connection, _| {
                let setup = setup.clone();
                Box::pin(async move {
                    sqlx::raw_sql(AssertSqlSafe(setup))
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&std::env::var(TEST_DATABASE_URL).unwrap())
            .await
            .unwrap();
        Self { name, pool }
    }

    async fn teardown(self, fixture: &IsolatedSchema) {
        self.pool.close().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP OWNED BY {}; DROP ROLE {}",
            self.name, self.name
        )))
        .execute(&fixture.admin_pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_accepts_runtime_column_grants_and_checks_each_permission() {
    let fixture = installed().await;
    let runtime = RuntimeRole::new(&fixture).await;
    let limiter = PostgresLimiter::new(runtime.pool.clone());
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    // Success must correspond to real usable admission and cleanup, including
    // UPDATE privilege used only for SELECT FOR UPDATE on the capacity ledger.
    let policy = FixedWindowPolicy::new(
        PolicyId::new("runtime").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_millis(1),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([72; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    assert!(limiter.check(&check).await.unwrap().permits_request());
    sleep(Duration::from_millis(5)).await;
    assert_eq!(limiter.cleanup_expired(10).await.unwrap(), 1);

    let cases = [
        (
            "SELECT ON runlimit_fixed_windows",
            "runlimit_fixed_windows.used",
            "SELECT",
        ),
        (
            "INSERT ON runlimit_fixed_windows",
            "runlimit_fixed_windows.policy_id",
            "INSERT",
        ),
        (
            "DELETE ON runlimit_fixed_windows",
            "runlimit_fixed_windows",
            "DELETE",
        ),
        (
            "UPDATE (used) ON runlimit_fixed_windows",
            "runlimit_fixed_windows.used",
            "UPDATE",
        ),
        (
            "UPDATE (window_expires_at) ON runlimit_fixed_windows",
            "runlimit_fixed_windows.window_expires_at",
            "UPDATE",
        ),
        (
            "UPDATE (capacity_shard) ON runlimit_capacity_shards",
            "runlimit_capacity_shards",
            "row locking",
        ),
        (
            "SELECT ON runlimit_capacity_shards",
            "runlimit_capacity_shards.row_count",
            "SELECT",
        ),
        ("SELECT ON _sqlx_migrations", "_sqlx_migrations", "SELECT"),
    ];
    for (grant, object, requirement) in cases {
        sqlx::query(AssertSqlSafe(format!(
            "REVOKE {grant} FROM {}",
            runtime.name
        )))
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
        assert_issue(
            limiter
                .validate_installation(MigrationHistory::Bundled)
                .await
                .unwrap_err(),
            object,
            requirement,
        );
        sqlx::query(AssertSqlSafe(format!("GRANT {grant} TO {}", runtime.name)))
            .execute(&fixture.primary_pool)
            .await
            .unwrap();
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap();
    }
    runtime.teardown(&fixture).await;
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_deadlines_and_cancellation_release_connections() {
    let fixture = installed().await;
    let config = PostgresConfig::new()
        .with_pool_acquire_timeout(Duration::from_millis(80))
        .unwrap()
        .with_operation_timeout(Duration::from_millis(150))
        .unwrap();
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone()).with_config(config);
    let held = fixture.primary_pool.acquire().await.unwrap();
    assert!(matches!(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await,
        Err(InstallationError::TimedOut {
            operation: "acquiring database connection"
        })
    ));
    drop(held);

    let mut blocker = fixture.admin_pool.begin().await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "LOCK TABLE {}.runlimit_capacity_shards IN ACCESS EXCLUSIVE MODE",
        fixture.schema
    )))
    .execute(&mut *blocker)
    .await
    .unwrap();
    assert!(matches!(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await,
        Err(InstallationError::TimedOut { .. })
    ));
    blocker.rollback().await.unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();

    let long = PostgresLimiter::new(fixture.primary_pool.clone());
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    let mut blocker = fixture.admin_pool.begin().await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "LOCK TABLE {}.runlimit_capacity_shards IN ACCESS EXCLUSIVE MODE",
        fixture.schema
    )))
    .execute(&mut *blocker)
    .await
    .unwrap();
    let task =
        tokio::spawn(async move { long.validate_installation(MigrationHistory::Bundled).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(pid).fetch_one(&fixture.admin_pool).await.unwrap();
            if waiting { break; }
            sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let replacement: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert_ne!(
        replacement, pid,
        "cancelled validation must discard its connection"
    );
    blocker.rollback().await.unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_column_select_cannot_replace_cleanup_system_column_access() {
    let fixture = installed().await;
    let runtime = RuntimeRole::new(&fixture).await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "REVOKE SELECT ON runlimit_fixed_windows FROM {0}; \
         GRANT SELECT (policy_id, scope_id, config_fingerprint, subject_key, \
         window_started_at, window_expires_at, used, capacity_shard) \
         ON runlimit_fixed_windows TO {0}",
        runtime.name
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let limiter = PostgresLimiter::new(runtime.pool.clone());
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::ApplicationManaged)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows.ctid",
        "SELECT",
    );
    assert!(
        limiter.cleanup_expired(1).await.is_err(),
        "missing grant actually prevents cleanup"
    );
    runtime.teardown(&fixture).await;
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_checks_definer_owner_privileges_and_accepts_inherited_grants() {
    let fixture = installed().await;
    let runtime = RuntimeRole::new(&fixture).await;
    let limiter = PostgresLimiter::new(runtime.pool.clone());
    let owner = format!("{}_owner", runtime.name);
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE ROLE {owner}; GRANT {0} TO {owner}; \
         ALTER FUNCTION runlimit_fixed_windows_capacity_after_insert() OWNER TO {owner}",
        runtime.name
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    // This owner inherits runtime grants but does not yet have permission to
    // maintain row_count. Reading the function's SECURITY DEFINER flag alone
    // would incorrectly certify the installation.
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows_capacity_after_insert",
        "function owner",
    );
    sqlx::query(AssertSqlSafe(format!(
        "GRANT UPDATE (row_count) ON runlimit_capacity_shards TO {owner}"
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    sqlx::query("REVOKE EXECUTE ON FUNCTION pg_catalog.format(text, \"any\") FROM PUBLIC")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows_capacity_after_insert",
        "capacity-function EXECUTE",
    );
    sqlx::query(AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION pg_catalog.format(text, \"any\") TO {owner}"
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    sqlx::query("GRANT EXECUTE ON FUNCTION pg_catalog.format(text, \"any\") TO PUBLIC")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    // The runtime role can also be inherited, rather than requiring direct ACLs.
    sqlx::query(AssertSqlSafe(format!("SET ROLE {owner}")))
        .execute(&runtime.pool)
        .await
        .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("definer").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([53; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    // Move the fixture function back before removing this fixture-only role.
    sqlx::query(
        "ALTER FUNCTION runlimit_fixed_windows_capacity_after_insert() OWNER TO CURRENT_USER",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    sqlx::query(AssertSqlSafe(format!("SET ROLE {}", runtime.name)))
        .execute(&runtime.pool)
        .await
        .unwrap();
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP OWNED BY {owner}; DROP ROLE {owner}"
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    runtime.teardown(&fixture).await;
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_obeys_search_path_and_rejects_disabled_session_triggers() {
    let fixture = installed().await;
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    sqlx::query("SET session_replication_role = replica")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::ApplicationManaged)
            .await
            .unwrap_err(),
        "session_replication_role",
        "origin",
    );
    sqlx::query("SET session_replication_role = origin")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    // pg_temp implicitly precedes the normal search path. Validation must inspect
    // the table that runtime SQL would actually resolve, not assume public.
    sqlx::query("CREATE TEMP TABLE runlimit_fixed_windows (used bigint)")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::ApplicationManaged)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows",
        "permanent",
    );
    sqlx::query("DROP TABLE pg_temp.runlimit_fixed_windows")
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    fixture.teardown().await;
}
