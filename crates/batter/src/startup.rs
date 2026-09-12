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
//! For a standalone command, use [`crate::command::Command`] to retain finite
//! work and its registered finalizers independently of borrowed waiters.
//! Configuring finite-work capacity supports component-free process supervision,
//! but does not turn this initializer into a finite-command owner.

mod driver;
mod report;
mod signals;

pub use driver::{StartingSupervisor, StartupObserver, StartupOutcome};
pub use report::{
    InitializationError, PanicPayload, PanicPayloadBusy, StartupCause, StartupError, StartupFailure,
};

use crate::{
    RegistrationError,
    cleanup::{CleanupBudget, CleanupSlot},
    lifecycle::Supervisor,
    operation::OperationContext,
    registration::{Registration, RegistrationTarget},
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
    approve_readiness: bool,
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
            approve_readiness: true,
            initialize,
        }
    }

    /// Hand off a successfully initialized driver without approving application
    /// readiness. Registered components may still acknowledge their own startup;
    /// admission remains [`crate::lifecycle::Readiness::Starting`] until the
    /// caller explicitly invokes [`crate::lifecycle::ShutdownHandle::mark_ready`].
    ///
    /// This is useful for a staged composition root that can run infrastructure
    /// before its business handler is available. Initialization failure retains
    /// the same owned cleanup behavior as [`Self::new`].
    ///
    /// ```
    /// use batter::{
    ///     cleanup::CleanupBudget,
    ///     lifecycle::{Readiness, ShutdownBudget, Supervisor},
    ///     operation::OperationContext,
    ///     startup::Startup,
    /// };
    /// use std::time::Duration;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter::BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let mut supervisor = Supervisor::new(ShutdownBudget::new(
    ///     second, second, second, cleanup,
    /// )?);
    /// supervisor.register("worker", |signal| async move {
    ///     signal.mark_started();
    ///     signal.draining().await;
    ///     Ok(())
    /// })?;
    /// let mut starting = Startup::new(
    ///     supervisor,
    ///     OperationContext::new(second)?,
    ///     cleanup,
    ///     |_| Box::pin(async { Ok::<_, std::io::Error>(()) }),
    /// )
    /// .without_readiness_approval()
    /// .start();
    /// let running = starting.wait().await?;
    /// assert_eq!(running.handle().readiness(), Readiness::Starting);
    /// assert!(running.handle().mark_ready());
    /// running.handle().wait_ready().await.unwrap();
    /// assert!(running.shutdown().await?.is_success());
    /// # Ok(()) }
    /// ```
    #[must_use = "the returned startup specification contains the selected approval policy"]
    pub fn without_readiness_approval(mut self) -> Self {
        self.approve_readiness = false;
        self
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

impl Startup<()> {
    /// Construct the canonical registration-constrained startup specification.
    ///
    /// The callback receives no supervisor or operation-context accessor. It can
    /// set stage metadata, reserve finalization before acquisition, and lend
    /// sealed registration authority to supported adapters. Construction is
    /// inert; [`ScopedStartup::start`] uses the same owned coordinator as legacy
    /// [`Startup::start`].
    ///
    /// ```
    /// use batter::{cleanup::CleanupBudget,
    ///     lifecycle::{ShutdownBudget, Supervisor}, operation::OperationContext,
    ///     startup::Startup};
    /// use std::time::Duration;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), batter::BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let process = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    /// let mut starting = Startup::scoped(process, OperationContext::new(second)?, cleanup,
    ///     |scope| Box::pin(async move {
    ///         scope.reserve_cleanup("dependency")?.register(|| async { Ok(()) });
    ///         scope.registration().register("worker", |shutdown| async move {
    ///             shutdown.mark_started();
    ///             shutdown.draining().await;
    ///             Ok(())
    ///         })?;
    ///         Ok::<_, batter::RegistrationError>(())
    ///     })).start();
    /// let running = starting.wait().await?;
    /// running.handle().wait_ready().await.unwrap();
    /// batter::lifecycle::check_shutdown(running.shutdown().await)?;
    /// # Ok(()) }
    /// ```
    pub fn scoped<F, E>(
        supervisor: Supervisor,
        context: OperationContext,
        cleanup: CleanupBudget,
        initialize: F,
    ) -> ScopedStartup<F>
    where
        F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E>,
    {
        ScopedStartup {
            supervisor,
            context,
            cleanup,
            approve_readiness: true,
            signals: signals::SignalPolicy::default(),
            initialize,
        }
    }
}

/// Inert protected startup specification returned by [`Startup::scoped`].
pub struct ScopedStartup<F> {
    supervisor: Supervisor,
    context: OperationContext,
    cleanup: CleanupBudget,
    approve_readiness: bool,
    signals: signals::SignalPolicy,
    initialize: F,
}

impl<F> ScopedStartup<F> {
    /// Keep readiness application-controlled after successful initialization.
    #[must_use = "the returned startup specification contains the selected approval policy"]
    pub fn without_readiness_approval(mut self) -> Self {
        self.approve_readiness = false;
        self
    }

    /// Select library-owned Unix SIGTERM/SIGINT handling for this startup.
    ///
    /// Selection is inert. [`Self::start`] validates and reserves `name`, then
    /// installs both native listeners synchronously before returning its owner.
    /// Selecting more than once is retained as
    /// [`InitializationError::SignalPolicyAlreadySelected`] during started
    /// cleanup; it is never last-wins. Dropping the unstarted specification
    /// installs nothing. Tokio's process-wide signal disposition is not restored
    /// when listeners are dropped.
    ///
    /// ```no_run
    /// # use batter::{cleanup::CleanupBudget, lifecycle::{ShutdownBudget, Supervisor}, operation::OperationContext, startup::Startup};
    /// # use std::time::Duration;
    /// # fn configured() -> Result<(), batter::BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let process = Supervisor::new(ShutdownBudget::new(second, second, second, cleanup)?);
    /// let startup = Startup::scoped(process, OperationContext::new(second)?, cleanup,
    ///     |_scope| Box::pin(async { Ok::<_, batter::RegistrationError>(()) }))
    ///     .with_unix_signals("signals");
    /// // `startup.start()` installs both listeners before returning its owner.
    /// # drop(startup); Ok(()) }
    /// ```
    #[must_use = "the returned startup specification contains the selected Unix signal policy"]
    pub fn with_unix_signals(mut self, name: &'static str) -> Self {
        self.signals = self.signals.select(name);
        self
    }

    /// Launch protected owned startup on the current Tokio runtime.
    ///
    /// Application failures are retained as
    /// [`InitializationError::Application`] without strengthening `E`'s bounds.
    pub fn start<E>(self) -> StartingSupervisor<InitializationError<E>>
    where
        F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E> + Send + 'static,
        E: Send + Sync + 'static,
    {
        driver::start_scoped(self)
    }
}

/// Registration-constrained access while an owned initializer runs.
///
/// This scope deliberately exposes no supervisor, context, process-start, or
/// cleanup-extraction operation. Application-owned handles captured before
/// startup remain application-owned and are not restricted by this view.
///
/// ```compile_fail,E0599
/// # fn restricted(scope: &mut batter::startup::ProtectedStartupScope) {
/// let _ = scope.supervisor();
/// # }
/// ```
///
/// ```compile_fail,E0599
/// # fn restricted(scope: &mut batter::startup::ProtectedStartupScope) {
/// let _replacement = scope.start();
/// # }
/// ```
pub struct ProtectedStartupScope {
    supervisor: Supervisor,
    stage: &'static str,
}

impl ProtectedStartupScope {
    /// Set redacted application stage metadata after validating its name.
    pub fn stage(&mut self, stage: &'static str) -> Result<(), RegistrationError> {
        crate::validation::name(stage)?;
        self.stage = stage;
        Ok(())
    }

    /// Reserve finalizer ownership directly so the slot can cross an acquisition await.
    pub fn reserve_cleanup(
        &mut self,
        name: &'static str,
    ) -> Result<CleanupSlot<'_>, RegistrationError> {
        self.supervisor.reserve_cleanup(name)
    }

    /// Borrow registration-only authority for ordinary or native components.
    pub fn registration(&mut self) -> Registration<'_> {
        Registration::new(&mut self.supervisor)
    }
}

impl crate::registration::private::Sealed for ProtectedStartupScope {}
impl RegistrationTarget for ProtectedStartupScope {
    fn registration(&mut self) -> Registration<'_> {
        ProtectedStartupScope::registration(self)
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
