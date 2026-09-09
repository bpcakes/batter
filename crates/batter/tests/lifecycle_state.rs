use batter::{
    RegistrationError,
    cleanup::CleanupBudget,
    lifecycle::{
        ProcessAdmissionError, ProcessHandle, Readiness, ShutdownBudget, ShutdownHandle, Supervisor,
    },
    operation::OperationContext,
};
use std::{
    convert::Infallible,
    future::{Future, pending, poll_fn},
    mem::discriminant,
    sync::{
        Arc,
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
        1,
    )
    .unwrap()
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
    let cancellation = handle.operation_token();
    let wake = Arc::new(WakeCount::default());
    let waker = Waker::from(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = Box::pin(handle.wait_ready());
    let observer = handle.observer();
    let mut completion = Box::pin(observer.wait());
    assert_eq!(waiter.as_mut().poll(&mut cx), Poll::Pending);

    drop(supervisor);

    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(cancellation.is_cancelled());
    assert!(wake.0.load(Ordering::SeqCst) > 0);
    assert_eq!(
        waiter.as_mut().poll(&mut cx),
        Poll::Ready(Err(Readiness::Draining))
    );
    assert!(!handle.mark_ready());
    assert!(
        completion.as_mut().poll(&mut cx).is_pending(),
        "abandonment must not fabricate a completion report"
    );
}

struct ObserveAbandonment(ShutdownHandle);

impl Drop for ObserveAbandonment {
    fn drop(&mut self) {
        assert!(self.0.is_draining());
        assert!(self.0.operation_token().is_cancelled());
    }
}

#[test]
fn abandonment_signals_before_dropping_inert_application_captures() {
    for transfer in [false, true] {
        let mut supervisor = supervisor();
        let captured = ObserveAbandonment(supervisor.handle());
        supervisor
            .register("inert-component", move |_| {
                let _capture = captured;
                panic!("abandonment must not start a component");
                #[allow(unreachable_code)]
                async {
                    Ok(())
                }
            })
            .unwrap();
        let cleanup_capture = ObserveAbandonment(supervisor.handle());
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
        if transfer {
            drop(supervisor.run_until(pending()));
        } else {
            drop(supervisor);
        }
    }
}

#[test]
fn unpolled_driver_retains_startup_ownership_until_dropped() {
    let supervisor = supervisor();
    let process = supervisor.process_handle().unwrap();
    let handle = supervisor.handle();
    assert!(handle.mark_ready());
    let driver = supervisor.run_until(pending());
    assert_eq!(handle.readiness(), Readiness::Starting);
    assert!(!handle.operation_token().is_cancelled());
    assert!(matches!(
        process.try_spawn("before-poll", |_| async { Ok::<_, Infallible>(()) }),
        Err(ProcessAdmissionError::NotRunning)
    ));
    drop(driver);
    assert_eq!(handle.readiness(), Readiness::Draining);
    assert!(handle.operation_token().is_cancelled());
}

#[tokio::test]
async fn extracted_cleanup_completes_after_supervisor_cancels_operations() {
    let mut supervisor = supervisor();
    let operation = supervisor.handle().operation_token();
    let observed_operation = operation.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    supervisor
        .on_cleanup("resource", move || async move {
            assert!(observed_operation.is_cancelled());
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
    assert!(operation.is_cancelled());
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
                    Ok::<_, Infallible>(())
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
    let mut driver = Box::pin(supervisor.run_until(pending()));
    poll_fn(|cx| {
        assert!(driver.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_rejections(&process, ProcessAdmissionError::NotReady);
    assert!(handle.mark_ready());
    // Keep the admitted task queued so its only permit cannot be released yet.
    let receipt = process
        .try_spawn("queued", |_| async { Ok::<_, Infallible>(7) })
        .unwrap();
    assert_rejections(&process, ProcessAdmissionError::Full);
    handle.request();
    assert_rejections(&process, ProcessAdmissionError::Closed);
    assert!(driver.await.is_success());
    assert_eq!(receipt.wait().await.unwrap(), 7);
    assert_eq!(handle.readiness(), Readiness::Stopped);
    assert_rejections(&process, ProcessAdmissionError::Closed);
}
