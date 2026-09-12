use batter::{
    cleanup::{CleanupBudget, CleanupOutcome},
    lifecycle::{Readiness, ShutdownBudget, Supervisor},
    operation::{Interruption, OperationContext},
    startup::{
        PanicPayloadBusy, Startup, StartupCause, StartupError, StartupFuture, StartupOutcome,
        StartupScope,
    },
};
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::oneshot;

#[derive(Debug, thiserror::Error)]
#[error("private-startup-marker")]
struct Failure;

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}
fn supervisor() -> Supervisor {
    let mut supervisor = Supervisor::new(
        ShutdownBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .unwrap(),
    );
    supervisor
        .register("component", |shutdown| async move {
            shutdown.mark_started();
            shutdown.draining().await;
            Ok(())
        })
        .unwrap();
    supervisor
}
fn startup<F>(initialize: F) -> Startup<F>
where
    F: for<'a> FnOnce(&'a mut StartupScope) -> StartupFuture<'a, Failure>,
{
    Startup::new(
        supervisor(),
        OperationContext::new(Duration::from_secs(10)).unwrap(),
        cleanup_budget(),
        initialize,
    )
}
fn failed<E>(
    result: Result<batter::lifecycle::RunningSupervisor, StartupError<E>>,
) -> Arc<batter::startup::StartupFailure<E>> {
    match result {
        Err(StartupError::Failed(report)) => report,
        _ => panic!("expected reported startup failure"),
    }
}

#[tokio::test]
async fn failed_initialization_preserves_concrete_cause_and_lifo_cleanup_errors() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let captured = order.clone();
    let mut starting = startup(move |scope| {
        Box::pin(async move {
            scope.stage("second-resource").unwrap();
            for name in ["first", "second"] {
                let slot = scope.supervisor().reserve_cleanup(name).unwrap();
                let order = captured.clone();
                slot.register(move || async move {
                    order.lock().unwrap().push(name);
                    Err(std::io::Error::other(name).into())
                });
            }
            Err(Failure)
        })
    })
    .start();
    let observer = starting.observer();
    let report = failed(starting.wait().await);
    assert!(matches!(report.cause, StartupCause::Failed(Failure)));
    assert_eq!(report.stage, "second-resource");
    assert_eq!(*order.lock().unwrap(), ["second", "first"]);
    assert_eq!(report.cleanup.records.len(), 2);
    for record in &report.cleanup.records {
        assert_eq!(record.outcome, CleanupOutcome::Failed);
        assert_eq!(record.error.as_ref().unwrap().to_string(), record.name);
    }
    let StartupOutcome::Failed(StartupError::Failed(observed)) = observer.wait().await else {
        panic!("expected same failure")
    };
    assert!(Arc::ptr_eq(&report, &observed));
    assert!(!format!("{report:?} {report}").contains("private-startup-marker"));
}

#[tokio::test]
async fn rejected_reservation_after_prior_acquisition_preserves_registration_and_cleanup() {
    let acquired = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let acquisition = acquired.clone();
    let cleanup = closed.clone();
    let mut starting = Startup::new(
        supervisor(),
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                let slot = scope.supervisor().reserve_cleanup("resource")?;
                acquisition.fetch_add(1, Ordering::SeqCst);
                slot.register(move || async move {
                    cleanup.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("cleanup-original").into())
                });
                let _slot = scope.supervisor().reserve_cleanup("resource")?;
                acquisition.fetch_add(1, Ordering::SeqCst);
                Ok::<_, batter::RegistrationError>(())
            })
        },
    )
    .start();
    let report = failed(starting.wait().await);
    assert!(matches!(
        report.cause,
        StartupCause::Failed(batter::RegistrationError::Duplicate("resource"))
    ));
    assert_eq!(acquired.load(Ordering::SeqCst), 1);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "cleanup-original"
    );
}

#[tokio::test]
async fn borrowed_waiter_cancellation_leaves_initializer_and_handoff_owned() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let mut starting = startup(move |_| {
        Box::pin(async move {
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            Ok(())
        })
    })
    .start();
    entered_rx.await.unwrap();
    let mut waiter = Box::pin(starting.wait());
    assert!(matches!(
        std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx))).await,
        Poll::Pending
    ));
    drop(waiter);
    release_tx.send(()).unwrap();
    let running = starting.wait().await.unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test]
