use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{FromRequestParts, Request},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    BrowserPolicy, GateView, Gated, GroupGate, GroupPolicy, GuardedRouter, HttpBoundary,
    HttpFailure, RequestPolicy, ResponseConstructionBudget,
    browser::{BrowserCredentials, BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
};
use batter_core::lifecycle::ShutdownHandle;
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

// A disabled request must drop an unread body before calling the renderer.
struct UnreadBody(Arc<AtomicBool>);
impl Drop for UnreadBody {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl http_body::Body for UnreadBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Infallible>>> {
        panic!("a rejected request must not poll its body")
    }
}

#[tokio::test]
async fn admission_then_gate_then_browser_then_application_with_private_headers_on_every_exit() {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    let enabled = Arc::new(AtomicBool::new(false));
    let captures = Arc::new(AtomicUsize::new(0));
    let browser_rejections = Arc::new(AtomicUsize::new(0));
    let layers = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let gate = GroupGate::new({
        let enabled = enabled.clone();
        let captures = captures.clone();
        move || {
            captures.fetch_add(1, Ordering::SeqCst);
            if enabled.load(Ordering::SeqCst) {
                Gated::Serve(17_u64)
            } else {
                Gated::Disabled
            }
        }
    });
    let request_policy = RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    )
    .with_failure_renderer({
        let dropped = dropped.clone();
        move |failure, _| {
            if failure == HttpFailure::Disabled {
                assert!(dropped.load(Ordering::SeqCst));
            }
            (failure.status(), failure.code()).into_response()
        }
    });
    let browser = browser_policy(browser_rejections.clone());
    let routes = GuardedRouter::new()
        .route(
            "/work",
            get(|| async { "ok" }).post(|view: GateView<u64>, _: BrowserCredentials| async move {
                view.value().to_string()
            }),
        )
        .layer(middleware::from_fn({
            let layers = layers.clone();
            move |request: Request, next: Next| {
                layers.fetch_add(1, Ordering::SeqCst);
                async move { next.run(request).await }
            }
        }));
    let app = HttpBoundary::new(GroupPolicy::browser_gated(request_policy, browser, gate))
        .assemble(routes)
        .await
        .unwrap()
        .in_process();
    let req = || {
        Request::builder()
            .method(Method::POST)
            .uri("/work")
            .body(Body::new(UnreadBody(dropped.clone())))
            .unwrap()
    };
    let response = app.request(req()).await;
    assert_private(&response);
    assert_eq!(body(response).await, "service_unavailable");
    assert_eq!(captures.load(Ordering::SeqCst), 0);
    approval.approve();
    dropped.store(false, Ordering::SeqCst);
    let response = app.request(req()).await;
    assert_private(&response);
    assert_eq!(body(response).await, "disabled");
    assert_eq!(browser_rejections.load(Ordering::SeqCst), 0);
    assert_eq!(layers.load(Ordering::SeqCst), 0);
    enabled.store(true, Ordering::SeqCst);
    let mut response = app.request(req()).await;
    assert_private(&response);
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let mut parts = response
        .extensions_mut()
        .remove::<axum::http::request::Parts>()
        .unwrap();
    assert!(matches!(
        GateView::<u64>::from_request_parts(&mut parts, &()).await,
        Err(HttpFailure::Internal)
    ));
    assert_eq!(browser_rejections.load(Ordering::SeqCst), 1);
    assert_eq!(layers.load(Ordering::SeqCst), 0);
    let accepted = Request::builder()
        .method(Method::POST)
        .uri("/work")
        .header("origin", "https://app.example")
        .body(Body::empty())
        .unwrap();
    let response = app.request(accepted).await;
    assert_private(&response);
    assert_eq!(body(response).await, "17");
    assert_eq!(layers.load(Ordering::SeqCst), 1);
    assert_eq!(captures.load(Ordering::SeqCst), 3);
    control.request();
    let response = app.request(req()).await;
    assert_private(&response);
    assert_eq!(body(response).await, "service_unavailable");
    assert_eq!(captures.load(Ordering::SeqCst), 3);
}

fn assert_private(response: &Response) {
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
}
async fn body(response: Response) -> String {
    String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec()).unwrap()
}

fn browser_policy(rejections: Arc<AtomicUsize>) -> BrowserPolicy {
    BrowserPolicy::with_mutation_checks(
        PrivateResponsePolicy::NoReferrer,
        MutationPolicy::exact_origin(BrowserOrigin::https("https://app.example").unwrap()),
        move |failure, parts| {
            rejections.fetch_add(1, Ordering::SeqCst);
            // Redispatched renderer metadata cannot recover the gate view.
            let mut response = (failure.status(), failure.code()).into_response();
            response.extensions_mut().insert(parts.clone());
            response
        },
    )
}
