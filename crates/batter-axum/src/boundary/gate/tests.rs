#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use super::*;
use crate::{GroupPolicy, GuardedRouter, HttpBoundary, ResponseConstructionBudget, correlation};
use axum::{body::Body, routing::get};
use batter_core::lifecycle::ShutdownHandle;
use std::time::Duration;

#[tokio::test]
async fn renderer_metadata_strips_every_value_type_and_preserves_public_extensions() {
    // Neither type implements Clone, and the removal path knows neither type.
    struct First;
    struct Second;
    for record in [GateRecord(Arc::new(First)), GateRecord(Arc::new(Second))] {
        let (mut parts, _) = Request::new(Body::empty()).into_parts();
        parts.extensions.insert(record);
        parts.extensions.insert(17_u64);
        let mut rendered = correlation::renderer_parts(parts);
        assert!(rendered.extensions.get::<GateRecord>().is_none());
        assert_eq!(rendered.extensions.get::<u64>(), Some(&17));
        assert!(matches!(
            GateView::<First>::from_request_parts(&mut rendered, &()).await,
            Err(HttpFailure::Internal)
        ));
        assert!(matches!(
            GateView::<Second>::from_request_parts(&mut rendered, &()).await,
            Err(HttpFailure::Internal)
        ));
    }
}

#[tokio::test]
async fn a_new_gate_replaces_an_inbound_record_instead_of_reusing_it() {
    let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let policy = RequestPolicy::new(
        control.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    );
    let app = HttpBoundary::new(GroupPolicy::gated(
        policy,
        GroupGate::new(|| Gated::Serve(23_u64)),
    ))
    .assemble(GuardedRouter::new().route(
        "/work",
        get(|view: GateView<u64>| async move {
            assert_eq!(*view.value(), 23);
        }),
    ))
    .await
    .unwrap()
    .in_process();
    let mut request = Request::builder().uri("/work").body(Body::empty()).unwrap();
    request.extensions_mut().insert(GateRecord(Arc::new(7_u64)));
    assert_eq!(
        app.request(request).await.status(),
        axum::http::StatusCode::OK
    );
}

#[tokio::test]
async fn disabled_has_a_permanent_sanitized_problem_contract() {
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
    assert_eq!(
        HttpFailure::Disabled.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(HttpFailure::Disabled.code(), "disabled");
    let response = HttpFailure::Disabled.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()["content-type"],
        "application/problem+json"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        to_bytes(response.into_body(), 4096).await.unwrap(),
        r#"{"type":"about:blank","title":"Service Unavailable","status":503,"code":"disabled"}"#
    );
}
