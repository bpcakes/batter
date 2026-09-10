//! Owned initialization, explicit resource registration and running-driver handoff.
//!
//! Construction is inert. [`Startup::start`] launches a coordinator that retains
//! the supervisor while the initializer runs and through failed-startup cleanup.
//! Borrowed waiter cancellation leaves that owner alone; dropping the startup
//! handle requests drain. No runtime-death, asynchronous Drop or detached-child
//! cleanup guarantee is implied. Acquisitions remain native futures.
//!
//! Successful initialization hands off to a running service; it does not mean
//! that a finite command has finished. A supervisor with neither critical
//! components nor finite-work capacity reports
//! [`crate::lifecycle::ShutdownCause::EmptySupervisor`], even if cleanup succeeds.
//! For a standalone command, run bounded work and then explicitly await
//! [`crate::cleanup::CleanupStack::close`], retaining both outcomes. Dropping
//! that command or its cleanup future can abandon finalization. Configuring
//! finite-work capacity supports component-free process supervision, but does
//! not turn this initializer into an independently owned command/cleanup scope.

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
/// This executable example initializes a service, handles a real channel request,
/// then shuts down. Its capacity permit stays owned until the service has joined.
/// Application-selected budgets and the supervisor are constructed here, rather
/// than supplied by a hidden caller. See the `finite_command` example for work
/// that finishes without starting a service.
///
/// ```
/// use batter::{
///     BoxError, cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor},
///     operation::OperationContext, startup::Startup,
/// };
/// use std::{sync::Arc, time::Duration};
/// use tokio::sync::{mpsc, oneshot, Semaphore};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), BoxError> {
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let supervisor = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
/// let context = OperationContext::new(second)?;
/// let request_context = OperationContext::new(second)?;
/// let capacity = Arc::new(Semaphore::new(1));
/// let acquiring = capacity.clone();
/// let (requests, mut inbox) = mpsc::channel::<(u32, oneshot::Sender<u32>)>(1);
/// let mut starting = Startup::new(supervisor, context, cleanup, move |scope| Box::pin(async move {
///     scope.stage("service.capacity")?;
///     let slot = scope.supervisor().reserve_cleanup("service.capacity")?;
///     let permit = acquiring.acquire_owned().await.expect("capacity remains open");
///     slot.register(move || async move { drop(permit); Ok(()) });
///     scope.supervisor().register("doubler", move |shutdown| async move {
///         shutdown.mark_started(); // The initialized inbox is now owned by this task.
///         loop {
///             tokio::select! {
///                 biased;
///                 _ = shutdown.draining() => break,
///                 request = inbox.recv() => {
///                     let Some((value, reply)) = request else { break };
///                     let _ = reply.send(value.saturating_mul(2));
///                 }
///             }
///         }
///         Ok(())
///     })?;
///     Ok::<_, batter::RegistrationError>(())
/// })).start();
/// let running = starting.wait().await?;
/// // Keep requests alive until shutdown: an early critical task exit is a failure.
/// let response = request_context.run("service.request", |_| async {
///     running.handle().wait_ready().await
///         .map_err(|_| std::io::Error::other("service did not become ready"))?;
///     let (reply, response) = oneshot::channel();
///     requests.send((21, reply)).await?;
///     Ok::<_, BoxError>(response.await?)
/// }).await;
/// let shutdown = running.shutdown().await;
/// // Both outcomes remain available after teardown, including if the request failed.
/// assert_eq!(response.ok(), Some(42));
/// batter::lifecycle::check_shutdown(shutdown)?;
/// assert_eq!(capacity.available_permits(), 1);
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
