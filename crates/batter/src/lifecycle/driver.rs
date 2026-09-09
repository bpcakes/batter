use super::{ShutdownHandle, ShutdownReport, Supervisor};
use crate::scoped_dispatch;
use std::{future::pending, ops::Deref, sync::Arc};
use tokio::{sync::watch, task::JoinError};
use tracing::Instrument;

/// An owned driver's retained result, including a coordinator panic/abort.
/// A JoinError does not establish that asynchronous cleanup completed.
/// An Ok report must still be inspected for task and cleanup failures.
pub type DriverOutcome = Result<SharedShutdownReport, Arc<JoinError>>;

/// An unsuccessful shutdown, retaining every task and cleanup outcome.
/// Default diagnostics never print native task, cleanup or panic contents.
#[derive(Clone)]
pub enum ShutdownFailure {
    /// The driver completed, but its report contains unsuccessful outcomes.
    Report(SharedShutdownReport),
    /// The coordinator terminated without producing a shutdown report.
    Coordinator(Arc<JoinError>),
}

impl std::fmt::Display for ShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Report(report) => write!(f, "shutdown failed; {}", **report),
            Self::Coordinator(_) => f.write_str("shutdown coordinator terminated without a report"),
        }
    }
}
impl std::fmt::Debug for ShutdownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Report(report) => Some(&**report),
            Self::Coordinator(error) => Some(&**error),
        }
    }
}

/// Interpret a driver outcome without discarding unsuccessful report contents.
///
/// ```no_run
/// # async fn example(running: batter::lifecycle::RunningSupervisor) -> Result<(), batter::lifecycle::ShutdownFailure> {
/// batter::lifecycle::check_shutdown(running.wait().await)
/// # }
/// ```
pub fn check_shutdown(outcome: DriverOutcome) -> Result<(), ShutdownFailure> {
    match outcome {
        Ok(report) if report.is_success() => Ok(()),
        Ok(report) => Err(ShutdownFailure::Report(report)),
        Err(error) => Err(ShutdownFailure::Coordinator(error)),
    }
}

/// A cheaply cloneable reference to an owned driver's completed report.
///
/// All observers and clones retain the same report and original failures.
/// Dereferencing borrows the [`ShutdownReport`]; it does not clone its contents
/// or extend ownership of the running process. Display identifies the shared
/// owner; its error source is the concrete report, which displays the summary.
/// Debug may contain application error contents and is not automatically logged.
/// In particular, do not let this error escape a `main` returning `Result`:
/// Rust's [`std::process::Termination`] prints its Debug representation to stderr.
/// Keep rich errors inside the application and choose output at an explicit
/// [`std::process::ExitCode`] boundary, as in the PostgreSQL lifecycle example.
///
/// `must_use` warns when this value is discarded as an expression, including
/// after `?` or `unwrap()`. Binding or explicitly dropping it bypasses the lint;
/// the compiler cannot prove that a caller inspected the outcomes.
///
/// ```no_run
/// #![deny(unused_must_use)]
/// use batter::{BoxError, lifecycle::RunningSupervisor};
/// use std::{io::Write, process::ExitCode};
///
/// // Internal propagation preserves the original failures for a trusted sink.
/// async fn stop(running: RunningSupervisor) -> Result<(), BoxError> {
///     let report = running.shutdown().await?;
///     if !report.is_success() {
///         // Retain the complete report for an application-selected error sink.
///         return Err(Box::new(report));
///     }
///     Ok(())
/// }
///
/// #[tokio::main(flavor = "current_thread")]
/// async fn main() -> ExitCode {
///     match run().await {
///         Ok(()) => ExitCode::SUCCESS,
///         Err(_failure) => {
///             // Inspect _failure only through an application-selected trusted sink.
///             // Do not format it here; even Display is not universally sanitized.
///             let _ = writeln!(std::io::stderr().lock(), "service failed");
///             ExitCode::FAILURE
///         }
///     }
/// }
/// # async fn run() -> Result<(), BoxError> { Ok(()) }
/// ```
///
/// Checking only coordinator completion discards task/cleanup failures:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::lifecycle::RunningSupervisor;
/// async fn ignored(running: RunningSupervisor) {
///     running.wait().await.unwrap();
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::{BoxError, lifecycle::RunningSupervisor};
/// async fn ignored(running: RunningSupervisor) -> Result<(), BoxError> {
///     running.wait().await?;
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone)]
#[must_use = "inspect the report for failures and incomplete cleanup"]
pub struct SharedShutdownReport(Arc<ShutdownReport>);

