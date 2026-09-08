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
        .with_subscriber(subscriber)
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
    .with_subscriber(subscriber)
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
    .with_subscriber(subscriber)
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
        tracing::Dispatch::new(
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

#[cfg(feature = "axum")]
#[tokio::test]
async fn http_observes_actual_failure_status_and_nested_context_without_untrusted_values() {
    use axum::{
        Extension, Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::any,
    };
    use batter::{
        http::{RequestPolicy, request_scope},
        lifecycle::ShutdownHandle,
    };
    use tower::ServiceExt;

    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = Buffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    async {
        let handle = ShutdownHandle::new();
        handle.mark_ready();
        let router = Router::new()
            .route(
                "/records/{id}",
                any(
                    |Extension(context): Extension<OperationContext>| async move {
                        let result: Result<(), _> = context
                            .run("domain.load", |_| async {
                                tracing::info!("nested dependency called");
                                Err(io::Error::other("secret-provider-error"))
                            })
                            .await;
                        assert!(result.is_err());
                        (StatusCode::INTERNAL_SERVER_ERROR, "secret-response-body")
                    },
                ),
            )
            .layer(middleware::from_fn_with_state(
                RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
                request_scope,
            ));
        for method in ["GET", "SECRET-CUSTOM-METHOD"] {
            let request = Request::builder()
                .method(method)
                .uri("/records/secret-record-id?secret-query=value")
                .header("authorization", "secret-authorization")
                .header("x-request-id", "secret-client-request-id")
                .body(Body::empty())
                .unwrap();
            let response = router
                .clone()
                .oneshot(request)
                .instrument(tracing::info_span!(
                    "application.request",
                    request_id = "trusted-request-17"
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        }
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/secret-unmatched-path?secret-query=value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    .with_subscriber(subscriber)
    .await;
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    let completions: Vec<_> = text
        .lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect();
    assert_eq!(completions.len(), 3, "{text}");
    for line in &completions[..2] {
        assert!(line.contains("status=500"), "{line}");
        assert!(line.contains("http_outcome=\"server_error\""), "{line}");
        assert!(line.contains("WARN"), "{line}");
        assert!(line.contains("route=\"/records/{id}\""), "{line}");
        assert!(line.contains("latency_ms="), "{line}");
        assert!(line.contains("trusted-request-17"), "{line}");
    }
    assert!(completions[0].contains("method=\"GET\""));
    assert!(completions[1].contains("method=\"OTHER\""));
    assert!(completions[2].contains("route=\"<unmatched>\""));
    assert!(completions[2].contains("status=404"));
    assert!(completions[2].contains("http_outcome=\"client_error\""));
    assert!(completions[2].contains("INFO"));
    for line in text
        .lines()
        .filter(|line| line.contains("nested dependency called"))
    {
        assert!(line.contains("application.request"), "{line}");
        assert!(line.contains("batter.http"), "{line}");
        assert!(line.contains("domain.load"), "{line}");
    }
    assert_eq!(text.matches("nested dependency called").count(), 2);
    for secret in ["secret-", "SECRET-CUSTOM-METHOD"] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn readiness_rejection_has_http_status_telemetry_before_any_handler_runs() {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use batter::{
        http::{RequestPolicy, request_scope},
        lifecycle::ShutdownHandle,
    };
    use tower::ServiceExt;

    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = Buffer(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let router = Router::new()
        .route("/work", get(|| async { "must not execute" }))
        .layer(middleware::from_fn_with_state(
            RequestPolicy::new(ShutdownHandle::new(), Duration::from_secs(1)).unwrap(),
            request_scope,
        ));
    let response = router
        .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
        .with_subscriber(subscriber)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    let line = text
        .lines()
        .find(|line| line.contains("HTTP response boundary finished"))
        .unwrap();
    assert!(line.contains("status=503"), "{line}");
    assert!(line.contains("http_outcome=\"server_error\""), "{line}");
    assert!(!text.contains("http.response_construction"), "{text}");
}
