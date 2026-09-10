#[path = "telemetry/filtered.rs"]
mod filtered;

#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;

use batter::operation::OperationContext;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::Instrument;
use tracing::instrument::WithSubscriber;

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn outcome_is_recorded_without_logging_application_error_text() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = Buffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let context = OperationContext::new(Duration::from_secs(1)).unwrap();
    let result: Result<(), _> = context
        .run("telemetry.failure", |_| async {
            Err(std::io::Error::other("secret-that-must-not-be-logged"))
        })
        .with_subscriber(test_dispatch::new(subscriber))
        .await;
    assert!(result.is_err());
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(text.contains("telemetry.failure"));
    assert!(text.contains("failed"));
    assert!(text.contains("WARN"));
    assert!(text.contains("operation boundary finished"));
    assert!(text.contains("elapsed_ms="));
    assert!(!text.contains("secret-that-must-not-be-logged"));
}

#[tokio::test]
async fn ordinary_info_subscriber_observes_success_interruption_and_drop_without_span_events() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = Buffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    async {
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        assert_eq!(
            context
                .run("telemetry.success", |_| async { Ok::<_, io::Error>(42) })
                .await
                .unwrap(),
            42
        );
        context.cancel();
        assert!(
            context
                .run("telemetry.cancelled", |_| async { Ok::<_, io::Error>(()) })
                .await
                .is_err()
        );
        assert!(
            OperationContext::at(tokio::time::Instant::now())
                .run("telemetry.expired", |_| async { Ok::<_, io::Error>(()) })
                .await
                .is_err()
        );
        let context = OperationContext::new(Duration::from_secs(1)).unwrap();
        let run = context.run("telemetry.dropped", |_| {
            std::future::pending::<Result<(), io::Error>>()
        });
        tokio::pin!(run);
        tokio::select! {
            biased;
            _ = &mut run => panic!("pending operation cannot complete"),
            _ = tokio::task::yield_now() => {},
        }
    }
    .with_subscriber(test_dispatch::new(subscriber))
    .await;
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for (operation, outcome, level) in [
        ("telemetry.success", "succeeded", "INFO"),
        ("telemetry.cancelled", "cancelled", "INFO"),
        ("telemetry.expired", "deadline_exceeded", "WARN"),
        ("telemetry.dropped", "dropped", "WARN"),
    ] {
        let line = text.lines().find(|line| line.contains(operation)).unwrap();
        assert!(line.contains(outcome), "{line}");
        assert!(line.contains(level), "{line}");
        assert!(line.contains("elapsed_ms="), "{line}");
    }
}

#[tokio::test]
async fn cleanup_events_preserve_parent_context_and_hide_error_contents_with_info_subscriber() {
    use batter::cleanup::{CleanupBudget, CleanupOutcome, CleanupStack};
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = Buffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    async {
        let mut cleanup = CleanupStack::new();
        cleanup
            .push("resource.close", || async {
                tracing::info!("inside resource finalizer");
                Ok(())
            })
            .unwrap();
        cleanup
            .push("provider.close", || async {
                Err(io::Error::other("secret-cleanup-error").into())
            })
            .unwrap();
        let report = cleanup
            .close(
                CleanupBudget::new(
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                )
                .unwrap(),
            )
            .instrument(tracing::info_span!(
                "application.shutdown",
                instance = "test-instance"
            ))
            .await;
        assert_eq!(report.records.len(), 2);
        assert_eq!(report.records[0].outcome, CleanupOutcome::Failed);
        assert_eq!(report.records[1].outcome, CleanupOutcome::Succeeded);
    }
    .with_subscriber(test_dispatch::new(subscriber))
    .await;
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for (name, outcome, level) in [
        ("resource.close", "Succeeded", "INFO"),
        ("provider.close", "Failed", "WARN"),
    ] {
        let line = text
            .lines()
            .find(|line| line.contains("cleanup observed") && line.contains(name))
            .unwrap();
        assert!(line.contains(outcome), "{line}");
        assert!(line.contains(level), "{line}");
        assert!(line.contains("application.shutdown"), "{line}");
    }
    let finalizer = text
        .lines()
        .find(|line| line.contains("inside resource finalizer"))
        .unwrap_or_else(|| panic!("spawned finalizer must inherit its subscriber: {text}"));
    assert!(finalizer.contains("application.shutdown"), "{finalizer}");
    assert!(finalizer.contains("batter.cleanup"), "{finalizer}");
    assert!(finalizer.contains("resource.close"), "{finalizer}");
    assert!(!text.contains("secret-cleanup-error"), "{text}");
}

