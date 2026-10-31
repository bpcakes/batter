use super::{CommandFailure, CommandReport, CommandSuccess};
use crate::{operation::OperationContext, scoped_dispatch::ScopedOutput};
use std::sync::Arc;
use tokio::task::JoinHandle;

/// The sole owner of a started command and the only recipient of its work
/// value. Obtain it from [`super::Command::start_with_unix_signals`] at an
/// executable root, or [`super::Command::start_without_signals`] elsewhere.
///
/// It has no observer and no borrowed waiter: [`Self::finish`] consumes it and
/// checks the complete command contract before moving the value out. Dropping
/// the owner, or an unfinished `finish` future, requests work cancellation,
/// even before the coordinator first runs, in which case the work factory is
/// never invoked. The coordinator still runs registered cleanup on the live
/// runtime; the unclaimed report is then destroyed under the coordinator's
/// tracing dispatcher. Use [`super::Command::start`] when other tasks must
/// observe the same report.
///
/// Nothing can join an owner dropped this way, so do not await one command
/// inside another command's work: an interrupted outer command drops the inner
/// owner and leaves its cleanup unjoined. Register resources on the outer
/// [`super::CommandScope`] instead.
///
/// Discarding the owner requests cancellation at once, so it must be kept:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_core::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
/// async fn discarded(context: OperationContext, cleanup: CleanupBudget) {
///     Command::new(context, cleanup, |_| Box::pin(async {
///         Ok::<_, std::convert::Infallible>(())
///     })).start_without_signals();
/// }
/// ```
///
/// It cannot hand out shared observation of a value it alone receives:
///
/// ```compile_fail,E0599
/// use batter_core::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
/// async fn observed(context: OperationContext, cleanup: CleanupBudget) {
///     let owner = Command::new(context, cleanup, |_| Box::pin(async {
///         Ok::<_, std::convert::Infallible>(())
///     })).start_without_signals();
///     let _ = owner.observer();
/// }
/// ```
#[must_use = "finish the command owner to receive its checked value; dropping it requests work cancellation"]
pub struct CommandOwner<T, E> {
    context: OperationContext,
    coordinator: JoinHandle<ScopedOutput<CommandReport<T, E>>>,
}

impl<T, E> CommandOwner<T, E> {
    pub(super) fn new(
        context: OperationContext,
        coordinator: JoinHandle<ScopedOutput<CommandReport<T, E>>>,
    ) -> Self {
        Self {
            context,
            coordinator,
        }
    }

    /// Request work cancellation before calling [`Self::finish`]. Cleanup and
    /// the parent context are unaffected; repeated requests are harmless.
    /// `finish` consumes the owner, so to cancel while it is pending, keep the
    /// parent [`crate::operation::OperationOwner`] and cancel that instead.
    pub fn cancel(&self) {
        self.context.cancel();
    }

    /// Await the command, then check the same contract as
    /// [`CommandReport::is_success`] and move the work value out.
    ///
    /// Returns [`CommandSuccess`] only when work returned `Ok`, no interruption
    /// was effective when it returned, its future was destroyed without a panic
    /// and every finalizer succeeded. Otherwise the complete report, with the
    /// original work error, panics and every cleanup error, is returned in
    /// [`CommandFailure::Report`]; a completed value there is reachable only by
    /// reference. A coordinator that ended without a report, including on
    /// runtime shutdown, is [`CommandFailure::Coordinator`]. This never panics.
    ///
    /// This is not a borrowed waiter: unlike [`super::RunningCommand::wait`],
    /// dropping this future drops the owner and requests work cancellation.
    /// The work deadline and cleanup budget already bound it. Do not wrap it in
    /// a timeout and then leave the root: runtime shutdown would abandon cleanup.
    ///
    /// The value is delivered once:
    ///
    /// ```compile_fail,E0382
    /// # use batter_core::command::CommandOwner;
    /// async fn twice(owner: CommandOwner<u8, std::io::Error>) {
    ///     let _ = owner.finish().await;
    ///     let _ = owner.finish().await;
    /// }
    /// ```
    ///
    /// and a failure never yields it by move:
    ///
    /// ```compile_fail,E0507
    /// # use batter_core::command::{CommandFailure, CommandOwner};
    /// async fn moved(owner: CommandOwner<String, std::io::Error>) -> Option<String> {
    ///     match owner.finish().await {
    ///         Ok(success) => Some(success.into_value()),
    ///         Err(CommandFailure::Report(report)) => report.work.ok(),
    ///         Err(CommandFailure::Coordinator(_)) => None,
    ///     }
    /// }
    /// ```
    pub async fn finish(mut self) -> Result<CommandSuccess<T>, CommandFailure<T, E>> {
        match (&mut self.coordinator).await {
            Ok(report) => report.into_inner().into_checked(),
            Err(error) => Err(CommandFailure::Coordinator(Arc::new(error))),
        }
    }
}

impl<T, E> Drop for CommandOwner<T, E> {
    fn drop(&mut self) {
        self.context.cancel();
    }
}
