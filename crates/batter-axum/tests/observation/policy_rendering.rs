use super::{capture::Capture, stacked_interruption::interrupted_extensions, support::request};
use axum::{
    Router,
    body::to_bytes,
    http::{Extensions, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{any, get},
};
use batter_axum::{
    BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, InProcessClient, ProbePath,
    ReadinessPolicy, RequestPolicy, ResponseConstructionBudget,
    browser::{BrowserOrigin, MutationPolicy, PrivateResponsePolicy},
    low_level::request_admission,
};
use batter_core::{lifecycle::ShutdownHandle, operation::Interruption};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Copy)]
enum Surface {
    Browser,
    Liveness,
    Readiness,
    Fallback,
}

#[derive(Clone, Debug, PartialEq)]
struct ApplicationMetadata(&'static str);

#[tokio::test]
async fn browser_rejection_clears_recycled_interruption_evidence() {
    assert_surface(Surface::Browser).await;
}

#[tokio::test]
async fn liveness_rendering_clears_recycled_interruption_evidence() {
    assert_surface(Surface::Liveness).await;
}

#[tokio::test]
async fn readiness_rendering_clears_recycled_interruption_evidence() {
    assert_surface(Surface::Readiness).await;
}

#[tokio::test]
async fn fallback_rendering_clears_recycled_interruption_evidence() {
    assert_surface(Surface::Fallback).await;
}

async fn assert_surface(surface: Surface) {
    for reason in [Interruption::Cancelled, Interruption::DeadlineExceeded] {
        let extensions = interrupted_extensions(reason).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = client(surface, extensions, calls.clone()).await;
        let method = if matches!(surface, Surface::Browser) {
            "POST"
        } else {
            "GET"
        };
        // Returning an in-process response through another admission must not
        // transfer interruption evidence from its renderer's previous response.
        let outer = Router::new()
            .route(
                "/outer",
                get(move || {
                    let inner = inner.clone();
                    async move { inner.request(request(method, "/rendered")).await }
                }),
            )
            .route_layer(middleware::from_fn_with_state(policy(), request_admission));
        let capture = Capture::new();
        let response = outer
            .oneshot(request("GET", "/outer"))
            .with_subscriber(capture.dispatch.clone())
            .await
            .unwrap();
        let expected = match surface {
            Surface::Liveness => StatusCode::OK,
            Surface::Readiness => StatusCode::SERVICE_UNAVAILABLE,
            Surface::Browser | Surface::Fallback => StatusCode::IM_A_TEAPOT,
        };
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["x-rendered"], "retained");
        assert_eq!(
            response.extensions().get::<ApplicationMetadata>(),
            Some(&ApplicationMetadata("retained"))
        );
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "retained"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let text = capture.text();
        let operations: Vec<_> = text
            .lines()
            .filter_map(|line| {
                line.split_once("operation boundary finished")
                    .map(|(_, fields)| fields)
            })
            .collect();
        assert_eq!(
            operations.len(),
            1 + usize::from(matches!(surface, Surface::Browser)),
            "{text}"
        );
        assert!(
            operations
                .iter()
                .all(|line| line.contains("outcome=\"succeeded\"")),
            "{text}"
        );
        let completions = super::support::completions(&text);
        assert_eq!(completions.len(), 1, "{text}");
        assert!(
            completions[0].contains(&format!("status={}", expected.as_u16())),
            "{text}"
        );
    }
}

async fn client(
    surface: Surface,
    extensions: Extensions,
    calls: Arc<AtomicUsize>,
) -> InProcessClient {
    let render = move || -> Response {
        calls.fetch_add(1, Ordering::SeqCst);
        let mut response = (
            StatusCode::IM_A_TEAPOT,
            [("x-rendered", "retained")],
            "retained",
        )
            .into_response();
        *response.extensions_mut() = extensions.clone();
        response
            .extensions_mut()
            .insert(ApplicationMetadata("retained"));
        response
    };
    let routes = GuardedRouter::new();
    let boundary = HttpBoundary::new(policy());
    let (boundary, routes) = match surface {
        Surface::Browser => (
            HttpBoundary::new(GroupPolicy::browser(
                policy(),
                BrowserPolicy::with_mutation_checks(
                    PrivateResponsePolicy::NoReferrer,
                    MutationPolicy::exact_origin(
                        BrowserOrigin::https("https://app.example").unwrap(),
                    ),
                    move |_, _| render(),
                ),
            )),
            routes.route("/rendered", any(|| async { "unexpected handler" })),
        ),
        Surface::Liveness => (
            boundary
                .with_rendered_liveness(ProbePath::new("/rendered").unwrap(), move |_| render())
                .unwrap(),
            routes,
        ),
        Surface::Readiness => {
            let (unready, _approval) = ShutdownHandle::new_with_readiness_approval();
            (
                boundary
                    .with_rendered_readiness(
                        ProbePath::new("/rendered").unwrap(),
                        ReadinessPolicy::lifecycle_only(unready.status()),
                        move |_, _| render(),
                    )
                    .unwrap(),
                routes,
            )
        }
        Surface::Fallback => (
            boundary.with_rendered_fallback(move |_| render()).unwrap(),
            routes,
        ),
    };
    boundary.assemble(routes).await.unwrap().in_process()
}

fn policy() -> RequestPolicy {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(60)).unwrap(),
    )
}
