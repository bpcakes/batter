//! Executed evidence that a facade consumer can compose the native Runlimit
//! transport packages without declaring them, and that doing so stays visibly
//! distinct from `batter::runlimit::http`.
//!
//! The application owns every trust-sensitive decision here: subject derivation,
//! the exhaustive rejection mapping, the response status and which policy name
//! is disclosed. Nothing in this file goes through Batter's protected
//! quota-before-body assembly, and none of it acquires an operation deadline,
//! admission ordering or telemetry from the foundation.

use axum::{
    Extension, Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use batter::runlimit::memory::{MemoryStore, MemoryStoreConfig};
use batter::runlimit::native::{
    Capacity, Check, Denial, FixedWindowPolicy, KeyHasher, PolicyId, RateLimitPolicy, ScopeId,
    SubjectKey,
};
use batter::runlimit::native_transport::axum::{
    Admissions, RateLimitLayer, RateLimitRejection, RejectionKind,
};
use batter::runlimit::native_transport::http::draft_11;
use std::{convert::Infallible, time::Duration};
use tower::ServiceExt;

/// The disclosed policy name is an application choice, not a native identifier.
const PUBLIC_NAME: &str = "reads";

fn policy() -> FixedWindowPolicy {
    FixedWindowPolicy::new(
        PolicyId::new("native.transport.read").expect("validated policy id"),
        ScopeId::new("owner").expect("validated scope id"),
        1,
        Duration::from_secs(60),
    )
    .expect("validated fixed-window policy")
}

/// Derive the opaque subject from material this application trusts. The layer
/// rebinds the unbound key to its own configured policy.
fn subject(request: &Request<Body>, policy: &FixedWindowPolicy) -> Result<SubjectKey, Infallible> {
    let hasher = KeyHasher::new([11; 32]).expect("fixture key material");
    let owner = request
        .headers()
        .get("x-owner")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("anonymous");
    Ok(hasher.hash_for(policy, owner).into_unbound_subject_key())
}

/// Every rejection category is named, so a new one becomes a compile error here
/// rather than falling into a wildcard arm.
fn reject(
    rejection: RateLimitRejection<
        Infallible,
        <MemoryStore as batter::runlimit::native::Limiter>::CheckError,
    >,
) -> Response {
    let (status, headers) = match rejection {
        RateLimitRejection::Key(never) => match never {},
        RateLimitRejection::Denied(Denial::QuotaExceeded(denial)) => {
            let (name, value) = draft_11::service_limit(PUBLIC_NAME, denial)
                .expect("a validated denial encodes into structured fields");
            let mut headers = HeaderMap::new();
            headers.insert(name, value);
            (StatusCode::TOO_MANY_REQUESTS, headers)
        }
        RateLimitRejection::Denied(Denial::StorageCapacity { .. }) => {
            (StatusCode::SERVICE_UNAVAILABLE, HeaderMap::new())
        }
        RateLimitRejection::Backend(_) => (StatusCode::SERVICE_UNAVAILABLE, HeaderMap::new()),
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

async fn read(Extension(admissions): Extension<Admissions>) -> Response {
    assert_eq!(
        admissions.len(),
        1,
        "the single layer records one admission"
    );
    let (name, value) = draft_11::quota_policy(PUBLIC_NAME, &policy())
        .expect("a validated policy encodes into structured fields");
    let mut response = Response::new(Body::empty());
    response.headers_mut().insert(name, value);
    response
}

fn router() -> Router {
    let store = MemoryStore::new(MemoryStoreConfig::new(16).expect("validated store capacity"));
    Router::new()
        .route("/read", get(read))
        .layer(RateLimitLayer::new(store, policy(), subject, reject))
}

async fn request(router: Router) -> Response {
    let request = Request::builder()
        .uri("/read")
        .header("x-owner", "owner-a")
        .body(Body::empty())
        .expect("valid fixture request");
    router.oneshot(request).await.expect("router is infallible")
}

#[tokio::test]
async fn facade_paths_compose_the_native_transport_packages() {
    let router = router();
    let admitted = request(router.clone()).await;
    assert_eq!(admitted.status(), StatusCode::OK);
    assert_eq!(
        admitted
            .headers()
            .get("ratelimit-policy")
            .and_then(|value| value.to_str().ok()),
        Some("\"reads\";q=1;w=60"),
    );

    let denied = request(router).await;
    assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
    let field = denied
        .headers()
        .get("ratelimit")
        .and_then(|value| value.to_str().ok())
        .expect("a denial carries the encoded service limit");
    assert!(
        field.starts_with("\"reads\";r=0;t="),
        "exhausted quota reports no remainder: {field}"
    );
}

#[test]
fn native_transport_types_keep_one_identity_through_the_facade() {
    fn native_policy(value: FixedWindowPolicy) -> runlimit_core::FixedWindowPolicy {
        value
    }
    fn native_state(value: draft_11::QuotaState) -> runlimit_http::draft_11::QuotaState {
        value
    }
    fn native_kind(value: RejectionKind) -> runlimit_axum::RejectionKind {
        value
    }
    fn native_capacity(value: Capacity) -> runlimit_core::Capacity {
        value
    }
    fn native_check(value: Check<'_>) -> runlimit_core::Check<'_> {
        value
    }
    fn native_store(value: MemoryStore) -> runlimit_memory::MemoryStore {
        value
    }
    let _ = (
        native_policy,
        native_state,
        native_kind,
        native_capacity,
        native_check,
        native_store,
    );
    let _: fn(&FixedWindowPolicy) -> Capacity = |configured| configured.quota();
}
