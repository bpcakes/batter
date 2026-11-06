//! PostgreSQL 18 migrations contracts.
#![allow(
    clippy::unwrap_used,
    clippy::missing_assert_message,
    reason = "schema compatibility assertions"
)]
#[path = "support/fixture.rs"]
mod fixture;
use batter_flags_postgres::*;
use fixture::*;
use sha2::{Digest, Sha384};
use sqlx::{
    Row, SqlSafeStr,
    migrate::{Migration, MigrationType, Migrator},
};

#[test]
fn first_migration_checksum_is_immutable() {
    let checksum: String = Sha384::digest(CREATE_FLAGS_SQL.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        checksum,
        "012496b1a41f85d61d47df47505258cb25f4a27b789a557f16c6f50f63050558ef896ec21ed5a50364f526a44997cff2"
    );
    assert_eq!(
        MIGRATOR.iter().next().unwrap().checksum.as_ref(),
        Sha384::digest(CREATE_FLAGS_SQL.as_bytes()).as_slice()
    );
    assert_eq!(REQUIRED_SCHEMA_REVISION, 1);
}
fn host() -> Migrator {
    Migrator::with_migrations(vec![Migration::new(
        1,
        "host".into(),
        MigrationType::Simple,
        "CREATE TABLE public.host_data(id integer)".into_sql_str(),
        false,
    )])
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn migration_lock_budget_applies_before_the_first_migration_transaction() {
    use sqlx::migrate::Migrate;
    use std::{error::Error, time::Duration};
    let f = Fixture::new().await;
    let mut holder = f.pool.acquire().await.unwrap();
    holder.lock().await.unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(f.lease.database_url())
        .await
        .unwrap();
    let store = PooledStore::new(
        pool.clone(),
        StoreTiming::new(
            Duration::from_secs(2),
            Duration::from_secs(3),
            Duration::from_millis(75),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let started = tokio::time::Instant::now();
    let error = store.migrate(deadline()).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "lock budget must precede the whole operation deadline"
    );
    assert_eq!(pool.size(), 0);
    let mut cause: &(dyn Error + 'static) = &error;
    let mut states = Vec::new();
    loop {
        if let Some(native) = cause.downcast_ref::<sqlx::Error>()
            && let Some(db) = native.as_database_error()
        {
            states.push(db.code().unwrap().into_owned());
        }
        match cause.source() {
            Some(next) => cause = next,
            None => break,
        }
    }
    assert!(
        states.iter().any(|c| c == "55P03"),
        "retain the actual server lock-timeout error"
    );
    holder.unlock().await.unwrap();
    drop(holder);
    store.migrate(deadline()).await.unwrap();
    assert_eq!(
        pool.size(),
        0,
        "migration settings must not leak through pool reuse"
    );
    pool.close().await;
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn native_migration_owner_inherits_all_limits_into_native_transactions() {
    use std::time::Duration;
    let f = Fixture::new().await;
    let owner = batter_core::operation::OperationOwner::new(Duration::from_secs(3)).unwrap();
    let lease = batter_sqlx::PgLease::acquire(&f.pool, owner.context())
        .await
        .unwrap();
    let migrator = Migrator::with_migrations(vec![Migration::new(1, "limits".into(), MigrationType::Simple,
        "CREATE TABLE public.observed_limits AS SELECT current_setting('statement_timeout') AS statement, current_setting('lock_timeout') AS lock, current_setting('idle_in_transaction_session_timeout') AS idle".into_sql_str(), false)]);
    let limits = batter_sqlx::PgMigrationTimeouts::new(
        Duration::from_secs(60),
        Duration::from_millis(200),
        Duration::from_millis(300),
    )
    .unwrap();
    lease
        .migrate_until(
            &migrator,
            limits,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    let (statement, lock, idle): (f64, f64, f64) = sqlx::query_as("SELECT extract(epoch FROM statement::interval)::float8, extract(epoch FROM lock::interval)::float8, extract(epoch FROM idle::interval)::float8 FROM public.observed_limits")
        .fetch_one(&f.pool).await.unwrap();
    assert!(statement > 0.0 && statement <= 2.0);
    assert_eq!(lock, 0.2);
    assert_eq!(idle, 0.3);
    f.close().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn dedicated_ledger_coexists_in_both_orders_and_strict_host_stays_strict() {
    for order in ["host_first", "flags_first"] {
        let f = Fixture::new().await;
        if order == "host_first" {
            host().run(&f.pool).await.unwrap();
        }
        f.store.migrate(deadline()).await.unwrap();
        if order == "flags_first" {
            host().run(&f.pool).await.unwrap();
        }
        host().run(&f.pool).await.unwrap();
        MIGRATOR.run(&f.pool).await.unwrap();
        let flags: i64 = sqlx::query_scalar("SELECT count(*) FROM flags._sqlx_migrations")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(flags, 1);
        let mut foreign = Migrator::with_migrations(vec![Migration::new(
            2,
            "foreign".into(),
            MigrationType::Simple,
            "CREATE TABLE public.foreign_data(id integer)".into_sql_str(),
            false,
        )]);
        foreign.set_ignore_missing(true);
        foreign.run(&f.pool).await.unwrap();
        assert!(matches!(
            host().run(&f.pool).await,
            Err(sqlx::migrate::MigrateError::VersionMissing(2))
        ));
        f.store.migrate(deadline()).await.unwrap();
        f.close().await;
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn v1_column_lists_and_definer_privileges_are_pinned() {
    let f = Fixture::migrated().await;
    let tables = [
        ("flag_control", "singleton version channel schema_revision"),
        (
            "flag_audit",
            "version flag_name operation actor reason guard recorded_by before_state after_state recorded_at",
        ),
        (
            "flag_state",
            "name kind value_type salt state created_version updated_version archived_version created_at updated_at",
        ),
        (
            "flag_catalogs",
            "manifest_digest catalog_digest manifest first_seen last_referenced_at",
        ),
        (
            "flag_catalog_entries",
            "manifest_digest key kind value_type fingerprint variants default_variant expiry owners only_in safe_state stale_rule governed_by flip_discipline",
        ),
        (
            "flag_processes",
            "process_id application environment build manifest_digest snapshot_source snapshot_version declared_revision declared_shared_revision row_encoding held_keys held_overflow floor_engaged_keys floor_overflow liveness_seconds retention_seconds expires_at started_at last_seen_at stopped_at",
        ),
    ];
    for (table, columns) in tables {
        let rows=sqlx::query("SELECT column_name,is_nullable,column_default FROM information_schema.columns WHERE table_schema='flags' AND table_name=$1 ORDER BY ordinal_position").bind(table).fetch_all(&f.pool).await.unwrap();
        let expected: Vec<_> = columns.split_whitespace().collect();
        for name in &expected {
            assert!(
                rows.iter()
                    .any(|r| r.get::<String, _>("column_name") == *name),
                "missing {table}.{name}"
            );
        }
        for row in rows {
            let name: String = row.get("column_name");
            if !expected.contains(&name.as_str()) {
                assert!(
                    row.get::<String, _>("is_nullable") == "YES"
                        || row.get::<Option<String>, _>("column_default").is_some(),
                    "new required column breaks v1 insert: {table}.{name}"
                );
            }
        }
    }
    let routines=sqlx::query("SELECT prosecdef,proconfig,NOT EXISTS (SELECT FROM aclexplode(p.proacl) a WHERE a.grantee=0 AND privilege_type='EXECUTE') AS revoked FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='flags'").fetch_all(&f.pool).await.unwrap();
    assert_eq!(routines.len(), 6);
    for row in routines {
        assert!(row.get::<bool, _>("prosecdef"));
        assert!(row.get::<bool, _>("revoked"));
        assert_eq!(
            row.get::<Vec<String>, _>("proconfig"),
            ["search_path=pg_catalog, pg_temp"]
        );
    }
    f.close().await;
}
