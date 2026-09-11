//! Runledger worker hosting with a scoped startup witness and explicit stop proof.
//!
//! The staged reference root registers only the private control contract. A
//! later provider task can use [`crate::worker::WorkerHost::start`] with the delivery catalog,
//! while retaining the same ownership and termination proof.
//!
//! ```no_run
//! use batter::{lifecycle::Supervisor, operation::OperationContext};
//! use batter_example_reference_service::{
//!     config::WorkerSettings,
//!     worker::{TerminationGate, prepare_probe_worker},
//! };
//! use sqlx::PgPool;
//! use std::time::Duration;
//!
//! # async fn register(
//! #     pool: &PgPool,
//! #     settings: &WorkerSettings,
//! #     supervisor: &mut Supervisor,
//! # ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let startup = OperationContext::new(Duration::from_secs(35))?;
//! let prepared = prepare_probe_worker(
//!     pool,
//!     settings,
//!     TerminationGate::new(),
//!     &startup,
//!     Duration::from_secs(20),
//! ).await?;
//! prepared.register(supervisor)?;
//! # Ok(())
//! # }
//! ```

mod cleanup;
mod errors;
mod lease;
mod preparation;
mod probe;
mod termination;
#[cfg(test)]
mod tests;

pub use cleanup::{DependencyCleanup, DependencyCleanupError};
use errors::classify_driver_completion;
pub use errors::{
    ProbeLeaseError, WorkerBuildError, WorkerDriverError, WorkerRegistrationError,
    WorkerStartupError,
};
use lease::release_probe_lease;
pub use lease::{CloseOutcome, ProbeLeaseRelease, UnlockOutcome};
pub use probe::{
    STARTUP_JOB_TYPE, STARTUP_LEASE_LOCK_CLASS, STARTUP_LEASE_LOCK_KEY, prepare_probe_worker,
};
use probe::{monitor_probe_lease, observe_witness};
pub use termination::{TerminationGate, WorkerSettlement};

use crate::config::WorkerSettings;
use batter::{
    BoxError,
    lifecycle::{ShutdownSignal, Supervisor as ProcessSupervisor},
};
use runledger_runtime::{SupervisorShutdown, catalog::JobCatalog};
use sqlx::{PgConnection, PgPool, types::Uuid};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

const UPSTREAM_SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);
const UPSTREAM_ABORT_DRAIN: Duration = Duration::from_secs(1);
const PROBE_LEASE_RELEASE_BUDGET: Duration = Duration::from_secs(2);
const SHUTDOWN_MARGIN: Duration = Duration::from_secs(1);
/// Complete worker stop allowance, including native abort drain, release and scheduling.
/// The process root reserves this interval before dependency cleanup.
pub const WORKER_SHUTDOWN_ALLOWANCE: Duration = Duration::from_secs(
    UPSTREAM_SHUTDOWN_BUDGET.as_secs()
        + UPSTREAM_ABORT_DRAIN.as_secs()
        + PROBE_LEASE_RELEASE_BUDGET.as_secs()
        + SHUTDOWN_MARGIN.as_secs(),
);
const UPSTREAM_SHUTDOWN_RESERVE: Duration = WORKER_SHUTDOWN_ALLOWANCE;

/// What the application has proved about work spawned by the native runtime.
///
/// `Unproven` deliberately does not distinguish a timeout, panic, returned loop
/// failure or dropped owner: none of those establishes that hidden descendants
/// stopped using dependent resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationState {
    /// Native construction has not reached a step that can spawn work.
    NotStarted,
    /// Native work may exist and cooperative termination has not been proved.
    Unproven,
    /// The owning native driver returned success after joining its loops.
    CooperativelyStopped,
}

/// A control-handler invocation observed before its successful return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupInvocation {
    /// Durable job identity selected by Runledger.
    pub job_id: Uuid,
    /// Durable workflow run number.
    pub run_number: i32,
    /// Durable attempt number within the run.
    pub attempt: i32,
    /// Fresh Batter operation deadline derived from this attempt's native deadline.
    pub operation_deadline: Instant,
    /// Unique startup witness carried in the typed payload.
    pub witness_id: Uuid,
}

