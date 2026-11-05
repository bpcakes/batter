use super::*;
use batter_core::startup::Startup;
use std::future::{Future, poll_fn};
use std::task::Poll;
use tokio::sync::oneshot;

#[tokio::test]
async fn unpolled_finish_requests_drain_before_initializer_can_run() {
    let events = Events::default();
    let cleanup_events = events.clone();
    let called = Arc::new(AtomicBool::new(false));
    let initializer_called = called.clone();
    let mut process = supervisor();
    process
        .reserve_cleanup("resource")
        .unwrap()
        .register(move || async move {
            cleanup_events.lock().unwrap().push("cleanup");
            Ok(())
        });
    let starting = Startup::protected(process, context(), cleanup_budget(), async move |_| {
        initializer_called.store(true, Ordering::SeqCst);
        Ok::<_, RegistrationError>(())
    })
    .start();
    let observer = starting.observer();
    drop(starting.finish());

    let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
        panic!("unpolled finish must retain failed-startup cleanup")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert!(!called.load(Ordering::SeqCst));
    assert!(report.cleanup.is_success());
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);
}

#[tokio::test]
async fn dropping_pending_finish_retains_cleanup_failure() {
    let events = Events::default();
    let cleanup_events = events.clone();
    let (entered_tx, entered_rx) = oneshot::channel();
    let starting = Startup::protected(
        supervisor(),
        context(),
        cleanup_budget(),
        async move |scope| {
            scope
                .reserve_cleanup("resource")?
                .register(move || async move {
                    cleanup_events.lock().unwrap().push("cleanup");
                    Err(io::Error::other("retained cleanup failure").into())
                });
            entered_tx.send(()).unwrap();
            std::future::pending::<Result<(), RegistrationError>>().await
        },
    )
    .start();
    let observer = starting.observer();
    let mut finish = Box::pin(starting.finish());
    entered_rx.await.unwrap();
    assert!(
        poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(finish);

    let StartupOutcome::Failed(StartupError::Failed(report)) = observer.wait().await else {
        panic!("pending finish abandonment must retain startup failure")
    };
    assert!(matches!(report.cause, StartupCause::Draining));
    assert_eq!(*events.lock().unwrap(), ["cleanup"]);
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(
        report.cleanup.records[0]
            .error
            .as_ref()
            .unwrap()
            .to_string(),
        "retained cleanup failure"
    );
}

#[tokio::test(start_paused = true)]
async fn typed_finish_resumes_after_borrowed_timeout_without_losing_output() {
    let events = Events::default();
    let work_events = events.clone();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let process = supervisor();
    let status = process.handle().status();
    let starting = Startup::protected(process, context(), cleanup_budget(), async move |scope| {
        record_cleanup(scope, work_events.clone());
        entered_tx.send(()).unwrap();
        release_rx.await.unwrap();
        scope
            .registration()
            .register("worker", |startup| async move {
                let shutdown = startup.acknowledge_started();
                shutdown.draining().await;
                Ok(shutdown.stopped())
            })?;
        Ok::<_, RegistrationError>(Output {
            value: 42,
            events: work_events,
        })
    })
    .start();
    let finish = starting.finish_with_output();
    tokio::pin!(finish);
    entered_rx.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut finish)
            .await
            .is_err()
    );
    assert_eq!(status.readiness(), Readiness::Starting);
    assert!(events.lock().unwrap().is_empty());

    release_tx.send(()).unwrap();
    let (running, output) = finish.await.unwrap();
    assert_eq!(output.value, 42);
    assert!(events.lock().unwrap().is_empty());
    assert!(running.shutdown().await.unwrap().is_success());
    drop(output);
    assert_eq!(*events.lock().unwrap(), ["cleanup", "output destroyed"]);
}
