//! Owned serving orchestration and its combined, retained completion.

use crate::diagnostics::MetricsExport;
use batter::{
    BoxError,
    service::{DiagnosticOutcome, ServiceOutcome},
};
use std::{error::Error, fmt, process::ExitCode, sync::Arc};

/// The original service failure, retained unchanged beside diagnostics.
///
/// Display and Debug are fixed. [`Self::error`] exposes the original boxed
/// error for explicit downcasting, for example to
/// [`super::ProtectedRuntimeStartupFailure`],
/// [`batter::lifecycle::ShutdownFailure`] or
/// [`super::RuntimePoolCleanupFailure`].
pub struct ServiceFailure(BoxError);

impl ServiceFailure {
    /// Borrow the original service error for typed inspection.
    pub fn error(&self) -> &(dyn Error + Send + Sync + 'static) {
        self.0.as_ref()
    }
}

impl fmt::Display for ServiceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reference service failed")
    }
}

impl fmt::Debug for ServiceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ServiceFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// The orchestration task terminated before publishing its own completion.
/// The retained join error is available through the error source.
pub struct OrchestrationFailure(Arc<tokio::task::JoinError>);

impl fmt::Display for OrchestrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reference service orchestration terminated without completion")
    }
}

impl fmt::Debug for OrchestrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for OrchestrationFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

struct Completed {
    service: Result<(), ServiceFailure>,
    metrics: MetricsExport,
    native: NativeCompletion,
}

/// The retained result of one serving run: the original service result and
/// the separate metrics diagnostics.
///
/// Diagnostics never replace the service result, its report or
/// [`Self::exit_code`]. Clones share one retained value. Display is fixed and
/// Debug shows only closed diagnostic categories.
///
/// ```
/// use batter_example_reference_service::runtime::ServiceCompletion;
/// use std::process::ExitCode;
///
/// fn classify(completion: &ServiceCompletion) -> ExitCode {
///     // Diagnostic failures, including a failed final export, never change
///     // this classification.
///     completion.exit_code()
/// }
/// ```
///
/// Completions are produced only by the owned orchestration:
///
/// ```compile_fail,E0423
/// use batter_example_reference_service::runtime::ServiceCompletion;
/// fn forge() -> ServiceCompletion {
///     ServiceCompletion(Default::default())
/// }
/// ```
#[derive(Clone)]
pub struct ServiceCompletion(Arc<Completed>);

impl ServiceCompletion {
    fn from_native(native: NativeCompletion) -> Self {
        let service = match native.service() {
            ServiceOutcome::StartupFailed(error) => Err(super::startup_failure(error.clone())),
            ServiceOutcome::Shutdown(Ok(success)) => {
                super::check_application_shutdown(success.clone())
            }
            ServiceOutcome::Shutdown(Err(error)) => Err(Box::new(error.clone()) as BoxError),
            ServiceOutcome::Coordinator(error) => {
                Err(Box::new(OrchestrationFailure(error.clone())) as BoxError)
            }
        };
        let metrics = match native.diagnostics() {
            DiagnosticOutcome::Completed(report) => report.clone(),
            DiagnosticOutcome::InstallationPanicked(_) | DiagnosticOutcome::TaskFailed(_) => {
                MetricsExport::Abandoned
            }
        };
        Self(Arc::new(Completed {
            service: service.map_err(ServiceFailure),
            metrics,
            native,
        }))
    }

    /// Original library completion, including retained diagnostic panic details.
    pub fn foundation(&self) -> &NativeCompletion {
        &self.0.native
    }

    /// The original service result, unchanged by diagnostics.
    pub fn service(&self) -> Result<(), &ServiceFailure> {
        self.0.service.as_ref().map(|_| ())
    }

    /// Metrics export diagnostics for this run.
    pub fn metrics(&self) -> &MetricsExport {
        &self.0.metrics
    }

    /// Process exit classification derived only from the service result.
    pub fn exit_code(&self) -> ExitCode {
        match self.0.service {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    }
}

impl fmt::Display for ServiceCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reference service completion")
    }
}

impl fmt::Debug for ServiceCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceCompletion")
            .field(
                "service",
                &if self.0.service.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                },
            )
            .field("metrics", &self.0.metrics)
            .finish()
    }
}

/// Native completion retained before application exit policy is applied.
pub type NativeCompletion =
    batter::service::ServiceCompletion<super::InitializationFailure, MetricsExport>;
type NativeOwner = batter::service::ServiceOwner<super::InitializationFailure, MetricsExport>;
type NativeObserver = batter::service::ServiceObserver<super::InitializationFailure, MetricsExport>;

/// Application owner backed by Batter's protected service lifecycle.
/// Drop requests drain; the runtime must remain alive through diagnostic closure.
#[must_use = "retain the owner; dropping it requests drain"]
pub struct ServiceOwner(NativeOwner);

impl ServiceOwner {
    pub(super) fn new(owner: NativeOwner) -> Self {
        Self(owner)
    }
    /// Observe completion without retaining service ownership.
    pub fn observer(&self) -> ServiceObserver {
        ServiceObserver(self.0.observer())
    }
    /// Await service and diagnostics. Cancelling this waiter requests nothing.
    /// Panics if runtime destruction prevents publication.
    pub async fn wait(&self) -> ServiceCompletion {
        ServiceCompletion::from_native(self.0.wait().await)
    }
}

impl fmt::Debug for ServiceOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceOwner")
    }
}

/// Read-only completion observer backed by Batter.
#[derive(Clone)]
pub struct ServiceObserver(NativeObserver);

impl ServiceObserver {
    /// Await completion without owning the service. Panics if runtime destruction
    /// prevents publication. Repeated waits preserve the native report identity.
    pub async fn wait(&self) -> ServiceCompletion {
        ServiceCompletion::from_native(self.0.wait().await)
    }
}

impl fmt::Debug for ServiceObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceObserver")
    }
}

#[cfg(test)]
mod tests;
