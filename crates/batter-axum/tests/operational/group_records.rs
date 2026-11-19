//! Copy only public HTTP extensions from a genuinely admitted request.
use crate::capture::Capture;
use axum::{
    Extension,
    body::{Body, to_bytes},
    extract::Request,
    http::{Extensions, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use batter_axum::{
    BrowserPolicy, GateView, Gated, GroupGate, GroupPolicy, GuardedRouter, HttpBoundary,
    MutationCheckWaiver, RequestPolicy, ResponseConstructionBudget, RouteGroup,
    browser::{BrowserCredentials, CookieHeaderPolicy, CookieName, PrivateResponsePolicy},
};
use batter_core::lifecycle::ShutdownHandle;
use std::time::Duration;

async fn captured(
    view: GateView<u64>,
    credentials: BrowserCredentials,
    request: Request,
) -> Response {
    assert_eq!(*view.value(), 7);
    assert_eq!(
        credentials
            .cookie(
                &CookieName::new("session").unwrap(),
                CookieHeaderPolicy::TargetOnly
            )
            .unwrap()
            .unwrap()
            .as_str(),
        "opaque"
    );
    let mut response = "captured".into_response();
    response
        .extensions_mut()
        .insert(request.extensions().clone());
    response
}

#[test]
fn copied_extensions_are_rejudged_by_the_destination_group() {
    let capture = Capture::new();
    capture.block_on(async {
        let (control, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let request_policy = || RequestPolicy::new(
            control.operation_admission(),
            ResponseConstructionBudget::new(Duration::from_secs(5)).unwrap(),
        );
        let browser = || BrowserPolicy::without_mutation_checks(
            PrivateResponsePolicy::NoReferrer,
            MutationCheckWaiver::NoAmbientCredentials,
        );
        let source = HttpBoundary::new(GroupPolicy::browser_gated(
            request_policy(), browser(), GroupGate::new(|| Gated::Serve(7_u64)),
        ))
        .assemble(GuardedRouter::new().route("/source", get(captured)))
        .await.unwrap().in_process();
        let mut request = Request::builder().uri("/source")
            .header("cookie", "session=opaque").body(Body::empty()).unwrap();
        request.extensions_mut().insert(17_u64);
        let mut response = source.request(request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let source_id = response.headers()["x-request-id"].clone();
        let inherited = response.extensions_mut().remove::<Extensions>().unwrap();

        // Cover default and named groups, absent policy and policy replacement.
        for named in [false, true] {
            for gated in [false, true] {
                let policy = if gated {
                    GroupPolicy::browser_gated(
                        request_policy(), browser(), GroupGate::new(|| Gated::Serve(23_u64)),
                    )
                } else {
                    GroupPolicy::new(request_policy())
                };
                let routes = GuardedRouter::new()
                    .route("/view", post(|view: GateView<u64>| async move { view.value().to_string() }))
                    .route("/cookies", post(|_: BrowserCredentials| async { "unreachable" }))
                    .route("/metadata", post(|Extension(value): Extension<u64>| async move { value.to_string() }));
                let destination = if named {
                    HttpBoundary::new(request_policy())
                        .with_group(RouteGroup::new("destination", policy, routes)).unwrap()
                        .assemble(GuardedRouter::new()).await.unwrap()
                } else {
                    HttpBoundary::new(policy).assemble(routes).await.unwrap()
                }.in_process();
                for path in ["/view", "/cookies", "/metadata"] {
                    let mut request = Request::builder().method("POST").uri(path)
                        .header("cookie", "session=opaque").body(Body::empty()).unwrap();
                    *request.extensions_mut() = inherited.clone();
                    let response = destination.request(request).await;
                    let expected = match path {
                        "/view" if gated => Some("23"),
                        "/metadata" => Some("17"),
                        _ => None,
                    };
                    assert_eq!(response.status(), if expected.is_some() { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR });
                    assert_ne!(response.headers().get("x-request-id"), Some(&source_id));
                    assert!(response.headers().contains_key("x-request-id"));
                    if expected.is_none() {
                        assert_eq!(response.headers()["content-type"], "application/problem+json");
                        assert_eq!(response.headers()["cache-control"], "no-store");
                    }
                    let body = to_bytes(response.into_body(), 4096).await.unwrap();
                    assert_eq!(body.as_ref(), expected.unwrap_or(r#"{"type":"about:blank","title":"Internal Server Error","status":500,"code":"internal_error"}"#).as_bytes());
                }
            }
        }
    });
    let logs = capture.text();
    assert_eq!(
        logs.lines()
            .filter(|line| line.contains("HTTP response boundary finished")
                && line.contains("gate_outcome=\"unmatched_view\""))
            .count(),
        2,
        "{logs}"
    );
}
