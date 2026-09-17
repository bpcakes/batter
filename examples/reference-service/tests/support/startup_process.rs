// Suite-specific signal controls on the existing private Unix process owner.
#[allow(dead_code)]
#[path = "../../../../test-support/process/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../../../../test-support/process/evidence.rs"]
mod evidence;
#[allow(dead_code)]
#[path = "../../../../test-support/process/launch.rs"]
pub mod launch;
#[allow(dead_code)]
#[path = "../../../../test-support/process/timing.rs"]
mod timing;

/// Static proof printed only after a child's typed assertions pass.
const CLEANUP_PROOF: &str = "startup-child:owned-cleanup-complete";
pub const AUTH_TOKEN: &str = "fixture-token";
pub const SIGNAL_LISTENERS_READY: &str = "signal-listeners-ready";
pub const SIGNAL_OBSERVED: &str = "startup-signal-observed";

pub fn provider_forbidden_values<'a>(endpoint: &'a str, provider_url: &'a str) -> [&'a str; 4] {
    [endpoint, provider_url, AUTH_TOKEN, super::provider::TOKEN]
}

#[allow(dead_code)]
mod watchdog {
    include!("../../../../test-support/process/watchdog.rs");
    use std::{net::SocketAddr, process::Stdio};

    #[path = "process_contracts.rs"]
    mod contracts;
    pub use contracts::process_contracts;
    #[path = "announcement_failure.rs"]
    pub(super) mod announcement_failure;
    #[path = "listener.rs"]
    mod listener;

    const REAP_LIMIT: Duration = Duration::from_secs(8);
    const BIND_LIMIT: Duration = Duration::from_secs(20);

    /// A Unix signal selected by one process case.
    #[derive(Clone, Copy, Debug)]
    pub enum Signal {
        Term,
        Int,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SignalRequest {
        Accepted,
        AlreadyExited,
    }

    impl Signal {
        fn flag(self) -> &'static str {
            match self {
                Self::Term => "-TERM",
                Self::Int => "-INT",
            }
        }

