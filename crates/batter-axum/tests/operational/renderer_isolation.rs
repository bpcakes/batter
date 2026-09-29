//! Renderer metadata can be reused as data, but cannot retain the original
//! request's private observation or operational ownership.

use crate::capture::Capture;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{Method, StatusCode, request::Parts},
    middleware,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use batter_axum::{
    BrowserPolicy, CorrelationId, GroupPolicy, GuardedRouter, HttpBoundary, ProbePath,
    ReadinessPolicy, RequestInterruptionResponder, RequestPolicy, ResponseConstructionBudget,
    browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
    operational_http, operational_http_with_quota,
    quota_observation::{QuotaRecorder, QuotaTerminalFacts},
};
use batter_core::{
    health::{HealthMonitor, HealthPolicy},
    lifecycle::ShutdownHandle,
    operation::Interruption,
};
use std::{
    future::Future,
    io,
    task::{Context, Poll, Waker},
    time::Duration,
};
use tower::ServiceExt;

const SECOND: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApplicationMetadata(&'static str);

#[derive(Clone, Copy)]
enum Surface {
    Liveness,
    Readiness,
    Admission,
    Interruption,
    Browser,
}

impl Surface {
    fn status(self) -> StatusCode {
        match self {
            Self::Liveness => StatusCode::OK,
            Self::Readiness | Self::Admission | Self::Interruption => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::Browser => StatusCode::FORBIDDEN,
        }
    }
}

/// Use only public APIs to try to affect the original observation indirectly.
/// The in-memory service must settle in one poll: no blocking, I/O or spawning
/// is needed to exercise a synchronous renderer's authority.
fn redispatch(parts: &Parts, quota: bool) -> Response {
    let original_id = parts.extensions.get::<CorrelationId>().unwrap().to_string();
    assert_eq!(parts.headers["x-request-id"], original_id);
    assert_eq!(
        parts.extensions.get::<ApplicationMetadata>(),
        Some(&ApplicationMetadata("kept"))
    );
    let mut request = Request::from_parts(parts.clone(), Body::empty());
    assert!(QuotaRecorder::take(&mut request).is_none());
    *request.uri_mut() = "/child".parse().unwrap();
    let inner = Router::new().route(
        "/child",
        any(move |mut request: Request| async move {
            let id = request
                .extensions()
                .get::<CorrelationId>()
                .unwrap()
                .to_string();
            assert_eq!(
                request.extensions().get::<ApplicationMetadata>(),
                Some(&ApplicationMetadata("kept"))
            );
            if quota {
                QuotaRecorder::take(&mut request)
                    .unwrap()
                    .start()
                    .finish(QuotaTerminalFacts::QuotaDenied);
            }
            ([("x-child-id", id)], "child")
        }),
    );
    let inner = if quota {
        inner.layer(middleware::from_fn(operational_http_with_quota))
    } else {
        inner.layer(middleware::from_fn(operational_http))
    };
    let mut future = Box::pin(inner.oneshot(request));
    let Poll::Ready(Ok(child)) = future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("in-memory child must complete synchronously");
    };
    let child_id = child.headers()["x-child-id"].to_str().unwrap().to_owned();
    // Assert outside the renderer, after the outer completion has also run.
    (
        [
            ("x-renderer-id", original_id),
            ("x-child-id", child_id),
            (
                "x-child-response-id",
                child
                    .headers()
                    .get("x-request-id")
                    .map(|v| v.to_str().unwrap())
                    .unwrap_or("missing")
                    .to_owned(),
            ),
        ],
        "rendered",
    )
        .into_response()
}

