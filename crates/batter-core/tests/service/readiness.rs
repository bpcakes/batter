use super::*;
use batter_core::service::{NoDiagnostics, ServiceReadinessError};
use tokio::time::{Instant, advance, timeout};

struct Output(Arc<AtomicUsize>);
impl Drop for Output {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn output_wait_is_resumable_and_readiness_survives_later_shutdown() {
    let drops = Arc::new(AtomicUsize::new(0));
    let output = Output(drops.clone());
    let (release, released) = oneshot::channel();
    let startup = Startup::protected(
        supervisor(),
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            scope
                .registration()
                .register("worker", |startup| async move {
                    released.await.unwrap();
                    let running = startup.acknowledge_started();
                    running.draining().await;
                    Ok(running.stopped())
                })?;
            Ok::<_, batter_core::RegistrationError>(output)
        },
    );
    let starting =
        service::start_with_readiness(startup, NoDiagnostics, Some(Instant::now() + SECOND * 10));
    let observer = starting.observer();
    let finish = starting.into_ready();
    tokio::pin!(finish);
    assert!(timeout(SECOND, &mut finish).await.is_err());
    assert_eq!(observer.status().readiness(), Readiness::Starting);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    let (owner, output) = finish.await.unwrap();
    assert_eq!(observer.wait_ready().await, Ok(()));
    advance(SECOND * 20).await;
    tokio::task::yield_now().await;
    assert_eq!(
        owner.status().readiness(),
        Readiness::Ready,
        "elapsed deadline cannot drain an already ready service"
    );
    owner.request_shutdown();
    let completion = observer.wait().await;
    assert_eq!(completion.readiness(), Ok(()));
    assert_eq!(observer.wait_ready().await, Ok(()));
    assert_eq!(observer.status().readiness(), Readiness::Stopped);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(output);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn deadline_covers_initializer_and_preserves_cleanup_and_diagnostic_failures() {
    let (entered, enter) = oneshot::channel();
    let startup = Startup::protected(
        supervisor(),
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            scope
                .reserve_cleanup("resource")?
                .register(|| async { Err(std::io::Error::other("retained cleanup error").into()) });
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
            Ok::<_, batter_core::RegistrationError>(7)
        },
    );
    let starting = service::start_with_readiness(
        startup,
        diagnostic(PanicAt::Final),
        Some(Instant::now() + SECOND),
    );
    let observer = starting.observer();
    enter.await.unwrap();
    assert_eq!(
        observer.wait_ready().await,
        Err(ServiceReadinessError::DeadlineExceeded)
    );
    let error = match starting.into_ready().await {
        Ok(_) => panic!("unexpected output"),
        Err(error) => error,
    };
    assert_eq!(
        error.completion().readiness(),
        Err(ServiceReadinessError::DeadlineExceeded)
    );
    let ServiceOutcome::StartupFailed(StartupError::Failed(failure)) = error.completion().service()
    else {
        panic!("startup failure lost")
    };
    assert_eq!(failure.cleanup.records[0].outcome, CleanupOutcome::Failed);
    assert!(failure.cleanup.records[0].error.is_some());
    assert!(matches!(
        error.completion().diagnostics(),
        DiagnosticOutcome::TaskFailed(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn late_acknowledgement_never_reopens_admission() {
    let process = supervisor();
    let status = process.status();
    let (ready, ready_rx) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (checked, check) = oneshot::channel();
    let deadline = Instant::now() + SECOND;
    let startup = Startup::protected(
        process,
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            scope
                .registration()
                .register("worker", |startup| async move {
                    ready.send(()).unwrap();
                    released.await.unwrap();
                    let running = startup.acknowledge_started();
                    checked.send(status.readiness()).unwrap();
                    running.draining().await;
                    Ok(running.stopped())
                })?;
            Ok::<_, batter_core::RegistrationError>(9)
        },
    );
    let starting = service::start_with_readiness(startup, NoDiagnostics, Some(deadline));
    let observer = starting.observer();
    ready_rx.await.unwrap();
    advance(SECOND).await;
    release.send(()).unwrap();
    assert_ne!(check.await.unwrap(), Readiness::Ready);
    assert_eq!(
        observer.wait_ready().await,
        Err(ServiceReadinessError::DeadlineExceeded)
    );
    assert!(starting.into_ready().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn past_deadline_skips_initializer_and_unpolled_owner_loss_still_settles() {
    for expired in [false, true] {
        let called = Arc::new(AtomicBool::new(false));
        let observed = called.clone();
        let startup = Startup::protected(
            supervisor(),
            OperationOwner::new(SECOND * 20).unwrap().into_context(),
            cleanup(),
            async move |_scope| {
                observed.store(true, Ordering::SeqCst);
                Ok::<_, std::io::Error>(42)
            },
        );
        let deadline = expired.then(Instant::now);
        let starting = service::start_with_readiness(startup, NoDiagnostics, deadline);
        let observer = starting.observer();
        drop(starting.into_ready());
        let completion = observer.wait().await;
        assert!(!called.load(Ordering::SeqCst));
        let expected = if expired {
            ServiceReadinessError::DeadlineExceeded
        } else {
            ServiceReadinessError::StoppedBeforeReady
        };
        assert_eq!(completion.readiness(), Err(expected));
        assert_eq!(observer.wait_ready().await, Err(expected));
        assert!(matches!(
            completion.diagnostics(),
            DiagnosticOutcome::Completed(())
        ));
    }
}

#[tokio::test(start_paused = true)]
async fn readiness_waiter_cancellation_does_not_cancel_service_or_reset_deadline() {
    let startup = Startup::protected(
        supervisor(),
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async |_scope| std::future::pending::<Result<u8, std::io::Error>>().await,
    );
    let beginning = Instant::now();
    let starting =
        service::start_with_readiness(startup, NoDiagnostics, Some(beginning + SECOND * 3));
    let observer = starting.observer();
    assert!(timeout(SECOND, observer.wait_ready()).await.is_err());
    assert_eq!(observer.status().readiness(), Readiness::Starting);
    assert_eq!(
        observer.wait_ready().await,
        Err(ServiceReadinessError::DeadlineExceeded)
    );
    assert_eq!(Instant::now(), beginning + SECOND * 3);
    assert!(starting.into_ready().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn rejected_output_panic_does_not_bypass_shutdown_or_diagnostics() {
    struct PanickingOutput;
    impl Drop for PanickingOutput {
        fn drop(&mut self) {
            panic!("private rejected output");
        }
    }
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleanup_seen = cleaned.clone();
    let startup = Startup::protected(
        supervisor(),
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async move |scope| {
            scope
                .reserve_cleanup("resource")?
                .register(move || async move {
                    cleaned.store(true, Ordering::SeqCst);
                    Ok(())
                });
            scope
                .registration()
                .register("worker", |startup| async move {
                    startup.shutdown().draining().await;
                    Ok(startup.abandon())
                })?;
            Ok::<_, batter_core::RegistrationError>(PanickingOutput)
        },
    );
    let starting =
        service::start_with_readiness(startup, NoDiagnostics, Some(Instant::now() + SECOND));
    let error = match starting.into_ready().await {
        Ok(_) => panic!("unexpected output"),
        Err(error) => error,
    };
    assert!(cleanup_seen.load(Ordering::SeqCst));
    assert!(error.completion().startup_output_failure().is_some());
    assert!(matches!(
        error.completion().diagnostics(),
        DiagnosticOutcome::Completed(())
    ));
    assert!(!format!("{:?}", error.completion()).contains("private rejected output"));
}

#[tokio::test(start_paused = true)]
async fn a_service_that_registers_nothing_never_reports_ready() {
    let startup = Startup::protected(
        supervisor(),
        OperationOwner::new(SECOND * 20).unwrap().into_context(),
        cleanup(),
        async move |_scope| Ok::<_, batter_core::RegistrationError>(7_u8),
    );
    let starting = service::start_with_readiness(startup, NoDiagnostics, None);
    let observer = starting.observer();
    let error = match starting.into_ready().await {
        Ok(_) => panic!("an empty supervisor reported readiness"),
        Err(error) => error,
    };
    let stopped = Err(ServiceReadinessError::StoppedBeforeReady);
    assert_eq!(error.completion().readiness(), stopped);
    assert_eq!(observer.wait_ready().await, stopped);
    assert_eq!(observer.wait().await.readiness(), stopped);
    let ServiceOutcome::Shutdown(Err(ShutdownFailure::Report(report))) =
        error.completion().service()
    else {
        panic!("the empty running service did not settle as a failed report")
    };
    assert_eq!(
        report.cause,
        batter_core::lifecycle::ShutdownCause::EmptySupervisor
    );
}
