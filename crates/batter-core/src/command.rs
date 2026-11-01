//! Finite work with independently owned, bounded finalization.
//!
//! Starting a [`Command`] retains the work and its registered cleanup on the live
//! Tokio runtime. An executable root calls [`Command::start_with_unix_signals`],
//! so SIGTERM and SIGINT cancel work and still finalize; reusable helpers and
//! in-process tests call [`Command::start_without_signals`]. Both return the sole
//! [`CommandOwner`], whose consuming [`CommandOwner::finish`] checks the complete
//! contract and moves the work value out. [`Command::start`] instead publishes one
//! shared report to every [`CommandObserver`], checked with [`check_command`].
//!
//! Dropping the command owner requests work cancellation; dropping a borrowed
//! waiter changes nothing. Returning `Err` or unwinding inside work cannot skip
//! registered finalizers. This owns the callback future, not arbitrary tasks it
//! spawns or remote effects. Use managed service adapters for runtimes with
//! descendants. Runtime death and non-yielding code cannot be repaired by Drop.

mod driver;
mod owner;
mod report;
mod signals;
#[cfg(test)]
mod tests;

pub use driver::{CommandObserver, RunningCommand};
pub use owner::CommandOwner;
pub use report::{
    CommandCause, CommandFailure, CommandOutcome, CommandReport, CommandSuccess,
    SharedCommandReport, check_command,
};

use crate::{
    ConfigurationError, RegistrationError,
    cleanup::{CleanupBudget, CleanupSlot, CleanupStack},
    operation::OperationContext,
};
use std::{future::Future, io, pin::Pin};
use tokio::time::Instant;

