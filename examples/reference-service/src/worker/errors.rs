use super::{NativeDriverCompletion, WorkerHost};
use runledger_runtime::catalog::CatalogError;
use std::sync::Arc;

/// Failure returned before the native supervisor can be hosted.
#[derive(Debug, thiserror::Error)]
pub enum WorkerBuildError {
    /// Validated application settings could not construct the native builder.
    #[error("worker settings could not construct the native builder")]
    Settings(#[source] batter::settings::SettingsError),
    /// This one-shot termination gate was already consumed.
    #[error("worker termination gate was already used")]
    GateAlreadyUsed,
    /// Native builder validation failed before it spawned a runtime loop.
    #[error("native worker construction failed")]
    Native(#[source] runledger_runtime::RuntimeError),
}

/// An owning upstream driver failure. The native error or Tokio join cause is retained.
#[derive(Debug, thiserror::Error)]
pub enum WorkerDriverError {
    /// The component could not record its one startup acknowledgement.
    #[error("worker startup acknowledgement was rejected")]
    StartupAcknowledgementRejected,
    /// The Runledger driver returned its first observed runtime error.
    #[error("native worker driver failed")]
    Native(#[source] Arc<runledger_runtime::Error>),
    /// The task driving Runledger panicked or was externally aborted.
    #[error("native worker driver task failed")]
    Join(#[source] Arc<tokio::task::JoinError>),
    /// The session proving exclusive startup-control ownership was lost.
    #[error("startup control ownership was lost while the native worker was running")]
    ProbeLease {
        /// The lease-monitor failure that initiated native shutdown.
        #[source]
        failure: Arc<ProbeLeaseError>,
        /// A separate failure observed while stopping the native driver, if any.
        shutdown: Option<Box<WorkerDriverError>>,
    },
    /// Lease settlement failed independently of native termination.
    #[error("startup control lease release failed")]
    Release {
        /// Both unlock and client-close observations.
        release: Arc<super::ProbeLeaseRelease>,
        /// Independent native termination error, if any.
        shutdown: Option<Box<WorkerDriverError>>,
    },
    /// Every native loop exited without the process requesting drain.
    #[error("native worker exited before process drain")]
    UnexpectedSuccess,
}

/// Failure from the live PostgreSQL session that owns startup-control exclusion.
#[derive(Debug, thiserror::Error)]
pub enum ProbeLeaseError {
    /// The periodic query proving that the same session remains usable failed.
    #[error("startup control ownership check failed")]
    Query(#[source] Arc<sqlx::Error>),
    /// PostgreSQL did not answer the ownership check within its local allowance.
    #[error("startup control ownership check timed out")]
    Timeout,
}

/// Registration rejection that retains the still-owned native worker host.
pub struct WorkerRegistrationError {
    pub(super) registration: batter::RegistrationError,
    pub(super) host: WorkerHost,
}

impl WorkerRegistrationError {
    /// Inspect why Batter rejected the critical-component registration.
    #[must_use]
    pub fn registration_error(&self) -> &batter::RegistrationError {
        &self.registration
    }

    /// Recover the already-stopping host so its retained driver can be awaited.
    ///
    /// ```no_run
    /// # use batter::lifecycle::Supervisor;
    /// # use batter_example_reference_service::worker::PreparedProbeWorker;
    /// # async fn recover(prepared: PreparedProbeWorker, supervisor: &mut Supervisor) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// if let Err(rejected) = prepared.register(supervisor) {
    ///     rejected.into_host().shutdown().await?;
    /// }
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn into_host(self) -> WorkerHost {
        self.host
    }

    /// Recover both the Batter rejection and the already-stopping host.
    ///
    /// ```no_run
    /// # use batter::lifecycle::Supervisor;
    /// # use batter_example_reference_service::worker::PreparedProbeWorker;
    /// # async fn recover(prepared: PreparedProbeWorker, supervisor: &mut Supervisor) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// if let Err(rejected) = prepared.register(supervisor) {
    ///     let (registration, host) = rejected.into_parts();
    ///     let shutdown = host.shutdown().await;
    ///     shutdown?;
    ///     return Err(Box::new(registration));
    /// }
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn into_parts(self) -> (batter::RegistrationError, WorkerHost) {
        (self.registration, self.host)
    }
}

impl std::fmt::Display for WorkerRegistrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("worker critical-component registration was rejected")
    }
}

impl std::fmt::Debug for WorkerRegistrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for WorkerRegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.registration)
    }
}

