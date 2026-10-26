//! Distinct operation and advisory-unlock errors survive both entry points.

use std::time::Duration;

use runledger_postgres::{
    PgSessionProfile, RunledgerDatabase, SchemaCompatibilityError,
    migrate_after_idempotency_cutover, prepare_online_indexes_for_external_migrator,
};
use sqlx::{AssertSqlSafe, migrate::MigrateError};

mod online_support;
use online_support::{Harness, LAST_VERSION_BEFORE_CUTOVER, raw_apply_through, wait_for};

#[tokio::test]
async fn both_entry_points_retain_independent_operation_and_unlock_failures() {
    for prepare in [false, true] {
        let harness = Harness::fresh("online_unlock_causes").await;
        raw_apply_through(&harness.pool, LAST_VERSION_BEFORE_CUTOVER).await;
        sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
            .bind(vec![0_u8; 48])
            .bind(LAST_VERSION_BEFORE_CUTOVER)
            .execute(&harness.pool)
            .await
            .expect("foreign history checksum");
        let role = format!("unlock_reader_{}", sqlx::types::Uuid::new_v4().simple());
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE ROLE {role};
             GRANT USAGE, CREATE ON SCHEMA public TO {role};
             GRANT SELECT ON ALL TABLES IN SCHEMA public TO {role};
             REVOKE EXECUTE ON FUNCTION pg_catalog.pg_advisory_unlock(bigint) FROM PUBLIC"
        )))
        .execute(&harness.pool)
        .await
        .expect("independently deny unlock in this database");
        let options = harness.pool.connect_options();
        let profile = PgSessionProfile::new(
            options.get_username(),
            &role,
            vec!["public".into()],
            Duration::ZERO,
            Duration::ZERO,
        )
        .expect("restricted profile");
        let database = RunledgerDatabase::connect((*options).clone(), profile, 1)
            .await
            .expect("restricted migration database");
        let pid: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(database.pool())
            .await
            .expect("owned connection pid");
        let error = if prepare {
            prepare_online_indexes_for_external_migrator(&database)
                .await
                .map(|_| ())
        } else {
            migrate_after_idempotency_cutover(&database).await
        }
        .expect_err("operation and unlock both fail");
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains("permission denied"));
        assert!(!rendered.contains("pg_advisory_unlock"));
        assert!(!rendered.contains(&LAST_VERSION_BEFORE_CUTOVER.to_string()));
        let SchemaCompatibilityError::MigrationCleanup(failure) = error else {
            panic!("combined failure required: {error}");
        };
        assert!(matches!(
            failure.operation_error(),
            SchemaCompatibilityError::Incompatible(MigrateError::VersionMismatch(
                LAST_VERSION_BEFORE_CUTOVER
            ))
        ));
        let MigrateError::Execute(unlock) = failure.unlock_error() else {
            panic!("native unlock failure required");
        };
        assert_eq!(
            unlock
                .as_database_error()
                .expect("permission error")
                .code()
                .as_deref(),
            Some("42501")
        );
        assert!(unlock.to_string().contains("pg_advisory_unlock"));
        // Confirm local session retirement independently; the error itself
        // makes no promise about backend death across network/runtime failure.
        wait_for("failed-unlock session and its locks to retire", || async {
            sqlx::query_scalar::<_, bool>(
                "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid = $1)
                 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE pid = $1)",
            )
            .bind(pid)
            .fetch_one(&harness.pool)
            .await
            .expect("backend observation")
        })
        .await;
        database.pool().close().await;
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP OWNED BY {role}; DROP ROLE {role}"
        )))
        .execute(&harness.pool)
        .await
        .expect("remove test role");
        harness.teardown().await;
    }
}
