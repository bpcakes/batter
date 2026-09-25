use super::*;
use runlimit_postgres::MigrationHistory;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn cleanup_uses_checked_count_and_restores_search_path() {
    let fixture = IsolatedSchema::for_migration_test(1).await;
    let pool = &fixture.primary_pool;
    let limiter = PostgresLimiter::new(pool.clone());
    limiter.migrate().await.unwrap();
    let policy = FixedWindowPolicy::new(
        PolicyId::new("shadow").unwrap(),
        ScopeId::new("test").unwrap(),
        1,
        Duration::from_millis(1),
    )
    .unwrap();
    let check = Check::new(SubjectKey::from_digest([83; 32]).bind(&policy));
    assert!(limiter.check(&check).await.unwrap().permits_request());

    sqlx::raw_sql(
        "CREATE FUNCTION shadow_count_step(bigint) RETURNS bigint \
         LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'shadowed count'; END$$; \
         CREATE AGGREGATE count(*) \
         (SFUNC = shadow_count_step, STYPE = bigint, INITCOND = '0')",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(
        sqlx::query("SELECT count(*) FROM runlimit_fixed_windows")
            .execute(pool)
            .await
            .is_err(),
        "fixture must resolve the application-schema aggregate first"
    );
    limiter
        .validate_installation(MigrationHistory::Bundled)
        .await
        .unwrap();
    let search_path: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(pool)
        .await
        .unwrap();
    sleep(Duration::from_millis(20)).await;
    assert_eq!(limiter.cleanup_expired(1).await.unwrap(), 1);
    let after: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        after, search_path,
        "cleanup restores the caller's search path"
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT pg_catalog.count(*) FROM runlimit_fixed_windows")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    fixture.teardown().await;
}