impl Deref for SharedShutdownReport {
    type Target = ShutdownReport;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for SharedShutdownReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("owned shutdown report")
    }
}

impl std::error::Error for SharedShutdownReport {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

/// A cloneable completion observer. It does not extend ownership of the running
/// process, and cancelling a waiter cannot cancel its driver or finalizers.
/// Obtain one from [`RunningSupervisor::observer`] after [`Supervisor::start`];
/// unstarted supervisors and caller-owned `run_until` drivers have no observer.
#[derive(Clone)]
pub struct SupervisorObserver {
    completion: watch::Receiver<Option<DriverOutcome>>,
}

impl SupervisorObserver {
    /// Wait for the same retained result available to every observer. The Tokio
    /// runtime must remain alive to drive shutdown and the completion monitor.
    ///
    /// # Panics
    ///
    /// Panics if the completion monitor is dropped before publishing an outcome,
    /// such as when its owning runtime shuts down. Awaiting the observer on
    /// another runtime cannot recover that outcome. An already published outcome
    /// remains available after runtime shutdown.
    pub async fn wait(&self) -> DriverOutcome {
        let mut completion = self.completion.clone();
        loop {
            if let Some(outcome) = completion.borrow_and_update().clone() {
                return outcome;
            }
            // Only start constructs this channel. Its monitor owns the sender
            // until publishing, independently of all owners and control handles.
            completion
                .changed()
                .await
                .expect("owned driver retains completion sender");
        }
    }
}

/// Ownership of an explicitly started process driver. Clones share ownership;
/// dropping the last owner requests graceful shutdown, while a separate owned
/// coordinator and monitor continue through cleanup on the live runtime.
#[derive(Clone)]
pub struct RunningSupervisor {
    owner: Arc<DriverOwner>,
    observer: SupervisorObserver,
}

struct DriverOwner {
    handle: ShutdownHandle,
}

impl Drop for DriverOwner {
    fn drop(&mut self) {
        self.handle.request();
    }
}

impl RunningSupervisor {
    /// Clone lifecycle control without prolonging ownership of the driver.
    pub fn handle(&self) -> ShutdownHandle {
        self.owner.handle.clone()
    }

    /// Observe completion even after all driver owners have been dropped.
    /// This is available immediately after `start`, before the coordinator's
    /// first poll; it does not require readiness or retain process ownership.
    pub fn observer(&self) -> SupervisorObserver {
        self.observer.clone()
    }

    /// Await completion without transferring ownership or cancellation authority.
    ///
    /// # Panics
    ///
    /// Panics if the monitor is dropped before publication; see
    /// [`SupervisorObserver::wait`].
    pub async fn wait(&self) -> DriverOutcome {
        self.observer.wait().await
    }

    /// Request shutdown and observe its retained report. Cancelling this waiter
    /// does not stop the separately owned shutdown/cleanup coordinator.
    ///
    /// # Panics
    ///
    /// Panics if the monitor is dropped before publication; see
    /// [`SupervisorObserver::wait`].
    pub async fn shutdown(&self) -> DriverOutcome {
        self.owner.handle.request();
        self.wait().await
    }
}

impl Supervisor {
    /// Start an owned driver on the current Tokio runtime. Calling this method
    /// is the explicit execution boundary; registration itself remains inert.
    ///
    /// A bounded pair of tasks drives and observes the coordinator. The monitor
    /// intentionally survives handle drop so it can retain coordinator panic and
    /// successful cleanup reports. Runtime/process termination cannot be shielded.
    pub fn start(self) -> RunningSupervisor {
        let handle = self.handle();
        let (sender, completion) = watch::channel(None);
        let observer = SupervisorObserver { completion };
        let coordinator = tokio::spawn(scoped_dispatch::scope(
            self.run_until(pending()).in_current_span(),
        ));
        let monitor = async move {
            let outcome = coordinator
                .await
                .map(|report| SharedShutdownReport(Arc::new(report)))
                .map_err(Arc::new);
            let failed = outcome.is_err();
            // Publish before diagnostics: an application subscriber that panics
            // cannot prevent observers from receiving the coordinator failure.
            sender.send_replace(Some(outcome));
            if failed {
                tracing::error!(target: "batter", "owned process coordinator terminated without a shutdown report");
            }
        };
        // This is the explicit daemon monitor: finite, one per started driver,
        // owns the coordinator JoinHandle, and publishes every joined outcome.
        drop(tokio::spawn(scoped_dispatch::scope(
            monitor.in_current_span(),
        )));
        RunningSupervisor {
            owner: Arc::new(DriverOwner { handle }),
            observer,
        }
    }
}
