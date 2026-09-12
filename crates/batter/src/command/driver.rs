use super::{
    Command, CommandCause, CommandFuture, CommandOutcome, CommandReport, CommandScope,
    SharedCommandReport,
};
use crate::{
    PanicPayload,
    cleanup::CleanupStack,
    completion::wait_published,
    operation::{Interruption, OperationContext},
    scoped_dispatch,
};
use std::{
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    task::Poll,
};
use tokio::sync::watch;
use tracing::Instrument;

/// A command's sole work-cancellation owner. Drop requests cancellation while
/// the independently owned coordinator continues registered finalization.
/// Observers do not retain this owner. The live runtime must outlast cleanup.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
/// async fn discarded(context: OperationContext, cleanup: CleanupBudget) {
///     Command::new(context, cleanup, |_| Box::pin(async {
///         Ok::<_, std::convert::Infallible>(())
///     })).start();
/// }
/// ```
#[must_use = "retain the command owner through work, or explicitly drop it to request cancellation"]
pub struct RunningCommand<T, E> {
    context: OperationContext,
    observer: CommandObserver<T, E>,
}

impl<T, E> RunningCommand<T, E> {
    /// Request work cancellation. Repeated requests do not affect cleanup.
    pub fn cancel(&self) {
        self.context.cancel();
    }

    /// Retain completion observation without retaining work ownership.
    pub fn observer(&self) -> CommandObserver<T, E> {
        self.observer.clone()
    }

    /// Borrow a waiter. Cancelling it does not cancel work or registered cleanup;
    /// repeated waits return the same shared report and original failures.
    pub async fn wait(&self) -> CommandOutcome<T, E> {
        self.observer.wait().await
    }
}

impl<T, E> Drop for RunningCommand<T, E> {
    fn drop(&mut self) {
        self.context.cancel();
    }
}

/// Cloneable command completion observation, independent of command ownership.
pub struct CommandObserver<T, E> {
    completion: watch::Receiver<Option<CommandOutcome<T, E>>>,
}

impl<T, E> Clone for CommandObserver<T, E> {
    fn clone(&self) -> Self {
        Self {
            completion: self.completion.clone(),
        }
    }
}

impl<T, E> CommandObserver<T, E> {
    /// Await the retained report. Cancelling this waiter has no ownership effect.
    /// Panics if runtime death destroys publication before a result exists;
    /// already published results remain observable after runtime destruction.
    pub async fn wait(&self) -> CommandOutcome<T, E> {
        wait_published(self.completion.clone())
            .await
            .expect("owned command monitor retains publication")
    }
}

pub(super) fn start<F, T, E>(command: Command<F>) -> RunningCommand<T, E>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    let context = command.context.clone();
    let (publication, completion) = watch::channel(None);
    let coordinator = tokio::spawn(scoped_dispatch::scope(drive(command).in_current_span()));
    drop(tokio::spawn(scoped_dispatch::scope(
        async move {
            let outcome = coordinator
                .await
                .map(|report| SharedCommandReport(Arc::new(report)))
                .map_err(Arc::new);
            publication.send_replace(Some(outcome));
        }
        .in_current_span(),
    )));
    RunningCommand {
        context,
        observer: CommandObserver { completion },
    }
}

async fn drive<F, T, E>(command: Command<F>) -> CommandReport<T, E>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
{
    let Command {
        context,
        cleanup,
        end,
        work,
    } = command;
    let mut scope = CommandScope {
        context: context.clone(),
        cleanup: CleanupStack::new(),
        stage: "command",
    };
    let (work, interruption_after_work, destruction_panic) =
        run_work(&mut scope, &context, work).await;
    context.cancel();
    // A cleanup coordinator panic cannot unwind through and discard work evidence.
    let cleanup = tokio::spawn(scoped_dispatch::scope(
        scope.cleanup.close_before(cleanup, end).in_current_span(),
    ))
    .await
    .map_err(Arc::new);
    CommandReport {
        stage: scope.stage,
        work,
        interruption_after_work,
        destruction_panic,
        cleanup,
    }
}

async fn run_work<F, T, E>(
    scope: &mut CommandScope,
    context: &OperationContext,
    work: F,
) -> (
    Result<T, CommandCause<E>>,
    Option<Interruption>,
    Option<PanicPayload>,
)
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
{
    if let Err(reason) = context.check() {
        let destruction = catch_unwind(AssertUnwindSafe(|| drop(work)))
            .err()
            .map(PanicPayload::new);
        return (Err(CommandCause::Interrupted(reason)), None, destruction);
    }
    let mut future = match catch_unwind(AssertUnwindSafe(|| work(scope))) {
        Ok(future) => future,
        Err(payload) => {
            return (
                Err(CommandCause::Panicked(PanicPayload::new(payload))),
                context.check().err(),
                None,
            );
        }
    };
    let mut interruption_after_work = None;
    let result = tokio::select! {
        biased;
        _ = context.cancelled() => Err(CommandCause::Interrupted(Interruption::Cancelled)),
        _ = tokio::time::sleep_until(context.deadline()) => Err(CommandCause::Interrupted(Interruption::DeadlineExceeded)),
        result = poll_fn(|cx| {
            let result = match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
                Ok(poll) => poll.map(|result| result.map_err(CommandCause::Failed)),
                Err(payload) => Poll::Ready(Err(CommandCause::Panicked(PanicPayload::new(payload)))),
            };
            if result.is_ready() {
                // Preserve the returned value/error and snapshot its boundary
                // before destruction can cancel the context or cross its deadline.
                interruption_after_work = context.check().err();
            }
            result
        }) => result,
    };
    let destruction = catch_unwind(AssertUnwindSafe(|| drop(future)))
        .err()
        .map(PanicPayload::new);
    (result, interruption_after_work, destruction)
}
