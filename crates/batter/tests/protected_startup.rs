use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::OperationContext,
    startup::{
        InitializationError, ProtectedStartupScope, Startup, StartupCause, StartupError,
        StartupFuture, StartupObserver,
    },
};
use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

fn cleanup_budget() -> CleanupBudget {
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
            cleanup_budget(),
        )
        .unwrap(),
    )
}

#[derive(Debug, PartialEq, Eq)]
struct Failure;
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("private-protected-marker")
    }
}
impl Error for Failure {}

#[tokio::test]
async fn protected_channel_component_joins_before_reserved_resource_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let component_events = events.clone();
    let cleanup_events = events.clone();
    let (requests, mut inbox) = mpsc::channel::<(u32, oneshot::Sender<u32>)>(1);
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("service.ready")?;
                let slot = scope.reserve_cleanup("service.resource")?;
                tokio::task::yield_now().await;
                slot.register(move || async move {
                    cleanup_events.lock().unwrap().push("resource released");
                    Ok(())
                });
                scope
                    .registration()
                    .register("service", move |shutdown| async move {
                        shutdown.mark_started();
                        loop {
                            tokio::select! {
                                biased;
                                _ = shutdown.draining() => break,
                                request = inbox.recv() => {
                                    let Some((value, reply)) = request else { break };
                                    let _ = reply.send(value.saturating_mul(2));
                                }
                            }
                        }
                        component_events.lock().unwrap().push("component stopped");
                        Ok(())
                    })?;
                Ok::<_, batter::RegistrationError>(())
            })
        },
    )
    .start();

    let running = starting.wait().await.unwrap();
    running.handle().wait_ready().await.unwrap();
    assert_eq!(running.handle().readiness(), Readiness::Ready);
    let (reply, response) = oneshot::channel();
    requests.send((21, reply)).await.unwrap();
    assert_eq!(response.await.unwrap(), 42);
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success(), "{report}");
    assert_eq!(report.cleanup.records[0].name, "service.resource");
    assert_eq!(
        *events.lock().unwrap(),
        ["component stopped", "resource released"]
    );
}

struct ReturnedThenPanics;
impl Future for ReturnedThenPanics {
    type Output = Result<(), Failure>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Err(Failure))
    }
}
impl Drop for ReturnedThenPanics {
    fn drop(&mut self) {
        panic!("protected-destruction-marker")
    }
}

#[tokio::test]
async fn protected_application_error_survives_future_destruction_panic_and_cleanup() {
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        |scope| {
            scope
                .reserve_cleanup("resource")
                .unwrap()
                .register(|| async { Ok(()) });
            Box::pin(ReturnedThenPanics)
        },
    )
    .start();
    let error = match starting.wait().await {
        Ok(running) => {
            drop(running);
            panic!("protected startup unexpectedly succeeded")
        }
        Err(error) => error,
    };
    let StartupError::Failed(report) = error else {
        panic!("coordinator failed")
    };
    assert!(matches!(
        report.cause,
        StartupCause::Failed(InitializationError::Application(Failure))
    ));
    report
        .destruction_panic
        .as_ref()
        .unwrap()
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"protected-destruction-marker")
            );
        })
        .unwrap();
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert!(!format!("{report:?}").contains("private-protected-marker"));
}

#[tokio::test]
async fn protected_failure_retains_stage_lifo_cleanup_errors_and_observer_identity() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let captured = order.clone();
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                scope.stage("second-resource").unwrap();
                for name in ["first", "second"] {
                    let slot = scope.reserve_cleanup(name).unwrap();
                    let order = captured.clone();
                    slot.register(move || async move {
                        order.lock().unwrap().push(name);
                        Err(std::io::Error::other(name).into())
                    });
                }
                Err(Failure)
            })
        },
    )
    .start();
    let observer = starting.observer();
    let error = match starting.wait().await {
        Ok(running) => {
            drop(running);
            panic!("protected startup unexpectedly succeeded")
        }
        Err(error) => error,
    };
    let StartupError::Failed(report) = error else {
        panic!("missing protected startup report")
    };
    assert_eq!(report.stage, "second-resource");
    assert!(matches!(
        report.cause,
        StartupCause::Failed(InitializationError::Application(Failure))
    ));
    assert_eq!(*order.lock().unwrap(), ["second", "first"]);
    assert_eq!(report.cleanup.records.len(), 2);
    assert!(
        report
            .cleanup
            .records
            .iter()
            .all(|record| record.outcome == CleanupOutcome::Failed)
    );
    let batter::startup::StartupOutcome::Failed(StartupError::Failed(observed)) =
        observer.wait().await
    else {
        panic!("missing observed protected report")
    };
    assert!(Arc::ptr_eq(&report, &observed));
    assert!(!format!("{report:?} {report}").contains("private-protected-marker"));
}

