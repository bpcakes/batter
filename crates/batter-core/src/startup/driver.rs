use super::{
    DeferredScopedStartup, DeferredStartup, InitializationError, Initializer, PanicPayload,
    ProtectedStartupScope, ScopedStartup, Startup, StartupCause, StartupFailure, StartupFuture,
    StartupScope,
    handoff::{StartingSupervisor, StartupHandoff, start_driver},
    initializer::Initialize,
};
use crate::{
    lifecycle::{
        InstalledSignals, Readiness, RunningSupervisor, ShutdownHandle, SignalRegistrationError,
        UnapprovedSupervisor, install_reserved_signals,
    },
    operation::OperationContext,
};
use std::{
    future::{Future, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    task::{Context, Poll},
};

pub(super) fn start<F, E>(startup: Startup<F>) -> StartingSupervisor<E>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
{
    start_with(startup, UnapprovedSupervisor::approve_readiness)
}

pub(super) fn start_deferred<F, E>(
    startup: DeferredStartup<F>,
) -> StartingSupervisor<E, UnapprovedSupervisor>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
{
    start_with(startup.startup, std::convert::identity)
}

fn start_with<F, E, R>(
    startup: Startup<F>,
    handoff: fn(UnapprovedSupervisor) -> R,
) -> StartingSupervisor<E, R>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, E> + Send + 'static,
    E: Send + Sync + 'static,
    R: StartupHandoff + Send + 'static,
{
    let Startup {
        supervisor,
        context,
        cleanup,
        initialize,
    } = startup;
    let handle = supervisor.handle();
    start_driver(handle, async move {
        drive(
            supervisor,
            context,
            cleanup,
            OwnedInitialization {
                state: Initialization::Callback(initialize),
                signals: None::<NoSignals>,
            },
            |supervisor| StartupScope {
                supervisor,
                stage: "startup",
            },
            |error| error,
        )
        .await
        .map(|(supervisor, ())| (handoff(supervisor), ()))
    })
}

pub(super) fn start_scoped<F, E>(
    startup: ScopedStartup<F>,
) -> StartingSupervisor<InitializationError<E>, RunningSupervisor, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
{
    start_scoped_with(startup, install_reserved_signals)
}

pub(super) fn start_scoped_deferred<F, E>(
    startup: DeferredScopedStartup<F>,
) -> StartingSupervisor<InitializationError<E>, UnapprovedSupervisor, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
{
    start_scoped_mapped(
        startup.startup,
        install_reserved_signals,
        std::convert::identity,
    )
}

fn start_scoped_with<F, E, G>(
    startup: ScopedStartup<F>,
    install: impl FnOnce(&'static str) -> Result<G, SignalRegistrationError>,
) -> StartingSupervisor<InitializationError<E>, RunningSupervisor, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
    G: StartupSignals + Send + 'static,
{
    start_scoped_mapped(startup, install, UnapprovedSupervisor::approve_readiness)
}

fn start_scoped_mapped<F, E, G, R>(
    startup: ScopedStartup<F>,
    install: impl FnOnce(&'static str) -> Result<G, SignalRegistrationError>,
    handoff: fn(UnapprovedSupervisor) -> R,
) -> StartingSupervisor<InitializationError<E>, R, F::Output>
where
    F: Initializer<E>,
    E: Send + Sync + 'static,
    G: StartupSignals + Send + 'static,
    R: StartupHandoff + Send + 'static,
{
    let ScopedStartup {
        mut supervisor,
        context,
        cleanup,
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
    start_driver(handle, async move {
        drive(
            supervisor,
            context,
            cleanup,
            OwnedInitialization {
                state: initialization,
                signals: installed_signals,
            },
            |supervisor| ProtectedStartupScope {
                supervisor,
                stage: "startup",
            },
            InitializationError::Application,
        )
        .await
        .map(|(supervisor, output)| (handoff(supervisor), output))
    })
}

enum Initialization<F, E> {
    Callback(F),
    Failed { cause: StartupCause<E>, unused: F },
}

struct OwnedInitialization<F, E, G> {
    state: Initialization<F, E>,
    signals: Option<G>,
}

trait StartupSignals {
    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()>;
    fn register_reserved(self, supervisor: &mut crate::lifecycle::Supervisor);
}

impl StartupSignals for InstalledSignals {
    fn poll_received(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        InstalledSignals::poll_received(self, cx)
    }

    fn register_reserved(self, supervisor: &mut crate::lifecycle::Supervisor) {
        InstalledSignals::register_reserved(self, supervisor);
    }
}

struct NoSignals;

impl StartupSignals for NoSignals {
    fn poll_received(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }

    fn register_reserved(self, _supervisor: &mut crate::lifecycle::Supervisor) {}
}

// Keep the full initialization/destruction/cleanup report until the monitor
// wraps it in Arc. Its native layout exceeds 128 bytes on macOS; boxing here
// would add an allocation immediately before that existing shared allocation.
#[allow(clippy::result_large_err)]
async fn drive<F, I, E, S, C, M, G>(
    supervisor: crate::lifecycle::Supervisor,
    context: OperationContext,
    cleanup: crate::cleanup::CleanupBudget,
    initialization: OwnedInitialization<F, E, G>,
    construct_scope: C,
    map_error: M,
) -> Result<(UnapprovedSupervisor, F::Output), StartupFailure<E>>
where
    F: Initialize<S, I>,
    S: DriverScope,
    C: FnOnce(crate::lifecycle::Supervisor) -> S,
    M: Fn(I) -> E + Copy,
    G: StartupSignals,
{
    let OwnedInitialization {
        state: initialization,
        mut signals,
    } = initialization;
    let handle = supervisor.handle();
    let mut scope = construct_scope(supervisor);
    let (result, mut destruction_panic) = initialize_owned(
        &mut scope,
        &context,
        &handle,
        initialization,
        map_error,
        signals.as_mut(),
    )
    .await;
    let cause = match result {
        Err(cause) => cause,
        Ok(output) => {
            if destruction_panic.is_none()
                && let Some(signals) = signals.take()
            {
                signals.register_reserved(scope.supervisor_mut());
            }
            let checked = check(&context, &handle);
            if checked.is_ok() && destruction_panic.is_none() {
                let (supervisor, _) = scope.into_parts();
                return Ok((supervisor.start_unapproved(), output));
            }
            // A failed startup never yields output; destroy it before cleanup.
            discard(output, &mut destruction_panic);
            checked.err().unwrap_or(StartupCause::DestructionPanicked)
        }
    };
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

/// Destroy rejected output, retaining its panic unless an earlier one exists.
fn discard<T>(unused: T, destruction_panic: &mut Option<PanicPayload>) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(unused)))
        && destruction_panic.is_none()
    {
        *destruction_panic = Some(PanicPayload::new(payload));
    }
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
    if handle.status().readiness() != Readiness::Starting {
        return Err(StartupCause::Draining);
    }
    context.check().map_err(StartupCause::Interrupted)
}

async fn initialize_owned<F, I, E, S, M, G>(
    scope: &mut S,
    context: &OperationContext,
    handle: &ShutdownHandle,
    initialization: Initialization<F, E>,
    map_error: M,
    mut signals: Option<&mut G>,
) -> (Result<F::Output, StartupCause<E>>, Option<PanicPayload>)
where
    F: Initialize<S, I>,
    M: Fn(I) -> E + Copy,
    G: StartupSignals,
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
    let mut future = match catch_unwind(AssertUnwindSafe(|| initialize.initialize(scope))) {
        Ok(future) => future,
        Err(payload) => {
            return (
                Err(StartupCause::Panicked(PanicPayload::new(payload))),
                None,
            );
        }
    };
    let shutdown = handle.signal();
    let draining = shutdown.draining();
    let cancelled = context.cancelled();
    let deadline = tokio::time::sleep_until(context.deadline());
    tokio::pin!(draining, cancelled, deadline);
    // A rejected successful return carries its output out for destruction.
    let result = poll_fn(|cx| {
        if let Err(cause) = check(context, handle) {
            return Poll::Ready(Err((cause, None)));
        }
        if signals
            .as_mut()
            .is_some_and(|signals| signals.poll_received(cx).is_ready())
        {
            handle.request();
            return Poll::Ready(Err((StartupCause::Draining, None)));
        }

        let initialization = match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(poll) => {
                poll.map(|result| result.map_err(|error| StartupCause::Failed(map_error(error))))
            }
            Err(payload) => Poll::Ready(Err(StartupCause::Panicked(PanicPayload::new(payload)))),
        };
        let draining = draining.as_mut().poll(cx).is_ready();
        let cancelled = cancelled.as_mut().poll(cx).is_ready();
        let deadline = deadline.as_mut().poll(cx).is_ready();
        let received = signals
            .as_mut()
            .is_some_and(|signals| signals.poll_received(cx).is_ready());
        if received {
            handle.request();
        }

        let interruption = if draining {
            Some(StartupCause::Draining)
        } else if cancelled {
            Some(StartupCause::Interrupted(
                crate::operation::Interruption::Cancelled,
            ))
        } else if deadline {
            Some(StartupCause::Interrupted(
                crate::operation::Interruption::DeadlineExceeded,
            ))
        } else {
            None
        };
        match (initialization, interruption) {
            (Poll::Ready(Err(cause)), _) => Poll::Ready(Err((cause, None))),
            (Poll::Ready(Ok(output)), Some(cause)) => Poll::Ready(Err((cause, Some(output)))),
            (Poll::Ready(Ok(output)), None) => Poll::Ready(Ok(output)),
            (Poll::Pending, Some(cause)) => Poll::Ready(Err((cause, None))),
            (Poll::Pending, None) if received => Poll::Ready(Err((StartupCause::Draining, None))),
            (Poll::Pending, None) => Poll::Pending,
        }
    })
    .await;
    let mut destruction = catch_unwind(AssertUnwindSafe(|| drop(future)))
        .err()
        .map(PanicPayload::new);
    let result = result.map_err(|(cause, unused)| {
        if let Some(unused) = unused {
            discard(unused, &mut destruction);
        }
        cause
    });
    (result, destruction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RegistrationError,
        cleanup::{CleanupBudget, CleanupOutcome},
        lifecycle::{ShutdownBudget, Supervisor},
        startup::{StartupError, StartupOutcome},
    };
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    struct PanicOnDrop;

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("unused-initializer-drop-marker");
        }
    }

    #[derive(Default)]
    struct ReadySignalState {
        ready: AtomicBool,
        polls: AtomicUsize,
        registrations: AtomicUsize,
        received_registrations: AtomicUsize,
    }

    struct ReadySignals {
        state: Arc<ReadySignalState>,
        received: bool,
    }

    impl StartupSignals for ReadySignals {
        fn poll_received(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
            self.state.polls.fetch_add(1, Ordering::SeqCst);
            if self.received || self.state.ready.load(Ordering::SeqCst) {
                self.received = true;
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }

        fn register_reserved(self, supervisor: &mut Supervisor) {
            self.state.registrations.fetch_add(1, Ordering::SeqCst);
            if self.received {
                self.state
                    .received_registrations
                    .fetch_add(1, Ordering::SeqCst);
            }
            let handle = supervisor.handle();
            supervisor.register_reserved("signals", |startup| async move {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
            });
            if self.received {
                handle.request();
            }
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
            crate::operation::OperationOwner::new(Duration::from_secs(1))
                .unwrap()
                .into_context(),
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
            Err::<InstalledSignals, _>(SignalRegistrationError::Install(io::Error::other(
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
        let context = crate::operation::OperationOwner::new(Duration::from_secs(1))
            .unwrap()
            .into_context();
        context.cancel();
        let install_calls = Arc::new(AtomicUsize::new(0));
        let install_called = install_calls.clone();
        let startup = Startup::scoped(supervisor(), context, budget(), |_scope| {
            Box::pin(async { Ok::<_, RegistrationError>(()) })
        })
        .with_unix_signals("signals");
        let mut owner = start_scoped_with(startup, move |_| {
            install_called.fetch_add(1, Ordering::SeqCst);
            Err::<InstalledSignals, _>(SignalRegistrationError::Install(io::Error::other(
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

    #[tokio::test(flavor = "current_thread")]
    async fn simultaneous_application_failure_and_signal_preserve_the_application_cause() {
        let state = Arc::new(ReadySignalState::default());
        let installed = state.clone();
        let received = state.clone();
        let process = supervisor();
        let handle = process.handle();
        let startup = Startup::scoped(
            process,
            crate::operation::OperationOwner::new(Duration::from_secs(1))
                .unwrap()
                .into_context(),
            budget(),
            move |_scope| {
                Box::pin(async move {
                    received.ready.store(true, Ordering::SeqCst);
                    Err::<(), _>(RegistrationError::Duplicate("application-resource"))
                })
            },
        )
        .with_unix_signals("signals");
        let mut owner = start_scoped_with(startup, move |_| {
            Ok(ReadySignals {
                state: installed,
                received: false,
            })
        });

        let Err(StartupError::Failed(report)) = owner.wait().await else {
            panic!("missing application failure")
        };
        assert!(matches!(
            report.cause,
            StartupCause::Failed(InitializationError::Application(
                RegistrationError::Duplicate("application-resource")
            ))
        ));
        assert_eq!(state.polls.load(Ordering::SeqCst), 2);
        assert_eq!(state.registrations.load(Ordering::SeqCst), 0);
        assert_eq!(state.received_registrations.load(Ordering::SeqCst), 0);
        assert_eq!(handle.status().readiness(), Readiness::Draining);
        assert!(report.cleanup.is_success());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simultaneous_success_and_signal_transfer_consumed_reception_before_final_check() {
        let state = Arc::new(ReadySignalState::default());
        let installed = state.clone();
        let received = state.clone();
        let process = supervisor();
        let handle = process.handle();
        let startup = Startup::scoped(
            process,
            crate::operation::OperationOwner::new(Duration::from_secs(1))
                .unwrap()
                .into_context(),
            budget(),
            move |_scope| {
                Box::pin(async move {
                    received.ready.store(true, Ordering::SeqCst);
                    Ok::<_, RegistrationError>(())
                })
            },
        )
        .with_unix_signals("signals");
        let mut owner = start_scoped_with(startup, move |_| {
            Ok(ReadySignals {
                state: installed,
                received: false,
            })
        });

        let Err(StartupError::Failed(report)) = owner.wait().await else {
            panic!("signal reception incorrectly published a running handoff")
        };
        assert!(matches!(report.cause, StartupCause::Draining));
        assert_eq!(state.polls.load(Ordering::SeqCst), 2);
        assert_eq!(state.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(state.received_registrations.load(Ordering::SeqCst), 1);
        assert_eq!(handle.status().wait_ready().await, Err(Readiness::Draining));
        assert!(report.cleanup.is_success());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn preexisting_drain_does_not_poll_a_pending_initializer_again() {
        struct PendingPolls(Arc<AtomicUsize>);

        impl Future for PendingPolls {
            type Output = Result<(), RegistrationError>;

            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Poll::Pending
            }
        }

        let polls = Arc::new(AtomicUsize::new(0));
        let counted = polls.clone();
        let process = supervisor();
        let handle = process.handle();
        let startup = Startup::scoped(
            process,
            crate::operation::OperationOwner::new(Duration::from_secs(1))
                .unwrap()
                .into_context(),
            budget(),
            move |_scope| Box::pin(PendingPolls(counted)),
        );
        let mut owner = start_scoped(startup);
        while polls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(polls.load(Ordering::SeqCst), 1);

        handle.request();
        let Err(StartupError::Failed(report)) = owner.wait().await else {
            panic!("missing drain failure")
        };
        assert!(matches!(report.cause, StartupCause::Draining));
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert!(report.cleanup.is_success());
    }
}
