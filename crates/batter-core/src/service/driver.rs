use super::readiness::{ServiceReadinessError, StartingService, readiness_result};
use super::{
    DiagnosticCompletion, DiagnosticOutcome, Diagnostics, ServiceCompletion, ServiceOutcome,
    coverage::coverage, report::Completed,
};
use crate::scoped_dispatch::ScopedOutput;
use crate::{
    PanicPayload,
    completion::wait_published,
    lifecycle::{LifecycleStatus, ShutdownHandle},
    scoped_dispatch,
    startup::{Initializer, ScopedStartup},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use tokio::{
    sync::{oneshot, watch},
    time::Instant,
};
use tracing::Instrument;

/// Sole owner of startup, running service and diagnostic settlement.
/// Dropping it requests drain. The runtime must remain alive through completion.
///
/// ```compile_fail,E0599
/// use batter_core::service::ServiceOwner;
/// fn duplicate(owner: ServiceOwner<std::io::Error, ()>) { let _ = owner.clone(); }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_core::{service, startup::{ScopedStartup, ProtectedStartupScope, StartupFuture}};
/// fn discard<F>(startup: ScopedStartup<F>)
/// where F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, std::io::Error> + Send + 'static {
///     service::start(startup, service::NoDiagnostics);
/// }
/// ```
#[must_use = "retain the owner; dropping it requests service drain"]
pub struct ServiceOwner<E, R> {
    handle: ShutdownHandle,
    observer: ServiceObserver<E, R>,
}

impl<E, R> ServiceOwner<E, R> {
    /// Observe completion without retaining service ownership.
    pub fn observer(&self) -> ServiceObserver<E, R> {
        self.observer.clone()
    }
    /// Borrow a cancellation-neutral waiter. Repeated waits share one result.
    /// Panics if runtime destruction prevents publication.
    pub async fn wait(&self) -> ServiceCompletion<E, R> {
        self.observer.wait().await
    }
    /// Read-only current lifecycle status, without approval or shutdown authority.
    pub fn status(&self) -> LifecycleStatus {
        self.observer.status()
    }

    /// Observe the retained first-readiness outcome. Later drain does not erase it.
    /// This observes lifecycle readiness, not optional dependency-health predicates.
    pub async fn wait_ready(&self) -> Result<(), ServiceReadinessError> {
        self.observer.wait_ready().await
    }

    /// Request ordinary drain explicitly, without cancelling diagnostic cleanup.
    pub fn request_shutdown(&self) {
        self.handle.request();
    }
}

impl<E, R> Drop for ServiceOwner<E, R> {
    fn drop(&mut self) {
        self.handle.request();
    }
}

/// Cloneable completion observation with no service ownership or control.
pub struct ServiceObserver<E, R> {
    completion: watch::Receiver<Option<ServiceCompletion<E, R>>>,
    status: LifecycleStatus,
}

impl<E, R> Clone for ServiceObserver<E, R> {
    fn clone(&self) -> Self {
        Self {
            completion: self.completion.clone(),
            status: self.status.clone(),
        }
    }
}

impl<E, R> ServiceObserver<E, R> {
    /// Current read-only lifecycle state. This grants no readiness approval.
    pub fn status(&self) -> LifecycleStatus {
        self.status.clone()
    }

    /// Wait for first readiness or its terminal failure, independently of other waiters.
    /// Success records a past transition; use `status()` for current readiness.
    /// Dependency-health and application HTTP probe predicates are separate.
    pub async fn wait_ready(&self) -> Result<(), ServiceReadinessError> {
        readiness_result(self.status.wait_startup().await)
    }

    /// Await both retained outcomes. Cancellation requests nothing.
    /// Panics if runtime destruction prevents publication.
    pub async fn wait(&self) -> ServiceCompletion<E, R> {
        wait_published(self.completion.clone())
            .await
            .expect("service monitor retains publication")
    }
}

/// Consume protected startup and explicitly selected diagnostics.
///
/// Installation happens synchronously before startup. Selected Unix listeners
/// are installed before return, as with [`ScopedStartup::start`]. Diagnostic
/// panics do not prevent service execution. Finalization is released only after
/// startup failure cleanup or checked running-driver completion is retained.
/// See the module example for owner/observer composition. The executable root
/// that calls this installs [`crate::redacting_panic_hook`] as the first
/// statement of `main`; the service installs no panic hook.
///
/// This completion-only entrypoint accepts unit initializer output. Use
/// [`super::start_with_readiness`] for a readiness deadline and typed output;
/// returning a value here is rejected rather than silently dropping it:
///
/// ```compile_fail,E0271
/// use batter_core::{cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext,
///     service::{self, NoDiagnostics}, startup::Startup};
/// fn typed(supervisor: Supervisor, context: OperationContext, cleanup: CleanupBudget) {
///     let startup = Startup::protected(supervisor, context, cleanup, async |_scope| {
///         Ok::<_, std::io::Error>(8080_u16)
///     });
///     let _owner = service::start(startup, NoDiagnostics);
/// }
/// ```
///
/// # Panics
/// Panics outside a Tokio runtime, before diagnostics are installed or startup
/// begins. Blocking installation cannot be preempted.
///
/// ```compile_fail
/// use batter_core::service::{self, NoDiagnostics};
/// let owner = service::start(async {}, NoDiagnostics);
/// ```
pub fn start<F, E, D>(startup: ScopedStartup<F>, diagnostics: D) -> ServiceOwner<E, D::Report>
where
    F: Initializer<E, Output = ()>,
    E: Send + Sync + 'static,
    D: Diagnostics,
{
    let StartingService { owner, output } = launch(startup, diagnostics, None);
    // Unit-output compatibility path has nothing to deliver to the caller.
    drop(output);
    owner
}

pub(super) fn launch<F, E, D>(
    startup: ScopedStartup<F>,
    diagnostics: D,
    deadline: Option<Instant>,
) -> StartingService<E, D::Report, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
    D: Diagnostics,
{
    tokio::runtime::Handle::try_current().expect("service startup requires a Tokio runtime");
    let handle = startup.shutdown_handle();
    let status = handle.status();
    if let Some(deadline) = deadline {
        handle.readiness_deadline(deadline);
    }
    let deadline_handle = handle.clone();
    let service_status = status.clone();
    let (output_tx, output) = oneshot::channel();
    let (settle, settled) = watch::channel(None);
    let diagnostic = catch_unwind(AssertUnwindSafe(|| {
        diagnostics.install(DiagnosticCompletion(settled))
    }))
    .map(|future| tokio::spawn(scoped_dispatch::scope(future.in_current_span())))
    .map_err(PanicPayload::new);
    let starting = startup.start();
    let service = tokio::spawn(scoped_dispatch::scope(
        async move {
            let run = async move {
                match starting.finish_with_output().await {
                    Err(error) => (ServiceOutcome::StartupFailed(error), None),
                    Ok((running, value)) => {
                        let value = ScopedOutput::capture(value);
                        let discarded =
                            if readiness_result(service_status.wait_startup().await).is_ok() {
                                output_tx.send(value).err()
                            } else {
                                Some(value)
                            };
                        // Destruction of rejected output cannot bypass process settlement.
                        let output_failure = catch_unwind(AssertUnwindSafe(|| drop(discarded)))
                            .err()
                            .map(PanicPayload::new);
                        (
                            ServiceOutcome::Shutdown(running.wait_checked().await),
                            output_failure,
                        )
                    }
                }
            };
            tokio::pin!(run);
            let expiry = async move {
                if let Some(deadline) = deadline {
                    tokio::time::sleep_until(deadline).await;
                    deadline_handle.expire_readiness();
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::select! {
                biased;
                _ = expiry => run.await,
                outcome = &mut run => outcome,
            }
        }
        .in_current_span(),
    ));
    let (publication, completion) = watch::channel(None);
    // The monitor retains the service result outside the diagnostic task. Even
    // a panic during final export cannot unwind through and destroy it.
    let report_status = status.clone();
    drop(tokio::spawn(scoped_dispatch::scope(
        async move {
            let (service, startup_output_failure) = service
                .await
                .unwrap_or_else(|error| (ServiceOutcome::Coordinator(Arc::new(error)), None));
            settle.send_replace(Some(coverage(&service)));
            let diagnostics = match diagnostic {
                Err(payload) => DiagnosticOutcome::InstallationPanicked(payload),
                Ok(task) => match task.await {
                    Ok(report) => DiagnosticOutcome::Completed(report),
                    Err(error) => DiagnosticOutcome::TaskFailed(error),
                },
            };
            publication.send_replace(Some(ServiceCompletion(Arc::new(Completed {
                readiness: readiness_result(report_status.startup_progress()),
                startup_output_failure,
                service,
                diagnostics,
            }))));
        }
        .in_current_span(),
    )));
    StartingService {
        owner: ServiceOwner {
            handle,
            observer: ServiceObserver { completion, status },
        },
        output,
    }
}
