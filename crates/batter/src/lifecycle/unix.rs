use super::Supervisor;
use crate::RegistrationError;
use std::{fmt, io};
use tokio::signal::unix::{Signal, SignalKind, signal};

/// Signal installation or component registration failed before readiness.
#[derive(thiserror::Error)]
pub enum SignalRegistrationError {
    /// Original OS/runtime installation failure, available for trusted inspection.
    #[error("Unix shutdown signal installation failed")]
    Install(#[source] io::Error),
    /// Invalid/duplicate component name; installation was not attempted.
    #[error("Unix shutdown signal registration failed")]
    Registration(#[from] RegistrationError),
}
impl fmt::Debug for SignalRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Install native SIGTERM/SIGINT listeners now and register their critical task.
///
/// Call inside owned initialization, before readiness approval. Installation
/// failure is returned immediately; the component acknowledges already installed
/// listeners when its driver starts. Either signal requests normal process drain.
/// Existing drain also stops the listener task. Names are validated first.
///
/// Tokio signal installation changes process-wide signal handling, and dropping
/// listeners does not restore the previous OS disposition. No Windows fallback,
/// subscriber or panic-hook installation is supplied. Requires a Tokio runtime
/// with signal support; Tokio may panic outside that runtime.
///
/// ```no_run
/// # fn example(supervisor: &mut batter::lifecycle::Supervisor) -> Result<(), batter::lifecycle::SignalRegistrationError> {
/// batter::lifecycle::register_signals(supervisor, "signals")?;
/// # Ok(()) }
/// ```
pub fn register_signals(
    supervisor: &mut Supervisor,
    name: &'static str,
) -> Result<(), SignalRegistrationError> {
    install_with(supervisor, name, || {
        Ok((
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        ))
    })?
    .register(supervisor)
}

/// Installed Unix signal sources that can be polled during owned initialization
/// and then transferred into their one critical supervisor component.
///
/// Installation changes process-wide signal handling immediately. Call
/// [`Self::received`] while lengthy initialization is in progress, then call
/// [`Self::register`] before handing the supervisor to its driver. Dropping this
/// value does not restore the previous OS disposition.
///
/// ```no_run
/// # use batter::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor, install_signals}};
/// # use std::time::Duration;
/// # async fn initialize() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # let second = Duration::from_secs(1);
/// # let cleanup = CleanupBudget::new(second, second, second)?;
/// # let mut supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
/// let mut signals = install_signals(&supervisor, "signals")?;
/// tokio::select! {
///     _ = signals.received() => supervisor.handle().request(),
///     _ = async { /* application initialization */ } => {}
/// }
/// signals.register(&mut supervisor)?;
/// # Ok(()) }
/// ```
pub struct InstalledSignals {
    name: &'static str,
    terminate: Signal,
    interrupt: Signal,
    received: bool,
}

impl InstalledSignals {
    /// Wait until SIGTERM or SIGINT is pending.
    ///
    /// This borrowed wait is cancellation-safe: the installed sources remain in
    /// this value and can still be registered afterward. Completed reception is
    /// retained: later waits complete immediately and successful registration
    /// requests drain without requiring a second signal.
    pub async fn received(&mut self) {
        if self.received {
            return;
        }
        tokio::select! {
            biased;
            _ = self.terminate.recv() => {},
            _ = self.interrupt.recv() => {},
        }
        self.received = true;
    }

    /// Transfer both installed sources into one critical supervisor component.
    pub fn register(self, supervisor: &mut Supervisor) -> Result<(), SignalRegistrationError> {
        let Self {
            name,
            mut terminate,
            mut interrupt,
            received,
        } = self;
        let handle = supervisor.handle();
        supervisor.register(name, move |shutdown| async move {
            shutdown.mark_started();
            tokio::select! {
                biased;
                _ = shutdown.draining() => {},
                _ = terminate.recv() => handle.request(),
                _ = interrupt.recv() => handle.request(),
            }
            Ok(())
        })?;
        if received {
            supervisor.handle().request();
        }
        Ok(())
    }
}

/// Install SIGTERM/SIGINT sources without delaying their consumption until the
/// supervisor driver starts.
///
/// Use [`InstalledSignals::received`] in a startup `select!` when initialization
/// can take long enough that normal process termination must already request
/// owned cleanup. The returned value retains the validated component name for
/// later [`InstalledSignals::register`].
pub fn install_signals(
    supervisor: &Supervisor,
    name: &'static str,
) -> Result<InstalledSignals, SignalRegistrationError> {
    install_with(supervisor, name, || {
        Ok((
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        ))
    })
}

fn install_with(
    supervisor: &Supervisor,
    name: &'static str,
    install: impl FnOnce() -> io::Result<(Signal, Signal)>,
) -> Result<InstalledSignals, SignalRegistrationError> {
    supervisor.check_component_name(name)?;
    let (terminate, interrupt) = install().map_err(SignalRegistrationError::Install)?;
    Ok(InstalledSignals {
        name,
        terminate,
        interrupt,
        received: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cleanup::CleanupBudget,
        lifecycle::ShutdownBudget,
        operation::OperationContext,
        startup::{Startup, StartupCause, StartupError},
    };
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    const SIGNAL_CHILD: &str = "BATTER_INSTALLED_SIGNAL_CHILD";

    #[test]
    fn installed_sources_receive_before_driver_registration() {
        if std::env::var_os(SIGNAL_CHILD).is_some() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let second = Duration::from_secs(1);
                let cleanup = CleanupBudget::new(second, second, second).unwrap();
                let mut supervisor =
                    Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
                let mut signals = install_signals(&supervisor, "signals").unwrap();
                println!("installed-signal-child-ready");
                std::io::stdout().flush().unwrap();
                tokio::time::timeout(Duration::from_secs(2), signals.received())
                    .await
                    .expect("installed source receives SIGTERM before registration");
                signals.register(&mut supervisor).unwrap();
                assert_eq!(
                    supervisor.handle().readiness(),
                    super::super::Readiness::Draining
                );
            });
            return;
        }

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lifecycle::unix::tests::installed_sources_receive_before_driver_registration",
                "--nocapture",
            ])
            .env(SIGNAL_CHILD, "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            let mut captured = String::new();
            loop {
                let mut line = String::new();
                match output.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        captured.push_str(&line);
                        if line.contains("installed-signal-child-ready") {
                            let _ = ready_sender.try_send(());
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok::<_, std::io::Error>(captured)
        });
        if ready_receiver.recv_timeout(Duration::from_secs(3)).is_err() {
            child.kill().unwrap();
            let _ = child.wait();
            let captured = reader.join().unwrap().unwrap();
            panic!("signal child exited or stalled before readiness: {captured:?}");
        }
        let signaled = Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(signaled.success());

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                let captured = reader.join().unwrap().unwrap();
                assert!(
                    status.success(),
                    "signal child failed: {status}; output: {captured:?}"
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("signal child did not observe SIGTERM");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn installation_failure_is_startup_failure_and_runs_prior_cleanup() {
        let second = Duration::from_secs(1);
        let cleanup = CleanupBudget::new(second, second, second).unwrap();
        let supervisor =
            Supervisor::new(ShutdownBudget::new(second, second, second, cleanup).unwrap());
        let mut starting = Startup::new(
            supervisor,
            OperationContext::new(second).unwrap(),
            cleanup,
            |scope| {
                Box::pin(async move {
                    scope
                        .supervisor()
                        .on_cleanup("resource", || async { Ok(()) })?;
                    install_with(scope.supervisor(), "signals", || {
                        Err(io::Error::other("installation-original"))
                    })?;
                    Ok(())
                })
            },
        )
        .start();
        let Err(StartupError::Failed(report)) = starting.wait().await else {
            panic!("expected failure")
        };
        let StartupCause::Failed(SignalRegistrationError::Install(error)) = &report.cause else {
            panic!("lost OS cause")
        };
        assert_eq!(error.to_string(), "installation-original");
        assert!(report.cleanup.is_success());
        assert_eq!(report.cleanup.records.len(), 1);
        assert!(!format!("{report:?}").contains("installation-original"));
    }
}
