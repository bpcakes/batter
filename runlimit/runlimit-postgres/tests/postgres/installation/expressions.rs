use super::*;

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
async fn admission_ignores_application_operator_overload_and_restores_search_path() {
    let fixture = installed().await;
    sqlx::raw_sql(
        "CREATE FUNCTION shadow_add(bigint, bigint) RETURNS bigint \
         LANGUAGE sql IMMUTABLE AS 'SELECT $1'; \
         CREATE OPERATOR + (LEFTARG = bigint, RIGHTARG = bigint, FUNCTION = shadow_add)",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let shadowed: i64 = sqlx::query_scalar("SELECT 1::bigint + 1::bigint")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert_eq!(shadowed, 1, "fixture must shadow bigint addition");

    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("operator").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_secs(60),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([85; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    assert!(limiter.check(&check).await.unwrap().permits_request());
    assert!(!limiter.check(&check).await.unwrap().permits_request());
    let stored: i64 = sqlx::query_scalar("SELECT used FROM runlimit_fixed_windows")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert_eq!(stored, 2);
    let path: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(&fixture.primary_pool)
        .await
        .unwrap();
    assert!(path.starts_with(&fixture.schema));
    fixture.teardown().await;
}