async fn app(surface: Surface, quota: bool) -> Router {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    if matches!(surface, Surface::Interruption | Surface::Browser) {
        approval.approve();
    }
    let policy = RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(SECOND).unwrap(),
    )
    .with_failure_renderer(move |failure, parts| {
        let mut response = redispatch(parts, quota);
        *response.status_mut() = failure.status();
        response
    });
    let mut boundary = HttpBoundary::new(policy.clone());
    match surface {
        Surface::Liveness => {
            boundary = boundary
                .with_rendered_liveness(ProbePath::new("/parent").unwrap(), move |parts| {
                    redispatch(parts, quota)
                })
                .unwrap();
        }
        Surface::Readiness => {
            let monitor = HealthMonitor::new(
                HealthPolicy::new(SECOND, SECOND, 3 * SECOND, SECOND).unwrap(),
                || async { Ok::<_, io::Error>(()) },
            );
            boundary = boundary
                .with_rendered_readiness(
                    ProbePath::new("/parent").unwrap(),
                    ReadinessPolicy::new(handle.status(), monitor.reader()),
                    move |_, parts| redispatch(parts, quota),
                )
                .unwrap();
        }
        Surface::Browser => {
            let browser = BrowserPolicy::with_mutation_checks(
                PrivateResponsePolicy::NoReferrer,
                MutationPolicy::exact_origin(BrowserOrigin::https("https://app.example").unwrap()),
                move |rejection, parts| {
                    let mut response = redispatch(parts, quota);
                    *response.status_mut() = rejection.status();
                    response
                },
            );
            boundary = HttpBoundary::new(GroupPolicy::browser(policy, browser));
        }
        Surface::Admission | Surface::Interruption => {}
    }
    let guarded = if matches!(surface, Surface::Liveness | Surface::Readiness) {
        GuardedRouter::new()
    } else {
        GuardedRouter::new().route(
            "/parent",
            get(
                |Extension(responder): Extension<RequestInterruptionResponder>| async move {
                    responder.render(Interruption::Cancelled)
                },
            )
            .post(unexpected_browser_handler),
        )
    };
    boundary
        .assemble(guarded)
        .await
        .unwrap()
        .into_router()
        .layer(middleware::from_fn(operational_http_with_quota))
}

async fn unexpected_browser_handler() -> Response {
    panic!("browser rejection must precede its handler");
}

fn check(surface: Surface) {
    // A fresh operational wrapper can rewrite correlation too; test both that
    // path and the quota wrapper's shared-record replacement.
    for quota in [false, true] {
        let capture = Capture::new();
        let (original_id, child_id, child_response_id) = capture.block_on(async {
            let method = if matches!(surface, Surface::Browser) {
                Method::POST
            } else {
                Method::GET
            };
            let mut request = Request::builder()
                .method(method)
                .uri("/parent")
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(ApplicationMetadata("kept"));
            let response = app(surface, quota).await.oneshot(request).await.unwrap();
            assert_eq!(response.status(), surface.status());
            let original_id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            assert_eq!(response.headers()["x-renderer-id"], original_id);
            let child_id = response.headers()["x-child-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let child_response_id = response.headers()["x-child-response-id"]
                .to_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                to_bytes(response.into_body(), 1024).await.unwrap(),
                "rendered"
            );
            (original_id, child_id, child_response_id)
        });
        let text = capture.text();
        let events: Vec<_> = text
            .lines()
            .filter_map(|line| {
                line.split_once("HTTP response boundary finished")
                    .map(|(_, fields)| fields)
            })
            .collect();
        assert_eq!(events.len(), 2, "{text}");
        assert_ne!(
            original_id, child_id,
            "child retained original ownership: {text}"
        );
        assert_eq!(
            child_response_id, child_id,
            "child did not own response rewriting: {text}"
        );
        let parent = events
            .iter()
            .find(|event| event.contains(&format!("request_id=\"{original_id}\"")))
            .unwrap();
        let child = events
            .iter()
            .find(|event| event.contains(&format!("request_id=\"{child_id}\"")))
            .unwrap();
        assert!(parent.contains("route=\"/parent\""), "{text}");
        assert!(parent.contains("quota_outcome=\"not_checked\""), "{text}");
        if quota {
            assert!(child.contains("quota_outcome=\"quota_denied\""), "{text}");
        } else {
            assert!(!child.contains("quota_outcome="), "{text}");
        }
    }
}

#[test]
fn liveness_renderer_redispatch_cannot_rewrite_original_observation() {
    check(Surface::Liveness);
}

#[test]
fn readiness_renderer_redispatch_cannot_rewrite_original_observation() {
    check(Surface::Readiness);
}

#[test]
fn admission_renderer_redispatch_cannot_rewrite_original_observation() {
    check(Surface::Admission);
}

#[test]
fn interruption_renderer_redispatch_cannot_rewrite_original_observation() {
    check(Surface::Interruption);
}

#[test]
fn browser_renderer_redispatch_cannot_rewrite_original_observation() {
    check(Surface::Browser);
}