        fn executable_event(self) -> &'static str {
            match self {
                Self::Term => "executable-sigterm-observed",
                Self::Int => "executable-sigint-observed",
            }
        }
    }

    fn configure(command: &mut Command, endpoint: &str, bind: &str) {
        command
            .env_clear()
            .env("DATABASE_URL", endpoint)
            .env("BATTER_BIND", bind)
            .env("BATTER_POOL_MAX_CONNECTIONS", "1")
            .env("BATTER_POOL_ACQUIRE_TIMEOUT_MS", "20000")
            .env("JOBS_WORKER_ID", "startup-signal")
            .env(
                "BATTER_AUTH_OWNER_ID",
                "00000000-0000-0000-0000-000000000001",
            )
            .env("BATTER_AUTH_TOKEN", super::AUTH_TOKEN)
            .env("BATTER_PROVIDER_BASE_URL", "http://127.0.0.1:9/")
            .env("BATTER_PROVIDER_TOKEN", "fixture-provider-token");
    }

    fn configure_provider_worker(
        command: &mut Command,
        endpoint: &str,
        bind: &str,
        provider_base_url: &str,
        worker_id: &str,
        maximum_concurrency: &str,
        retry_delay_ms: &str,
    ) {
        configure(command, endpoint, bind);
        command
            .env("BATTER_PROVIDER_BASE_URL", provider_base_url)
            .env("JOBS_WORKER_ID", worker_id)
            .env("JOBS_POLL_INTERVAL_MS", "20")
            .env("JOBS_CLAIM_BATCH_SIZE", "1")
            .env("JOBS_LEASE_TTL_SECONDS", "1")
            .env("JOBS_MAX_GLOBAL_CONCURRENCY", maximum_concurrency)
            .env("JOBS_REAPER_INTERVAL_SECONDS", "1")
            .env("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS", "1")
            .env("JOBS_REAPER_RETRY_DELAY_MS", retry_delay_ms)
            .env("BATTER_PROVIDER_CAPACITY", "1");
    }

    /// A launched process with separately bounded stdout and stderr capture.
    struct SeparateChild {
        child: Child,
        stdout: Capture,
        stderr: Capture,
        parent_pipe: Option<io::PipeWriter>,
        label: String,
    }

    struct SeparateRun {
        label: String,
        status: ExitStatus,
        killed: bool,
        stdout: Output,
        stderr: Output,
    }

    struct SafeOutput<'a>(&'a Output);

    impl std::fmt::Debug for SafeOutput<'_> {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("Output")
                .field("retained_chars", &self.0.text().len())
                .finish_non_exhaustive()
        }
    }

    impl std::fmt::Debug for SeparateRun {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("SeparateRun")
                .field("label", &self.label)
                .field("status", &self.status)
                .field("killed", &self.killed)
                .field("stdout", &SafeOutput(&self.stdout))
                .field("stderr", &SafeOutput(&self.stderr))
                .finish()
        }
    }

    fn safe_check(output: &Output, label: &str) -> Result<(), String> {
        output
            .check()
            .map_err(|_| format!("{label} failed: {:?}", SafeOutput(output)))
    }

    fn safe_validate(
        output: &Output,
        required: &[&str],
        forbidden: &[&str],
        label: &str,
    ) -> Result<(), String> {
        output
            .validate(required, forbidden)
            .map_err(|_| format!("{label} failed validation: {:?}", SafeOutput(output)))
    }

    fn start_captures(
        command: Command,
        stdout_reader: io::PipeReader,
        stderr_reader: io::PipeReader,
        mut start: impl FnMut(io::PipeReader) -> io::Result<Capture>,
    ) -> io::Result<(Command, Capture, Capture)> {
        let stdout = start(stdout_reader)?;
        let stderr = match start(stderr_reader) {
            Ok(stderr) => stderr,
            Err(error) => {
                // Command still owns both writers. Close them before Capture::drop
                // joins the first reader, or a second-reader setup failure deadlocks.
                drop(command);
                drop(stdout);
                return Err(error);
            }
        };
        Ok((command, stdout, stderr))
    }

    impl SeparateChild {
        fn spawn(
            label: &str,
            mut command: Command,
            parent_pipe: Option<io::PipeWriter>,
        ) -> io::Result<Self> {
            let (stdout_reader, stdout_writer) = io::pipe()?;
            let (stderr_reader, stderr_writer) = io::pipe()?;
            command
                .env("RUST_BACKTRACE", "0")
                .stdout(stdout_writer)
                .stderr(stderr_writer);
            let (mut command, stdout, stderr) =
                start_captures(command, stdout_reader, stderr_reader, Capture::start)?;
            let spawned = command.spawn();
            // Command retains both writers; drop it so capture can observe EOF.
            drop(command);
            Ok(Self {
                child: spawned?,
                stdout,
                stderr,
                parent_pipe,
                label: label.to_owned(),
            })
        }

        fn signal(&mut self, signal: Signal) -> Result<SignalRequest, String> {
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(SignalRequest::AlreadyExited);
            }
            let sent = Command::new("/bin/kill")
                .args([signal.flag(), &self.child.id().to_string()])
                .status()
                .map_err(|error| error.to_string())?;
            if sent.success() {
                Ok(SignalRequest::Accepted)
            } else if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                // The child exited between the liveness check and /bin/kill.
                // Its cached status and captured streams remain available to wait().
                Ok(SignalRequest::AlreadyExited)
            } else {
                Err(format!(
                    "could not send {signal:?} to {}; {}",
                    self.label,
                    self.diagnostics()
                ))
            }
        }

        fn signal_kill(&mut self) -> Result<SignalRequest, String> {
            if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(SignalRequest::AlreadyExited);
            }
            let sent = Command::new("/bin/kill")
                .args(["-KILL", &self.child.id().to_string()])
                .status()
                .map_err(|error| error.to_string())?;
            if sent.success() {
                Ok(SignalRequest::Accepted)
            } else if self
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                Ok(SignalRequest::AlreadyExited)
            } else {
                Err(format!(
                    "could not send SIGKILL to {}; {}",
                    self.label,
                    self.diagnostics()
                ))
            }
        }

        fn diagnostics(&self) -> String {
            format!(
                "stdout={:?}; stderr={:?}",
                SafeOutput(&self.stdout.snapshot()),
                SafeOutput(&self.stderr.snapshot())
            )
        }

        fn wait_event(&mut self, event: &str, limit: Duration) -> Result<(), String> {
            let deadline = Instant::now() + limit;
            loop {
                self.stderr
                    .inspect(|output, _| safe_check(output, "child stderr"))?;
                if self
                    .stdout
                    .inspect(|output, now| {
                        output.event_by(event, deadline, now).map_err(|_| {
                            format!(
                                "child stdout failed before {event}: {:?}",
                                SafeOutput(output)
                            )
                        })
                    })?
                    .is_some()
                {
                    return Ok(());
                }
                if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    let stdout = self.stdout.finish().map_err(|e| e.to_string())?;
                    let stderr = self.stderr.finish().map_err(|e| e.to_string())?;
                    safe_validate(&stdout, &[event], &[], "child stdout")?;
                    safe_validate(&stderr, &[], &[], "child stderr")?;
                    return Ok(());
                }
                std::thread::sleep(POLL);
            }
        }

        fn wait(mut self, limit: Duration) -> Result<SeparateRun, String> {
            let deadline = Instant::now() + limit;
            let (status, killed) = loop {
                if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                    break (status, false);
                }
                if Instant::now() >= deadline {
                    let _ = self.child.kill();
                    break (self.child.wait().map_err(|e| e.to_string())?, true);
                }
                std::thread::sleep(POLL);
            };
            Ok(SeparateRun {
                label: self.label.clone(),
                status,
                killed,
                stdout: self.stdout.finish().map_err(|e| e.to_string())?,
                stderr: self.stderr.finish().map_err(|e| e.to_string())?,
            })
        }

        fn terminate(mut self) -> Result<SeparateRun, String> {
            let status = match self.child.try_wait().map_err(|error| error.to_string())? {
                Some(status) => status,
                None => {
                    self.child.kill().map_err(|error| error.to_string())?;
                    self.child.wait().map_err(|error| error.to_string())?
                }
            };
            Ok(SeparateRun {
                label: self.label.clone(),
                status,
                killed: true,
                stdout: self.stdout.finish().map_err(|error| error.to_string())?,
                stderr: self.stderr.finish().map_err(|error| error.to_string())?,
            })
        }

        fn abort_diagnostics(self, forbidden: &[&str]) -> String {
            match self.terminate() {
                Ok(run) => match run.check_streams(forbidden) {
                    Ok(()) => format!("child terminated after setup failure: {run:?}"),
                    Err(error) => error,
                },
                Err(_) => "child termination failed after setup failure".to_owned(),
            }
        }
    }

    impl Drop for SeparateChild {
        fn drop(&mut self) {
            // Reap before the capture fields join their readers.
            if !matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    impl SeparateRun {
        fn check_streams(&self, forbidden: &[&str]) -> Result<(), String> {
            safe_validate(&self.stdout, &[], &[], "child stdout")?;
            safe_validate(&self.stderr, &[], &[], "child stderr")?;
            let streams = self.stdout.text() + &self.stderr.text();
            if forbidden.iter().any(|value| streams.contains(value)) {
                return Err(format!("{} disclosed private configuration", self.label));
            }
            Ok(())
        }

        fn check(&self, forbidden: &[&str]) -> Result<(), String> {
            self.check_streams(forbidden)?;
            if self.killed {
                return Err(format!("{} exceeded its reap limit: {self:?}", self.label));
            }
            Ok(())
        }

        /// Assertion child: ordinary success, empty application stderr and the
        /// static proof. Rust test-harness banners share stdout with events.
        fn assertion_child(&self, events: &[&str], forbidden: &[&str]) -> Result<(), String> {
            self.check(forbidden)?;
            safe_validate(&self.stdout, events, &[], "assertion child stdout")?;
            if !self.status.success() || !self.stderr.text().is_empty() {
                return Err(format!("{} assertion child failed: {self:?}", self.label));
            }
            if !self
                .stdout
                .text()
                .lines()
                .any(|line| line == super::CLEANUP_PROOF)
            {
                return Err(format!("missing startup cleanup proof: {self:?}"));
            }
            Ok(())
        }

        fn signal_fixture_child(&self, signal: Signal, forbidden: &[&str]) -> Result<(), String> {
            self.check(forbidden)?;
            let expected = format!("batter-fixture:{}\n", signal.executable_event());
            if self.status.signal().is_some() || self.status.code() != Some(1) {
                return Err(format!(
                    "signal fixture did not exit with status 1: {self:?}"
                ));
            }
            if self.stdout.text() != expected
                || self.stderr.text() != "Error: reference service failed\n"
            {
                return Err(format!("unexpected signal fixture diagnostics: {self:?}"));
            }
            Ok(())
        }

        fn production_child(&self, forbidden: &[&str]) -> Result<(), String> {
            self.check(forbidden)?;
            if self.status.signal().is_some() || self.status.code() != Some(1) {
                return Err(format!(
                    "production executable did not exit with status 1: {self:?}"
                ));
            }
            if !self.stdout.text().is_empty()
                || self.stderr.text() != "Error: reference service failed\n"
            {
                return Err(format!(
                    "unexpected production executable diagnostics: {self:?}"
                ));
            }
            Ok(())
        }
    }

    /// A `child_fixture` process running a typed startup assertion scenario.
    pub struct StartupChild(SeparateChild);
    impl StartupChild {
        pub fn start(stage: &str, endpoint: &str) -> io::Result<Self> {
            Self::start_at(stage, endpoint, "127.0.0.1:0")
        }

        pub fn start_at(stage: &str, endpoint: &str, bind: &str) -> io::Result<Self> {
            let (input, writer) = io::pipe()?;
            let mut command = Command::new(std::env::current_exe()?);
            configure(&mut command, endpoint, bind);
            command.args(launch::CHILD_ARGS).stdin(input);
            let mut child = SeparateChild::spawn(stage, command, Some(writer))?;
            writeln!(
                child.parent_pipe.as_mut().unwrap(),
                "batter-fixture-v1 {} {stage}",
                child.child.id()
            )?;
            Ok(Self(child))
        }

        /// Stop a child after parent-side setup failed and retain bounded,
        /// secret-free stream evidence for the returned test failure.
        pub fn abort_diagnostics(self, forbidden: &[&str]) -> String {
            self.0.abort_diagnostics(forbidden)
        }

        /// Wait for a `batter-fixture:` event before acting on the child.
        pub fn wait_event(&mut self, event: &str, limit: Duration) -> Result<(), String> {
            self.0.wait_event(event, limit)
        }

        /// Request one signal after acknowledgement if the child is still running,
        /// then require typed success and retain its captured diagnostics.
        pub fn stop(
            self,
            signal: Signal,
            events: &[&str],
            forbidden: &[&str],
        ) -> Result<(), String> {
            self.request_stop(signal, forbidden)?
                .finish(events, forbidden)
        }

        /// Require the OS to accept one signal request for a child that was not
        /// already observed exited, retaining ownership for a separate wait.
        pub fn request_stop(mut self, signal: Signal, forbidden: &[&str]) -> Result<Self, String> {
            if self.0.signal(signal)? != SignalRequest::Accepted {
                let run = self.0.wait(REAP_LIMIT)?;
                run.check(forbidden)?;
                return Err(format!(
                    "{} exited before the {signal:?} request: {run:?}",
                    run.label
                ));
            }
            Ok(self)
        }

        /// Require typed success without sending any signal.
        pub fn finish(self, events: &[&str], forbidden: &[&str]) -> Result<(), String> {
            self.0.wait(REAP_LIMIT)?.assertion_child(events, forbidden)
        }
    }

    #[derive(Clone, Copy)]
    enum ExecutableKind {
        SignalFixture,
        ProductionStartupFailure,
        ProductionRunningSuccess,
    }

    /// A separately built executable, never a Rust test-harness child.
    pub struct ExecutableChild {
        child: SeparateChild,
        kind: ExecutableKind,
        listener: Option<SocketAddr>,
    }

    impl ExecutableChild {
        pub fn lifecycle_observation_budget() -> Duration {
            BIND_LIMIT + REAP_LIMIT
        }

        pub fn start_signal_fixture(endpoint: &str) -> io::Result<Self> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_signal_witness_fixture"));
            configure(&mut command, endpoint, "127.0.0.1:0");
            command.stdin(Stdio::null());
            SeparateChild::spawn("signal witness fixture", command, None).map(|child| Self {
                child,
                kind: ExecutableKind::SignalFixture,
                listener: None,
            })
        }

        pub fn start_production(endpoint: &str) -> io::Result<Self> {
            // Cargo builds both package binaries for the same test invocation and
            // toolchain; no stale executable is discovered from the environment.
            let mut command = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"));
            configure(&mut command, endpoint, "127.0.0.1:0");
            command.stdin(Stdio::null());
            SeparateChild::spawn("production reference executable", command, None).map(|child| {
                Self {
                    child,
                    kind: ExecutableKind::ProductionStartupFailure,
                    listener: None,
                }
            })
        }

        /// Start the real production executable with fast, bounded native worker
        /// timings for the isolated provider-effect acceptance probe.
        pub fn start_provider_worker(
            endpoint: &str,
            provider_base_url: &str,
            worker_id: &str,
        ) -> io::Result<Self> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"));
            configure_provider_worker(
                &mut command,
                endpoint,
                "127.0.0.1:0",
                provider_base_url,
                worker_id,
                "1",
                "1",
            );
            command.stdin(Stdio::null());
            let announcement = listener::Announcement::new(&mut command)?;
            let mut child = SeparateChild::spawn("production provider worker", command, None)?;
            let listener = child
                .wait_for_listener(&announcement, BIND_LIMIT)
                .map_err(io::Error::other)?;
            Ok(Self {
                child,
                kind: ExecutableKind::ProductionRunningSuccess,
                listener: Some(listener),
            })
        }

        /// Start the production worker with three native execution slots and one
        /// provider permit so the live probe can observe bounded waiting while
        /// an accepted request remains behind its response barrier.
        pub fn start_admission_worker(
            endpoint: &str,
            provider_base_url: &str,
            worker_id: &str,
        ) -> io::Result<Self> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_batter-example-reference-service"));
            configure_provider_worker(
                &mut command,
                endpoint,
                "127.0.0.1:0",
                provider_base_url,
                worker_id,
                "3",
                "60000",
            );
            command.stdin(Stdio::null());
            let announcement = listener::Announcement::new(&mut command)?;
            let mut child = SeparateChild::spawn("production admission worker", command, None)?;
            let listener = child
                .wait_for_listener(&announcement, BIND_LIMIT)
                .map_err(io::Error::other)?;
            Ok(Self {
                child,
                kind: ExecutableKind::ProductionRunningSuccess,
                listener: Some(listener),
            })
        }

        /// Return the loopback address selected and acknowledged by a running child.
        pub fn listener(&self) -> SocketAddr {
            self.listener
                .expect("only a running production child exposes its listener")
        }

        /// Stop and reap after parent-side setup failed, retaining redacted
        /// bounded output evidence.
        pub fn abort_diagnostics(self, forbidden: &[&str]) -> String {
            self.child.abort_diagnostics(forbidden)
        }

        /// Require the kind-specific startup failure or clean running shutdown.
        pub fn stop(mut self, signal: Signal, forbidden: &[&str]) -> Result<(), String> {
            if self.child.signal(signal)? != SignalRequest::Accepted {
                let run = self.child.wait(REAP_LIMIT)?;
                run.check(forbidden)?;
                return Err(format!(
                    "{} exited before the {signal:?} request: {run:?}",
                    run.label
                ));
            }
            let run = self.child.wait(REAP_LIMIT)?;
            match self.kind {
                ExecutableKind::SignalFixture => run.signal_fixture_child(signal, forbidden),
                ExecutableKind::ProductionStartupFailure => run.production_child(forbidden),
                ExecutableKind::ProductionRunningSuccess => run.production_running_child(forbidden),
            }
        }

        /// Deliver SIGKILL, reap the process, and require signal termination with
        /// bounded, empty, secret-free streams. This is an expected crash witness,
        /// not the timeout fallback used by the ordinary child owner.
        pub fn crash(mut self, forbidden: &[&str]) -> Result<(), String> {
            if self.child.signal_kill()? != SignalRequest::Accepted {
                let run = self.child.wait(REAP_LIMIT)?;
                run.check(forbidden)?;
                return Err(format!("{} exited before SIGKILL: {run:?}", run.label));
            }
            self.child.wait(REAP_LIMIT)?.production_crash(forbidden)
        }
    }
}
pub use watchdog::announcement_failure::{
    observation_budget as announcement_failure_budget, probe as check_announcement_failure,
};
pub use watchdog::{ExecutableChild, Signal, StartupChild, process_contracts};

