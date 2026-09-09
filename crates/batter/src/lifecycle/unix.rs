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
    register_with(supervisor, name, || {
        Ok((
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        ))
    })
}

fn register_with(
    supervisor: &mut Supervisor,
    name: &'static str,
    install: impl FnOnce() -> io::Result<(Signal, Signal)>,
) -> Result<(), SignalRegistrationError> {
    supervisor.check_component_name(name)?;
    let (mut terminate, mut interrupt) = install().map_err(SignalRegistrationError::Install)?;
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
    Ok(())
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
    use std::time::Duration;

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
                    register_with(scope.supervisor(), "signals", || {
                        Err(io::Error::other("installation-original"))
                    })
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
