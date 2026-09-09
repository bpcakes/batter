use super::{
    capture::Capture,
    composition::identity,
    support::{Boundary, assert_completion, completions, request},
};
use axum::{
    Extension, Router,
    body::to_bytes,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter::lifecycle::ShutdownHandle;
use batter_axum::{HttpObservationLevel, RequestPolicy, observe_http, readiness};
use std::time::Duration;
use tower::ServiceExt;
use tracing::{Level, instrument::WithSubscriber};

fn assert_level(capture: &Capture, level: Level) {
    let text = capture.text();
    let events = completions(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert!(
        events[0].trim_start().starts_with(&format!("{level} ")),
        "{text}"
    );
}

#[test]
fn explicit_response_levels_preserve_status_fields_identity_and_response() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        for level in [
            Level::ERROR,
            Level::WARN,
            Level::INFO,
            Level::DEBUG,
            Level::TRACE,
        ] {
            let capture = Capture::with_max_level(Level::TRACE);
            let ambient = Capture::new();
            ambient.block_on(async {
                let handle = ShutdownHandle::new();
                handle.mark_ready();
                let router = mode
                    .apply(
                        Router::new().route(
                            "/records/{id}",
                            get(move || async move {
                                (
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    Extension(HttpObservationLevel(level)),
                                    [("x-private", "secret-response-header")],
                                    "secret-response-body",
                                )
                            }),
                        ),
                        RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
                    )
                    .layer(middleware::from_fn(identity));
                let response = router
                    .oneshot(request("GET", "/records/secret-id?secret-query=1"))
                    .with_subscriber(capture.dispatch.clone())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(response.headers()["x-private"], "secret-response-header");
                assert_eq!(response.headers()["x-request-id"], "trusted-request-17");
                assert_eq!(
                    response.extensions().get::<HttpObservationLevel>(),
                    Some(&HttpObservationLevel(level))
                );
                assert_eq!(
                    to_bytes(response.into_body(), 4096).await.unwrap(),
                    "secret-response-body"
                );
            });
            assert_completion(&capture.text(), 503, "server_error", "/records/{id}", "GET");
            assert_level(&capture, level);
            assert!(capture.text().contains("trusted-request-17"));
            assert!(ambient.text().is_empty(), "{}", ambient.text());
        }
    }
}

#[test]
fn defaults_ignore_request_extensions_and_client_level_headers() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        for (status, outcome, level) in [
            (StatusCode::OK, "completed", Level::INFO),
            (StatusCode::FOUND, "completed", Level::INFO),
            (StatusCode::NOT_FOUND, "client_error", Level::INFO),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                Level::WARN,
            ),
            (StatusCode::SERVICE_UNAVAILABLE, "server_error", Level::WARN),
        ] {
            let capture = Capture::new();
            capture.block_on(async {
                let handle = ShutdownHandle::new();
                handle.mark_ready();
                let router = mode.apply(
                    Router::new().route("/work", get(move || async move { status })),
                    RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
                );
                let mut request = request("GET", "/work");
                request
                    .extensions_mut()
                    .insert(HttpObservationLevel(Level::TRACE));
                request
                    .headers_mut()
                    .insert("x-observation-level", "TRACE".parse().unwrap());
                let response = router.oneshot(request).await.unwrap();
                assert_eq!(response.status(), status);
                assert!(
                    response
                        .extensions()
                        .get::<HttpObservationLevel>()
                        .is_none()
                );
            });
            assert_completion(&capture.text(), status.as_u16(), outcome, "/work", "GET");
            assert_level(&capture, level);
        }
    }
}

#[test]
fn readiness_policy_is_explicit_and_does_not_demote_other_unguarded_failures() {
    for phase in ["starting", "ready", "draining"] {
        for (path, expected_level) in [("/ready", Level::INFO), ("/failure", Level::WARN)] {
            let capture = Capture::new();
            capture.block_on(async {
                let handle = ShutdownHandle::new();
                if phase != "starting" {
                    handle.mark_ready();
                }
                if phase == "draining" {
                    handle.request();
                }
                let router = Router::new()
                    .route(
                        "/ready",
                        get(|state| async move {
                            // This application treats its lifecycle probe as informational.
                            (
                                Extension(HttpObservationLevel(Level::INFO)),
                                readiness(state).await,
                            )
                        }),
                    )
                    .route(
                        "/failure",
                        get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
                    )
                    .with_state(handle)
                    .layer(middleware::from_fn(observe_http));
                let response = router.oneshot(request("GET", path)).await.unwrap();
                let status = if path == "/ready" && phase == "ready" {
                    200
                } else {
                    503
                };
                assert_eq!(response.status().as_u16(), status);
                assert_completion(
                    &capture.text(),
                    status,
                    if status == 200 {
                        "completed"
                    } else {
                        "server_error"
                    },
                    path,
                    "GET",
                );
                assert!(!capture.text().contains("operation boundary finished"));
            });
            assert_level(&capture, expected_level);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn failure_renderers_select_severity_for_admission_and_deadline_responses() {
    for mode in [Boundary::Split, Boundary::Combined] {
        for phase in ["starting", "draining", "deadline"] {
            let capture = Capture::new();
            let handle = ShutdownHandle::new();
            if phase != "starting" {
                handle.mark_ready();
            }
            if phase == "draining" {
                handle.request();
            }
            let policy = RequestPolicy::new(handle, Duration::from_secs(1))
                .unwrap()
                .with_failure_renderer(|failure, _parts| {
                    (
                        failure.status(),
                        Extension(HttpObservationLevel(Level::INFO)),
                        [("x-failure-code", failure.code())],
                        "secret-renderer-body",
                    )
                        .into_response()
                });
            let router = mode.apply(
                Router::new().route(
                    "/work",
                    get(move || async move {
                        assert_eq!(
                            phase, "deadline",
                            "admission must reject before the handler"
                        );
                        std::future::pending::<StatusCode>().await
                    }),
                ),
                policy,
            );
            let response = router
                .oneshot(request("GET", "/work"))
                .with_subscriber(capture.dispatch.clone())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                response.headers()["x-failure-code"],
                if phase == "deadline" {
                    "deadline_exceeded"
                } else {
                    "service_unavailable"
                }
            );
            assert_eq!(
                to_bytes(response.into_body(), 4096).await.unwrap(),
                "secret-renderer-body"
            );
            assert_completion(&capture.text(), 503, "server_error", "/work", "GET");
            assert_level(&capture, Level::INFO);
        }
    }
}

async fn replace_status_and_level(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
        .extensions_mut()
        .insert(HttpObservationLevel(Level::ERROR));
    response
}

async fn remove_level(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.extensions_mut().remove::<HttpObservationLevel>();
    response
}

#[test]
fn inner_response_middleware_can_replace_or_remove_an_override() {
    for remove in [false, true] {
        let capture = Capture::new();
        capture.block_on(async {
            let mut router = Router::new()
                .route(
                    "/work",
                    get(|| async {
                        (Extension(HttpObservationLevel(Level::INFO)), StatusCode::OK)
                    }),
                )
                .layer(middleware::from_fn(replace_status_and_level));
            if remove {
                router = router.layer(middleware::from_fn(remove_level));
            }
            let response = router
                .layer(middleware::from_fn(observe_http))
                .oneshot(request("GET", "/work"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        });
        assert_completion(&capture.text(), 500, "server_error", "/work", "GET");
        assert_level(&capture, if remove { Level::WARN } else { Level::ERROR });
    }
}
