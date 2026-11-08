//! A disposable database seeded through real heartbeats, and the binary.
#![allow(
    dead_code,
    reason = "shared by the live test targets; each uses part of it"
)]

use std::{
    io::Read,
    process::{Child, Command, Stdio},
    time::Duration,
};

use batter_flags_core::{
    CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle, Version,
    low_level::BaseBuilder,
};
use batter_flags_postgres::{
    Cadence, KeySample, PooledStore, PresenceReport, ProcessLabel, ProcessLabels, ProcessPresence,
    SnapshotReport, StoreTiming,
};
use postgres_test_harness::{DatabaseLease, HarnessConfig, PostgresHarness};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::time::Instant;

/// One run of the binary.
#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).expect("one JSON document")
    }
}

pub fn run_cli(env: &[(&str, &str)], args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_batter-flags"))
        .env_clear()
        .envs(env.iter().copied())
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn signal(child: &Child, name: &str) {
    let status = Command::new("kill")
        .args([name, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

pub fn finish(mut child: Child) -> Run {
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    Run {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    }
}

pub struct Database {
    pub pool: PgPool,
    lease: DatabaseLease,
    harness: PostgresHarness,
    url: String,
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn process(catalog_keys: &[&str], build: &str) -> ProcessPresence {
    process_with(catalog_keys, build, 3_600)
}

fn process_with(catalog_keys: &[&str], build: &str, liveness: u64) -> ProcessPresence {
    let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = catalog.process_kind("serving").unwrap();
    catalog.environment_class("hosted").unwrap();
    let later = Expiry::on(CivilDate::new(2027, 3, 31).unwrap());
    let past = Expiry::on(CivilDate::new(2020, 1, 1).unwrap());
    for key in catalog_keys {
        match *key {
            "halt" => {
                catalog
                    .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
                    .declare()
                    .unwrap();
            }
            "rollout" => {
                catalog
                    .release_flag("rollout", Toggle::Off, later)
                    .coordinated()
                    .declare()
                    .unwrap();
            }
            "old_path" => {
                catalog
                    .release_flag("old_path", Toggle::Off, past)
                    .declare()
                    .unwrap();
            }
            other => {
                catalog
                    .release_flag(other, Toggle::Off, later)
                    .declare()
                    .unwrap();
            }
        }
    }
    let base = BaseBuilder::new(catalog.build().unwrap(), serving)
        .unwrap()
        .finish();
    let labels = ProcessLabels {
        application: ProcessLabel::new("service").unwrap(),
        environment: ProcessLabel::new("hosted").unwrap(),
        build: ProcessLabel::new(build).unwrap(),
    };
    let cadence = Cadence::new(Duration::from_secs(liveness), Duration::from_secs(3_600)).unwrap();
    ProcessPresence::new(labels, &base, cadence)
}

fn report(floor: &[&str]) -> PresenceReport {
    PresenceReport {
        snapshot: SnapshotReport::ControlPlane(Version::new(0)),
        held: KeySample::default(),
        floor_engaged: KeySample::of(
            floor
                .iter()
                .map(|key| batter_flags_core::FlagKey::new(key).unwrap()),
        ),
    }
}

impl Database {
    /// Migrated, with three processes of one build (two Live, one Stopped),
    /// one Live process's settings floor engaging `halt`, and an orphaned
    /// catalog declaring `orphan_path`.
    pub async fn seeded() -> Self {
        let admin = std::env::var("BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL")
            .expect("explicit disposable PostgreSQL 18 URL required");
        let harness = PostgresHarness::start(
            HarnessConfig::new("batter_flags_cli")
                .unwrap()
                .with_admin_database_url(admin),
        )
        .await
        .unwrap();
        let lease = harness.empty_database().await.unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(lease.database_url())
            .await
            .unwrap();
        let second = Duration::from_secs(1);
        let store = PooledStore::new(
            pool.clone(),
            StoreTiming::new(second * 3, second * 3, second * 3, second * 3).unwrap(),
        );
        store
            .migrate(Instant::now() + Duration::from_secs(30))
            .await
            .unwrap();
        let keys = ["fast_path", "halt", "rollout", "old_path"];
        let mut first = process(&keys, "build-a");
        let mut second_live = process(&keys, "build-a");
        let mut stopped = process(&keys, "build-a");
        store
            .heartbeat(&mut first, &report(&["halt"]), deadline())
            .await
            .unwrap();
        store
            .heartbeat(&mut second_live, &report(&[]), deadline())
            .await
            .unwrap();
        store
            .heartbeat(&mut stopped, &report(&[]), deadline())
            .await
            .unwrap();
        store.stop(stopped, deadline()).await.unwrap();
        let mut orphan = process(&["orphan_path"], "build-b");
        store
            .heartbeat(&mut orphan, &report(&[]), deadline())
            .await
            .unwrap();
        sqlx::query("DELETE FROM flags.flag_processes WHERE build = 'build-b'")
            .execute(&pool)
            .await
            .unwrap();
        // The leased URL as given, credentials and escaping included, with
        // only its transport parameter set for the loopback server.
        let base = lease.database_url().split('?').next().unwrap();
        let url = format!("{base}?sslmode=disable");
        Self {
            pool,
            lease,
            harness,
            url,
        }
    }

    /// The URL with `password` in place of any it had.
    pub fn url_with_password(&self, password: &str) -> String {
        let (scheme, rest) = self.url.split_once("://").unwrap();
        let (userinfo, host) = rest.split_once('@').unwrap();
        let user = userinfo.split(':').next().unwrap();
        format!("{scheme}://{user}:{password}@{host}")
    }

    pub fn cli(&self, args: &[String]) -> Run {
        let mut full = vec!["--output".to_owned(), "json".to_owned()];
        full.extend(args.iter().cloned());
        run_cli(
            &[("BATTER_FLAGS_DATABASE_URL", &self.url)],
            &full.iter().map(String::as_str).collect::<Vec<_>>(),
        )
    }

    pub fn cli_args(&self, args: &[&str]) -> Run {
        self.cli(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    pub fn spawn(&self, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_batter-flags"))
            .env_clear()
            .env("BATTER_FLAGS_DATABASE_URL", &self.url)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    pub async fn head(&self) -> i64 {
        sqlx::query_scalar("SELECT version FROM flags.flag_control")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn audits(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM flags.flag_audit")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn wait_for(&self, sql: &'static str) {
        for _ in 0..200 {
            let found: bool = sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap();
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the awaited session state never appeared");
    }

    pub async fn wait_for_lock_wait(&self) {
        self.wait_for("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND datname = current_database())").await;
    }

    pub async fn wait_for_sleep(&self) {
        self.wait_for("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE wait_event = 'PgSleep' AND datname = current_database())").await;
    }

    /// A test-only deferred trigger that holds every COMMIT that audits.
    pub async fn slow_commit(&self, seconds: u32) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION flags.slow_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep({seconds}); RETURN NULL; END $$; \
             CREATE CONSTRAINT TRIGGER slow_commit AFTER INSERT ON flags.flag_audit DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.slow_commit();"
        )))
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// A test-only deferred trigger that rejects every COMMIT that audits.
    pub async fn failing_commit(&self) {
        sqlx::raw_sql("CREATE FUNCTION flags.refuse_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'refused at commit'; END $$; \
             CREATE CONSTRAINT TRIGGER refuse_commit AFTER INSERT ON flags.flag_audit DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.refuse_commit();")
            .execute(&self.pool)
            .await
            .unwrap();
    }

    /// A test-only deferred trigger that ends the writing session while its
    /// COMMIT runs, so the command never learns the outcome.
    pub async fn lost_commit(&self) {
        sqlx::raw_sql("CREATE FUNCTION flags.lose_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_terminate_backend(pg_backend_pid()); RETURN NULL; END $$; \
             CREATE CONSTRAINT TRIGGER lose_commit AFTER INSERT ON flags.flag_audit DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION flags.lose_commit();")
            .execute(&self.pool)
            .await
            .unwrap();
    }

    /// Adds a Silent process of `build` declaring `fast_path`.
    pub async fn silent(&self, build: &str) {
        let mut silent = process_with(&["fast_path"], build, 1);
        let second = Duration::from_secs(1);
        let store = PooledStore::new(
            self.pool.clone(),
            StoreTiming::new(second * 3, second * 3, second * 3, second * 3).unwrap(),
        );
        store
            .heartbeat(&mut silent, &report(&[]), deadline())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1_500)).await;
    }

    pub async fn close(self) {
        self.pool.close().await;
        self.lease.cleanup().await.unwrap();
        self.harness.shutdown().await.unwrap();
    }
}
