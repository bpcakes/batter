use super::{StartupError, StartupFailure};
use crate::{
    completion::wait_published,
    lifecycle::{RunningSupervisor, ShutdownHandle, SupervisorObserver, UnapprovedSupervisor},
    scoped_dispatch,
};
use std::{future::Future, sync::Arc};
use tokio::sync::{oneshot, watch};
use tracing::Instrument;

/// Startup observation, with no ownership of the running service.
pub enum StartupOutcome<E> {
    /// Initialization succeeded; observe the separately owned shutdown driver.
    /// This proves neither application approval nor acknowledgement from every
    /// critical component; a deferred startup handoff can still be unapproved.
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
        wait_published(self.completion.clone())
            .await
            .expect("owned startup monitor retains publication")
    }
}

/// Sole owner of initialization and its pending driver handoff.
/// Drop requests drain; the startup coordinator continues through cleanup on
/// the live runtime. Observers never keep this owner or a running owner alive.
/// `R` is [`RunningSupervisor`] for default startup and
/// [`UnapprovedSupervisor`] after selecting deferred readiness approval.
pub struct StartingSupervisor<E, R = RunningSupervisor> {
    handle: Option<ShutdownHandle>,
    receiver: oneshot::Receiver<Result<R, StartupError<E>>>,
    observer: StartupObserver<E>,
}

impl<E, R> StartingSupervisor<E, R> {
    /// Clone a startup observer without prolonging service ownership.
    pub fn observer(&self) -> StartupObserver<E> {
        self.observer.clone()
    }

    /// Borrow the handoff waiter. Cancellation leaves this owner and receiver
    /// intact. Call only until its first completed result; a second completed
    /// wait panics, as with the underlying one-shot receiver.
    pub async fn wait(&mut self) -> Result<R, StartupError<E>> {
        let result = (&mut self.receiver)
            .await
            .expect("owned startup monitor retains handoff");
        self.handle.take();
        result
    }
}

impl<E, R> Drop for StartingSupervisor<E, R> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.request();
        }
        // A running owner already queued in receiver is dropped after this
        // method and requests drain too; the native owned driver remains alive.
    }
}

pub(super) trait StartupHandoff {
    fn observer(&self) -> SupervisorObserver;
}

impl StartupHandoff for RunningSupervisor {
    fn observer(&self) -> SupervisorObserver {
        RunningSupervisor::observer(self)
    }
}

impl StartupHandoff for UnapprovedSupervisor {
    fn observer(&self) -> SupervisorObserver {
        UnapprovedSupervisor::observer(self)
    }
}

pub(super) fn start_driver<E, R>(
    handle: ShutdownHandle,
    drive: impl Future<Output = Result<R, StartupFailure<E>>> + Send + 'static,
) -> StartingSupervisor<E, R>
where
    E: Send + Sync + 'static,
    R: StartupHandoff + Send + 'static,
{
    let (handoff, receiver) = oneshot::channel();
    let (publication, completion) = watch::channel(None);
    let coordinator = tokio::spawn(scoped_dispatch::scope(drive.in_current_span()));
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
        // Failed delivery drops the driver owner and requests native drain.
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
