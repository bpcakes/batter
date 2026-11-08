//! The registered control plane against a disposable PostgreSQL 18 database:
//! the heartbeat row it writes, the snapshot it loads and the stop marker that
//! runs before the pool closes.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use std::time::Duration;

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use batter_flags::{
    control_plane::{
        ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, LoadPolicy, Presence,
        RefreshTiming, SnapshotStatus, register_in,
    },
    core::{CatalogBuilder, CivilDate, SafeState, SettingsPrefix, StaleRule},
    declared,
    postgres::{PooledStore, StoreTiming},
};
use batter_sqlx::{PgSessionProfile, pool_in};
use postgres_test_harness::{HarnessConfig, PostgresHarness};
use sqlx::{
    Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::time::Instant;

const SECOND: Duration = Duration::from_secs(1);

#[tokio::test]
#[ignore = "requires disposable PostgreSQL 18"]
async fn a_registered_process_heartbeats_loads_and_records_its_stop_before_the_pool_closes() {
    let url = std::env::var("BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL")
        .expect("explicit disposable PostgreSQL 18 URL required");
    let harness = PostgresHarness::start(
        HarnessConfig::new("batter_flags_cp")
            .unwrap()
            .with_admin_database_url(url),
    )
    .await
    .unwrap();
    let lease = harness.empty_database().await.unwrap();
    let options: PgConnectOptions = lease.database_url().parse().unwrap();
    let login = options.get_username().to_owned();
    let timing = StoreTiming::new(SECOND, SECOND, SECOND, SECOND).unwrap();

    // A finite migration step, as a root runs it before serving.
    let migrator = PgPoolOptions::new()
        .connect(lease.database_url())
        .await
        .unwrap();
    PooledStore::new(migrator.clone(), timing)
        .migrate(Instant::now() + SECOND * 30)
        .await
        .unwrap();

    let resolved = resolved();
    let mut supervisor = supervisor();
    let profile = PgSessionProfile::new(
        &login,
        &login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let pool = pool_in(
        supervisor.reserve_cleanup("database").unwrap(),
        PgPoolOptions::new().max_connections(4),
        options,
        profile,
    )
    .unwrap();
    let presence = Presence::new(
        ApplicationName::new("service").unwrap(),
        BuildId::new("1.0.0+live").unwrap(),
        HeartbeatCadence::new(SECOND * 5, SECOND * 60).unwrap(),
    );
    let flags = register_in(
        &mut supervisor,
        ControlPlaneStore::new(&pool, timing),
        resolved,
        LoadPolicy::start_on_defaults(RefreshTiming::new(SECOND * 5, SECOND * 20).unwrap()),
        presence,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    // Give the first load and heartbeat a moment to commit.
    for _ in 0..50 {
        if flags.status() == SnapshotStatus::Fresh && flags.presence().succeeded > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(flags.status(), SnapshotStatus::Fresh);
    assert!(flags.presence().succeeded > 0, "{:?}", flags.presence());

    assert_heartbeat_row(&migrator, flags.process().as_uuid()).await;

    // The stop marker runs before the pool registered first closes.
    running.shutdown_checked().await.unwrap();
    assert!(pool.pool().is_closed());
    let stopped: bool = sqlx::query_scalar(
        "SELECT stopped_at IS NOT NULL FROM flags.flag_processes WHERE process_id = $1",
    )
    .bind(flags.process().as_uuid())
    .fetch_one(&migrator)
    .await
    .unwrap();
    assert!(stopped, "the stop marker ran before the pool closed");
    assert_eq!(flags.status(), SnapshotStatus::LoaderStopped);
    migrator.close().await;
    lease.cleanup().await.unwrap();
    harness.shutdown().await.unwrap();
}

fn resolved() -> declared::Resolved {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let halt_handle_catalog = catalog.build().unwrap();
    let source = SettingsSource::from_pairs([("FLAG_HALT".into(), "engaged".into())]).unwrap();
    declared::resolve(
        halt_handle_catalog,
        &source,
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap()
}

fn supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            SECOND * 5,
            SECOND * 5,
            SECOND * 5,
            CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5).unwrap(),
        )
        .unwrap(),
    )
}

/// What the registered process's heartbeat row says once it reports the
/// publication.
async fn assert_heartbeat_row(migrator: &sqlx::PgPool, process: uuid::Uuid) {
    // The first heartbeat runs beside the first load and may report the
    // declared base; the next one reports the publication.
    for _ in 0..80 {
        let source: Option<String> = sqlx::query_scalar(
            "SELECT snapshot_source FROM flags.flag_processes WHERE process_id = $1",
        )
        .bind(process)
        .fetch_optional(migrator)
        .await
        .unwrap();
        if source.as_deref() == Some("control_plane") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let row = sqlx::query("SELECT application, environment, build, snapshot_source, snapshot_version, liveness_seconds, retention_seconds, floor_engaged_keys, stopped_at IS NULL AS running FROM flags.flag_processes WHERE process_id = $1")
        .bind(process)
        .fetch_one(migrator)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("application"), "service");
    assert_eq!(row.get::<String, _>("environment"), "hosted");
    assert_eq!(row.get::<String, _>("build"), "1.0.0+live");
    assert_eq!(row.get::<String, _>("snapshot_source"), "control_plane");
    assert_eq!(row.get::<Option<i64>, _>("snapshot_version"), Some(0));
    // Twice the interval plus the heartbeat budget: 1 s + 7 x 1 s + 1 s.
    assert_eq!(row.get::<i32, _>("liveness_seconds"), 19);
    assert_eq!(row.get::<i32, _>("retention_seconds"), 60);
    assert_eq!(row.get::<Vec<String>, _>("floor_engaged_keys"), ["halt"]);
    assert!(row.get::<bool, _>("running"));
    let catalogs: i64 = sqlx::query_scalar("SELECT count(*) FROM flags.flag_catalogs")
        .fetch_one(migrator)
        .await
        .unwrap();
    assert_eq!(catalogs, 1, "the heartbeat published the catalog");
}
