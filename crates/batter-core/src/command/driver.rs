use super::{
    Command, CommandCause, CommandFuture, CommandOutcome, CommandOwner, CommandReport,
    CommandScope, SharedCommandReport,
    signals::{NoSignals, WorkSignals},
};
use crate::{
    PanicPayload,
    cleanup::CleanupStack,
    completion::wait_published,
    operation::{Interruption, OperationContext},
    scoped_dispatch::{self, ScopedOutput},
};
use std::{
    future::{Future, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::watch;
use tracing::Instrument;

/// A command's sole work-cancellation owner. Drop requests cancellation while
/// the independently owned coordinator continues registered finalization.
/// Observers do not retain this owner. The live runtime must outlast cleanup.
/// Every observer shares one report, so the work value stays behind an `Arc`;
/// to receive it by move, use [`CommandOwner`] instead.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_core::{command::Command, cleanup::CleanupBudget, operation::OperationContext};
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
    let coordinator = tokio::spawn(scoped_dispatch::scope(
        drive(command, NoSignals).in_current_span(),
    ));
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

/// Launch with one sole owner. The coordinator's output carries its own
/// dispatcher, so a report nobody claims is destroyed under it.
pub(super) fn start_owned<F, T, E, S>(command: Command<F>, signals: S) -> CommandOwner<T, E>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E> + Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    S: WorkSignals + Send + 'static,
{
    let context = command.context.clone();
    let coordinator = tokio::spawn(scoped_dispatch::scope(
        async move { ScopedOutput::capture(drive(command, signals).await) }.in_current_span(),
    ));
    CommandOwner::new(context, coordinator)
}

async fn drive<F, T, E, S>(command: Command<F>, mut signals: S) -> CommandReport<T, E>
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
    S: WorkSignals,
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
        run_work(&mut scope, &context, work, &mut signals).await;
    // Listeners live only while work can be cancelled: they are removed after
    // work destruction and before cleanup on every path, so no signal reaches it.
    drop(signals);
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

async fn run_work<F, T, E, S>(
    scope: &mut CommandScope,
    context: &OperationContext,
    work: F,
    signals: &mut S,
) -> (
    Result<T, CommandCause<E>>,
    Option<Interruption>,
    Option<PanicPayload>,
)
where
    F: for<'a> FnOnce(&'a mut CommandScope) -> CommandFuture<'a, T, E>,
    S: WorkSignals,
{
    // A signal already visible is a cancellation request: no factory runs.
    if poll_fn(|cx| Poll::Ready(signal_visible(signals, cx))).await {
        context.cancel();
    }
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
    let cancelled = context.cancelled();
    let deadline = tokio::time::sleep_until(context.deadline());
    tokio::pin!(cancelled, deadline);
    // Biased order: cancellation, signal, deadline, then work.
    let result = poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(CommandCause::Interrupted(Interruption::Cancelled)));
        }
        if signal_visible(signals, cx) {
            context.cancel();
            return Poll::Ready(Err(CommandCause::Interrupted(Interruption::Cancelled)));
        }
        if deadline.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(CommandCause::Interrupted(
                Interruption::DeadlineExceeded,
            )));
        }
        let result = match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => poll.map(|result| result.map_err(CommandCause::Failed)),
            Err(payload) => Poll::Ready(Err(CommandCause::Panicked(PanicPayload::new(payload)))),
        };
        if result.is_ready() {
            // A signal visible when work returns is effective cancellation, as
            // a concurrent `cancel` is. Preserve the returned value/error and
            // snapshot its boundary before destruction can cancel the context
            // or cross its deadline.
            if signal_visible(signals, cx) {
                context.cancel();
            }
            interruption_after_work = context.check().err();
        }
        result
    })
    .await;
    let destruction = catch_unwind(AssertUnwindSafe(|| drop(future)))
        .err()
        .map(PanicPayload::new);
    (result, interruption_after_work, destruction)
}

/// Observe the listeners once, outside Tokio's cooperative budget. Work may have
/// spent the task's budget in this very poll, and a budget-gated `Pending` would
/// silently miss a signal that is already visible when work returns.
fn signal_visible<S: WorkSignals>(signals: &mut S, cx: &mut Context<'_>) -> bool {
    let observing = pin!(tokio::task::unconstrained(poll_fn(|cx| {
        signals.poll_received(cx)
    })));
    observing.poll(cx).is_ready()
}
