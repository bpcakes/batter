use super::{ShutdownHandle, ShutdownReport, Supervisor};
use crate::scoped_dispatch;
use std::{future::pending, sync::Arc};
use tokio::{sync::watch, task::JoinError};
use tracing::Instrument;

/// An owned driver's retained result, including a coordinator panic/abort.
/// A JoinError does not establish that asynchronous cleanup completed.
pub type DriverOutcome = Result<Arc<ShutdownReport>, Arc<JoinError>>;

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
            let outcome = coordinator.await.map(Arc::new).map_err(Arc::new);
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
