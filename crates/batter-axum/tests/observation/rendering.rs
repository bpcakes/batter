use super::{
    capture::Capture,
    composition::{TrustedId, identity},
    support::{Boundary, assert_completion, request},
};
use axum::{
    Extension, Router,
    body::to_bytes,
    extract::Request,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use batter::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, ShutdownHandle, Supervisor},
    operation::{Interruption, OperationContext},
};
use batter_axum::{HttpObservationLevel, RequestPolicy};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

fn policy(handle: ShutdownHandle, budget: Duration) -> RequestPolicy {
    RequestPolicy::new(handle, budget)
        .unwrap()
        .with_failure_renderer(|failure, parts| {
            let id = parts.extensions.get::<TrustedId>().unwrap().0;
            (
                StatusCode::TOO_MANY_REQUESTS,
                Extension(HttpObservationLevel(tracing::Level::ERROR)),
                [("x-failure-code", failure.code()), ("x-renderer-id", id)],
                format!("{}:{id}", failure.code()),
            )
                .into_response()
        })
}

async fn assert_rendered(response: Response, capture: &Capture, code: &str) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["x-request-id"], "trusted-request-17");
    assert_eq!(response.headers()["x-renderer-id"], "trusted-request-17");
    assert_eq!(response.headers()["x-failure-code"], code);
    assert!(!response.headers().contains_key("retry-after"));
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        format!("{code}:trusted-request-17")
    );
    let text = capture.text();
    assert_completion(&text, 429, "client_error", "/work", "GET");
    assert!(
        text.lines()
            .any(|line| line.trim_start().starts_with("ERROR ")
                && line.contains("HTTP response boundary finished")),
        "{text}"
    );
    assert!(text.contains("trusted-request-17"), "{text}");
}

#[tokio::test]
async fn custom_readiness_and_drain_rejections_preserve_responses_and_observe_actual_status() {
    async fn forbidden_handler() -> &'static str {
        panic!("rejected handler must not run")
    }
    for mode in [Boundary::Split, Boundary::Combined] {
        for draining in [false, true] {
            let capture = Capture::new();
            let handle = ShutdownHandle::new();
            if draining {
                handle.mark_ready();
                handle.request();
            }
            let router = mode
                .apply(
                    Router::new().route("/work", get(forbidden_handler)),
                    policy(handle, Duration::from_secs(1)),
                )
                .layer(middleware::from_fn(identity));
            let response = router
                .oneshot(request("GET", "/work"))
                .with_subscriber(capture.dispatch.clone())
                .await
                .unwrap();
            assert_rendered(response, &capture, "service_unavailable").await;
            assert!(!capture.text().contains("operation boundary finished"));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_retains_original_metadata_and_cancels_admitted_context() {
    for mode in [Boundary::Split, Boundary::Combined] {
        let capture = Capture::new();
        let handle = ShutdownHandle::new();
        handle.mark_ready();
        let escaped = Arc::new(Mutex::new(None));
        let saved = escaped.clone();
        let router = mode
            .apply(
                Router::new().route(
                    "/work",
                    get(move |mut request: Request| {
                        let saved = saved.clone();
                        async move {
                            request.extensions_mut().remove::<TrustedId>();
                            *saved.lock().unwrap() =
                                request.extensions_mut().remove::<OperationContext>();
                            std::future::pending::<&'static str>().await
                        }
                    }),
                ),
                policy(handle, Duration::from_secs(1)),
            )
            .layer(middleware::from_fn(identity));
        let response = router
            .oneshot(request("GET", "/work"))
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        assert_rendered(response, &capture, "deadline_exceeded").await;
        let context: OperationContext = escaped.lock().unwrap().take().unwrap();
        assert_eq!(context.check(), Err(Interruption::Cancelled));
        assert!(capture.text().contains("deadline_exceeded"));
    }
}

#[tokio::test(start_paused = true)]
async fn forced_cancellation_retains_original_metadata_and_actual_rendered_status() {
    for mode in [Boundary::Split, Boundary::Combined] {
        let capture = Capture::new();
        let one = Duration::from_secs(1);
        let mut supervisor = Supervisor::new(
            ShutdownBudget::new(
                Duration::ZERO,
                one,
                one,
                CleanupBudget::new(one, one, one).unwrap(),
            )
            .unwrap(),
        );
        let handle = supervisor.handle();
        supervisor
            .register("http-owner", |shutdown| async move {
                shutdown.mark_started();
                shutdown.cancelled().await;
                Ok(())
            })
            .unwrap();
        handle.mark_ready();
        let running = supervisor.start();
        handle.wait_ready().await.unwrap();
        let inside = handle.clone();
        let router = mode
            .apply(
                Router::new().route(
                    "/work",
                    get(move |mut request: Request| {
                        let inside = inside.clone();
                        async move {
                            request.extensions_mut().remove::<TrustedId>();
                            inside.request();
                            std::future::pending::<&'static str>().await
                        }
                    }),
                ),
                policy(handle, Duration::from_secs(60)),
            )
            .layer(middleware::from_fn(identity));
        let response = router
            .oneshot(request("GET", "/work"))
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        assert_rendered(response, &capture, "operation_cancelled").await;
        assert!(capture.text().contains("cancelled"));
        let report = running.wait().await.unwrap();
        assert!(report.forced_cancellation);
        assert!(report.is_success());
    }
}
