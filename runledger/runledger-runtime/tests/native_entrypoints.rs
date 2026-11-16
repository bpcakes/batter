//! Isolated Unix signals and environment input through native entrypoints.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]

#[allow(dead_code, reason = "shared bounded process capture")]
#[path = "../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code, reason = "shared bounded process evidence")]
#[path = "../../../test-support/process/evidence.rs"]
mod evidence;
#[path = "../../../test-support/process/launch.rs"]
mod launch;
#[path = "../examples/support/worker.rs"]
mod worker;

use runledger_runtime::{
    RuntimeError, RuntimeSettlement, RuntimeShutdownBudget, RuntimeShutdownSignal, Supervisor,
    config::JobsConfig, registry::JobRegistry,
};
use sqlx::postgres::PgPoolOptions;
use std::{
    io::{self, Write},
    os::unix::{ffi::OsStringExt, process::ExitStatusExt},
    process::{Child, Command},
    time::{Duration, Instant},
};

fn config() -> JobsConfig {
    JobsConfig {
        worker_id: "entrypoint-test".into(),
        poll_interval: Duration::from_secs(1),
        claim_batch_size: 1,
        lease_ttl_seconds: 10,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(1),
        schedule_poll_interval: Duration::from_secs(1),
        reaper_retry_delay_ms: 1000,
    }
}

fn budget() -> RuntimeShutdownBudget {
    RuntimeShutdownBudget::new(Duration::from_secs(1), Duration::from_secs(1)).unwrap()
}

fn pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap()
}

fn event(name: &str) {
    println!("\nbatter-fixture:{name}");
    io::stdout().flush().unwrap();
}

#[test]
fn child_fixture() {
    let Some(scenario) = launch::scenario() else {
        return;
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let pool = pool();
            if scenario.starts_with("JOBS_") {
                let Err(RuntimeError::InvalidEnvironment(error)) =
                    Supervisor::builder_from_env(&pool)
                else {
                    panic!("invalid environment must fail before startup");
                };
                assert_eq!(error.setting(), scenario);
                assert!(!format!("{error:?}: {error}").contains("secret-marker"));
                pool.close().await;
                event("rejected");
                return;
            }
            let signal = match scenario.as_str() {
                "unix" | "unix-before-submit" => RuntimeShutdownSignal::unix_termination().unwrap(),
                "ctrl-c-only" => RuntimeShutdownSignal::ctrl_c(),
                _ => panic!("unknown scenario"),
            };
            if scenario == "unix-before-submit" {
                let mut proceed =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())
                        .unwrap();
                event("installed");
                proceed.recv().await.unwrap();
            }
            let supervisor = Supervisor::builder(&pool, config())
                .unwrap()
                .disable_worker()
                .disable_reaper()
                .disable_scheduler()
                .build()
                .unwrap();
            let report = worker::run(supervisor, signal, budget(), || event("initialized")).await;
            let RuntimeSettlement::Clean(clean) = report.classify() else {
                panic!("expected clean signal settlement");
            };
            let _permit = clean.into_cleanup_permit();
            event("settled");
            pool.close().await;
            assert!(pool.is_closed());
            event("pool-closed");
        });
}

struct Fixture {
    child: Child,
    capture: capture::Capture,
    _authority: io::PipeWriter,
}

impl Fixture {
    fn spawn(scenario: &str, value: Option<std::ffi::OsString>) -> Self {
        let (input, authority) = io::pipe().unwrap();
        let (reader, writer) = io::pipe().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(launch::CHILD_ARGS)
            .stdin(input)
            .stdout(writer.try_clone().unwrap())
            .stderr(writer);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("JOBS_") {
                command.env_remove(name);
            }
        }
        command.env("RUST_BACKTRACE", "0");
        if let Some(value) = value {
            command.env(scenario, value);
        }
        let capture = capture::Capture::start(reader).unwrap();
        let child = command.spawn().unwrap();
        drop(command);
        let mut fixture = Self {
            child,
            capture,
            _authority: authority,
        };
        writeln!(
            fixture._authority,
            "batter-fixture-v1 {} {scenario}",
            fixture.child.id()
        )
        .unwrap();
        fixture
    }

    fn event(&mut self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self
                .capture
                .inspect(|output, now| output.event_by(name, deadline, now))
            {
                Ok(Some(_)) => return,
                Err(reason) => panic!("{reason}"),
                Ok(None) => {}
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                let output = self.capture.finish().unwrap();
                output
                    .require_final_event(name)
                    .unwrap_or_else(|reason| panic!("{status}: {reason}"));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn signal(&mut self, signal: &str) {
        assert!(self.child.try_wait().unwrap().is_none());
        assert!(
            Command::new("/bin/kill")
                .args([signal, &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }

    fn finish(mut self, signal: Option<i32>, required: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "child exit timeout");
            std::thread::sleep(Duration::from_millis(5));
        };
        let output = self.capture.finish().unwrap();
        if let Some(signal) = signal {
            assert_eq!(status.signal(), Some(signal), "{output:?}");
        } else {
            assert!(status.success(), "{status}: {output:?}");
        }
        output.validate(required, &[]).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[test]
fn unix_signals_reach_clean_settlement_and_awaited_pool_close() {
    for signal in ["-INT", "-TERM"] {
        let mut fixture = Fixture::spawn("unix", None);
        fixture.event("initialized");
        fixture.signal(signal);
        fixture.finish(None, &["settled", "pool-closed"]);
    }
}

#[test]
fn ctrl_c_alone_does_not_handle_sigterm() {
    let mut fixture = Fixture::spawn("ctrl-c-only", None);
    fixture.event("initialized");
    fixture.signal("-TERM");
    fixture.finish(Some(15), &[]);
}

#[test]
fn unix_signals_received_before_submission_are_retained() {
    for signal in ["-INT", "-TERM"] {
        let mut fixture = Fixture::spawn("unix-before-submit", None);
        fixture.event("installed");
        fixture.signal(signal);
        fixture.signal("-USR1");
        fixture.finish(None, &["settled", "pool-closed"]);
    }
}

#[test]
fn public_environment_builder_rejects_invalid_worker_and_promoter_settings() {
    for setting in [
        "JOBS_MAX_GLOBAL_CONCURRENCY",
        "JOBS_INTENT_PROMOTER_BATCH_SIZE",
    ] {
        for value in [
            std::ffi::OsString::from("secret-marker"),
            std::ffi::OsString::from_vec(vec![0xff]),
        ] {
            let fixture = Fixture::spawn(setting, Some(value));
            fixture.finish(None, &["rejected"]);
        }
    }
}

#[tokio::test]
async fn readiness_announcement_observes_initialization_and_suppresses_stopped_startup() {
    for stop_before_initialization in [false, true] {
        let pool = pool();
        pool.close().await;
        let supervisor = Supervisor::builder(&pool, config())
            .unwrap()
            .with_registry(JobRegistry::new())
            .build()
            .unwrap();
        let shutdown = supervisor.shutdown_handle();
        let startup = supervisor.startup_observer();
        if stop_before_initialization {
            shutdown.request_shutdown();
        }
        let mut announced = false;
        let report = worker::run(
            supervisor,
            RuntimeShutdownSignal::pending(),
            budget(),
            || {
                assert_eq!(
                    startup.snapshot(),
                    runledger_runtime::RuntimeStartup::Initialized
                );
                announced = true;
                shutdown.request_shutdown();
            },
        )
        .await;
        assert_eq!(announced, !stop_before_initialization);
        assert!(matches!(report.classify(), RuntimeSettlement::Clean(_)));
    }
}
