use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{MethodRouter, any},
};
use batter_axum::{
    BrowserPolicy, CorrelationId, GroupPolicy, RequestPolicy, ResponseConstructionBudget,
    browser::{BrowserOrigin, MutationPolicy, MutationRejection, PrivateResponsePolicy},
};
use batter_core::{lifecycle::ShutdownHandle, operation::OperationContext};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub const ORIGIN: &str = "https://app.example";
pub const SECOND: Duration = Duration::from_secs(1);

pub fn ready_handle() -> ShutdownHandle {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    handle
}

pub fn request_policy(handle: &ShutdownHandle, budget: Duration) -> RequestPolicy {
    RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(budget).unwrap(),
    )
}

/// The application's own envelope for browser mutation rejections. It also
/// reports the request metadata the boundary hands to the renderer.
pub fn render_rejection(rejection: MutationRejection, parts: &Parts) -> Response {
    let correlated = parts.extensions.get::<CorrelationId>().is_some();
    let admitted = parts.extensions.get::<OperationContext>().is_some();
    (
        rejection.status(),
        format!(
            "browser:{}:correlated={correlated}:admitted={admitted}",
            rejection.code()
        ),
    )
        .into_response()
}

/// A private browser group requiring the exact configured Origin on mutations.
pub fn browser_group(
    handle: &ShutdownHandle,
    budget: Duration,
    referrer: PrivateResponsePolicy,
    mutation: MutationPolicy,
) -> GroupPolicy {
    GroupPolicy::browser(
        request_policy(handle, budget),
        BrowserPolicy::with_mutation_checks(referrer, mutation, render_rejection),
    )
}

pub fn exact_origin() -> MutationPolicy {
    MutationPolicy::exact_origin(BrowserOrigin::https(ORIGIN).unwrap())
}

/// A handler for every method that counts its calls.
pub fn counted(calls: &Arc<AtomicUsize>, body: &'static str) -> MethodRouter {
    let calls = calls.clone();
    any(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async move { body }
    })
}

pub fn count(calls: &Arc<AtomicUsize>) -> usize {
    calls.load(Ordering::SeqCst)
}

pub fn request(method: Method, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

pub fn from_origin(method: Method, path: &str, origin: &str) -> Request<Body> {
    let mut request = request(method, path);
    request
        .headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    request
}

pub async fn body_text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

pub fn assert_private(headers: &HeaderMap, referrer: &str) {
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{headers:?}");
    assert_eq!(headers["referrer-policy"], referrer, "{headers:?}");
    assert_eq!(headers["x-content-type-options"], "nosniff", "{headers:?}");
    for name in ["cache-control", "referrer-policy", "x-content-type-options"] {
        assert_eq!(headers.get_all(name).iter().count(), 1, "{name}");
    }
}

pub fn assert_not_private(headers: &HeaderMap) {
    assert!(!headers.contains_key("referrer-policy"), "{headers:?}");
    assert!(
        !headers.contains_key("x-content-type-options"),
        "{headers:?}"
    );
}