#[derive(Clone)]
enum NativeDriverCompletion {
    Succeeded,
    Native(Arc<runledger_runtime::Error>),
    Join(Arc<tokio::task::JoinError>),
    Release {
        release: Arc<ProbeLeaseRelease>,
        native: Box<NativeDriverCompletion>,
    },
    ProbeLease {
        failure: Arc<ProbeLeaseError>,
        shutdown: Box<NativeDriverCompletion>,
    },
}

struct WorkerShutdown {
    native: SupervisorShutdown,
    driver: Mutex<Option<oneshot::Sender<()>>>,
}

impl WorkerShutdown {
    fn request(&self) {
        self.native.request_shutdown();
        if let Some(sender) = self
            .driver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = sender.send(());
        }
    }
}

struct WorkerDriver {
    shutdown: Arc<WorkerShutdown>,
    completion: watch::Sender<Option<NativeDriverCompletion>>,
}

impl WorkerDriver {
    fn start(
        supervisor: runledger_runtime::Supervisor,
        termination: TerminationGate,
        probe_lease: Option<PgConnection>,
    ) -> Arc<Self> {
        let native_shutdown = supervisor.shutdown_handle();
        let (driver_shutdown, shutdown_requested) = oneshot::channel();
        let (completion, _) = watch::channel(None);
        let shutdown = Arc::new(WorkerShutdown {
            native: native_shutdown,
            driver: Mutex::new(Some(driver_shutdown)),
        });
        let driver = Arc::new(Self {
            shutdown: shutdown.clone(),
            completion,
        });
        let mut native = tokio::spawn(supervisor.run_until_shutdown(
            async move {
                let _ = shutdown_requested.await;
            },
            UPSTREAM_SHUTDOWN_BUDGET,
        ));
        let published = driver.completion.clone();
        // This observer owns the native join and the optional probe lease. A
        // cancelled startup waiter or Batter wrapper therefore cannot detach the
        // upstream driver or let another probe owner overlap it.
        let fallback_gate = termination.clone();
        let fallback_publication = published.clone();
        let observer = tokio::spawn(async move {
            let mut probe_lease = probe_lease;
            let (native_result, lease_failure) = if let Some(lease) = probe_lease.as_mut() {
                tokio::select! {
                    biased;
                    result = &mut native => (result, None),
                    failure = monitor_probe_lease(lease) => {
                        shutdown.request();
                        (native.await, Some(Arc::new(failure)))
                    }
                }
            } else {
                (native.await, None)
            };
            let native_completion = match native_result {
                Ok(Ok(())) => NativeDriverCompletion::Succeeded,
                Ok(Err(error)) => NativeDriverCompletion::Native(Arc::new(error)),
                Err(error) => NativeDriverCompletion::Join(Arc::new(error)),
            };
            let result = match lease_failure {
                Some(failure) => NativeDriverCompletion::ProbeLease {
                    failure,
                    shutdown: Box::new(native_completion),
                },
                None => native_completion,
            };
            let stopped = matches!(result, NativeDriverCompletion::Succeeded);
            let result = if let Some(lease) = probe_lease.take() {
                let usable = !matches!(result, NativeDriverCompletion::ProbeLease { .. });
                let release = Arc::new(release_probe_lease(lease, usable).await);
                termination.record_release(release.clone());
                NativeDriverCompletion::Release {
                    release,
                    native: Box::new(result),
                }
            } else {
                result
            };
            if let Err(error) = classify_driver_completion(result.clone(), true) {
                termination.record_driver(Arc::new(error));
            }
            if stopped {
                termination.mark_cooperatively_stopped();
            } else {
                termination.mark_driver_observed();
            }
            published.send_replace(Some(result));
        });
        drop(tokio::spawn(async move {
            if let Err(error) = observer.await {
                let error = Arc::new(error);
                fallback_gate.record_driver(Arc::new(WorkerDriverError::Join(error.clone())));
                fallback_gate.mark_driver_observed();
                fallback_publication.send_replace(Some(NativeDriverCompletion::Join(error)));
            }
        }));
        driver
    }

