use super::{
    capture::Capture,
    composition::identity,
    support::{Boundary, assert_completion, assert_no_secrets, completions, request},
};
use axum::{
    Extension, Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use batter::lifecycle::ShutdownHandle;
use batter_axum::{HttpObservationLevel, RequestPolicy, observe_http};
use std::time::Duration;
use tower::ServiceExt;
use tracing::Level;

fn ready_policy() -> RequestPolicy {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    RequestPolicy::new(handle, Duration::from_secs(1)).unwrap()
}

#[test]
fn response_overrides_below_info_are_filtered_without_changing_the_response() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        // WARN is a positive control for this same subscriber and response.
        for level in [Level::WARN, Level::DEBUG, Level::TRACE] {
            let capture = Capture::new();
            capture.block_on(async {
                let app = mode.apply(
                    Router::new().route(
                        "/work",
                        get(move || async move {
                            tracing::info!("handler returned");
                            (
                                Extension(HttpObservationLevel(level)),
                                StatusCode::SERVICE_UNAVAILABLE,
                            )
                        }),
                    ),
                    ready_policy(),
                );
                let response = app.oneshot(request("GET", "/work")).await.unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(
                    response.extensions().get::<HttpObservationLevel>(),
                    Some(&HttpObservationLevel(level))
                );
            });
            let text = capture.text();
            assert!(text.contains("handler returned"), "{text}");
            if level == Level::WARN {
                assert_completion(&text, 503, "server_error", "/work", "GET");
                assert!(
                    completions(&text)[0].trim_start().starts_with("WARN "),
                    "{text}"
                );
            } else {
                assert!(completions(&text).is_empty(), "{text}");
            }
        }
    }
}

async fn rewrite_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
        .extensions_mut()
        .insert(HttpObservationLevel(Level::ERROR));
    response
}

#[test]
fn outer_rewriting_does_not_retroactively_change_the_observed_response() {
    for mode in [Boundary::Observation, Boundary::Split, Boundary::Combined] {
        let capture = Capture::new();
        capture.block_on(async {
            let app = mode
                .apply(
                    Router::new().route(
                        "/work",
                        get(|| async {
                            (Extension(HttpObservationLevel(Level::INFO)), StatusCode::OK)
                        }),
                    ),
                    ready_policy(),
                )
                .layer(middleware::from_fn(rewrite_response));
            let response = app.oneshot(request("GET", "/work")).await.unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(
                response.extensions().get::<HttpObservationLevel>(),
                Some(&HttpObservationLevel(Level::ERROR))
            );
        });
        let text = capture.text();
        assert_completion(&text, 200, "completed", "/work", "GET");
        assert!(
            completions(&text)[0].trim_start().starts_with("INFO "),
            "{text}"
        );
        assert!(!text.contains("status=500"), "{text}");
    }
}

#[test]
fn each_nested_observer_inherits_the_retained_response_override() {
    for inner in [Boundary::Observation, Boundary::Combined] {
        for level in [Level::INFO, Level::ERROR] {
            let capture = Capture::new();
            capture.block_on(async {
                let app = inner
                    .apply(
                        Router::new().route(
                            "/work",
                            get(move || async move {
                                (
                                    Extension(HttpObservationLevel(level)),
                                    StatusCode::SERVICE_UNAVAILABLE,
                                )
                            }),
                        ),
                        ready_policy(),
                    )
                    .layer(middleware::from_fn(observe_http))
                    .layer(middleware::from_fn(identity));
                let response = app.oneshot(request("GET", "/work")).await.unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(response.headers()["x-request-id"], "trusted-request-17");
                assert_eq!(
                    response.extensions().get::<HttpObservationLevel>(),
                    Some(&HttpObservationLevel(level))
                );
            });
            let text = capture.text();
            assert_no_secrets(&text);
            let events = completions(&text);
            assert_eq!(events.len(), 2, "{text}");
            for event in events {
                assert_completion(event, 503, "server_error", "/work", "GET");
                assert!(
                    event.trim_start().starts_with(&format!("{level} ")),
                    "{text}"
                );
                assert!(event.contains("trusted-request-17"), "{text}");
            }
        }
    }
}
