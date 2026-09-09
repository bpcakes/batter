use super::{
    PanicPayload, Startup, StartupCause, StartupError, StartupFailure, StartupFuture, StartupScope,
};
use crate::{
    lifecycle::{Readiness, RunningSupervisor, ShutdownHandle, SupervisorObserver},
    operation::OperationContext,
    scoped_dispatch,
};
use std::{
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    task::Poll,
};
use tokio::sync::{oneshot, watch};
use tracing::Instrument;

/// Startup observation, with no ownership of the running service.
pub enum StartupOutcome<E> {
    /// Initialization succeeded; observe the separately owned shutdown driver.
    /// This does not imply every critical component has acknowledged readiness.
    Running(SupervisorObserver),
    /// Startup failed, with retained failure or coordinator termination.
    Failed(StartupError<E>),
}
impl<E> Clone for StartupOutcome<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Running(observer) => Self::Running(observer.clone()),
            Self::Failed(error) => Self::Failed(error.clone()),
        }
    }
}

/// Cloneable observation of owned startup, independent of handoff ownership.
pub struct StartupObserver<E> {
    completion: watch::Receiver<Option<StartupOutcome<E>>>,
}
impl<E> Clone for StartupObserver<E> {
    fn clone(&self) -> Self {
        Self {
            completion: self.completion.clone(),
        }
    }
}
impl<E> StartupObserver<E> {
    /// Await the retained outcome. Cancelling this waiter changes no ownership.
    /// Panics if the runtime destroys the monitor before publication; no cleanup
    /// report can be fabricated after runtime death. Published results survive.
    pub async fn wait(&self) -> StartupOutcome<E> {
        let mut completion = self.completion.clone();
        loop {
            if let Some(outcome) = completion.borrow_and_update().clone() {
                return outcome;
            }
            completion
                .changed()
                .await
                .expect("owned startup monitor retains publication");
        }
    }
}

/// Sole owner of initialization and its pending running-driver handoff.
/// Drop requests drain; the startup coordinator continues through cleanup on
/// the live runtime. Observers never keep this owner or a running owner alive.
pub struct StartingSupervisor<E> {
    handle: Option<ShutdownHandle>,
    receiver: oneshot::Receiver<Result<RunningSupervisor, StartupError<E>>>,
    observer: StartupObserver<E>,
}
impl<E> StartingSupervisor<E> {
    /// Clone a startup observer without prolonging service ownership.
    pub fn observer(&self) -> StartupObserver<E> {
        self.observer.clone()
    }

    /// Borrow the handoff waiter. Cancellation leaves this owner and receiver
    /// intact. Call only until its first completed result; a second completed
    /// wait panics, as with the underlying one-shot receiver.
    pub async fn wait(&mut self) -> Result<RunningSupervisor, StartupError<E>> {
        let result = (&mut self.receiver)
            .await
            .expect("owned startup monitor retains handoff");
        self.handle.take();
        result
    }
}
impl<E> Drop for StartingSupervisor<E> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.request();
        }
        // A running owner already queued in receiver is dropped after this
        // method and requests drain too; the native owned driver remains alive.
    }
}

pub(super) fn start<F, E>(startup: Startup<F>) -> StartingSupervisor<E>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
{
    let handle = startup.supervisor.handle();
    let (handoff, receiver) = oneshot::channel();
    let (publication, completion) = watch::channel(None);
    let coordinator = tokio::spawn(scoped_dispatch::scope(drive(startup).in_current_span()));
    let monitor = async move {
        let result = match coordinator.await {
            Ok(result) => result.map_err(|failure| StartupError::Failed(Arc::new(failure))),
            Err(error) => Err(StartupError::Coordinator(Arc::new(error))),
        };
        let observation = match &result {
            Ok(running) => StartupOutcome::Running(running.observer()),
            Err(error) => StartupOutcome::Failed(error.clone()),
        };
        publication.send_replace(Some(observation));
        // Failed delivery drops RunningSupervisor and requests native drain.
        drop(handoff.send(result));
    };
    drop(tokio::spawn(scoped_dispatch::scope(
        monitor.in_current_span(),
    )));
    StartingSupervisor {
        handle: Some(handle),
        receiver,
        observer: StartupObserver { completion },
    }
}

async fn drive<F, E>(startup: Startup<F>) -> Result<RunningSupervisor, StartupFailure<E>>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E>,
{
    let Startup {
        supervisor,
        context,
        cleanup,
        initialize,
    } = startup;
    let handle = supervisor.handle();
    let mut scope = StartupScope {
        supervisor,
        stage: "startup",
    };
    let (mut result, destruction_panic) =
        initialize_owned(&mut scope, &context, &handle, initialize).await;
    if result.is_ok() {
        result = check(&context, &handle);
    }
    if result.is_ok() && destruction_panic.is_none() {
        handle.mark_ready();
        return Ok(scope.supervisor.start());
    }
    let cause = result.err().unwrap_or(StartupCause::DestructionPanicked);
    handle.request();
    let pending = scope.supervisor.take_cleanup();
    drop(scope.supervisor);
    let cleanup = pending.close(cleanup).await;
    Err(StartupFailure {
        stage: scope.stage,
        cause,
        destruction_panic,
        cleanup,
    })
}

fn check<E>(context: &OperationContext, handle: &ShutdownHandle) -> Result<(), StartupCause<E>> {
    if handle.readiness() != Readiness::Starting {
        return Err(StartupCause::Draining);
    }
    context.check().map_err(StartupCause::Interrupted)
}

async fn initialize_owned<F, E>(
    scope: &mut StartupScope,
    context: &OperationContext,
    handle: &ShutdownHandle,
    initialize: F,
) -> (Result<(), StartupCause<E>>, Option<PanicPayload>)
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E>,
{
    if let Err(error) = check(context, handle) {
        return (
            Err(error),
            catch_unwind(AssertUnwindSafe(|| drop(initialize)))
                .err()
                .map(PanicPayload::new),
        );
    }
    let mut future = match catch_unwind(AssertUnwindSafe(|| initialize(scope))) {
        Ok(future) => future,
        Err(payload) => {
            return (
                Err(StartupCause::Panicked(PanicPayload::new(payload))),
                None,
            );
        }
    };
    let result = tokio::select! {
        biased;
        _ = handle.draining() => Err(StartupCause::Draining),
        _ = context.cancelled() => Err(StartupCause::Interrupted(crate::operation::Interruption::Cancelled)),
        _ = tokio::time::sleep_until(context.deadline()) => Err(StartupCause::Interrupted(crate::operation::Interruption::DeadlineExceeded)),
        result = poll_fn(|cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => poll.map(|result| result.map_err(StartupCause::Failed)),
            Err(payload) => Poll::Ready(Err(StartupCause::Panicked(PanicPayload::new(payload)))),
        }) => result,
    };
    let destruction = catch_unwind(AssertUnwindSafe(|| drop(future)))
        .err()
        .map(PanicPayload::new);
    (result, destruction)
}
