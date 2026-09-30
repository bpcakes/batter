use crate::capture::Capture;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    AdmittedRequest, BoundaryAssemblyError, CorrelationId, GuardedRouter, HttpBoundary, ProbePath,
    RequestPolicy, ResponseConstructionBudget, RouteGroup, RouteInventory,
    browser::PrivateResponsePolicy,
};
use batter_core::lifecycle::ShutdownHandle;
use std::time::Duration;
use tower::ServiceExt;

fn policy(control: &ShutdownHandle) -> RequestPolicy {
    RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    )
}

fn render(parts: &Parts) -> Response {
    let id = parts.extensions.get::<CorrelationId>().unwrap().to_string();
    let mut response =
        (StatusCode::NOT_FOUND, [("x-renderer-id", id)], "not_found").into_response();
    if parts.uri.path().starts_with("/private/") {
        PrivateResponsePolicy::NoReferrer.apply(response.headers_mut());
    }
    response
}

async fn work(admitted: AdmittedRequest) -> &'static str {
    admitted.context().check().unwrap();
    "work"
}

#[test]
fn fallback_keeps_envelope_prefix_headers_and_one_observation_through_lifecycle() {
    for declared in [false, true] {
        let capture = Capture::new();
        let requests = capture.block_on(async {
            let (control, approval) = ShutdownHandle::new_with_readiness_approval();
            let mut approval = Some(approval);
            let app = app(&control, declared).await;
            let mut count = 0;
            for state in 0..3 {
                if state == 1 {
                    approval.take().unwrap().approve();
                }
                if state == 2 {
                    control.request();
                }
                for (method, path, expected) in cases(state == 1) {
                    let response = app
                        .clone()
                        .oneshot(
                            Request::builder()
                                .method(method)
                                .uri(path)
                                .header("x-request-id", "forged")
                                .body(Body::empty())
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    count += 1;
                    assert_eq!(
                        response.status(),
                        expected,
                        "state={state}, path={path}, declared={declared}"
                    );
                    assert_ne!(response.headers()["x-request-id"], "forged");
                    if expected == StatusCode::NOT_FOUND {
                        assert_eq!(
                            response.headers()["x-renderer-id"],
                            response.headers()["x-request-id"]
                        );
                        if path.starts_with("/private/") {
                            assert_eq!(response.headers()["cache-control"], "no-store");
                            assert_eq!(response.headers()["referrer-policy"], "no-referrer");
                        } else {
                            assert!(response.headers().get("cache-control").is_none());
                        }
                        assert_eq!(
                            to_bytes(response.into_body(), 1024).await.unwrap(),
                            "not_found"
                        );
                    }
                }
            }
            count
        });
        let text = capture.text();
        assert_eq!(
            text.matches("HTTP response boundary finished").count(),
            requests,
            "{text}"
        );
        assert!(
            !text.contains("retired") && !text.contains("hidden"),
            "{text}"
        );
        for event in text.lines().filter(|line| {
            line.contains("HTTP response boundary finished") && line.contains("status=404")
        }) {
            assert!(event.contains("INFO"), "{event}");
        }
    }
}

#[tokio::test]
async fn rendered_fallback_rejects_conflicts_and_does_not_disable_route_validation() {
    let control = ShutdownHandle::new_unapproved();
    let boundary = || {
        HttpBoundary::new(policy(&control))
            .with_rendered_fallback(|_| panic!("assembly cannot render a fallback"))
            .unwrap()
    };
    assert!(matches!(
        boundary().with_rendered_fallback(render),
        Err(BoundaryAssemblyError::ConflictingFallback)
    ));
    for routes in [
        GuardedRouter::new().fallback(|| async { "guarded" }),
        GuardedRouter::new().nest(
            "/nested/{tenant}",
            GuardedRouter::new().fallback(|| async { "nested" }),
        ),
    ] {
        assert!(matches!(
            boundary().assemble(routes).await,
            Err(BoundaryAssemblyError::ConflictingFallback)
        ));
    }
    let probe = boundary()
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .assemble(GuardedRouter::new().route("/{*rest}", get(work)))
        .await;
    assert!(matches!(
        probe,
        Err(BoundaryAssemblyError::GuardedProbePath)
    ));
    let overlap = boundary()
        .with_group(RouteGroup::new(
            "named",
            policy(&control),
            GuardedRouter::new().route("/work", get(work)),
        ))
        .unwrap()
        .assemble(GuardedRouter::new().route("/{name}", get(work)))
        .await;
    assert!(matches!(
        overlap,
        Err(BoundaryAssemblyError::OverlappingGroupPaths { .. })
    ));
}

fn unreachable_fallback() -> StatusCode {
    panic!("undeclared fallback must never run")
}

async fn app(control: &ShutdownHandle, declared: bool) -> Router {
    let routes = if declared {
        GuardedRouter::from_router(
            Router::new()
                .route("/private/work", get(work))
                .fallback(|| async { unreachable_fallback() }),
            RouteInventory::new(["/private/work"]).unwrap(),
        )
    } else {
        GuardedRouter::new().nest("/private", GuardedRouter::new().route("/work", get(work)))
    };
    HttpBoundary::new(policy(control))
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .with_rendered_fallback(render)
        .unwrap()
        .with_group(RouteGroup::new("private", policy(control), routes))
        .unwrap()
        .assemble(GuardedRouter::new().route("/default", get(work)))
        .await
        .unwrap()
        .into_router()
}

fn cases(ready: bool) -> [(Method, &'static str, StatusCode); 7] {
    let work = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let method = if ready {
        StatusCode::METHOD_NOT_ALLOWED
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    [
        (Method::GET, "/absent", StatusCode::NOT_FOUND),
        (
            Method::POST,
            "/private/retired?secret=hidden",
            StatusCode::NOT_FOUND,
        ),
        (Method::GET, "/live", StatusCode::OK),
        (Method::POST, "/live", StatusCode::METHOD_NOT_ALLOWED),
        (Method::GET, "/private/work", work),
        (Method::POST, "/private/work", method),
        (Method::GET, "/default", work),
    ]
}