/// Emit a parseable fixture event on its own line.
pub fn event(name: &str) {
    println!("batter-fixture:{name}");
}

/// Child endpoint for the explicit live fixture, without SQLx tuning parameters
/// that the production root deliberately rejects.
pub fn endpoint(pool: &sqlx::PgPool) -> String {
    use sqlx::ConnectOptions;
    let mut endpoint = pool.connect_options().to_url_lossy();
    endpoint.set_query(Some("sslmode=disable"));
    endpoint.to_string()
}

pub fn child(scenario: &str) {
    // End any harness banner so fixture events start at a line boundary.
    println!();
    let mut runtime = if scenario == "postgres.schema" {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(2);
        builder
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    runtime.enable_all().build().unwrap().block_on(async {
        match scenario {
            "production" => production().await,
            "announcement.failure" => watchdog::announcement_failure::child().await,
            "postgres.acquire" | "postgres.schema" => startup_drain(scenario).await,
            "signal.broadcast.term" | "signal.broadcast.int" => signal_broadcast(scenario).await,
            "protected.waiter-loss" => super::protected_startup::waiter_loss_child().await,
            "protected.owner-loss" => super::protected_startup::owner_loss_child().await,
            _ => panic!("unknown startup child scenario"),
        }
    });
    println!("{CLEANUP_PROOF}");
}

async fn signal_broadcast(scenario: &str) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut observed = signal(match scenario {
        "signal.broadcast.term" => SignalKind::terminate(),
        "signal.broadcast.int" => SignalKind::interrupt(),
        _ => unreachable!("signal broadcast scenario was matched by the caller"),
    })
    .expect("first signal listener installs");
    let mut peer = signal(match scenario {
        "signal.broadcast.term" => SignalKind::terminate(),
        "signal.broadcast.int" => SignalKind::interrupt(),
        _ => unreachable!("signal broadcast scenario was matched by the caller"),
    })
    .expect("second signal listener installs");
    event(SIGNAL_LISTENERS_READY);
    let observed = async {
        observed.recv().await.expect("signal stream remains open");
        event(SIGNAL_OBSERVED);
    };
    let peer = async {
        peer.recv().await.expect("peer signal stream remains open");
    };
    tokio::join!(observed, peer);
}

