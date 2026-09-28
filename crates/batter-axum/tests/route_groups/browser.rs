//! Per-group browser policy: mutation checks and private-response headers.

use super::support::{
    ORIGIN, SECOND, assert_not_private, assert_private, body_text, browser_group, count, counted,
    exact_origin, from_origin, ready_handle, request, request_policy,
};
use axum::{
    Extension, Router,
    extract::Request,
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use batter_axum::{
    BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, ProbePath, RouteGroup,
    browser::{FetchSitePolicy, PrivateResponsePolicy},
};
use batter_core::{lifecycle::ShutdownHandle, operation::OperationContext};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

/// A private group whose layer records that it ran inside admission.
async fn private_app(
    handle: &ShutdownHandle,
    handled: &Arc<AtomicUsize>,
    layered: &Arc<AtomicUsize>,
) -> Router {
    let layer_calls = layered.clone();
    let account = GuardedRouter::new()
        .route("/account", counted(handled, "handled"))
        .route("/account/view", get(|| async { "view" }))
        .route("/account/slow", get(std::future::pending::<&'static str>))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            // Application layers run inside admission and the mutation checks.
            assert!(request.extensions().get::<OperationContext>().is_some());
            layer_calls.fetch_add(1, Ordering::SeqCst);
            async move { next.run(request).await }
        }));
    let mutation = exact_origin()
        .with_fetch_site(FetchSitePolicy::RejectCrossSite)
        .unwrap();
    let policy = browser_group(
        handle,
        SECOND,
        PrivateResponsePolicy::SameOriginReferrer,
        mutation,
    );
    HttpBoundary::new(request_policy(handle, SECOND))
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .with_group(RouteGroup::new("account", policy, account))
        .unwrap()
        .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
        .await
        .unwrap()
        .into_router()
}

async fn assert_rejected(app: &Router, request: axum::http::Request<axum::body::Body>, code: &str) {
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN, "{code}");
    assert_private(response.headers(), "same-origin");
    assert!(response.headers().contains_key("x-request-id"));
    assert_eq!(
        body_text(response).await,
        format!("browser:{code}:correlated=true:admitted=true")
    );
}

#[tokio::test]
async fn cross_site_and_ambiguous_mutations_are_rejected_before_application_code() {
    let (handled, layered) = (Arc::default(), Arc::default());
    let app = private_app(&ready_handle(), &handled, &layered).await;
    assert_rejected(&app, request(Method::POST, "/account"), "origin_missing").await;
    let forged = from_origin(Method::POST, "/account", "https://attacker.example");
    assert_rejected(&app, forged, "origin_mismatch").await;
    // Two Origin fields are ambiguous even when both carry the trusted origin.
    let mut duplicated = from_origin(Method::PUT, "/account", ORIGIN);
    duplicated
        .headers_mut()
        .append(header::ORIGIN, ORIGIN.parse().unwrap());
    assert_rejected(&app, duplicated, "origin_ambiguous").await;
    let mut cross_site = from_origin(Method::DELETE, "/account", ORIGIN);
    cross_site
        .headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert_rejected(&app, cross_site, "fetch_site_cross_site").await;
    assert_eq!((count(&handled), count(&layered)), (0, 0));

    let accepted = app
        .clone()
        .oneshot(from_origin(Method::POST, "/account", ORIGIN))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_private(accepted.headers(), "same-origin");
    assert_eq!((count(&handled), count(&layered)), (1, 1));
}

#[tokio::test]
async fn only_safe_methods_skip_mutation_checks() {
    let (handled, layered) = (Arc::default(), Arc::default());
    let app = private_app(&ready_handle(), &handled, &layered).await;
    for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
        let response = app
            .clone()
            .oneshot(request(method.clone(), "/account"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{method}");
        assert_private(response.headers(), "same-origin");
    }
    assert_eq!(count(&handled), 4);
    let extension = Method::from_bytes(b"PURGE").unwrap();
    for method in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::CONNECT,
        extension,
    ] {
        assert_rejected(&app, request(method, "/account"), "origin_missing").await;
    }
    assert_eq!(count(&handled), 4);
}

