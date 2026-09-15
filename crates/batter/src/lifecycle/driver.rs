use super::{
    LifecycleStatus, OperationAdmission, ReadinessApproval, ShutdownHandle, ShutdownReport,
    Supervisor,
};
use crate::{completion::wait_published, scoped_dispatch};
use std::{
    future::{Future, pending},
    ops::Deref,
    sync::Arc,
};
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
/// Obtain one from [`RunningSupervisor::observer`] after [`Supervisor::start`]
/// or from [`UnapprovedSupervisor::observer`] after
/// [`Supervisor::start_unapproved`]; unstarted supervisors and caller-owned
/// `run_until` drivers have no observer.
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
        // Only start constructs this channel. Its monitor owns the sender until
        // publishing, independently of all owners and control handles.
        wait_published(self.completion.clone())
            .await
            .expect("owned driver retains completion sender")
    }
}

/// A started process whose one application-readiness decision is still pending.
///
/// This non-cloneable value keeps the decision paired with its driver. Consuming
/// [`Self::approve_readiness`] records application approval and returns ordinary
/// [`RunningSupervisor`] ownership. The process can still remain Starting while
/// registered components initialize. Dropping this value requests shutdown like
/// dropping the last running owner; approval during drain cannot revive readiness.
///
/// ```no_run
/// use batter::{
///     cleanup::CleanupBudget,
///     lifecycle::{ShutdownBudget, Supervisor, UnapprovedSupervisor},
/// };
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), batter::BoxError> {
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let pending: UnapprovedSupervisor = Supervisor::new(ShutdownBudget::new(
///     second, second, second, cleanup,
/// )?)
/// .start_unapproved();
/// let running = pending.approve_readiness();
/// let _report = running.shutdown().await?;
/// # Ok(()) }
/// ```
///
/// ```compile_fail,E0599
/// use batter::lifecycle::UnapprovedSupervisor;
/// fn cannot_clone(pending: UnapprovedSupervisor) {
///     let duplicate = pending.clone();
/// }
/// ```
///
/// ```compile_fail,E0382
/// use batter::lifecycle::UnapprovedSupervisor;
/// fn cannot_approve_twice(pending: UnapprovedSupervisor) {
///     let running = pending.approve_readiness();
///     let again = pending.approve_readiness();
/// }
/// ```
#[must_use = "retain the process owner and decide whether to approve readiness"]
pub struct UnapprovedSupervisor {
    running: RunningSupervisor,
    approval: ReadinessApproval,
}

impl UnapprovedSupervisor {
    /// Consume the process's only application-readiness decision.
    ///
    /// Approval arms readiness but does not bypass pending component startup.
    /// A concurrent drain remains irreversible.
    pub fn approve_readiness(self) -> RunningSupervisor {
        let Self { running, approval } = self;
        approval.approve();
        running
    }

    /// Clone root shutdown control without exposing readiness approval.
    pub fn handle(&self) -> ShutdownHandle {
        self.running.handle()
    }

    /// Clone read-only readiness and lifecycle status.
    pub fn status(&self) -> LifecycleStatus {
        self.running.status()
    }

    /// Clone readiness-gated admission. It rejects while approval is pending.
    pub fn operation_admission(&self) -> OperationAdmission {
        self.running.operation_admission()
    }

    /// Observe completion without retaining process ownership.
    pub fn observer(&self) -> SupervisorObserver {
        self.running.observer()
    }

    /// Await completion without approving readiness or requesting shutdown.
    pub async fn wait(&self) -> DriverOutcome {
        self.running.wait().await
    }

    /// Request shutdown and observe its retained report without approving readiness.
    pub async fn shutdown(&self) -> DriverOutcome {
        self.running.shutdown().await
    }
}

/// Ownership of an explicitly started process driver after application approval.
/// Clones share ownership;
/// dropping the last owner requests graceful shutdown, while a separate owned
/// coordinator and monitor continue through cleanup on the live runtime.
///
/// Application approval is no longer available after this transition:
///
/// ```compile_fail,E0599
/// use batter::lifecycle::RunningSupervisor;
/// fn cannot_approve_again(running: RunningSupervisor) {
///     running.approve_readiness();
/// }
/// ```
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
    /// Clone root shutdown control without prolonging ownership of the driver.
    pub fn handle(&self) -> ShutdownHandle {
        self.owner.handle.clone()
    }

    /// Clone read-only readiness and lifecycle status without retaining control.
    pub fn status(&self) -> LifecycleStatus {
        self.owner.handle.status()
    }

    /// Clone readiness-gated admission for transient operation contexts.
    pub fn operation_admission(&self) -> OperationAdmission {
        self.owner.handle.operation_admission()
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
    /// Application readiness is approved once at this boundary. The process can
    /// still remain Starting until the driver polls and every registered
    /// component acknowledges its own initialization. Use
    /// [`Self::start_unapproved`] only when a separate policy stage deliberately
    /// withholds application approval.
    ///
    /// A bounded pair of tasks drives and observes the coordinator. The monitor
    /// intentionally survives handle drop so it can retain coordinator panic and
    /// successful cleanup reports. Runtime/process termination cannot be shielded.
    pub fn start(self) -> RunningSupervisor {
        let handle = self.handle();
        let (approval, driver) = self.into_unapproved_driver(pending());
        // Ordinary start approves before the coordinator can poll. Otherwise a
        // fast component can acknowledge initialization and still observe the
        // application as unapproved on another runtime worker.
        approval.approve();
        spawn_owned_driver(handle, driver)
    }

    /// Start an owned driver while deliberately retaining application-readiness
    /// approval in the returned typestate.
    ///
    /// Consume [`UnapprovedSupervisor::approve_readiness`] after the additional
    /// checks pass. Dropping the owner requests shutdown; a late approval cannot
    /// reverse drain.
    pub fn start_unapproved(self) -> UnapprovedSupervisor {
        let handle = self.handle();
        let (approval, driver) = self.into_unapproved_driver(pending());
        UnapprovedSupervisor {
            running: spawn_owned_driver(handle, driver),
            approval,
        }
    }
}

fn spawn_owned_driver<F>(handle: ShutdownHandle, driver: F) -> RunningSupervisor
where
    F: Future<Output = ShutdownReport> + Send + 'static,
{
    let (sender, completion) = watch::channel(None);
    let observer = SupervisorObserver { completion };
    let coordinator = tokio::spawn(scoped_dispatch::scope(driver.in_current_span()));
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