#[tokio::test]
async fn cancelling_a_protected_borrowed_waiter_leaves_initialization_owned() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(2)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap();
                scope
                    .registration()
                    .register("worker", |shutdown| async move {
                        shutdown.mark_started();
                        shutdown.draining().await;
                        Ok(())
                    })?;
                Ok::<_, batter::RegistrationError>(())
            })
        },
    )
    .start();
    entered_rx.await.unwrap();
    let mut waiter = Box::pin(starting.wait());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiter);
    release_tx.send(()).unwrap();
    let running = starting.wait().await.unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test]
async fn dropping_protected_owner_during_initialization_drives_cleanup_once() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let closed = Arc::new(AtomicUsize::new(0));
    let closing = closed.clone();
    let mut base = supervisor();
    base.reserve_cleanup("resource")
        .unwrap()
        .register(move || async move {
            closing.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
    let starting = Startup::scoped(
        base,
        OperationContext::new(Duration::from_secs(10)).unwrap(),
        cleanup_budget(),
        move |_scope| {
            Box::pin(async move {
                entered_tx.send(()).unwrap();
                std::future::pending::<Result<(), Failure>>().await
            })
        },
    )
    .start();
    let observer = starting.observer();
    entered_rx.await.unwrap();
    drop(starting);
    let batter::startup::StartupOutcome::Failed(StartupError::Failed(report)) =
        observer.wait().await
    else {
        panic!("missing drain report")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(report.cleanup.is_success());
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_protected_owner_before_first_poll_skips_factory_and_cleans_prior_resource() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let mut base = supervisor();
    base.reserve_cleanup("resource")
        .unwrap()
        .register(|| async { Ok(()) });
    let starting = Startup::scoped(
        base,
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |_scope| {
            called.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok::<_, Failure>(()) })
        },
    )
    .start();
    let observer = starting.observer();
    drop(starting);
    let batter::startup::StartupOutcome::Failed(StartupError::Failed(report)) =
        observer.wait().await
    else {
        panic!("missing drain report")
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn protected_factory_panic_still_closes_registered_resources() {
    let mut starting = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        |scope| {
            scope
                .reserve_cleanup("resource")
                .unwrap()
                .register(|| async { Ok(()) });
            panic!("protected-constructor-marker");
        },
    )
    .start::<Failure>();
    let error = match starting.wait().await {
        Ok(running) => {
            drop(running);
            panic!("protected startup unexpectedly succeeded")
        }
        Err(error) => error,
    };
    let StartupError::Failed(report) = error else {
        panic!("missing protected startup report")
    };
    let StartupCause::Panicked(payload) = &report.cause else {
        panic!("missing constructor panic")
    };
    payload
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"protected-constructor-marker")
            );
        })
        .unwrap();
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn protected_owner_and_observer_keep_the_stable_error_type() {
    let startup = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        |_scope| Box::pin(async { Err::<(), _>(Failure) }),
    );
    let mut owner: batter::startup::StartingSupervisor<InitializationError<Failure>> =
        startup.start();
    let observer: StartupObserver<InitializationError<Failure>> = owner.observer();
    let waited = match owner.wait().await {
        Ok(running) => {
            drop(running);
            panic!("protected startup unexpectedly succeeded")
        }
        Err(error) => error,
    };
    let observed = observer.wait().await;
    let StartupError::Failed(waited) = waited else {
        panic!("missing report")
    };
    let batter::startup::StartupOutcome::Failed(StartupError::Failed(observed)) = observed else {
        panic!("missing observed report")
    };
    assert!(Arc::ptr_eq(&waited, &observed));
}

#[test]
fn protected_callback_type_can_borrow_the_scope() {
    fn callback(scope: &mut ProtectedStartupScope) -> StartupFuture<'_, BoxError> {
        Box::pin(async move {
            scope.stage("typed.callback")?;
            Ok(())
        })
    }
    let specification = Startup::scoped(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        callback,
    );
    drop(specification);
}