    fn request_shutdown(&self) {
        // The native handle closes claim admission before resolving the future
        // that starts run_until_shutdown's bounded termination window.
        self.shutdown.request();
    }

    async fn wait(&self) -> NativeDriverCompletion {
        let mut completion = self.completion.subscribe();
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                return result;
            }
            completion
                .changed()
                .await
                .expect("worker driver owner retains completion publication");
        }
    }
}

/// One application component owning the complete Runledger supervisor driver.
///
/// Dropping this value requests native shutdown. An internal owner retains and
/// observes the native join, but only [`Self::register`] or [`Self::shutdown`]
/// lets the caller await and inspect its result.
pub struct WorkerHost {
    driver: Option<Arc<WorkerDriver>>,
}

/// Probe-only host plus the exact handler invocation that authorized its acknowledgement.
pub struct PreparedProbeWorker {
    host: WorkerHost,
    invocation: StartupInvocation,
}

impl PreparedProbeWorker {
    /// Inspect the durable attempt and fresh operation deadline observed by the handler.
    #[must_use]
    pub fn invocation(&self) -> StartupInvocation {
        self.invocation
    }

    /// Register the witnessed host as one Batter critical component.
    pub fn register(
        self,
        supervisor: &mut ProcessSupervisor,
    ) -> Result<(), WorkerRegistrationError> {
        self.host.register(supervisor)
    }

    /// Stop a witnessed host that cannot be registered and retain its outcome.
    ///
    /// ```no_run
    /// # use batter::operation::OperationContext;
    /// # use batter_example_reference_service::{config::WorkerSettings, worker::{TerminationGate, prepare_probe_worker}};
    /// # use sqlx::PgPool;
    /// # use std::time::Duration;
    /// # async fn stop(pool: &PgPool, settings: &WorkerSettings) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// let startup = OperationContext::new(Duration::from_secs(35))?;
    /// let prepared = prepare_probe_worker(
    ///     pool,
    ///     settings,
    ///     TerminationGate::new(),
    ///     &startup,
    ///     Duration::from_secs(20),
    /// ).await?;
    /// prepared.shutdown().await?;
    /// # Ok(()) }
    /// ```
    pub async fn shutdown(self) -> Result<(), WorkerDriverError> {
        self.host.shutdown().await
    }
}

impl WorkerHost {
    /// Start every native loop selected by `settings` and `catalog` on the
    /// current Tokio runtime, returning the single owning application wrapper.
    ///
    /// This is not a readiness witness. Callers must prove their actual handler
    /// path before registering the host as an acknowledged component. A returned
    /// build error is verified to precede spawning in the pinned Runledger
    /// revision and restores the supplied gate to `NotStarted`.
    pub fn start(
        settings: &WorkerSettings,
        pool: &PgPool,
        catalog: &JobCatalog,
        termination: TerminationGate,
    ) -> Result<Self, WorkerBuildError> {
        let supervisor = Self::build_supervisor(settings, pool, catalog, &termination)?;
        Ok(Self::host_supervisor(supervisor, termination, None))
    }

    /// Construct the native supervisor and arm `termination` for it.
    ///
    /// A returned error restores the gate to `NotStarted` and owns no lease, so
    /// a probe caller can still release its session before propagating.
    fn build_supervisor(
        settings: &WorkerSettings,
        pool: &PgPool,
        catalog: &JobCatalog,
        termination: &TerminationGate,
    ) -> Result<runledger_runtime::Supervisor, WorkerBuildError> {
        let builder = settings
            .builder(pool)
            .map_err(WorkerBuildError::Settings)?
            .with_catalog(catalog);
        termination.arm()?;
        match builder.build() {
            Ok(supervisor) => Ok(supervisor),
            Err(error) => {
                // The pinned builder returns every error before its first spawn.
                termination.restore_not_started();
                Err(WorkerBuildError::Native(error))
            }
        }
    }

