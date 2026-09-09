//! Owned initialization, explicit resource registration and running-driver handoff.
//!
//! Construction is inert. [`Startup::start`] launches a coordinator that retains
//! the supervisor while the initializer runs and through failed-startup cleanup.
//! Borrowed waiter cancellation leaves that owner alone; dropping the startup
//! handle requests drain. No runtime-death, asynchronous Drop or detached-child
//! cleanup guarantee is implied. Acquisitions remain native futures.

mod driver;
mod report;

pub use driver::{StartingSupervisor, StartupObserver, StartupOutcome};
pub use report::{PanicPayload, StartupCause, StartupError, StartupFailure};

use crate::{
    RegistrationError, cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext,
};
use std::{future::Future, pin::Pin};

/// A native initializer future borrowing its startup scope.
/// One box permits that borrow across awaits without a shared resource registry.
pub type StartupFuture<'a, E> = Pin<Box<dyn Future<Output = Result<(), E>> + Send + 'a>>;

/// Inert startup specification. Dropping it abandons the unstarted supervisor
/// without invoking initialization or asynchronous cleanup.
///
/// ```no_run
/// use batter::{startup::Startup, lifecycle::Supervisor, operation::OperationContext};
/// # async fn example(supervisor: Supervisor, context: OperationContext, budget: batter::cleanup::CleanupBudget) -> Result<(), Box<dyn std::error::Error>> {
/// let mut starting = Startup::new(supervisor, context, budget, |scope| Box::pin(async move {
///     scope.stage("resource")?;
///     let slot = scope.supervisor().reserve_cleanup("resource")?;
///     let resource = String::from("acquired"); // Await native acquisition here.
///     slot.register(move || async move { drop(resource); Ok(()) });
///     Ok::<_, batter::RegistrationError>(())
/// })).start();
/// let running = starting.wait().await?;
/// let report = running.shutdown().await?;
/// assert!(report.is_success());
/// # Ok(()) }
/// ```
pub struct Startup<F> {
    supervisor: Supervisor,
    context: OperationContext,
    cleanup: CleanupBudget,
    initialize: F,
}

impl<F> Startup<F> {
    /// Construct without invoking the initializer or spawning work.
    /// The context bounds initialization; cleanup uses its separate explicit budget.
    pub fn new<E>(
        supervisor: Supervisor,
        context: OperationContext,
        cleanup: CleanupBudget,
        initialize: F,
    ) -> Self
    where
        F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E>,
    {
        Self {
            supervisor,
            context,
            cleanup,
            initialize,
        }
    }

    /// Launch owned startup on the current Tokio runtime.
    /// Captured values must be owned and Send; failures remain concrete E.
    /// The runtime must outlive initialization, handoff and awaited cleanup.
    pub fn start<E>(self) -> StartingSupervisor<E>
    where
        F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E> + Send + 'static,
        E: Send + Sync + 'static,
    {
        driver::start(self)
    }
}

/// Exclusive access to the supervisor while its owned initializer runs.
pub struct StartupScope {
    supervisor: Supervisor,
    stage: &'static str,
}

impl StartupScope {
    /// Set application-owned stage metadata using the component-name contract.
    /// Error diagnostics expose this validated static name, never cause contents.
    pub fn stage(&mut self, stage: &'static str) -> Result<(), RegistrationError> {
        crate::validation::name(stage)?;
        self.stage = stage;
        Ok(())
    }

    /// Configure the unstarted supervisor using its existing native APIs.
    ///
    /// Reserve cleanup names before acquisition and register immediately after
    /// success. Resources or cleanup explicitly extracted/replaced here leave
    /// this owner's contract. Do not start a replacement driver from this borrow.
    pub fn supervisor(&mut self) -> &mut Supervisor {
        &mut self.supervisor
    }
}
