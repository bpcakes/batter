//! The flags control plane through the facade against a dedicated
//! disposable PostgreSQL 18 cluster: the migration ledger shared with
//! Runledger and Runlimit, and the restore remedy. Run explicitly:
//!
//! ```text
//! BATTER_SQLX_ADMIN_URL=postgres://... \
//!   cargo test -p batter --features runledger,runlimit-postgres,flags-postgres \
//!   --test flags_live -- --ignored --test-threads=1
//! ```
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "flags_live/support.rs"]
mod support;

use std::time::Duration;

use batter::flags::{
    control_plane::{Gate, LatestRead, StartupFile},
    core::{Source, SwitchState, Version},
    low_level::capture_now,
    postgres::{PooledStore, Refusal, Write, WriteOutcome},
};
use sqlx::postgres::PgPoolOptions;
use support::*;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn runledger_runlimit_and_flags_migrate_in_that_order_into_one_database() {
    let database = Database::create("ledger").await;
    let body = {
        let database = database.clone();
        async move {
            let login = database.options.get_username().to_owned();
            let profile = batter::runledger::PgSessionProfile::new(
                &login,
                &login,
                vec!["public".into()],
                Duration::ZERO,
                Duration::ZERO,
            )
            .unwrap();
            let runledger = batter::runledger::native::postgres::low_level::connect_database_lazy(
                database.options.clone(),
                profile,
                PgPoolOptions::new().max_connections(2),
            )
            .unwrap();
            batter::runledger::native::postgres::migrate_after_idempotency_cutover(&runledger)
                .await
                .unwrap();
            let pool = database.pool().await;
            batter::runlimit::postgres::PostgresLimiter::new(pool.clone())
                .migrate()
                .await
                .unwrap();
            let store = PooledStore::new(pool.clone(), timing());
            store.migrate(deadline()).await.unwrap();
            // Every package's ledger tolerates the others' rows, in either rerun.
            store.migrate(deadline()).await.unwrap();
            batter::runledger::native::postgres::migrate_after_idempotency_cutover(&runledger)
                .await
                .unwrap();
            batter::runlimit::postgres::PostgresLimiter::new(pool.clone())
                .migrate()
                .await
                .unwrap();
            assert_eq!(
                store.load(deadline()).await.unwrap().version(),
                Version::new(0)
            );
            runledger.pool().close().await;
            pool.close().await;
        }
    };
    with_cleanup(
        body,
        database.drop_all(std::slice::from_ref(&database.name)),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
async fn a_restore_below_the_fleet_refuses_writes_until_the_process_restarts_on_the_restored_head()
{
    let database = Database::create("restore").await;
    let restored = format!("{}_at90", database.name);
    let retired = format!("{}_old", database.name);
    let directory = std::env::temp_dir().join(format!(
        "batter-flags-live-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&directory).unwrap();
    let names = [database.name.clone(), restored.clone(), retired.clone()];
    let cleanup_directory = directory.clone();
    let cleanup = {
        let database = database.clone();
        async move {
            database.drop_all(&names).await;
            let _ = std::fs::remove_dir_all(&cleanup_directory);
        }
    };
    with_cleanup(
        restore_remedy(database, restored, retired, directory),
        cleanup,
    )
    .await;
}

async fn restore_remedy(
    database: Database,
    restored: String,
    retired: String,
    directory: std::path::PathBuf,
) {
    reach_100_with_a_restore_point_at_90(&database, &restored).await;
    // A serving process applies 100, persists it and reports it.
    let (flags, running) = start(&database.options, &directory).await;
    until(|| matches!(capture_now(&flags).source(), Source::ControlPlane { version, .. } if version == Version::new(100))).await;
    let probe = database.pool().await;
    heartbeat_reports(&probe, flags.process().as_uuid(), ("control_plane", 100)).await;
    probe.close().await;
    fail_over_under_the_running_process(&database, &restored, &retired, &flags).await;
    running.shutdown_checked().await.unwrap();
    restart_over_the_file(&database, &directory).await;
}

/// The store reaches 90, is copied as a restore point, then reaches 100.
async fn reach_100_with_a_restore_point_at_90(database: &Database, restored: &str) {
    let pool = database.pool().await;
    let store = PooledStore::new(pool.clone(), timing());
    store.migrate(deadline()).await.unwrap();
    write_until(&store, 90).await;
    pool.close().await;
    database
        .exclusive(
            &database.name,
            &format!("CREATE DATABASE {restored} TEMPLATE {}", database.name),
        )
        .await;
    let pool = database.pool().await;
    write_until(&PooledStore::new(pool.clone(), timing()), 100).await;
    pool.close().await;
}

/// The database name now serves the restore point at 90 under the running
/// process, which keeps 100 and reports it, so writes are refused.
async fn fail_over_under_the_running_process(
    database: &Database,
    restored: &str,
    retired: &str,
    flags: &batter::flags::control_plane::RegisteredFlags,
) {
    database
        .exclusive(
            &database.name,
            &format!("ALTER DATABASE {} RENAME TO {retired}", database.name),
        )
        .await;
    database
        .exclusive(
            restored,
            &format!("ALTER DATABASE {restored} RENAME TO {}", database.name),
        )
        .await;
    until(|| {
        flags.status().latest_read
            == LatestRead::Refused {
                refusal: batter::flags::core::low_level::Refusal::Regressed {
                    applied: Version::new(100),
                    observed: Version::new(90),
                },
                observed: Version::new(90),
            }
    })
    .await;
    let pool = database.pool().await;
    heartbeat_reports(&pool, flags.process().as_uuid(), ("control_plane", 100)).await;
    let refused = PooledStore::new(pool.clone(), timing())
        .write(
            Write::kill(halt(), SwitchState::Engaged, change()),
            deadline(),
        )
        .await
        .unwrap();
    assert!(
        matches!(refused, WriteOutcome::Refused(Refusal::StoreBehindFleet)),
        "{refused:?}"
    );
    pool.close().await;
}

/// The remedy: the process restarts over its version-100 file and publishes
/// the restored head from the control plane, after which a write succeeds.
async fn restart_over_the_file(database: &Database, directory: &std::path::Path) {
    let (flags, running) = start(&database.options, directory).await;
    assert_eq!(
        flags.startup_file(),
        StartupFile::Published {
            version: Version::new(100)
        }
    );
    until(|| matches!(capture_now(&flags).source(), Source::ControlPlane { version, .. } if version == Version::new(90))).await;
    assert!(
        flags.status().satisfies(Gate::Fresh),
        "{:?}",
        flags.status()
    );
    until(|| {
        std::fs::read(directory.join("flags.bfsn"))
            .is_ok_and(|bytes| bytes.windows(8).any(|w| w == 90u64.to_be_bytes()))
    })
    .await;
    let pool = database.pool().await;
    heartbeat_reports(&pool, flags.process().as_uuid(), ("control_plane", 90)).await;
    let written = PooledStore::new(pool.clone(), timing())
        .write(
            Write::kill(halt(), SwitchState::Engaged, change()),
            deadline(),
        )
        .await
        .unwrap();
    assert!(
        matches!(written, WriteOutcome::Written { .. }),
        "{written:?}"
    );
    running.shutdown_checked().await.unwrap();
    pool.close().await;
}