    /// Drive a built supervisor, transferring the optional lease to its owner.
    fn host_supervisor(
        supervisor: runledger_runtime::Supervisor,
        termination: TerminationGate,
        probe_lease: Option<PgConnection>,
    ) -> Self {
        Self {
            driver: Some(WorkerDriver::start(supervisor, termination, probe_lease)),
        }
    }

    /// Register the witnessed runtime as one critical Batter component.
    ///
    /// The component immediately acknowledges its already-completed control
    /// witness, observes unexpected native exit, requests native stop when the
    /// process begins drain, and awaits the owning driver result.
    pub fn register(
        mut self,
        supervisor: &mut ProcessSupervisor,
    ) -> Result<(), WorkerRegistrationError> {
        let registered_host = Self {
            driver: Some(self.driver().clone()),
        };
        match supervisor.register("worker", move |signal| registered_host.run(signal)) {
            Ok(()) => {
                self.driver.take();
                Ok(())
            }
            Err(registration) => {
                self.request_shutdown();
                Err(WorkerRegistrationError {
                    registration,
                    host: self,
                })
            }
        }
    }

    async fn run(self, signal: ShutdownSignal) -> Result<(), BoxError> {
        if !signal.mark_started() && !signal.is_draining() {
            return Err(Box::new(WorkerDriverError::StartupAcknowledgementRejected));
        }
        let driver = self.driver().clone();
        let completion = tokio::select! {
            biased;
            result = driver.wait() => {
                return classify_driver_completion(result, false).map_err(|error| Box::new(error) as BoxError)
            },
            _ = signal.draining() => {
                driver.request_shutdown();
                driver.wait().await
            }
        };
        classify_driver_completion(completion, true).map_err(|error| Box::new(error) as BoxError)
    }

    /// Request native shutdown and await the independently retained driver.
    ///
    /// Use this when a host cannot be registered after construction. Dropping a
    /// host requests the same shutdown but cannot return its retained outcome.
    ///
    /// ```no_run
    /// # use batter_example_reference_service::{config::WorkerSettings, worker::{TerminationGate, WorkerHost}};
    /// # use runledger_runtime::catalog::JobCatalog;
    /// # use sqlx::PgPool;
    /// # async fn stop(settings: &WorkerSettings, pool: &PgPool, catalog: &JobCatalog) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// let host = WorkerHost::start(settings, pool, catalog, TerminationGate::new())?;
    /// host.shutdown().await?;
    /// # Ok(()) }
    /// ```
    pub async fn shutdown(self) -> Result<(), WorkerDriverError> {
        self.request_shutdown();
        classify_driver_completion(self.driver().wait().await, true)
    }

    async fn witness(
        &mut self,
        pool: &PgPool,
        expected_job: Uuid,
        expected_witness: Uuid,
        invocations: &mut mpsc::UnboundedReceiver<StartupInvocation>,
        deadline: Instant,
    ) -> Result<StartupInvocation, WorkerStartupError> {
        let driver = self.driver().clone();
        let witness = tokio::select! {
            biased;
            result = driver.wait() => {
                // The driver already completed, so there is no separate shutdown
                // outcome to await: that one completion is classified exactly once.
                let failure = classify_driver_completion(result, false)
                    .expect_err("driver completion before startup witness is unexpected");
                return Err(WorkerStartupError::Driver(Box::new(failure)));
            }
            result = observe_witness(pool, expected_job, expected_witness, invocations, deadline) => result,
        };
        let witness = match witness {
            Ok(invocation) => return Ok(invocation),
            Err(witness) => witness,
        };
        self.request_shutdown();
        match classify_driver_completion(driver.wait().await, true) {
            Ok(()) => Err(witness),
            Err(shutdown) => Err(WorkerStartupError::WitnessAndShutdown {
                witness: Box::new(witness),
                shutdown: Box::new(shutdown),
            }),
        }
    }

    fn request_shutdown(&self) {
        if let Some(driver) = &self.driver {
            driver.request_shutdown();
        }
    }

    fn driver(&self) -> &Arc<WorkerDriver> {
        self.driver
            .as_ref()
            .expect("worker host retains its driver")
    }
}

impl Drop for WorkerHost {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
