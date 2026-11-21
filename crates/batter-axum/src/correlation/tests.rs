//! Every record of the one private-record list has a fixture holding it and
//! no other listed record. The in-process client refuses each alone, and the
//! renderer filter removes each while keeping public values.

use super::{
    CorrelationId, OperationalScope, carries_private_record, private_record_names,
    private_records_present, renderer_parts,
};
use crate::{
    InProcessRefusal, RequestInterruptionResponder, RequestPolicy, ResponseConstructionBudget,
    admitted,
    browser::{CredentialAccess, record_credential_access},
    low_level::RouterClient,
    observation::ObservationState,
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

fn alone<T: Clone + Send + Sync + 'static>(record: T) -> (&'static str, Extensions) {
    let mut extensions = Extensions::new();
    extensions.insert(record);
    (std::any::type_name::<T>(), extensions)
}

/// Each private record, alone in a map and named by its type.
fn each_alone() -> Vec<(&'static str, Extensions)> {
    let mut credential = Extensions::new();
    record_credential_access(&mut credential, CredentialAccess::Accepted);
    vec![
        alone(QuotaObservation::default()),
        alone(ObservationState::default()),
        alone(scope()),
        (std::any::type_name::<CredentialAccess>(), credential),
        (
            std::any::type_name::<admitted::Admission>(),
            admission_alone(),
        ),
    ]
}

#[test]
fn every_listed_private_record_has_a_fixture_holding_only_it() {
    let mut listed = private_record_names();
    let mut fixtures = Vec::new();
    for (name, extensions) in each_alone() {
        // A fixture that also held another listed record would let that
        // record's detection stand in for this one's.
        assert_eq!(private_records_present(&extensions), [name]);
        fixtures.push(name);
    }
    listed.sort_unstable();
    fixtures.sort_unstable();
    assert_eq!(listed, fixtures);
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
            response.extensions().get::<InProcessRefusal>().is_some(),
            "{name}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the router ran");

    let served = client.request(Request::new(Body::empty())).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// An application extension the renderer filter must keep.
#[derive(Clone, Debug, PartialEq)]
struct Label(&'static str);

#[test]
fn the_renderer_filter_removes_each_private_record_and_keeps_public_values() {
    let public = scope().0;
    for (name, extensions) in each_alone() {
        let mut request = Request::new(());
        *request.extensions_mut() = extensions;
        request.extensions_mut().insert(public.clone());
        request.extensions_mut().insert(Label("kept"));
        let (parts, ()) = request.into_parts();
        let filtered = renderer_parts(parts);
        assert!(
            private_records_present(&filtered.extensions).is_empty(),
            "{name}"
        );
        let correlation = filtered.extensions.get::<CorrelationId>();
        assert_eq!(
            correlation.map(CorrelationId::as_str),
            Some(public.as_str())
        );
        assert_eq!(filtered.extensions.get::<Label>(), Some(&Label("kept")));
    }
}
