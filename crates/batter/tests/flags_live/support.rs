//! Databases, a serving process and its helpers for the flags live tests.

use std::time::Duration;

use batter::{
    cleanup::CleanupBudget,
    flags::{
        control_plane::{
            ApplicationName, BuildId, ControlPlaneStore, HeartbeatCadence, LoadPolicy, Presence,
            RefreshTiming, RegisteredFlags, SnapshotDirectory, SnapshotFilePath, register_in,
        },
        core::{
            CatalogBuilder, CivilDate, FlagKey, SafeState, SettingsPrefix, StaleRule, SwitchState,
        },
        declared,
        postgres::{
            Actor, Change, ChangeReason, ExplicitValue, PooledStore, Precondition, StoreTiming,
            Write, WriteOutcome,
        },
    },
    lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use sqlx::{
    Connection, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::time::Instant;

pub const SECOND: Duration = Duration::from_secs(1);

/// A database on the admin cluster, by name.
#[derive(Clone)]
pub struct Database {
    admin: String,
    pub name: String,
    pub options: PgConnectOptions,
}

impl Database {
    pub async fn create(label: &str) -> Self {
        let admin = std::env::var("BATTER_SQLX_ADMIN_URL").expect(
            "BATTER_SQLX_ADMIN_URL, a superuser connection to a disposable PostgreSQL 18 cluster",
        );
        let name = format!("batter_flags_{label}_{}", uuid::Uuid::new_v4().simple());
        let mut connection = PgConnection::connect(&admin).await.unwrap();
        let version: String = sqlx::query_scalar("SHOW server_version_num")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            version.parse::<u32>().unwrap() / 10_000,
            18,
            "PostgreSQL 18 required"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut connection)
            .await
            .unwrap();
        let options = admin.parse::<PgConnectOptions>().unwrap().database(&name);
        Self {
            admin,
            name,
            options,
        }
    }

    pub async fn admin(&self) -> PgConnection {
        PgConnection::connect(&self.admin).await.unwrap()
    }

    pub async fn pool(&self) -> PgPool {
        PgPoolOptions::new()
            .max_connections(4)
            .connect_with(self.options.clone())
            .await
            .unwrap()
    }

    /// Ends every session on `name` and then runs `statement`, retrying
    /// while a reconnecting session gets in first.
    pub async fn exclusive(&self, name: &str, statement: &str) {
        let mut connection = self.admin().await;
        for _ in 0..50 {
            sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()")
                .bind(name)
                .execute(&mut connection)
                .await
                .unwrap();
            if sqlx::raw_sql(sqlx::AssertSqlSafe(statement.to_owned()))
                .execute(&mut connection)
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the database stayed in use");
    }

    /// Drops every named database, whatever its state.
    pub async fn drop_all(&self, names: &[String]) {
        let mut connection = self.admin().await;
        for name in names {
            let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
            )))
            .execute(&mut connection)
            .await;
        }
    }
}

/// Runs `body`, then always runs `cleanup`, and then fails as `body` did.
pub async fn with_cleanup<B, C>(body: B, cleanup: C)
where
    B: Future<Output = ()> + Send + 'static,
    C: Future<Output = ()>,
{
    let outcome = tokio::spawn(body).await;
    cleanup.await;
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure.into_panic());
    }
}

pub fn timing() -> StoreTiming {
    StoreTiming::new(SECOND * 3, SECOND * 3, SECOND * 2, SECOND * 3).unwrap()
}

pub fn deadline() -> Instant {
    Instant::now() + SECOND * 20
}

pub fn change() -> Change {
    Change::new(
        Actor::new("operator").unwrap(),
        ChangeReason::new("live test").unwrap(),
        Precondition::Unconditional,
    )
}

pub fn halt() -> FlagKey {
    FlagKey::new("halt").unwrap()
}

/// Advances the store's head to `version` by alternating explicit writes,
/// which need no published catalog.
pub async fn write_until(store: &PooledStore, version: u64) {
    loop {
        let head = store.load(deadline()).await.unwrap().version().get();
        if head >= version {
            assert_eq!(head, version);
            return;
        }
        let state = if head % 2 == 0 {
            SwitchState::Engaged
        } else {
            SwitchState::Released
        };
        let outcome = store
            .write(
                Write::explicit(halt(), ExplicitValue::Switch(state), change()),
                deadline(),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                outcome,
                WriteOutcome::Written { .. } | WriteOutcome::WrittenUndeclared { .. }
            ),
            "{outcome:?}"
        );
    }
}

/// Registers and starts a serving process over the database `options` name,
/// with its last-good file in `directory`.
pub async fn start(
    options: &PgConnectOptions,
    directory: &std::path::Path,
) -> (RegisteredFlags, RunningSupervisor) {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    let hosted = catalog.environment_class("hosted").unwrap();
    catalog
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    let today = CivilDate::new(2026, 11, 8).unwrap();
    let resolved = declared::resolve(
        catalog.build().unwrap(),
        &SettingsSource::from_pairs([]).unwrap(),
        serving,
        hosted,
        today,
    )
    .unwrap();
    let budget = CleanupBudget::new(SECOND * 5, SECOND * 5, SECOND * 5).unwrap();
    let mut supervisor =
        Supervisor::new(ShutdownBudget::new(SECOND * 5, SECOND * 5, SECOND * 5, budget).unwrap());
    let login = options.get_username().to_owned();
    let profile = batter::sqlx::PgSessionProfile::new(
        &login,
        &login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let pool = batter::sqlx::pool_in(
        supervisor.reserve_cleanup("database").unwrap(),
        PgPoolOptions::new().max_connections(4),
        options.clone(),
        profile,
    )
    .unwrap();
    let last_good =
        SnapshotDirectory::open(SnapshotFilePath::new(directory, "flags.bfsn").unwrap()).unwrap();
    let policy =
        LoadPolicy::start_on_defaults(RefreshTiming::new(SECOND * 2, SECOND * 30).unwrap())
            .with_last_good(last_good);
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
    (flags, running)
}

/// Waits, for at most twenty seconds, for `condition`.
pub async fn until(condition: impl Fn() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition never held");
}

/// What the process's heartbeat row in `pool` reports, waiting for `expected`.
pub async fn heartbeat_reports(pool: &PgPool, process: uuid::Uuid, expected: (&str, i64)) {
    for _ in 0..200 {
        let row: Option<(String, Option<i64>)> = sqlx::query_as("SELECT snapshot_source, snapshot_version FROM flags.flag_processes WHERE process_id = $1")
            .bind(process)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
        if row
            .as_ref()
            .is_some_and(|(source, version)| source == expected.0 && *version == Some(expected.1))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the heartbeat never reported {expected:?}");
}
