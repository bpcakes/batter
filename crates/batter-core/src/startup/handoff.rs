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
/// [`UnapprovedSupervisor`] after selecting deferred readiness approval. `T` is
/// the initializer's typed output, delivered only with that owner by
/// [`Self::finish_with_output`]; observers never receive it.
///
/// Discarding the started owner requests drain before initialization can
/// hand off, so the owner must be retained:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_core::{cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext,
///     startup::Startup};
/// fn discarded(supervisor: Supervisor, context: OperationContext, cleanup: CleanupBudget) {
///     Startup::scoped(supervisor, context, cleanup, |_| Box::pin(async {
///         Ok::<_, batter_core::RegistrationError>(())
///     })).start();
/// }
/// ```
///
/// Typed output cannot be skipped by waiting for the owner alone:
///
/// ```compile_fail,E0599
/// use batter_core::{cleanup::CleanupBudget, lifecycle::Supervisor, operation::OperationContext,
///     startup::Startup};
/// async fn skipped(supervisor: Supervisor, context: OperationContext, cleanup: CleanupBudget) {
///     let starting = Startup::protected(supervisor, context, cleanup, async |_scope| {
///         Ok::<_, std::io::Error>(8080_u16)
///     }).start();
///     let _running = starting.finish().await;
/// }
/// ```
#[must_use = "retain the startup owner; dropping it requests drain"]
pub struct StartingSupervisor<E, R = RunningSupervisor, T = ()> {
    handle: Option<ShutdownHandle>,
    receiver: oneshot::Receiver<Result<(R, T), StartupError<E>>>,
    observer: StartupObserver<E>,
}

impl<E, R, T> StartingSupervisor<E, R, T> {
    /// Clone a startup observer without prolonging service ownership.
    pub fn observer(&self) -> StartupObserver<E> {
        self.observer.clone()
    }

    /// Consume startup ownership and receive the owner with the initializer's
    /// typed output. A failed startup never yields output: output from an
    /// initializer that startup then rejected was destroyed before cleanup.
    /// The returned future owns the handoff, even before its first poll.
    /// Dropping it requests drain and destroys any queued owner and output.
    /// Retain and pin that future to resume after cancelling a borrowed wait;
    /// like any future, it must not be polled again after completion.
    ///
    /// Migration: `StartingSupervisor::wait_with_output(&mut self)` is replaced
    /// by this consuming method. A deprecated borrowed shim is omitted because
    /// it would restore the repeatable-completed-handoff trap. To resume after
    /// a timeout, keep one pinned future instead of calling the method again:
    ///
    /// ```no_run
    /// # use batter_core::startup::StartingSupervisor;
    /// # async fn example(starting: StartingSupervisor<std::io::Error,
    /// #     batter_core::lifecycle::RunningSupervisor, u16>) {
    /// let finish = starting.finish_with_output();
    /// tokio::pin!(finish);
    /// let result = match tokio::time::timeout(
    ///     std::time::Duration::from_secs(1), &mut finish,
    /// ).await {
    ///     Ok(result) => result,
    ///     Err(_) => finish.await,
    /// };
    /// let (running, port) = result.unwrap();
    /// # let _ = (running, port);
    /// # }
    /// ```
    ///
    /// One startup cannot deliver its output twice:
    ///
    /// ```compile_fail,E0382
    /// # use batter_core::startup::StartingSupervisor;
    /// async fn twice(starting: StartingSupervisor<std::io::Error,
    ///     batter_core::lifecycle::RunningSupervisor, u16>) {
    ///     let _ = starting.finish_with_output().await;
    ///     let _ = starting.finish_with_output().await;
    /// }
    /// ```
    pub async fn finish_with_output(mut self) -> Result<(R, T), StartupError<E>> {
        let result = (&mut self.receiver)
            .await
            .expect("owned startup monitor retains handoff");
        self.handle.take();
        result
    }
}

impl<E, R> StartingSupervisor<E, R> {
    /// Consume startup ownership and receive the running or unapproved owner.
    /// Available only for unit initializer output; otherwise use
    /// [`Self::finish_with_output`]. The returned future owns the handoff:
    /// dropping it requests drain, including before its first poll. Retain and
    /// pin it when cancelling and resuming a borrowed wait.
    ///
    /// Migration: `StartingSupervisor::wait(&mut self)` is replaced by
    /// `finish(self)`. No deprecated borrowed shim is retained: it would permit
    /// a second completed handoff. Obtain any observer before consuming `self`.
    ///
    /// ```no_run
    /// # use batter_core::startup::StartingSupervisor;
    /// # async fn example(starting: StartingSupervisor<std::io::Error>) {
    /// let observer = starting.observer();
    /// let running = starting.finish().await.unwrap();
    /// let report = running.shutdown().await.unwrap();
    /// assert!(report.is_success());
    /// # let _ = observer;
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0382
    /// # use batter_core::startup::StartingSupervisor;
    /// async fn twice(starting: StartingSupervisor<std::io::Error>) {
    ///     let _ = starting.finish().await;
    ///     let _ = starting.finish().await;
    /// }
    /// ```
    pub async fn finish(self) -> Result<R, StartupError<E>> {
        self.finish_with_output().await.map(|(running, ())| running)
    }
}

impl<E, R, T> Drop for StartingSupervisor<E, R, T> {
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

pub(super) fn start_driver<E, R, T>(
    handle: ShutdownHandle,
    drive: impl Future<Output = Result<(R, T), StartupFailure<E>>> + Send + 'static,
) -> StartingSupervisor<E, R, T>
where
    E: Send + Sync + 'static,
    R: StartupHandoff + Send + 'static,
    T: Send + 'static,
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
            Ok((running, _)) => StartupOutcome::Running(running.observer()),
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
