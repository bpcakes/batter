use super::*;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_rejects_inbound_foreign_key_that_blocks_cleanup() {
    let fixture = installed().await;
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    let policy = FixedWindowPolicy::new(
        PolicyId::new("reference").unwrap(),
        ScopeId::new("test").unwrap(),
        1,
        Duration::from_millis(1),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([86; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    sqlx::raw_sql(
        "CREATE TABLE dependent_windows (
             config_fingerprint bytea, subject_key bytea,
             FOREIGN KEY (config_fingerprint, subject_key) REFERENCES runlimit_fixed_windows
         );
         INSERT INTO dependent_windows
             SELECT config_fingerprint, subject_key FROM runlimit_fixed_windows",
    )
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    assert_issue(
        limiter
            .validate_installation(MigrationHistory::Bundled)
            .await
            .unwrap_err(),
        "runlimit_fixed_windows.dependent_windows_config_fingerprint_subject_key_fkey",
        "external foreign keys",
    );
    sleep(Duration::from_millis(10)).await;
    match limiter.cleanup_expired(1).await.unwrap_err() {
        MaintenanceError::Database(sqlx::Error::Database(database)) => {
            assert_eq!(database.code().as_deref(), Some("23503"));
        }
        other => panic!("expected foreign-key cleanup failure: {other:?}"),
    }
    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_rejects_publication_settings_that_block_updates() {
    for (setup, table_clause, requirement, sqlstate) in [
        (
            "ALTER TABLE runlimit_fixed_windows REPLICA IDENTITY NOTHING",
            "",
            "default replica identity",
            "55000",
        ),
        ("", "WHERE (used > 1)", "no row filter", "42P10"),
        ("", "(used)", "primary-key columns", "42P10"),
    ] {
        let fixture = installed().await;
        let publication = format!(
            "publication_{}",
            fixture.schema.trim_start_matches("runlimit_migration_")
        );
        if !setup.is_empty() {
            sqlx::query(setup)
                .execute(&fixture.primary_pool)
                .await
                .unwrap();
        }
        sqlx::query(AssertSqlSafe(format!(
            "CREATE PUBLICATION {publication} FOR TABLE runlimit_fixed_windows {table_clause}"
        )))
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
        let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
        assert_issue(
            limiter
                .validate_installation(MigrationHistory::Bundled)
                .await
                .unwrap_err(),
            &format!("runlimit_fixed_windows.{publication}"),
            requirement,
        );
        let policy = FixedWindowPolicy::new(
            PolicyId::new("publication").unwrap(),
            ScopeId::new("test").unwrap(),
            2,
            Duration::from_secs(60),
        )
        .unwrap();
        let check = Check::new(SubjectKey::from_digest([87; 32]).bind(&policy));
        let first = limiter.check(&check).await;
        let error = match first {
            Ok(decision) => {
                assert!(decision.permits_request());
                limiter.check(&check).await.unwrap_err()
            }
            Err(error) => error,
        };
        match error {
            CheckError::DefinitelyNotConsumed(sqlx::Error::Database(database)) => {
                assert_eq!(database.code().as_deref(), Some(sqlstate));
            }
            other => panic!("expected publication write failure: {other:?}"),
        }
        sqlx::query(AssertSqlSafe(format!("DROP PUBLICATION {publication}")))
            .execute(&fixture.primary_pool)
            .await
            .unwrap();
        fixture.teardown().await;
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn installation_accepts_compatible_publication() {
    let fixture = installed().await;
    let publication = format!(
        "publication_{}",
        fixture.schema.trim_start_matches("runlimit_migration_")
    );
    sqlx::query(AssertSqlSafe(format!(
        "CREATE PUBLICATION {publication} FOR TABLE runlimit_fixed_windows"
    )))
    .execute(&fixture.primary_pool)
    .await
    .unwrap();
    let limiter = PostgresLimiter::new(fixture.primary_pool.clone());
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("publication").unwrap(),
        ScopeId::new("test").unwrap(),
        2,
        Duration::from_millis(1),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([88; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());
    assert!(limiter.check(&check).await.unwrap().permits_request());
    sleep(Duration::from_millis(10)).await;
    assert_eq!(limiter.cleanup_expired(1).await.unwrap(), 1);
    sqlx::query(AssertSqlSafe(format!("DROP PUBLICATION {publication}")))
        .execute(&fixture.primary_pool)
        .await
        .unwrap();
    fixture.teardown().await;
}
