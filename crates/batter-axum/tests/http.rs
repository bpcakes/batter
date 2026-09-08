use axum::{
    Extension, Json, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use batter::{
    lifecycle::ShutdownHandle,
    operation::{Interruption, OperationContext},
};
use batter_axum::{HttpFailure, RequestPolicy, liveness, readiness, request_scope};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

fn request(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}
fn application(handle: ShutdownHandle) -> Router {
    Router::new()
        .route(
            "/work",
            get(
                |Extension(context): Extension<OperationContext>| async move {
                    assert!(context.check().is_ok());
                    "ok"
                },
            ),
        )
        .layer(middleware::from_fn_with_state(
            RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
            request_scope,
        ))
}

#[test]
fn request_budget_preserves_the_positive_representable_one_year_limit() {
    let year = Duration::from_secs(365 * 24 * 60 * 60);
    assert!(RequestPolicy::new(ShutdownHandle::new(), year).is_ok());
    assert!(matches!(
        RequestPolicy::new(ShutdownHandle::new(), Duration::ZERO),
        Err(batter::ConfigurationError::Zero("HTTP request budget"))
    ));
    for budget in [year + Duration::from_nanos(1), Duration::MAX] {
        assert!(matches!(
            RequestPolicy::new(ShutdownHandle::new(), budget),
            Err(batter::ConfigurationError::TooLarge("HTTP request budget"))
        ));
    }
}

#[tokio::test]
async fn starting_process_is_not_admitted() {
    let response = application(ShutdownHandle::new())
        .oneshot(request("/work"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("service_unavailable")
    );
}

#[tokio::test]
async fn admitted_handler_receives_native_operation_extension() {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let response = application(handle).oneshot(request("/work")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn draining_rejects_new_requests() {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = application(handle.clone());
    handle.request();
    assert_eq!(
        router.oneshot(request("/work")).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn drain_during_handler_does_not_interrupt_admitted_request() {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let inside = handle.clone();
    let router = Router::new()
        .route(
            "/work",
            get(move |Extension(context): Extension<OperationContext>| {
                let inside = inside.clone();
                async move {
                    inside.request();
                    assert!(context.check().is_ok());
                    "drained"
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
            request_scope,
        ));
    assert_eq!(
        router.oneshot(request("/work")).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test(start_paused = true)]
async fn total_handler_deadline_returns_sanitized_problem() {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "late"
            }),
        )
        .layer(middleware::from_fn_with_state(
            RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
            request_scope,
        ));
    let response = router.oneshot(request("/slow")).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(!response.headers().contains_key("retry-after"));
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("deadline_exceeded")
    );
}

#[tokio::test]
async fn request_context_is_cancelled_after_response_construction() {
    let escaped = Arc::new(Mutex::new(None));
    let inside = escaped.clone();
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = Router::new()
        .route(
            "/work",
            get(move |Extension(context): Extension<OperationContext>| {
                let inside = inside.clone();
                async move {
                    *inside.lock().unwrap() = Some(context);
                    "ok"
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            RequestPolicy::new(handle, Duration::from_secs(1)).unwrap(),
            request_scope,
        ));
    router.oneshot(request("/work")).await.unwrap();
    assert_eq!(
        escaped.lock().unwrap().as_ref().unwrap().check(),
        Err(Interruption::Cancelled)
    );
}

#[tokio::test]
async fn probes_remain_reachable_without_readiness() {
    let handle = ShutdownHandle::new();
    let router = application(handle.clone()).merge(
        Router::new()
            .route("/live", get(liveness))
            .route("/ready", get(readiness))
            .with_state(handle.clone()),
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("/live"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("/ready"))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    handle.mark_ready();
    assert_eq!(
        router.oneshot(request("/ready")).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn public_infrastructure_error_has_no_raw_error_channel() {
    use axum::response::IntoResponse;
    let response = HttpFailure::Internal.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("internal_error"));
    assert!(!text.contains("detail"));
}

#[derive(Clone)]
struct TrustedRequestId(&'static str);

#[derive(serde::Serialize)]
struct ApplicationError {
    code: &'static str,
    message: &'static str,
    request_id: &'static str,
}

fn application_policy(handle: ShutdownHandle) -> RequestPolicy {
    RequestPolicy::new(handle, Duration::from_secs(1))
        .unwrap()
        .with_failure_renderer(|failure, parts| {
            // This is an application-provided extension, not a client header.
            let request_id = parts.extensions.get::<TrustedRequestId>().unwrap().0;
            (
                failure.status(),
                Json(ApplicationError {
                    code: failure.code(),
                    message: "The operation could not be completed",
                    request_id,
                }),
            )
                .into_response()
        })
}

fn correlated_request() -> Request<Body> {
    let mut request = request("/work");
    request
        .headers_mut()
        .insert("x-request-id", "untrusted-client-value".parse().unwrap());
    request
        .extensions_mut()
        .insert(TrustedRequestId("trusted-request-17"));
    request
}

async fn assert_application_failure(response: Response, code: &str) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        format!(
            "{{\"code\":\"{code}\",\"message\":\"The operation could not be completed\",\"request_id\":\"trusted-request-17\"}}"
        )
    );
}

#[tokio::test]
async fn readiness_and_drain_rejections_use_application_renderer_without_response_rewriting() {
    async fn rejected_handler() -> &'static str {
        panic!("rejected handler must not run");
    }
    for draining in [false, true] {
        let handle = ShutdownHandle::new();
        if draining {
            handle.mark_ready();
            handle.request();
        }
        let router = Router::new().route("/work", get(rejected_handler)).layer(
            middleware::from_fn_with_state(application_policy(handle), request_scope),
        );
        let response = router.oneshot(correlated_request()).await.unwrap();
        assert_application_failure(response, "service_unavailable").await;
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_renderer_keeps_original_trusted_metadata_after_handler_takes_the_request() {
    let handle = ShutdownHandle::new();
    handle.mark_ready();
    let router = Router::new()
        .route(
            "/work",
            get(|mut request: Request<Body>| async move {
                // The renderer must retain the original snapshot independently.
                request.extensions_mut().remove::<TrustedRequestId>();
                tokio::time::sleep(Duration::from_secs(5)).await;
                "late"
            }),
        )
        .layer(middleware::from_fn_with_state(
            application_policy(handle),
            request_scope,
        ));
    let response = router.oneshot(correlated_request()).await.unwrap();
    assert_application_failure(response, "deadline_exceeded").await;
}

#[tokio::test]
async fn custom_renderer_owns_status_and_headers_as_well_as_the_error_body() {
    let policy = RequestPolicy::new(ShutdownHandle::new(), Duration::from_secs(1))
        .unwrap()
        .with_failure_renderer(|failure, _parts| {
            assert_eq!(failure, HttpFailure::Unavailable);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("x-application-error", "capacity-unavailable")],
                "Application-selected response",
            )
                .into_response()
        });
    let router = Router::new()
        .route("/work", get(|| async { "not called" }))
        .layer(middleware::from_fn_with_state(policy, request_scope));
    let response = router.oneshot(request("/work")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers()["x-application-error"],
        "capacity-unavailable"
    );
    assert!(!response.headers().contains_key("retry-after"));
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        "Application-selected response"
    );
}

#[tokio::test(start_paused = true)]
async fn forced_process_cancellation_uses_application_renderer_with_original_correlation() {
    use batter::{
        cleanup::CleanupBudget,
        lifecycle::{ShutdownBudget, Supervisor},
    };
    let budget = ShutdownBudget::new(
        Duration::ZERO,
        Duration::from_secs(1),
        Duration::from_secs(1),
        CleanupBudget::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
    )
    .unwrap();
    let mut supervisor = Supervisor::new(budget);
    let handle = supervisor.handle();
    let (started, startup) = tokio::sync::oneshot::channel();
    supervisor
        .register("http-owner", |shutdown| async move {
            shutdown.mark_started();
            started.send(()).unwrap();
            shutdown.cancelled().await;
            Ok(())
        })
        .unwrap();
    handle.mark_ready();
    let supervisor = tokio::spawn(supervisor.run_until(std::future::pending()));
    startup.await.unwrap();
    let inside = handle.clone();
    let router = Router::new()
        .route(
            "/work",
            get(move || {
                let inside = inside.clone();
                async move {
                    inside.request();
                    std::future::pending::<&'static str>().await
                }
            }),
        )
        .layer(middleware::from_fn_with_state(
            application_policy(handle),
            request_scope,
        ));
    let response = router.oneshot(correlated_request()).await.unwrap();
    assert_application_failure(response, "operation_cancelled").await;
    let report = supervisor.await.unwrap();
    assert!(report.forced_cancellation);
    assert!(report.is_success());
}
