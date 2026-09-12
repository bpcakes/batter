use super::{
    InitializationError, PanicPayload, ProtectedStartupScope, ScopedStartup, Startup, StartupCause,
    StartupError, StartupFailure, StartupFuture, StartupScope,
};
use crate::{
    lifecycle::{
        InstalledSignals, Readiness, RunningSupervisor, ShutdownHandle, SignalRegistrationError,
        SupervisorObserver, install_reserved_signals,
    },
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
    let Startup {
        supervisor,
        context,
        cleanup,
        approve_readiness,
        initialize,
    } = startup;
    let handle = supervisor.handle();
    start_driver(
        handle,
        drive(
            supervisor,
            context,
            cleanup,
            approve_readiness,
            OwnedInitialization {
                state: Initialization::Callback(initialize),
                signals: None,
            },
            |supervisor| StartupScope {
                supervisor,
                stage: "startup",
            },
            |error| error,
        ),
    )
}

pub(super) fn start_scoped<F, E>(
    startup: ScopedStartup<F>,
) -> StartingSupervisor<InitializationError<E>>
where
    F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
{
    start_scoped_with(startup, install_reserved_signals)
}

fn start_scoped_with<F, E>(
    startup: ScopedStartup<F>,
    install: impl FnOnce(&'static str) -> Result<InstalledSignals, SignalRegistrationError>,
) -> StartingSupervisor<InitializationError<E>>
where
    F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
{
    let ScopedStartup {
        mut supervisor,
        context,
        cleanup,
        approve_readiness,
        signals,
        initialize,
    } = startup;
    let handle = supervisor.handle();
    let (initialization, installed_signals) = match signals.into_selection() {
        Err(()) => (
            Initialization::Failed {
                cause: StartupCause::Failed(InitializationError::SignalPolicyAlreadySelected),
                unused: initialize,
            },
            None,
        ),
        Ok(None) => (Initialization::Callback(initialize), None),
        Ok(Some(name)) => match check::<InitializationError<E>>(&context, &handle) {
            Err(cause) => (
                Initialization::Failed {
                    cause,
                    unused: initialize,
                },
                None,
            ),
            Ok(()) => match supervisor.reserve_component_name(name) {
                Err(error) => (
                    Initialization::Failed {
                        cause: StartupCause::Failed(InitializationError::Signals(
                            SignalRegistrationError::Registration(error),
                        )),
                        unused: initialize,
                    },
                    None,
                ),
                Ok(()) => match install(name) {
                    Ok(signals) => (Initialization::Callback(initialize), Some(signals)),
                    Err(error) => (
                        Initialization::Failed {
                            cause: StartupCause::Failed(InitializationError::Signals(error)),
                            unused: initialize,
                        },
                        None,
                    ),
                },
            },
        },
    };
    start_driver(
        handle,
        drive(
            supervisor,
            context,
            cleanup,
            approve_readiness,
            OwnedInitialization {
                state: initialization,
                signals: installed_signals,
            },
            |supervisor| ProtectedStartupScope {
                supervisor,
                stage: "startup",
            },
            InitializationError::Application,
        ),
    )
}

enum Initialization<F, E> {
    Callback(F),
    Failed { cause: StartupCause<E>, unused: F },
}

struct OwnedInitialization<F, E> {
    state: Initialization<F, E>,
    signals: Option<InstalledSignals>,
}

fn start_driver<E>(
    handle: ShutdownHandle,
    drive: impl Future<Output = Result<RunningSupervisor, StartupFailure<E>>> + Send + 'static,
) -> StartingSupervisor<E>
where
    E: Send + Sync + 'static,
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

// Keep the full initialization/destruction/cleanup report until the monitor
// wraps it in Arc. Its native layout exceeds 128 bytes on macOS; boxing here
// would add an allocation immediately before that existing shared allocation.
#[allow(clippy::result_large_err)]
async fn drive<F, I, E, S, C, M>(
    supervisor: crate::lifecycle::Supervisor,
    context: OperationContext,
    cleanup: crate::cleanup::CleanupBudget,
    approve_readiness: bool,
    initialization: OwnedInitialization<F, E>,
    construct_scope: C,
    map_error: M,
) -> Result<RunningSupervisor, StartupFailure<E>>
where
    F: for<'a> FnOnce(&'a mut S) -> StartupFuture<'a, I>,
    S: DriverScope,
    C: FnOnce(crate::lifecycle::Supervisor) -> S,
    M: Fn(I) -> E + Copy,
{
    let OwnedInitialization {
        state: initialization,
        mut signals,
    } = initialization;
    let handle = supervisor.handle();
    let mut scope = construct_scope(supervisor);
    let (mut result, destruction_panic) = initialize_owned(
        &mut scope,
        &context,
        &handle,
        initialization,
        map_error,
        signals.as_mut(),
    )
    .await;
    if result.is_ok() && destruction_panic.is_none() {
        if let Some(signals) = signals.take() {
            signals.register_reserved(scope.supervisor_mut());
        }
        result = check(&context, &handle);
    }
    if result.is_ok() && destruction_panic.is_none() {
        if approve_readiness {
            handle.mark_ready();
        }
        let (supervisor, _) = scope.into_parts();
        return Ok(supervisor.start());
    }
    let cause = result.err().unwrap_or(StartupCause::DestructionPanicked);
    handle.request();
    let (mut supervisor, stage) = scope.into_parts();
    let pending = supervisor.take_cleanup();
    drop(supervisor);
    let cleanup = pending.close(cleanup).await;
    Err(StartupFailure {
        stage,
        cause,
        destruction_panic,
        cleanup,
    })
}

