//! A consumer-shaped service: ordinary routes under a short budget, uploads
//! under a longer one, and private account pages with browser mutation checks.

use super::support::{
    ORIGIN, SECOND, assert_not_private, assert_private, body_text, browser_group, count,
    exact_origin, from_origin, request, request_policy,
};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
    routing::{MethodFilter, MethodRouter, get, on},
};
use batter_axum::{
    GuardedRouter, HttpBoundary, ProbePath, ReadinessPolicy, RouteGroup,
    browser::PrivateResponsePolicy,
};
use batter_core::{
    health::{HealthMonitor, HealthPolicy, HealthReader},
    lifecycle::ShutdownHandle,
};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use tower::ServiceExt;

fn sleeping(filter: MethodFilter, duration: Duration, body: &'static str) -> MethodRouter {
    on(filter, move || async move {
        tokio::time::sleep(duration).await;
        body
    })
}

async fn service(
    handle: &ShutdownHandle,
    readiness: HealthReader<Infallible>,
    saves: &Arc<AtomicUsize>,
) -> Router {
    let default = GuardedRouter::new()
        .route(
            "/reports",
            sleeping(MethodFilter::GET, 2 * SECOND, "report"),
        )
        .route("/items/{id}", get(|| async { "item" }))
        .fallback(|| async { (StatusCode::NOT_FOUND, "application fallback") });
    let uploads = GuardedRouter::new()
        .route(
            "/uploads/{name}",
            sleeping(MethodFilter::PUT, 2 * SECOND, "stored"),
        )
        .route(
            "/uploads/{name}/parts",
            sleeping(MethodFilter::PUT, 4 * SECOND, "late"),
        );
    let saved = saves.clone();
    let account = GuardedRouter::new().nest(
        "/account",
        GuardedRouter::new().route(
            "/profile",
            get(|| async { "profile" }).post(move || {
                saved.fetch_add(1, Ordering::SeqCst);
                async { "saved" }
            }),
        ),
    );
    HttpBoundary::new(request_policy(handle, SECOND))
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .with_readiness(
            ProbePath::new("/ready").unwrap(),
            ReadinessPolicy::new(handle.status(), readiness),
        )
        .unwrap()
        .with_group(RouteGroup::new(
            "uploads",
            request_policy(handle, 3 * SECOND),
            uploads,
        ))
        .unwrap()
        .with_group(RouteGroup::new(
            "account",
            browser_group(
                handle,
                SECOND,
                PrivateResponsePolicy::NoReferrer,
                exact_origin(),
            ),
            account,
        ))
        .unwrap()
        .assemble(default)
        .await
        .unwrap()
        .into_router()
}

async fn timed(app: &Router, request: Request<Body>) -> (StatusCode, Duration, String) {
    let started = Instant::now();
    let response = app.clone().oneshot(request).await.unwrap();
    assert!(response.headers().contains_key("x-request-id"));
    (
        response.status(),
        started.elapsed(),
        body_text(response).await,
    )
}

/// Before readiness every group rejects through its own admission; probes and
/// the account group's private headers stay in place.
async fn assert_starting(app: &Router) {
    let live = app
        .clone()
        .oneshot(request(Method::GET, "/live"))
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let ready = app
        .clone()
        .oneshot(request(Method::GET, "/ready"))
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_text(ready).await, "", "readiness is not admission");
    for (method, path) in [
        (Method::GET, "/items/7"),
        (Method::PUT, "/uploads/report.csv"),
        (Method::GET, "/missing"),
    ] {
        let response = app.clone().oneshot(request(method, path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_not_private(response.headers());
        assert!(body_text(response).await.contains("service_unavailable"));
    }
    let account = app
        .clone()
        .oneshot(request(Method::GET, "/account/profile"))
        .await
        .unwrap();
    assert_eq!(account.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(account.headers(), "no-referrer");
}

/// The same two-second work times out under the default budget but not under
/// the upload budget, which expires at its own three seconds.
async fn assert_group_deadlines(app: &Router) {
    let (status, elapsed, body) = timed(app, request(Method::GET, "/reports")).await;
    assert_eq!((status, elapsed), (StatusCode::SERVICE_UNAVAILABLE, SECOND));
    assert!(body.contains("deadline_exceeded"), "{body}");
    let (status, elapsed, body) = timed(app, request(Method::PUT, "/uploads/report.csv")).await;
    assert_eq!(
        (status, elapsed, body.as_str()),
        (StatusCode::OK, 2 * SECOND, "stored")
    );
    let parts = request(Method::PUT, "/uploads/report.csv/parts");
    let (status, elapsed, body) = timed(app, parts).await;
    assert_eq!(
        (status, elapsed),
        (StatusCode::SERVICE_UNAVAILABLE, 3 * SECOND)
    );
    assert!(body.contains("deadline_exceeded"), "{body}");
}

/// The private group rejects a cross-site mutation before its handler and
/// sends its private headers on every response.
async fn assert_browser_group(app: &Router, saves: &Arc<AtomicUsize>) {
    let page = app
        .clone()
        .oneshot(request(Method::GET, "/account/profile"))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_private(page.headers(), "no-referrer");
    let forged = from_origin(Method::POST, "/account/profile", "https://attacker.example");
    let rejected = app.clone().oneshot(forged).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_private(rejected.headers(), "no-referrer");
    assert_eq!(
        body_text(rejected).await,
        "browser:origin_mismatch:correlated=true:admitted=true"
    );
    assert_eq!(count(saves), 0);
    let accepted = app
        .clone()
        .oneshot(from_origin(Method::POST, "/account/profile", ORIGIN))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_private(accepted.headers(), "no-referrer");
    assert_eq!(body_text(accepted).await, "saved");
    assert_eq!(count(saves), 1);
}

/// Default-group responses carry no private headers, and unmatched paths,
/// including one under the account prefix, reach the default fallback.
async fn assert_default_group(app: &Router) {
    let item = app
        .clone()
        .oneshot(request(Method::GET, "/items/7"))
        .await
        .unwrap();
    assert_eq!(item.status(), StatusCode::OK);
    assert_not_private(item.headers());
    for path in ["/missing", "/account/missing"] {
        let response = app
            .clone()
            .oneshot(request(Method::GET, path))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_not_private(response.headers());
        assert_eq!(body_text(response).await, "application fallback");
    }
}

#[tokio::test(start_paused = true)]
async fn one_boundary_serves_default_upload_and_private_browser_groups() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let health = HealthPolicy::new(SECOND, SECOND, 3 * SECOND, SECOND).unwrap();
    let monitor = HealthMonitor::new(health, || async { Ok::<_, Infallible>(()) });
    let saves = Arc::new(AtomicUsize::new(0));
    let app = service(&handle, monitor.reader(), &saves).await;
    assert_starting(&app).await;
    approval.approve();
    assert_group_deadlines(&app).await;
    assert_browser_group(&app, &saves).await;
    assert_default_group(&app).await;
    drop(monitor);
}
