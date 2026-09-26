//! Owned serving orchestration and its combined, retained completion.

use crate::diagnostics::MetricsExport;
use batter::{BoxError, lifecycle::ShutdownHandle};
use std::{error::Error, fmt, process::ExitCode, sync::Arc};
use tokio::sync::watch;

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
pub struct OrchestrationFailure(tokio::task::JoinError);

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
        Some(&self.0)
    }
}

struct Completed {
    service: Result<(), ServiceFailure>,
    metrics: MetricsExport,
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
    pub(super) fn new(service: Result<(), BoxError>, metrics: MetricsExport) -> Self {
        Self(Arc::new(Completed {
            service: service.map_err(ServiceFailure),
            metrics,
        }))
    }

    fn abandoned(error: tokio::task::JoinError) -> Self {
        Self::new(
            Err(Box::new(OrchestrationFailure(error))),
            MetricsExport::Abandoned,
        )
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

type Publication = watch::Receiver<Option<ServiceCompletion>>;

async fn published(mut completion: Publication) -> ServiceCompletion {
    let observed = completion
        .wait_for(Option::is_some)
        .await
        .expect("serving monitor retains publication")
        .clone();
    observed.expect("wait_for returned a published completion")
}

/// Sole owner of one started serving orchestration.
///
/// Dropping the owner requests ordinary service drain; the orchestration keeps
/// running on the live runtime through cleanup, the final metrics export and
/// diagnostic closure. Cancelling [`Self::wait`] changes no ownership and
/// requests nothing. Take an [`Self::observer`] before dropping the owner to
/// inspect the retained completion later. No runtime-death guarantee exists.
///
/// ```compile_fail,E0599
/// use batter_example_reference_service::runtime::ServiceOwner;
/// fn cannot_duplicate(owner: ServiceOwner) {
///     let duplicate = owner.clone();
/// }
/// ```
///
/// Discarding the started owner requests drain immediately:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_example_reference_service::{config::PreparedServing, runtime};
/// fn discarded(prepared: PreparedServing) {
///     runtime::start(prepared);
/// }
/// ```
#[must_use = "retain the service owner; dropping it requests drain"]
pub struct ServiceOwner {
    handle: ShutdownHandle,
    completion: Publication,
}

impl ServiceOwner {
    pub(super) fn spawn<F>(handle: ShutdownHandle, orchestration: F) -> Self
    where
        F: Future<Output = ServiceCompletion> + Send + 'static,
    {
        let (publish, completion) = watch::channel(None);
        let orchestration = tokio::spawn(orchestration);
        // The monitor owns publication independently of every owner and waiter,
        // including an orchestration panic.
        tokio::spawn(async move {
            let completion = orchestration
                .await
                .unwrap_or_else(ServiceCompletion::abandoned);
            publish.send_replace(Some(completion));
        });
        Self { handle, completion }
    }

    /// Clone a completion observer without prolonging service ownership.
    pub fn observer(&self) -> ServiceObserver {
        ServiceObserver {
            completion: self.completion.clone(),
        }
    }

    /// Await the retained completion. Cancelling this waiter requests nothing;
    /// repeated waits return the same retained value.
    ///
    /// # Panics
    ///
    /// Panics if the runtime destroys the monitor before publication.
    pub async fn wait(&self) -> ServiceCompletion {
        published(self.completion.clone()).await
    }
}

impl Drop for ServiceOwner {
    fn drop(&mut self) {
        self.handle.request();
    }
}

impl fmt::Debug for ServiceOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServiceOwner")
    }
}

/// Cloneable observation of one serving run, independent of its owner.
#[derive(Clone)]
pub struct ServiceObserver {
    completion: Publication,
}

impl ServiceObserver {
    /// Await the retained completion without owning or stopping the service.
    ///
    /// # Panics
    ///
    /// Panics if the runtime destroys the monitor before publication.
    pub async fn wait(&self) -> ServiceCompletion {
        published(self.completion.clone()).await
    }
}

impl fmt::Debug for ServiceObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServiceObserver")
    }
}

#[cfg(test)]
mod tests;
