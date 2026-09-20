use batter_core::lifecycle::Fatal;
use batter_core::{BoxError, lifecycle::ComponentExit};
use batter_core::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{
        ProcessAdmissionError, ProcessCapacity, ProcessHandle, Readiness, ShutdownBudget,
        ShutdownSignal, Supervisor,
    },
    operation::OperationContext,
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    mem::discriminant,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

fn supervisor() -> Supervisor {
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second).unwrap();
    Supervisor::with_process_capacity(
        ShutdownBudget::new(second, second, second, cleanup).unwrap(),
        ProcessCapacity::new(1).unwrap(),
    )
}

#[test]
fn operation_admission_returns_the_observed_lifecycle_state() {
    let (control, approval) = batter_core::lifecycle::ShutdownHandle::new_with_readiness_approval();
    let admission = control.operation_admission();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);

    assert!(matches!(
        admission.admit(deadline),
        Err(Readiness::Starting)
    ));
    approval.approve();
    let expired = admission.admit(tokio::time::Instant::now()).unwrap();
    assert_eq!(
        expired.check(),
        Err(batter_core::operation::Interruption::DeadlineExceeded)
    );
    let admitted = admission.admit(deadline).unwrap();
    control.request();
    assert!(matches!(
        admission.admit(deadline),
        Err(Readiness::Draining)
    ));
    assert_eq!(admitted.check(), Ok(()), "drain is not forced cancellation");
}

#[tokio::test]
async fn operation_admission_is_downward_only_and_closes_after_stop() {
    let process = supervisor();
    let control = process.handle();
    let status = process.status();
    let admission = process.operation_admission();
    let shutdown = control.signal();
    let running = process.start();
    status.wait_ready().await.unwrap();

    let context = admission
        .admit(tokio::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    context.cancel();
    assert_eq!(
        context.check(),
        Err(batter_core::operation::Interruption::Cancelled)
    );
    assert!(
        !shutdown.is_cancelled(),
        "child cancellation cannot travel upward"
    );
    assert_eq!(status.readiness(), Readiness::Ready);

    assert!(running.shutdown().await.unwrap().is_success());
    assert_eq!(status.readiness(), Readiness::Stopped);
    assert!(matches!(
        admission.admit(tokio::time::Instant::now() + Duration::from_secs(1)),
        Err(Readiness::Stopped)
    ));
}

#[tokio::test(start_paused = true)]
async fn operation_admission_racing_drain_has_only_linearized_outcomes() {
    let mut process = supervisor();
    process
        .register("drain-anchor", |startup| async move {
            let shutdown = startup.acknowledge_started();
            shutdown.cancelled().await;
            Ok(shutdown.stopped())
        })
        .unwrap();
    let control = process.handle();
    let status = process.status();
    let admission = process.operation_admission();
    let running = process.start();
    status.wait_ready().await.unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let admitting = {
        let barrier = barrier.clone();
        tokio::task::spawn_blocking(move || {
            barrier.wait();
            admission.admit(tokio::time::Instant::now() + Duration::from_secs(10))
        })
    };
    let requesting = tokio::task::spawn_blocking(move || {
        barrier.wait();
        control.request();
    });
    let admitted = admitting.await.unwrap();
    requesting.await.unwrap();

    assert_eq!(status.readiness(), Readiness::Draining);
    match &admitted {
        Ok(context) => assert_eq!(
            context.check(),
            Ok(()),
            "an admission linearized before drain survives the drain phase"
        ),
        Err(state) => assert_eq!(*state, Readiness::Draining),
    }

    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(running.wait().await.unwrap().is_success());
    if let Ok(context) = admitted {
        assert_eq!(
            context.check(),
            Err(batter_core::operation::Interruption::Cancelled)
        );
    }
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn abandoned_startup_wakes_registered_readiness_waiter() {
    let supervisor = supervisor();
    let handle = supervisor.handle();
    let cancellation = handle.signal();
    let wake = Arc::new(WakeCount::default());
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let status = handle.status();
    let mut waiter = Box::pin(status.wait_ready());
    assert_eq!(waiter.as_mut().poll(&mut cx), Poll::Pending);

    drop(supervisor);

    assert_eq!(handle.status().readiness(), Readiness::Draining);
    assert!(cancellation.is_cancelled());
    assert!(wake.0.load(Ordering::SeqCst) > 0);
    assert_eq!(
        waiter.as_mut().poll(&mut cx),
        Poll::Ready(Err(Readiness::Draining))
    );
}

struct ObserveAbandonment(ShutdownSignal);

impl Drop for ObserveAbandonment {
    fn drop(&mut self) {
        assert!(self.0.is_draining());
        assert!(self.0.is_cancelled());
    }
}

#[test]
fn abandonment_signals_before_dropping_inert_application_captures() {
    enum Path {
        Unstarted,
        OrdinaryDriver,
        UnapprovedDriver,
    }

    for path in [
        Path::Unstarted,
        Path::OrdinaryDriver,
        Path::UnapprovedDriver,
    ] {
        let mut supervisor = supervisor();
        let captured = ObserveAbandonment(supervisor.handle().signal());
        supervisor
            .register(
                "inert-component",
                move |_startup| -> std::future::Ready<Result<ComponentExit, BoxError>> {
                    let _capture = captured;
                    panic!("abandonment must not start a component")
                },
            )
            .unwrap();
        let cleanup_capture = ObserveAbandonment(supervisor.handle().signal());
        supervisor
            .on_cleanup("inert-resource", move || {
                let _capture = cleanup_capture;
                panic!("abandonment must not run async cleanup");
                #[allow(unreachable_code)]
                async {
                    Ok(())
                }
            })
            .unwrap();
        match path {
            Path::Unstarted => drop(supervisor),
            Path::OrdinaryDriver => drop(supervisor.run_until(pending())),
            Path::UnapprovedDriver => drop(supervisor.run_until_unapproved(pending())),
        }
    }
}

#[test]
fn unpolled_driver_retains_startup_ownership_until_dropped() {
    let supervisor = supervisor();
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    let driver = supervisor.run_until(pending());
    assert_eq!(handle.status().readiness(), Readiness::Starting);
    assert!(!handle.signal().is_cancelled());
    assert!(matches!(
        process.try_spawn("before-poll", |_| async { Ok::<_, Fatal<Infallible>>(()) }),
        Err(ProcessAdmissionError::NotRunning)
    ));
    drop(driver);
    assert_eq!(handle.status().readiness(), Readiness::Draining);
    assert!(handle.signal().is_cancelled());
}

#[tokio::test]
async fn extracted_cleanup_completes_after_supervisor_signals_cancellation() {
    let mut supervisor = supervisor();
    let shutdown = supervisor.handle().signal();
    let observed_shutdown = shutdown.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            assert!(observed_shutdown.is_cancelled());
            // Teardown has its own context; process cancellation cannot skip it.
            let cleanup = OperationContext::new(Duration::from_secs(1))?;
            cleanup
                .run("resource-close", |_| async move {
                    tokio::task::yield_now().await;
                    closing.store(true, Ordering::SeqCst);
                    Ok::<_, Infallible>(())
                })
                .await?;
            Ok(())
        })
        .unwrap();
    let stack = supervisor.take_cleanup();
    drop(supervisor);
    assert!(shutdown.is_cancelled());
    assert!(!closed.load(Ordering::SeqCst), "drop must not run teardown");

    let second = Duration::from_secs(1);
    let report = stack
        .close(CleanupBudget::new(second, second, second).unwrap())
        .await;
    assert!(report.is_success());
    assert!(closed.load(Ordering::SeqCst));
}

