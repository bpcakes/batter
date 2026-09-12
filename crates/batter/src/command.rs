//! Finite work with independently owned, bounded finalization.
//!
//! [`Command::start`] retains the work and its registered cleanup on the live
//! Tokio runtime. Dropping the command owner requests work cancellation; dropping
//! a borrowed waiter changes nothing. Returning `Err` or unwinding inside work
//! cannot skip registered finalizers. This owns the callback future, not arbitrary
//! tasks it spawns or remote effects. Use managed service adapters for runtimes
//! with descendants. Runtime death and non-yielding code cannot be repaired by Drop.

mod driver;
mod report;

pub use driver::{CommandObserver, RunningCommand};
pub use report::{
    CommandCause, CommandFailure, CommandOutcome, CommandReport, SharedCommandReport, check_command,
};

use crate::{
    ConfigurationError, RegistrationError,
    cleanup::{CleanupBudget, CleanupSlot, CleanupStack},
    operation::OperationContext,
};
use std::{future::Future, pin::Pin};
use tokio::time::Instant;

/// One allocation permits a command future to borrow its exclusive cleanup scope
/// across native acquisitions and work. Domain result and error types stay concrete.
pub type CommandFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Inert finite-work specification. No factory invocation, resource acquisition
/// or background task occurs until [`Self::start`].
///
/// ```
/// use batter::{BoxError, command::{Command, check_command}, cleanup::CleanupBudget,
///     operation::OperationContext};
/// use std::{sync::Arc, time::Duration};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), BoxError> {
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let resource = Arc::new(tokio::sync::Semaphore::new(1));
/// let acquiring = resource.clone();
/// let command = Command::new(OperationContext::new(second)?, cleanup, move |scope| {
///     Box::pin(async move {
///         let slot = scope.reserve_cleanup("capacity")?;
///         let permit = acquiring.acquire_owned().await.expect("capacity stays open");
///         slot.register(move || async move { drop(permit); Ok(()) });
///         // A ? here returns to the owner, which still finalizes the permit.
///         Ok::<_, batter::RegistrationError>(42)
///     })
/// }).start();
/// let report = check_command(command.wait().await)?;
/// assert_eq!(report.work.as_ref().ok(), Some(&42));
/// assert_eq!(resource.available_permits(), 1);
/// # Ok(()) }
/// ```
#[must_use = "start this inert command specification, or explicitly drop it"]
pub struct Command<F> {
    context: OperationContext,
    cleanup: CleanupBudget,
    end: Option<Instant>,
    work: F,
}

impl<F> Command<F> {
    /// Bound work by this context's absolute deadline and inherited cancellation.
    /// Cleanup receives its own budget afterward, independently of work cancellation.
    /// Command cancellation never travels upward into the supplied parent context.
    pub fn new<T, E>(context: OperationContext, cleanup: CleanupBudget, work: F) -> Self
    where
        F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
    {
        Self {
            context: context.scoped_child(),
            cleanup,
            end: None,
            work,
        }
    }

    /// Reserve the complete cleanup allowance inside an enclosing absolute total.
    /// Work ends at `total.deadline() - cleanup.total_allowance()`. Cleanup is
    /// independent of cancellation but cannot extend that total deadline; late
    /// scheduling may therefore produce explicit skipped finalizers.
    /// Validation occurs before invoking the work factory.
    ///
    /// ```
    /// # use batter::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
    /// # use std::time::Duration;
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second).unwrap();
    /// let total = OperationContext::new(Duration::from_secs(5)).unwrap();
    /// let command = Command::within(total, cleanup, |_| Box::pin(async {
    ///     Ok::<_, std::convert::Infallible>(())
    /// })).unwrap();
    /// // The specification is inert until start; dropping it runs no work.
    /// drop(command);
    /// ```
    pub fn within<T, E>(
        total: OperationContext,
        cleanup: CleanupBudget,
        work: F,
    ) -> Result<Self, ConfigurationError>
    where
        F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
    {
        let phases = total.reserve_finalization(cleanup.total_allowance())?;
        Ok(Self {
            context: phases.work().clone(),
            cleanup,
            end: Some(total.deadline()),
            work,
        })
    }

    /// Launch work and its retained cleanup coordinator on the current runtime.
    /// Outputs must be shareable because all observers retain the same report.
    pub fn start<T, E>(self) -> RunningCommand<T, E>
    where
        F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
        T: Send + Sync + 'static,
        E: Send + Sync + 'static,
    {
        driver::start(self)
    }
}

/// Work context and exclusive cleanup registration. The stack cannot be taken
/// out of this owner; reserve names before acquisition and register immediately
/// afterward, before another await. No resource registry or task spawner is hidden here.
pub struct CommandScope {
    context: OperationContext,
    cleanup: CleanupStack,
    stage: &'static str,
}

impl CommandScope {
    /// Absolute work deadline and downward cancellation for child operations.
    /// These tokens must not be captured by cleanup hooks: work completion cancels
    /// them before independently driven finalization.
    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// Attach a validated static stage to the completed report.
    pub fn stage(&mut self, stage: &'static str) -> Result<(), RegistrationError> {
        crate::validation::name(stage)?;
        self.stage = stage;
        Ok(())
    }

    /// Validate and reserve before native acquisition. Register the acquired
    /// resource in the slot immediately after success. Cancellation of acquisition
    /// retains the native acquisition contract; no async Drop guarantee is implied.
    pub fn reserve_cleanup(
        &mut self,
        name: &'static str,
    ) -> Result<CleanupSlot<'_>, RegistrationError> {
        self.cleanup.reserve(name)
    }
}
