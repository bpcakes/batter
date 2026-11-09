//! A database, serving processes and probes for the NOTIFY live tests.
#![allow(dead_code, reason = "shared support; each test uses part of it")]

use std::time::Duration;

use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use batter_flags::{
    control_plane::{
        ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, ListenEndpoint, LoadPolicy,
        Presence, RefreshTiming, RegisteredFlags, register_in,
    },
    core::{
        CatalogBuilder, CivilDate, FlagKey, KillSwitch, SafeState, SettingsPrefix, StaleRule,
        SwitchState,
    },
    declared,
    low_level::capture_now,
    postgres::{
        Actor, Change, ChangeReason, ExplicitValue, PooledStore, Precondition, StoreTiming, Write,
    },
};
use batter_sqlx::{PgSessionProfile, pool_in};
use postgres_test_harness::{DatabaseLease, HarnessConfig, PostgresHarness};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::time::Instant;

pub const SECOND: Duration = Duration::from_secs(1);
/// So long that only a wake can explain a change within seconds.
pub const POLL: Duration = Duration::from_secs(60);

pub struct Database {
    harness: PostgresHarness,
    lease: DatabaseLease,
    pub pool: PgPool,
    pub options: PgConnectOptions,
}

impl Database {
    pub async fn migrated() -> Self {
        let url = std::env::var("BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL")
            .expect("explicit disposable PostgreSQL 18 URL required");
        let harness = PostgresHarness::start(
            HarnessConfig::new("flags_listen")
                .unwrap()
                .with_admin_database_url(url),
        )
        .await
        .unwrap();
        let lease = harness.empty_database().await.unwrap();
        let options: PgConnectOptions = lease.database_url().parse().unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        PooledStore::new(pool.clone(), timing())
            .migrate(Instant::now() + SECOND * 30)
            .await
            .unwrap();
        Self {
            harness,
            lease,
            pool,
            options,
        }
    }

    pub async fn write(&self, state: SwitchState) {
        let change = Change::new(
            Actor::new("operator").unwrap(),
            ChangeReason::new("listen test").unwrap(),
            Precondition::Unconditional,
        );
        let write = Write::explicit(
            FlagKey::new("halt").unwrap(),
            ExplicitValue::Switch(state),
            change,
        );
        let _ = PooledStore::new(self.pool.clone(), timing())
            .write(write, Instant::now() + SECOND * 10)
            .await
            .unwrap();
    }

    pub async fn close(self) {
        self.pool.close().await;
        self.lease.cleanup().await.unwrap();
        self.harness.shutdown().await.unwrap();
    }
}

pub fn timing() -> StoreTiming {
    StoreTiming::new(SECOND * 2, SECOND * 2, SECOND, SECOND * 2).unwrap()
}

/// A serving process that listens when `listen` is set.
pub async fn process(
    database: &Database,
    listen: Listening,
) -> (RegisteredFlags, RunningSupervisor, KillSwitch) {
    let (mut catalog, serving, hosted) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let halt = catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let resolved = declared::resolve(
        catalog.build(),
        &SettingsSource::from_pairs([]).unwrap(),
        serving,
        hosted,
        CivilDate::new(2026, 11, 8).unwrap(),
    )
    .unwrap();
    let budget = CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(SECOND * 5, SECOND * 5, SECOND * 5, budget).unwrap());
    let login = database.options.get_username().to_owned();
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
        database.options.clone(),
        profile,
    )
    .unwrap();
    let (poll, maximum_age) = match listen {
        Listening::Yes | Listening::No => (POLL, POLL * 2),
        Listening::PollFast => (SECOND * 2, SECOND * 30),
    };
    let policy = LoadPolicy::start_on_defaults(RefreshTiming::new(poll, maximum_age).unwrap());
    let policy = match listen {
        Listening::Yes => policy
            .with_listen(ListenEndpoint::new(database.options.clone(), SECOND).unwrap())
            .unwrap(),
        Listening::No | Listening::PollFast => policy,
    };
    let presence = Presence::new(
        ApplicationName::new("service").unwrap(),
        BuildId::new("1.0.0").unwrap(),
        HeartbeatCadence::new(SECOND * 5, SECOND * 600).unwrap(),
    );
    let flags = register_in(
        &mut supervisor,
        ControlPlaneStore::new(&pool, timing()),
        resolved,
        policy,
        presence,
    )
    .unwrap();
    let running = supervisor.start();
    running.status().wait_ready().await.unwrap();
    (flags, running, halt)
}

#[derive(Clone, Copy)]
pub enum Listening {
    Yes,
    No,
    PollFast,
}

pub async fn engaged_within(flags: &RegisteredFlags, halt: &KillSwitch, within: Duration) -> bool {
    let until = Instant::now() + within;
    while Instant::now() < until {
        if capture_now(flags).switch(halt).value() == SwitchState::Engaged {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// The backends of every listener session.
pub async fn listener_pids(database: &Database) -> Vec<i32> {
    sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname = current_database() AND application_name = 'batter-flags-listener' AND query ILIKE 'LISTEN%' ORDER BY pid")
        .fetch_all(&database.pool)
        .await
        .unwrap()
}

/// Waits until `sessions` listener sessions are listening.
pub async fn listening(database: &Database) {
    listening_count(database, 1).await;
}

pub async fn listening_count(database: &Database, sessions: i64) {
    for _ in 0..100 {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND query ILIKE 'LISTEN%'")
            .fetch_one(&database.pool)
            .await
            .unwrap();
        if count >= sessions {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no listener session appeared");
}