fn assert_rejections(process: &ProcessHandle, expected: ProcessAdmissionError) {
    for (name, expected) in [
        ("valid-name", expected),
        (
            "",
            ProcessAdmissionError::InvalidName(RegistrationError::InvalidName),
        ),
    ] {
        let error = process
            .try_spawn(name, |_| {
                panic!("rejected factory must remain inert");
                #[allow(unreachable_code)]
                async {
                    Ok::<_, Fatal<Infallible>>(())
                }
            })
            .err()
            .expect("admission must reject");
        assert_eq!(discriminant(&error), discriminant(&expected), "{error:?}");
    }
}

#[tokio::test]
async fn admission_precedence_covers_startup_capacity_drain_and_completion() {
    let supervisor = supervisor();
    let handle = supervisor.handle();
    let process = supervisor.process_handle().unwrap();
    assert_rejections(&process, ProcessAdmissionError::NotRunning);
    let mut pending_driver = supervisor.run_until_unapproved(pending());
    poll_fn(|cx| {
        assert!(
            std::pin::Pin::new(&mut pending_driver)
                .poll(cx)
                .is_pending()
        );
        Poll::Ready(())
    })
    .await;
    assert_rejections(&process, ProcessAdmissionError::NotReady);
    let driver = pending_driver.approve_readiness();
    // Keep the admitted task queued so its only permit cannot be released yet.
    let receipt = process
        .try_spawn("queued", |_| async { Ok::<_, Fatal<Infallible>>(7) })
        .unwrap();
    assert_rejections(&process, ProcessAdmissionError::Full);
    handle.request();
    assert_rejections(&process, ProcessAdmissionError::Closed);
    assert!(driver.await.is_success());
    assert_eq!(receipt.wait().await.unwrap(), 7);
    assert_eq!(handle.status().readiness(), Readiness::Stopped);
    assert_rejections(&process, ProcessAdmissionError::Closed);
}

#[tokio::test]
async fn caller_owned_unapproved_driver_can_finish_without_publishing_ready() {
    let supervisor = supervisor();
    let status = supervisor.status();
    let process = supervisor.process_handle().unwrap();
    let report = supervisor
        .run_until_unapproved(async move {
            assert_rejections(&process, ProcessAdmissionError::NotReady);
        })
        .await;
    assert!(report.is_success());
    assert_eq!(status.readiness(), Readiness::Stopped);
}