async fn owner_drop_during_initialization_drives_registered_cleanup_exactly_once() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let closed = Arc::new(AtomicUsize::new(0));
    let count = closed.clone();
    let mut base = supervisor();
    let handle = base.handle();
    base.reserve_cleanup("resource")
        .unwrap()
        .register(move || async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
    let starting = Startup::new(
        base,
        OperationContext::new(Duration::from_secs(10)).unwrap(),
        cleanup_budget(),
        move |_| {
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
    let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
        panic!("expected drain failure")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(report.cleanup.is_success());
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(handle.readiness(), Readiness::Draining);
}

#[tokio::test]
async fn dropping_unclaimed_running_handoff_does_not_let_observers_keep_it_alive() {
    let closed = Arc::new(AtomicUsize::new(0));
    let count = closed.clone();
    let starting = startup(move |scope| {
        Box::pin(async move {
            scope
                .supervisor()
                .on_cleanup("resource", move || async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
            Ok(())
        })
    })
    .start();
    let observer = starting.observer();
    let StartupOutcome::Running(driver) = observer.wait().await else {
        panic!("expected running handoff")
    };
    drop(starting);
    assert!(driver.wait().await.unwrap().is_success());
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn never_started_and_already_cancelled_initializers_are_inert() {
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    drop(startup(move |_| {
        called.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    context.cancel();
    let called = calls.clone();
    let mut starting = Startup::new(supervisor(), context, cleanup_budget(), move |_| {
        called.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok::<_, Failure>(()) })
    })
    .start();
    assert!(matches!(
        failed(starting.wait().await).cause,
        StartupCause::Interrupted(Interruption::Cancelled)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn drain_at_initializer_completion_cannot_publish_ready() {
    let base = supervisor();
    let handle = base.handle();
    let request = handle.clone();
    let mut starting = Startup::new(
        base,
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |_| {
            Box::pin(async move {
                request.request();
                Ok::<_, Failure>(())
            })
        },
    )
    .start();
    assert!(matches!(
        failed(starting.wait().await).cause,
        StartupCause::Draining
    ));
    assert_eq!(handle.wait_ready().await, Err(Readiness::Draining));
}

struct DropPanic;
impl Future for DropPanic {
    type Output = Result<(), Failure>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Err(Failure))
    }
}
impl Drop for DropPanic {
    fn drop(&mut self) {
        panic!("destruction-original")
    }
}

struct SuccessfulDrainDropPanic {
    handle: batter::lifecycle::ShutdownHandle,
}

impl Future for SuccessfulDrainDropPanic {
    type Output = Result<(), Failure>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(()))
    }
}

impl Drop for SuccessfulDrainDropPanic {
    fn drop(&mut self) {
        self.handle.request();
        panic!("successful-destruction-original")
    }
}

#[tokio::test]
async fn application_failure_and_future_destruction_panic_both_survive() {
    let mut starting = startup(|scope| {
        scope
            .supervisor()
            .on_cleanup("resource", || async { Ok(()) })
            .unwrap();
        Box::pin(DropPanic)
    })
    .start();
    let report = failed(starting.wait().await);
    assert!(matches!(report.cause, StartupCause::Failed(Failure)));
    report
        .destruction_panic
        .as_ref()
        .unwrap()
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"destruction-original")
            );
        })
        .expect("payload inspection is uncontended");
    assert_eq!(report.cleanup.records.len(), 1);
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn successful_future_destruction_panic_does_not_replace_a_concurrent_drain() {
    let process = supervisor();
    let handle = process.handle();
    let future_handle = handle.clone();
    let mut starting = Startup::new(
        process,
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |_scope| {
            Box::pin(SuccessfulDrainDropPanic {
                handle: future_handle,
            })
        },
    )
    .start();

    let report = failed(starting.wait().await);
    assert!(matches!(report.cause, StartupCause::Draining));
    report
        .destruction_panic
        .as_ref()
        .unwrap()
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"successful-destruction-original")
            );
        })
        .expect("payload inspection is uncontended");
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn initializer_poll_panic_retains_payload_and_drives_cleanup() {
    let mut starting = startup(|scope| {
        Box::pin(async move {
            scope
                .supervisor()
                .on_cleanup("resource", || async { Ok(()) })
                .unwrap();
            panic!("poll-original");
        })
    })
    .start();
    let report = failed(starting.wait().await);
    let StartupCause::Panicked(payload) = &report.cause else {
        panic!("expected panic")
    };
    payload
        .try_inspect(|value| {
            assert_eq!(value.downcast_ref::<&str>(), Some(&"poll-original"));
            assert_eq!(payload.try_inspect(|_| ()), Err(PanicPayloadBusy));
        })
        .expect("outer payload inspection is uncontended");
    let inspection_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = payload.try_inspect::<()>(|_| panic!("inspection callback failed"));
    }));
    assert!(inspection_panic.is_err());
    payload
        .try_inspect(|value| assert_eq!(value.downcast_ref::<&str>(), Some(&"poll-original")))
        .expect("a callback panic cannot prevent later inspection");
    assert_eq!(report.cleanup.records.len(), 1);
}