/// Startup could not prove dispatch and durable success through the control handler.
#[derive(Debug, thiserror::Error)]
pub enum WorkerStartupError {
    /// The preparation caller or parent requested cancellation.
    #[error("startup control preparation was cancelled")]
    Cancelled,
    /// The independent preparation task failed before publishing its result.
    #[error("startup control preparation task failed")]
    PreparationJoin(#[source] Arc<tokio::task::JoinError>),
    /// Closing the temporary native preparation pool exceeded its allowance.
    #[error("startup preparation pool closure timed out")]
    PreparationPoolClose {
        /// Independent preparation failure, if one preceded closure.
        preparation: Option<Box<WorkerStartupError>>,
    },
    /// Preparation and lease settlement both failed.
    #[error("startup control preparation and release failed")]
    PreparationAndRelease {
        /// The original preparation failure.
        preparation: Box<WorkerStartupError>,
        /// Independent release observations.
        release: Arc<super::ProbeLeaseRelease>,
    },
    /// The probe-only catalog could not be assembled.
    #[error("startup control catalog is invalid")]
    Catalog(#[source] CatalogError),
    /// The probe-only definition could not be synchronized.
    #[error("startup control definition synchronization failed")]
    Definition(#[source] CatalogError),
    /// The unique startup control job could not be serialized.
    #[error("startup control payload could not be serialized")]
    Submission(#[source] runledger_core::jobs::JobSubmissionError),
    /// The startup control job could not be durably enqueued.
    #[error("startup control enqueue failed")]
    Enqueue(#[source] runledger_postgres::Error),
    /// Existing pending or leased startup controls could not be enumerated.
    #[error("stale startup controls could not be enumerated")]
    StaleControlRead(#[source] sqlx::Error),
    /// An existing pending or leased startup control could not be canceled.
    #[error("stale startup control could not be canceled")]
    StaleControlCancel(#[source] runledger_postgres::Error),
    /// The native worker could not be constructed.
    #[error(transparent)]
    Build(#[from] WorkerBuildError),
    /// Another staged probe worker already owns this database.
    #[error("another startup control owner is active")]
    ProbeAlreadyActive,
    /// The database-scoped startup-control lease could not be acquired.
    #[error("startup control ownership could not be established")]
    ProbeLease(#[source] sqlx::Error),
    /// The acquired session failed a liveness check during database preparation.
    #[error("startup control ownership was lost during preparation")]
    ProbeLeaseLost(#[source] Arc<ProbeLeaseError>),
    /// The owning native driver exited before the witness completed.
    #[error("native worker exited before its startup witness")]
    Driver(#[source] Box<WorkerDriverError>),
    /// The handler was not observed within the startup allowance.
    #[error("startup control handler was not observed before its deadline")]
    WitnessTimeout,
    /// The caller's relative witness allowance could not form a Tokio deadline.
    #[error("startup control witness allowance exceeds the clock range")]
    WitnessBudgetOverflow,
    /// The actual handler closed its acknowledgement channel without a witness.
    #[error("startup control witness channel closed")]
    WitnessClosed,
    /// The parent deadline leaves no positive witness interval before native shutdown.
    #[error("startup control has no time remaining before its shutdown reserve")]
    WitnessBudgetUnavailable,
    /// Database preparation consumed the interval reserved ahead of native shutdown.
    #[error("startup control preparation exceeded its parent work deadline")]
    PreparationTimeout,
    /// Persisted terminal state could not be read after handler invocation.
    #[error("startup control durable state could not be read")]
    Status(#[source] sqlx::Error),
    /// The handler returned but its durable state was terminal without success.
    #[error("startup control reached an unsuccessful terminal state")]
    UnsuccessfulStatus,
    /// Witness failure was followed by a native shutdown failure; both are retained.
    #[error("startup witness failed and native worker shutdown also failed")]
    WitnessAndShutdown {
        /// Original witness failure.
        witness: Box<WorkerStartupError>,
        /// Independent driver failure while cleaning up failed startup.
        shutdown: Box<WorkerDriverError>,
    },
}

pub(super) fn classify_driver_completion(
    result: NativeDriverCompletion,
    shutdown_requested: bool,
) -> Result<(), WorkerDriverError> {
    match result {
        NativeDriverCompletion::Release { release, native } => {
            let native = classify_driver_completion(*native, shutdown_requested);
            if release.is_success() {
                native
            } else {
                Err(WorkerDriverError::Release {
                    release,
                    shutdown: native.err().map(Box::new),
                })
            }
        }
        NativeDriverCompletion::Succeeded if shutdown_requested => Ok(()),
        NativeDriverCompletion::Succeeded => Err(WorkerDriverError::UnexpectedSuccess),
        NativeDriverCompletion::Native(error) => Err(WorkerDriverError::Native(error)),
        NativeDriverCompletion::Join(error) => Err(WorkerDriverError::Join(error)),
        NativeDriverCompletion::ProbeLease { failure, shutdown } => {
            let shutdown = classify_driver_completion(*shutdown, true)
                .err()
                .map(Box::new);
            Err(WorkerDriverError::ProbeLease { failure, shutdown })
        }
    }
}