trait DriverScope {
    fn supervisor_mut(&mut self) -> &mut crate::lifecycle::Supervisor;
    fn into_parts(self) -> (crate::lifecycle::Supervisor, &'static str);
}

impl DriverScope for StartupScope {
    fn supervisor_mut(&mut self) -> &mut crate::lifecycle::Supervisor {
        &mut self.supervisor
    }
    fn into_parts(self) -> (crate::lifecycle::Supervisor, &'static str) {
        (self.supervisor, self.stage)
    }
}

impl DriverScope for ProtectedStartupScope {
    fn supervisor_mut(&mut self) -> &mut crate::lifecycle::Supervisor {
        &mut self.supervisor
    }
    fn into_parts(self) -> (crate::lifecycle::Supervisor, &'static str) {
        (self.supervisor, self.stage)
    }
}

fn check<E>(context: &OperationContext, handle: &ShutdownHandle) -> Result<(), StartupCause<E>> {
    if handle.readiness() != Readiness::Starting {
        return Err(StartupCause::Draining);
    }
    context.check().map_err(StartupCause::Interrupted)
}

async fn initialize_owned<F, I, E, S, M>(
    scope: &mut S,
    context: &OperationContext,
    handle: &ShutdownHandle,
    initialization: Initialization<F, E>,
    map_error: M,
    signals: Option<&mut InstalledSignals>,
) -> (Result<(), StartupCause<E>>, Option<PanicPayload>)
where
    F: for<'a> FnOnce(&'a mut S) -> StartupFuture<'a, I>,
    M: Fn(I) -> E + Copy,
{
    let initialize = match initialization {
        Initialization::Callback(initialize) => initialize,
        Initialization::Failed { cause, unused } => {
            return (
                Err(cause),
                catch_unwind(AssertUnwindSafe(|| drop(unused)))
                    .err()
                    .map(PanicPayload::new),
            );
        }
    };
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
        _ = receive_signal(signals) => {
            handle.request();
            Err(StartupCause::Draining)
        },
        result = poll_fn(|cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => poll.map(|result| result.map_err(|error| StartupCause::Failed(map_error(error)))),
            Err(payload) => Poll::Ready(Err(StartupCause::Panicked(PanicPayload::new(payload)))),
        }) => result,
    };
    let destruction = catch_unwind(AssertUnwindSafe(|| drop(future)))
        .err()
        .map(PanicPayload::new);
    (result, destruction)
}

