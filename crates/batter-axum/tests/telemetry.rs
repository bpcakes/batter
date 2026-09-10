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
async fn http_observes_actual_failure_status_and_nested_context_without_untrusted_values() {
    use axum::{
        Extension, Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::any,
    };
    use batter::lifecycle::ShutdownHandle;
    use batter_axum::{RequestPolicy, request_scope};
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
    .with_subscriber(test_dispatch::new(subscriber))
    .await;
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert_http_completion_events(&text);
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

fn assert_http_completion_events(text: &str) {
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
}

#[tokio::test]
async fn readiness_rejection_has_http_status_telemetry_before_any_handler_runs() {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use batter::lifecycle::ShutdownHandle;
    use batter_axum::{RequestPolicy, request_scope};
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
        .with_subscriber(test_dispatch::new(subscriber))
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