#[tokio::test(start_paused = true)]
async fn private_headers_cover_admission_deadline_and_method_rejections() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let (handled, layered) = (Arc::default(), Arc::default());
    let app = private_app(&handle, &handled, &layered).await;
    let starting = app
        .clone()
        .oneshot(request(Method::GET, "/account/view"))
        .await
        .unwrap();
    assert_eq!(starting.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(starting.headers(), "same-origin");
    approval.approve();
    let timeout = app
        .clone()
        .oneshot(request(Method::GET, "/account/slow"))
        .await
        .unwrap();
    assert_eq!(timeout.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(timeout.headers(), "same-origin");
    assert!(body_text(timeout).await.contains("deadline_exceeded"));
    // An unsupported method on a group route stays inside the group's layers.
    let method = app
        .clone()
        .oneshot(from_origin(Method::POST, "/account/view", ORIGIN))
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_private(method.headers(), "same-origin");
    handle.request();
    let draining = app
        .clone()
        .oneshot(request(Method::GET, "/account/view"))
        .await
        .unwrap();
    assert_eq!(draining.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(draining.headers(), "same-origin");
    // Probes and the default group sit outside the account group's headers.
    let live = app
        .clone()
        .oneshot(request(Method::GET, "/live"))
        .await
        .unwrap();
    assert_not_private(live.headers());
    assert_eq!(count(&handled), 0);
}

#[tokio::test]
async fn a_default_group_browser_policy_covers_its_fallbacks() {
    let handle = ready_handle();
    let handled = Arc::new(AtomicUsize::new(0));
    let default = GroupPolicy::browser(
        request_policy(&handle, SECOND).with_infrastructure_json(),
        BrowserPolicy::with_mutation_checks(
            PrivateResponsePolicy::NoReferrer,
            exact_origin(),
            super::support::render_rejection,
        ),
    );
    let routes = GuardedRouter::new()
        .route("/pages", counted(&handled, "page"))
        .fallback(|| async { (StatusCode::NOT_FOUND, "fallback") });
    let app = HttpBoundary::new(default)
        .assemble(routes)
        .await
        .unwrap()
        .into_router();
    let missing = app
        .clone()
        .oneshot(request(Method::GET, "/missing"))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_private(missing.headers(), "no-referrer");
    let forged = from_origin(Method::POST, "/missing", "https://attacker.example");
    let rejected = app.clone().oneshot(forged).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_private(rejected.headers(), "no-referrer");
    handle.request();
    let draining = app.oneshot(request(Method::GET, "/pages")).await.unwrap();
    assert_eq!(draining.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_private(draining.headers(), "no-referrer");
    assert_eq!(draining.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(count(&handled), 0);
}

#[tokio::test]
async fn a_group_without_mutation_checks_only_adds_private_headers() {
    let handle = ready_handle();
    let policy = GroupPolicy::browser(
        request_policy(&handle, SECOND),
        BrowserPolicy::without_mutation_checks(PrivateResponsePolicy::NoReferrer),
    );
    let reports = GuardedRouter::new().route(
        "/reports",
        get(
            |Extension(context): Extension<OperationContext>| async move {
                context.check().unwrap();
                "report"
            },
        )
        .post(|| async { "created" }),
    );
    let app = HttpBoundary::new(request_policy(&handle, SECOND))
        .with_group(RouteGroup::new("reports", policy, reports))
        .unwrap()
        .assemble(GuardedRouter::new())
        .await
        .unwrap()
        .into_router();
    for method in [Method::GET, Method::POST] {
        let response: Response = app
            .clone()
            .oneshot(request(method, "/reports"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_private(response.headers(), "no-referrer");
    }
}

#[tokio::test]
async fn unmatched_paths_keep_the_default_group_policy_without_a_custom_fallback() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    let (handled, layered) = (Arc::default(), Arc::default());
    let app = private_app(&handle, &handled, &layered).await;
    let starting = app
        .clone()
        .oneshot(request(Method::GET, "/missing"))
        .await
        .unwrap();
    assert_eq!(starting.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_not_private(starting.headers());
    approval.approve();
    for request in [
        request(Method::GET, "/missing"),
        from_origin(Method::POST, "/missing", "https://attacker.example"),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_not_private(response.headers());
    }
    assert_eq!((count(&handled), count(&layered)), (0, 0));
}
