//! The entry check's private-record predicate and the renderer filter cover
//! the same records, and the in-process client refuses each of them alone.

use super::{CorrelationId, OperationalScope, carries_private_record, renderer_parts};
use crate::{
    RequestInterruptionResponder, RequestPolicy, ResponseConstructionBudget, ReusedRoutingState,
    admitted,
    browser::{CredentialAccess, record_credential_access},
    low_level::RouterClient,
    quota_observation::QuotaObservation,
};
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Extensions, HeaderValue, StatusCode},
};
use batter_core::{lifecycle::ShutdownHandle, operation::RootDeadline};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use tower_http::request_id::RequestId;

fn scope() -> OperationalScope {
    OperationalScope(CorrelationId(RequestId::new(HeaderValue::from_static(
        "00000000-0000-4000-8000-000000000000",
    ))))
}

/// An admission record with nothing around it. Admission records one only
/// beside the operational wrapper's marker, which is removed again here.
fn admission_alone() -> Extensions {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let policy = RequestPolicy::new(
        handle.operation_admission(),
        ResponseConstructionBudget::new(Duration::from_secs(1)).unwrap(),
    );
    let context = policy
        .admission
        .admit_root(RootDeadline::at(Instant::now() + policy.budget))
        .unwrap()
        .into_context();
    let (parts, ()) = Request::new(()).into_parts();
    let interruption = RequestInterruptionResponder::from_policy(&policy, &parts);
    let mut extensions = Extensions::new();
    extensions.insert(scope());
    admitted::record(&mut extensions, context, interruption);
    extensions.remove::<OperationalScope>();
    assert!(extensions.get::<admitted::Admission>().is_some());
    extensions
}

/// Each private record the renderer filter removes, alone in a map. The
/// observation state's own module covers that fifth type.
fn each_alone() -> Vec<(&'static str, Extensions)> {
    let mut quota = Extensions::new();
    quota.insert(QuotaObservation::default());
    let mut operational = Extensions::new();
    operational.insert(scope());
    let mut credential = Extensions::new();
    record_credential_access(&mut credential, CredentialAccess::Accepted);
    vec![
        ("quota", quota),
        ("operational", operational),
        ("credential", credential),
        ("admission", admission_alone()),
    ]
}

#[tokio::test]
async fn each_private_record_alone_is_detected_and_refused_before_the_router_runs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let client = RouterClient::new(Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    }));
    assert!(!carries_private_record(&Extensions::new()));

    for (name, extensions) in each_alone() {
        assert!(carries_private_record(&extensions), "{name}");
        let mut request = Request::new(Body::empty());
        *request.extensions_mut() = extensions;
        let response = client.request(request).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{name}"
        );
        assert!(
            response.extensions().get::<ReusedRoutingState>().is_some(),
            "{name}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the router ran");

    let served = client.request(Request::new(Body::empty())).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_renderer_filter_removes_every_record_the_predicate_detects() {
    for (name, extensions) in each_alone() {
        let mut request = Request::new(());
        *request.extensions_mut() = extensions;
        let (parts, ()) = request.into_parts();
        let filtered = renderer_parts(parts);
        assert!(!carries_private_record(&filtered.extensions), "{name}");
    }
}