async fn receive_signal(signals: Option<&mut InstalledSignals>) {
    match signals {
        Some(signals) => signals.received().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RegistrationError,
        cleanup::{CleanupBudget, CleanupOutcome},
        lifecycle::{ShutdownBudget, Supervisor},
    };
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    struct PanicOnDrop;

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("unused-initializer-drop-marker");
        }
    }

    fn budget() -> CleanupBudget {
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap()
    }

    fn supervisor() -> Supervisor {
        Supervisor::new(
            ShutdownBudget::new(
                Duration::from_secs(1),
                Duration::from_secs(1),
                Duration::from_secs(1),
                budget(),
            )
            .unwrap(),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn installed_error_survives_owner_loss_and_unused_capture_panic() {
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let cleanup_called = cleanup_calls.clone();
        let initializer_calls = Arc::new(AtomicUsize::new(0));
        let initializer_called = initializer_calls.clone();
        let install_calls = Arc::new(AtomicUsize::new(0));
        let install_called = install_calls.clone();
        let mut process = supervisor();
        process
            .reserve_cleanup("resource")
            .unwrap()
            .register(move || async move {
                cleanup_called.fetch_add(1, Ordering::SeqCst);
                Err(io::Error::other("cleanup-private-marker").into())
            });
        let bomb = PanicOnDrop;
        let startup = Startup::scoped(
            process,
            OperationContext::new(Duration::from_secs(1)).unwrap(),
            budget(),
            move |_scope| {
                let _capture = bomb;
                initializer_called.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok::<_, RegistrationError>(()) })
            },
        )
        .with_unix_signals("signals");
        let owner = start_scoped_with(startup, move |_| {
            install_called.fetch_add(1, Ordering::SeqCst);
            Err(SignalRegistrationError::Install(io::Error::other(
                "install-private-marker",
            )))
        });
        let observer = owner.observer();
        drop(owner);

        let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
            panic!("missing installed-error report")
        };
        let StartupCause::Failed(InitializationError::Signals(SignalRegistrationError::Install(
            error,
        ))) = &report.cause
        else {
            panic!("wrong startup cause")
        };
        assert_eq!(error.to_string(), "install-private-marker");
        report
            .destruction_panic
            .as_ref()
            .unwrap()
            .try_inspect(|payload| {
                assert_eq!(
                    payload.downcast_ref::<&str>(),
                    Some(&"unused-initializer-drop-marker")
                );
            })
            .unwrap();
        assert_eq!(install_calls.load(Ordering::SeqCst), 1);
        assert_eq!(initializer_calls.load(Ordering::SeqCst), 0);
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(report.cleanup.records.len(), 1);
        assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Failed);
        let formatted = format!("{report:?} {report}");
        assert!(!formatted.contains("install-private-marker"));
        assert!(!formatted.contains("cleanup-private-marker"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn preexisting_cancellation_skips_injected_installation() {
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        context.cancel();
        let install_calls = Arc::new(AtomicUsize::new(0));
        let install_called = install_calls.clone();
        let startup = Startup::scoped(supervisor(), context, budget(), |_scope| {
            Box::pin(async { Ok::<_, RegistrationError>(()) })
        })
        .with_unix_signals("signals");
        let mut owner = start_scoped_with(startup, move |_| {
            install_called.fetch_add(1, Ordering::SeqCst);
            Err(SignalRegistrationError::Install(io::Error::other(
                "must-not-install",
            )))
        });
        let Err(StartupError::Failed(report)) = owner.wait().await else {
            panic!("missing interrupted report")
        };
        assert!(matches!(
            report.cause,
            StartupCause::Interrupted(crate::operation::Interruption::Cancelled)
        ));
        assert_eq!(install_calls.load(Ordering::SeqCst), 0);
    }
}