fn prepared() -> batter_example_reference_service::config::PreparedServing {
    use batter::settings::SettingsSource;
    use batter_example_reference_service::{config::ServingSettings, runtime};
    let settings = ServingSettings::from_process(None, SettingsSource::default())
        .expect("child settings are valid");
    runtime::prepare(settings).expect("child preparation is valid")
}

async fn production() {
    batter_example_reference_service::runtime::run(prepared())
        .await
        .expect("clean production shutdown");
}

/// Actual `runtime::run`, drained by a parent signal at an acknowledged stage.
async fn startup_drain(stage: &str) {
    use batter::{
        cleanup::CleanupOutcome,
        startup::{StartupCause, StartupError},
    };
    use batter_example_reference_service::runtime::{self, ProtectedRuntimeStartupFailure};
    let signal_observer = if stage == "postgres.schema" {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("TERM observer installs");
        let mut interrupt = signal(SignalKind::interrupt()).expect("INT observer installs");
        Some(tokio::spawn(async move {
            tokio::select! {
                received = term.recv() => received.expect("TERM stream remains open"),
                received = interrupt.recv() => received.expect("INT stream remains open"),
            };
            event(SIGNAL_OBSERVED);
        }))
    } else {
        None
    };
    let error = runtime::run(prepared())
        .await
        .expect_err("startup must drain");
    if let Some(observer) = signal_observer {
        observer.await.expect("signal observer completes");
    }
    let failure = error
        .downcast_ref::<ProtectedRuntimeStartupFailure>()
        .expect("retained protected startup failure");
    let StartupError::Failed(startup) = failure.startup() else {
        panic!("startup coordinator failed")
    };
    // A failed startup has no running handoff, so Ready is structurally
    // unreachable; this process child does not observe readiness directly.
    assert_eq!(startup.stage, stage);
    assert!(matches!(startup.cause, StartupCause::Draining));
    assert!(startup.destruction_panic.is_none());
    assert!(startup.cleanup.skipped.is_empty());
    assert_eq!(startup.cleanup.records.len(), 1);
    assert_eq!(startup.cleanup.records[0].name, "postgres.pool");
    assert_eq!(
        startup.cleanup.records[0].outcome,
        CleanupOutcome::Succeeded
    );
}