#[tokio::test]
async fn finite_work_keeps_submitter_telemetry_after_receipt_drop_without_reparenting_the_driver() {
    use batter::{
        cleanup::CleanupBudget,
        lifecycle::{ShutdownBudget, Supervisor},
    };
    use std::convert::Infallible;

    let driver_output = Arc::new(Mutex::new(Vec::new()));
    let request_output = Arc::new(Mutex::new(Vec::new()));
    let dispatch = |output: Arc<Mutex<Vec<u8>>>| {
        let writer = Buffer(output);
        crate::test_dispatch::new(
            tracing_subscriber::fmt()
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .without_time()
                .with_max_level(tracing::Level::INFO)
                .finish(),
        )
    };
    let driver_dispatch = dispatch(driver_output.clone());
    let request_dispatch = dispatch(request_output.clone());
    let second = Duration::from_secs(1);
    let budget = ShutdownBudget::new(
        second,
        second,
        second,
        CleanupBudget::new(second, second, second).unwrap(),
    )
    .unwrap();
    let mut supervisor = Supervisor::with_process_capacity(budget, 1).unwrap();
    let process = supervisor.process_handle().unwrap();
    supervisor
        .register("component", |signal| async move {
            tracing::info!("driver component started");
            signal.mark_started();
            signal.draining().await;
            tracing::info!("driver component stopping");
            Ok(())
        })
        .unwrap();
    supervisor
        .on_cleanup("resource.close", || async {
            tracing::info!("driver finalizer invoked");
            Ok(())
        })
        .unwrap();
    supervisor.handle().mark_ready();
    let running = tracing::dispatcher::with_default(&driver_dispatch, || {
        let span = tracing::info_span!("driver.scope", driver_marker = "driver-1");
        let _entered = span.enter();
        supervisor.start()
    });
    running.handle().wait_ready().await.unwrap();

    let (release, released) = tokio::sync::oneshot::channel();
    let receipt = tracing::dispatcher::with_default(&request_dispatch, || {
        let span = tracing::info_span!("request.scope", request_marker = "request-17");
        let _entered = span.enter();
        process
            .try_spawn("correlated.work", |_| async move {
                released.await.unwrap();
                tracing::info!("finite callback invoked");
                Ok::<_, Infallible>(42)
            })
            .unwrap()
    });
    // Its submitting span/subscriber and result waiter end before work resumes.
    drop(receipt);
    release.send(()).unwrap();
    let report = running.shutdown().await.unwrap();
    assert!(report.is_success());
    assert_eq!(report.completed_process_tasks, 1);

    let request = String::from_utf8(request_output.lock().unwrap().clone()).unwrap();
    let driver = String::from_utf8(driver_output.lock().unwrap().clone()).unwrap();
    let callback = request
        .lines()
        .find(|line| line.contains("finite callback invoked"))
        .unwrap_or_else(|| {
            panic!("task lost its submitting subscriber: {request}\ndriver: {driver}")
        });
    assert!(callback.contains("request.scope"), "{callback}");
    assert!(callback.contains("request-17"), "{callback}");
    assert!(callback.contains("batter.process_task"), "{callback}");
    assert!(!callback.contains("driver.scope"), "{callback}");
    assert!(!driver.contains("finite callback invoked"), "{driver}");
    for event in [
        "driver component started",
        "driver component stopping",
        "shutdown drain started",
        "driver finalizer invoked",
    ] {
        let line = driver.lines().find(|line| line.contains(event)).unwrap();
        assert!(line.contains("driver.scope"), "{line}");
        assert!(line.contains("driver-1"), "{line}");
        assert!(!line.contains("request.scope"), "{line}");
        assert!(!request.contains(event), "{request}");
    }
}
