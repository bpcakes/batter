use super::{
    DiagnosticCompletion, DiagnosticOutcome, Diagnostics, ServiceCompletion, ServiceOutcome,
    coverage::coverage, report::Completed,
};
use crate::{
    PanicPayload,
    completion::wait_published,
    lifecycle::ShutdownHandle,
    scoped_dispatch,
    startup::{ProtectedStartupScope, ScopedStartup, StartupFuture},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use tokio::sync::watch;
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
}

impl<E, R> Clone for ServiceObserver<E, R> {
    fn clone(&self) -> Self {
        Self {
            completion: self.completion.clone(),
        }
    }
}

impl<E, R> ServiceObserver<E, R> {
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
/// See the module example for owner/observer composition.
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
    F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
    D: Diagnostics,
{
    tokio::runtime::Handle::try_current().expect("service startup requires a Tokio runtime");
    let handle = startup.shutdown_handle();
    let (settle, settled) = watch::channel(None);
    let diagnostic = catch_unwind(AssertUnwindSafe(|| {
        diagnostics.install(DiagnosticCompletion(settled))
    }))
    .map(|future| tokio::spawn(scoped_dispatch::scope(future.in_current_span())))
    .map_err(PanicPayload::new);
    let mut starting = startup.start();
    let service = tokio::spawn(scoped_dispatch::scope(
        async move {
            match starting.wait().await {
                Err(error) => ServiceOutcome::StartupFailed(error),
                Ok(running) => ServiceOutcome::Shutdown(running.wait_checked().await),
            }
        }
        .in_current_span(),
    ));
    let (publication, completion) = watch::channel(None);
    // The monitor retains the service result outside the diagnostic task. Even
    // a panic during final export cannot unwind through and destroy it.
    drop(tokio::spawn(scoped_dispatch::scope(
        async move {
            let service = service
                .await
                .unwrap_or_else(|error| ServiceOutcome::Coordinator(Arc::new(error)));
            settle.send_replace(Some(coverage(&service)));
            let diagnostics = match diagnostic {
                Err(payload) => DiagnosticOutcome::InstallationPanicked(payload),
                Ok(task) => match task.await {
                    Ok(report) => DiagnosticOutcome::Completed(report),
                    Err(error) => DiagnosticOutcome::TaskFailed(error),
                },
            };
            publication.send_replace(Some(ServiceCompletion(Arc::new(Completed {
                service,
                diagnostics,
            }))));
        }
        .in_current_span(),
    )));
    ServiceOwner {
        handle,
        observer: ServiceObserver { completion },
    }
}