/// One allocation permits a command future to borrow its exclusive cleanup scope
/// across native acquisitions and work. Domain result and error types stay concrete.
pub type CommandFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Inert finite-work specification. No factory invocation, resource acquisition
/// or background task occurs until it is started with
/// [`Self::start_with_unix_signals`], [`Self::start_without_signals`] or
/// [`Self::start`].
///
/// ```
/// use batter_core::{BoxError, RegistrationError, cleanup::CleanupBudget, command::Command,
///     operation::OperationOwner};
/// use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), BoxError> {
/// // No Clone derive: the checked value is moved out, never copied.
/// struct Summary { rows: u32 }
/// let second = Duration::from_secs(1);
/// let cleanup = CleanupBudget::new(second, second, second)?;
/// let released = Arc::new(AtomicBool::new(false));
/// let releasing = released.clone();
/// let command = Command::new(OperationOwner::new(second)?.into_context(), cleanup, move |scope| {
///     Box::pin(async move {
///         // Reserve before acquisition, register immediately afterward.
///         scope.reserve_cleanup("summary.source")?.register(move || async move {
///             releasing.store(true, Ordering::SeqCst);
///             Ok(())
///         });
///         // A ? here returns to the owner, which still runs the finalizer.
///         Ok::<_, RegistrationError>(Summary { rows: 3 })
///     })
/// });
/// // An executable root calls `start_with_unix_signals()?` here instead.
/// let success = command.start_without_signals().finish().await?;
/// assert!(released.load(Ordering::SeqCst));
/// assert_eq!(success.into_value().rows, 3);
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
    /// # use batter_core::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
    /// # use std::time::Duration;
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second).unwrap();
    /// let total = batter_core::operation::OperationOwner::new(Duration::from_secs(5)).unwrap().into_context();
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

    /// Install SIGTERM and SIGINT listeners owned by this command, then launch
    /// it with the sole [`CommandOwner`], whose consuming
    /// [`CommandOwner::finish`] moves the checked work value out. Use this at an
    /// executable root, so an operator's Ctrl-C or a service manager's SIGTERM
    /// cancels work and still runs registered cleanup.
    ///
    /// Both listeners are installed synchronously, SIGTERM then SIGINT, before
    /// this returns. Only the command coordinator polls them, and only while work
    /// can still be cancelled. Either signal cancels this command's own work
    /// context and its descendants, exactly as [`CommandOwner::cancel`] does,
    /// never the supplied parent: the report records
    /// `CommandCause::Interrupted(Interruption::Cancelled)`, or, when work had
    /// just returned, a late `interruption_after_work`. A signal visible before
    /// the work factory runs prevents its invocation. Registered cleanup then runs
    /// under its own budget. The listeners are dropped as soon as work stops,
    /// before cleanup, on every path. The signal's identity is not recorded.
    ///
    /// If the context is already cancelled or expired, nothing is installed and
    /// the report records that interruption without invoking the factory.
    ///
    /// Limits: Tokio replaces each signal's process-wide disposition at
    /// installation and never restores it. After work stops, during cleanup,
    /// after `finish` and between two commands, SIGTERM and SIGINT are consumed
    /// without effect; a later phase that must stop on a signal needs its own
    /// signal policy. A signal arriving between two signal-owning commands may or
    /// may not cancel the second before its work starts, depending on when the
    /// runtime processes it. Reception is cooperative, not a fence at kernel
    /// delivery: a signal becomes visible on the runtime's next signal-driver
    /// turn, and non-yielding work is not preempted. Select signals only at the
    /// executable root, never in reusable helpers or in-process tests.
    ///
    /// # Errors
    ///
    /// The native installation error. Nothing was spawned and the work factory
    /// was destroyed uninvoked before this returned. If SIGINT installation
    /// fails after SIGTERM succeeded, SIGTERM's disposition stays replaced.
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime with its I/O (signal) driver enabled, as Tokio
    /// does. A panic from destroying the factory after an installation error
    /// propagates. The coordinator also needs the runtime's time driver; without
    /// it the coordinator panics and `finish` returns
    /// [`CommandFailure::Coordinator`], with no cleanup claim.
    ///
    /// ```no_run
    /// use batter_core::{BoxError, RegistrationError, cleanup::CleanupBudget,
    ///     command::{Command, CommandFailure}, operation::OperationOwner};
    /// use std::{process::ExitCode, time::Duration};
    /// #[tokio::main(flavor = "current_thread")]
    /// async fn main() -> Result<ExitCode, BoxError> {
    ///     let second = Duration::from_secs(1);
    ///     let cleanup = CleanupBudget::new(second, second, second)?;
    ///     let context = OperationOwner::new(Duration::from_secs(30))?.into_context();
    ///     let owner = Command::new(context, cleanup, |scope| Box::pin(async move {
    ///         scope.stage("export")?;
    ///         Ok::<_, RegistrationError>(String::from("exported"))
    ///     }))
    ///     .start_with_unix_signals()?;
    ///     match owner.finish().await {
    ///         Ok(success) => {
    ///             println!("{}", success.into_value());
    ///             Ok(ExitCode::SUCCESS)
    ///         }
    ///         // Work, interruption and cleanup outcomes remain in the report.
    ///         Err(CommandFailure::Report(report)) => {
    ///             eprintln!("{report:?}");
    ///             Ok(ExitCode::FAILURE)
    ///         }
    ///         Err(CommandFailure::Coordinator(_)) => Ok(ExitCode::FAILURE),
    ///     }
    /// }
    /// ```
    pub fn start_with_unix_signals<T, E>(self) -> io::Result<CommandOwner<T, E>>
    where
        F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        signals::start_with(self, signals::UnixStopSignals::install)
    }

    /// Launch work and its retained cleanup with the sole [`CommandOwner`],
    /// installing no signal listeners: SIGTERM and SIGINT keep the process's
    /// existing disposition. Use this in reusable helpers and in-process tests;
    /// an executable root that must finalize on a signal uses
    /// [`Self::start_with_unix_signals`].
    ///
    /// Do not start a second command inside another command's work. If the
    /// outer work is cancelled or expires while it awaits the inner `finish`,
    /// the inner owner is dropped and its cleanup runs detached: nothing joins
    /// or reports it, and leaving the root can abandon it. Code running inside
    /// command work takes the `&mut CommandScope` and registers its resources
    /// there; a helper that is not the root returns an inert [`Command`].
    ///
    /// The value is never shared, so `T` and `E` need only `Send + 'static`:
    /// no `Clone` or `Sync` bound applies. [`CommandFailure`] converts into
    /// [`crate::BoxError`] with `?` only when `T: Send + Sync` and
    /// `E: Error + Send + Sync`; otherwise match it. Requires a live Tokio
    /// runtime with its time driver enabled. See [`Command`] for an example.
    pub fn start_without_signals<T, E>(self) -> CommandOwner<T, E>
    where
        F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        driver::start_owned(self, signals::NoSignals)
    }

    /// Launch work and its retained cleanup coordinator on the current runtime.
    /// Outputs must be shareable because all observers retain the same report.
    /// To receive the work value by move, without `Clone`, use
    /// [`Self::start_with_unix_signals`] or [`Self::start_without_signals`].
    ///
    /// ```
    /// use batter_core::{BoxError, command::{Command, check_command}, cleanup::CleanupBudget,
    ///     operation::OperationContext};
    /// use std::{sync::Arc, time::Duration};
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), BoxError> {
    /// let second = Duration::from_secs(1);
    /// let cleanup = CleanupBudget::new(second, second, second)?;
    /// let resource = Arc::new(tokio::sync::Semaphore::new(1));
    /// let acquiring = resource.clone();
    /// let command = Command::new(batter_core::operation::OperationOwner::new(second)?.into_context(), cleanup, move |scope| {
    ///     Box::pin(async move {
    ///         let slot = scope.reserve_cleanup("capacity")?;
    ///         let permit = acquiring.acquire_owned().await.expect("capacity stays open");
    ///         slot.register(move || async move { drop(permit); Ok(()) });
    ///         // A ? here returns to the owner, which still finalizes the permit.
    ///         Ok::<_, batter_core::RegistrationError>(42)
    ///     })
    /// }).start();
    /// let report = check_command(command.wait().await)?;
    /// assert_eq!(report.work.as_ref().ok(), Some(&42));
    /// assert_eq!(resource.available_permits(), 1);
    /// # Ok(()) }
    /// ```
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
    /// Request cancellation of this command's work, independently of cleanup.
    pub fn cancel_work(&self) {
        self.context.cancel();
    }

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
