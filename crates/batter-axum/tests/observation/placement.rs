use super::{
    capture::Capture,
    support::{Boundary, assert_completion, completions, request},
};
use axum::{Router, middleware, routing::get};
use batter::lifecycle::ShutdownHandle;
use batter_axum::{RequestPolicy, observe_http};
use std::time::Duration;
use tower::{Layer, ServiceExt};
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn router_layer_covers_existing_routes_and_fallback_but_not_later_routes() {
    let router = Router::new()
        .route("/records/{id}", get(|| async { "ok" }))
        .layer(middleware::from_fn(observe_http))
        .route("/later", get(|| async { "ok" }));
    for (path, status, route) in [
        ("/records/secret-id", 200, "/records/{id}"),
        ("/secret-missing", 404, "<unmatched>"),
        ("/later", 200, ""),
    ] {
        let capture = Capture::new();
        let response = router
            .clone()
            .oneshot(request("GET", path))
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        let text = capture.text();
        if path == "/later" {
            assert!(text.is_empty(), "{text}");
        } else {
            assert_completion(
                &text,
                status,
                if status == 404 {
                    "client_error"
                } else {
                    "completed"
                },
                route,
                "GET",
            );
        }
    }
}

#[tokio::test]
async fn service_wrapper_before_routing_has_no_matched_path_at_entry() {
    let capture = Capture::new();
    let service = middleware::from_fn(observe_http)
        .layer(Router::new().route("/records/{id}", get(|| async { "ok" })));
    let response = service
        .oneshot(request("GET", "/records/secret-id"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_completion(&capture.text(), 200, "completed", "<unmatched>", "GET");
}

#[tokio::test]
async fn legacy_wrapper_and_outer_observer_each_emit_their_own_completion() {
    let capture = Capture::new();
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = Boundary::Combined
        .apply(
            Router::new().route("/work", get(|| async { "ok" })),
            RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
        )
        .layer(middleware::from_fn(observe_http));
    let response = router
        .oneshot(request("GET", "/work"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = capture.text();
    assert_eq!(completions(&text).len(), 2, "{text}");
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
}

#[tokio::test]
async fn admission_alone_does_not_emit_http_observations() {
    let capture = Capture::new();
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = Boundary::Admission.apply(
        Router::new().route("/work", get(|| async { "ok" })),
        RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
    );
    let response = router
        .oneshot(request("GET", "/work"))
        .with_subscriber(capture.dispatch.clone())
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = capture.text();
    assert!(completions(&text).is_empty(), "{text}");
    assert!(!text.contains("batter.http"), "{text}");
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
}