#[tokio::test]
async fn waiter_drop_across_acquisition_and_registration_retains_one_cleanup_owner() {
    let (acquired_tx, acquired_rx) = oneshot::channel();
    let (register_tx, register_rx) = oneshot::channel();
    let (registered_tx, registered_rx) = oneshot::channel();
    let (closing_tx, closing_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let closed = Arc::new(AtomicUsize::new(0));
    let count = closed.clone();
    let mut starting = startup(move |scope| {
        Box::pin(async move {
            let slot = scope.supervisor().reserve_cleanup("resource").unwrap();
            let resource = String::from("acquired");
            acquired_tx.send(()).unwrap();
            // A deliberately suspended transfer proves cancelling the borrowed
            // waiter does not cancel the owning initializer. Production code should
            // register immediately, without inserting this test-only suspension.
            register_rx.await.unwrap();
            slot.register(move || async move {
                closing_tx.send(()).unwrap();
                finish_rx.await.unwrap();
                drop(resource);
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
            registered_tx.send(()).unwrap();
            Err(Failure)
        })
    })
    .start();
    let observer = starting.observer();
    acquired_rx.await.unwrap();
    let mut waiter = Box::pin(starting.wait());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(waiter.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiter);
    register_tx.send(()).unwrap();
    registered_rx.await.unwrap();
    closing_rx.await.unwrap();
    drop(starting);
    assert_eq!(closed.load(Ordering::SeqCst), 0);
    finish_tx.send(()).unwrap();
    let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
        panic!("expected retained startup failure")
    };
    assert!(matches!(report.cause, StartupCause::Failed(Failure)));
    assert!(report.cleanup.is_success());
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn initialization_deadline_uses_separate_cleanup_budget() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let mut starting = startup(move |scope| {
        Box::pin(async move {
            scope
                .supervisor()
                .on_cleanup("resource", || async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok(())
                })
                .unwrap();
            entered_tx.send(()).unwrap();
            std::future::pending().await
        })
    })
    .start();
    entered_rx.await.unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    let report = failed(starting.wait().await);
    assert!(matches!(
        report.cause,
        StartupCause::Interrupted(Interruption::DeadlineExceeded)
    ));
    assert_eq!(report.cleanup.records.len(), 1);
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn factory_construction_panic_still_closes_registered_resources() {
    let mut starting = startup(|scope| {
        scope
            .supervisor()
            .on_cleanup("resource", || async { Ok(()) })
            .unwrap();
        panic!("constructor-original");
    })
    .start();
    let report = failed(starting.wait().await);
    let StartupCause::Panicked(payload) = &report.cause else {
        panic!("expected panic")
    };
    payload
        .try_inspect(|payload| {
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"constructor-original")
            )
        })
        .expect("payload inspection is uncontended");
    assert_eq!(report.cleanup.records.len(), 1);
    assert!(report.cleanup.is_success());
}

#[tokio::test]
async fn owner_drop_before_first_poll_skips_factory_and_drives_existing_cleanup() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut base = supervisor();
    base.on_cleanup("resource", || async { Ok(()) }).unwrap();
    let starting = Startup::new(
        base,
        OperationContext::new(Duration::from_secs(1)).unwrap(),
        cleanup_budget(),
        move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok::<_, Failure>(()) })
        },
    )
    .start();
    let observer = starting.observer();
    drop(starting);
    let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
        panic!("expected drain")
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(report.cause, StartupCause::Draining));
    assert_eq!(report.cleanup.records.len(), 1);
}

#[tokio::test]
async fn successful_initialization_still_requires_actual_component_acknowledgement() {
    let (release_tx, release_rx) = oneshot::channel();
    let mut starting = startup(move |scope| {
        Box::pin(async move {
            scope
                .supervisor()
                .register("delayed", move |shutdown| async move {
                    release_rx.await.unwrap();
                    shutdown.mark_started();
                    shutdown.draining().await;
                    Ok(())
                })
                .unwrap();
            Ok(())
        })
    })
    .start();
    let running = starting.wait().await.unwrap();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    release_tx.send(()).unwrap();
    running.handle().wait_ready().await.unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test]
async fn successful_initialization_can_withhold_application_readiness_approval() {
    let acknowledged = Arc::new(AtomicUsize::new(0));
    let component_acknowledged = acknowledged.clone();
    let mut starting = startup(move |scope| {
        Box::pin(async move {
            scope
                .supervisor()
                .register("staged", move |shutdown| async move {
                    assert!(shutdown.mark_started());
                    component_acknowledged.store(1, Ordering::SeqCst);
                    shutdown.draining().await;
                    Ok(())
                })
                .unwrap();
            Ok(())
        })
    })
    .without_readiness_approval()
    .start();
    let running = starting.wait().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while acknowledged.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(running.handle().readiness(), Readiness::Starting);
    assert!(running.handle().mark_ready());
    running.handle().wait_ready().await.unwrap();
    assert!(running.shutdown().await.unwrap().is_success());
}

#[tokio::test]
async fn shutdown_projection_retains_coordinator_failure_without_formatting_it() {
    let error = Arc::new(
        tokio::spawn(async { panic!("private-coordinator-marker") })
            .await
            .unwrap_err(),
    );
    let failure = batter::lifecycle::check_shutdown(Err(error.clone())).unwrap_err();
    assert!(!format!("{failure:?} {failure}").contains("private-coordinator-marker"));
    let batter::lifecycle::ShutdownFailure::Coordinator(retained) = failure else {
        panic!("expected coordinator failure")
    };
    assert!(Arc::ptr_eq(&error, &retained));
    assert!(retained.is_panic());
}
