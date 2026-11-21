//! The application's request client inherits the adapter client's entry
//! check, for Axum's routing records and for the adapter's own private
//! records alike: it only adds the synthetic peer before delegating.

use super::*;
use axum::{
    extract::{
        FromRequestParts, MatchedPath, NestedPath, OriginalUri, RawPathParams,
        rejection::RawPathParamsRejection,
    },
    http::Extensions,
    middleware::from_fn,
    response::IntoResponse,
};
use batter::axum::{InProcessRefusal, low_level};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::{Layer, ServiceExt};

#[tokio::test]
async fn the_request_client_refuses_a_map_from_an_earlier_dispatch() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let captured = Arc::new(Mutex::new(None));
    let slot = captured.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let routes = GuardedRouter::new().route(
        "/inspect",
        get(move |request: axum::extract::Request| {
            counted.fetch_add(1, Ordering::SeqCst);
            *slot.lock().unwrap() = Some(request.extensions().clone());
            async { "inspected" }
        }),
    );
    let client = wrapped_boundary_app(routes, &handle).await;
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)), 41_000);
    let fresh = || {
        HttpRequest::builder()
            .uri("/inspect")
            .header(header::AUTHORIZATION, "Bearer fake-token")
            .body(Body::empty())
            .unwrap()
    };

    let served = client.request(fresh(), peer).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert!(served.extensions().get::<InProcessRefusal>().is_none());
    let earlier = captured.lock().unwrap().take().unwrap();

    // The whole map of that dispatch, on an otherwise identical request.
    let mut reused = fresh();
    *reused.extensions_mut() = earlier;
    let refused = client.request(reused, peer).await;
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(refused.extensions().get::<InProcessRefusal>().is_some());
    assert!(refused.headers().get("x-request-id").is_none());

    // That map also holds the adapter's own records. A map from a bare Axum
    // router holds only Axum's routing records, so only the routing check
    // can refuse it.
    let native = Arc::new(Mutex::new(None));
    let slot = native.clone();
    let bare = axum::Router::new().route(
        "/inspect",
        get(move |request: axum::extract::Request| {
            *slot.lock().unwrap() = Some(request.extensions().clone());
            async { "inspected" }
        }),
    );
    bare.oneshot(fresh()).await.unwrap();
    let routed = native.lock().unwrap().take().unwrap();
    assert!(routed.get::<MatchedPath>().is_some());
    let mut reused = fresh();
    *reused.extensions_mut() = routed;
    let refused = client.request(reused, peer).await;
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(refused.extensions().get::<InProcessRefusal>().is_some());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the handler ran for a reused map"
    );
}

/// A map holding only the adapter's own records: captured behind the
/// operational wrapper around a service that is not a `Router`, so no Axum
/// routing record is in it and only the private-record check can refuse it.
async fn batter_records_alone() -> Extensions {
    let captured = Arc::new(Mutex::new(None));
    let slot = captured.clone();
    let service = tower::service_fn(move |request: axum::extract::Request| {
        *slot.lock().unwrap() = Some(request.extensions().clone());
        async { Ok::<_, Infallible>(StatusCode::OK.into_response()) }
    });
    let request = HttpRequest::builder()
        .uri("/anything")
        .body(Body::empty())
        .unwrap();
    from_fn(low_level::operational_http)
        .layer(service)
        .oneshot(request)
        .await
        .unwrap();
    let extensions = captured.lock().unwrap().take().unwrap();
    assert!(extensions.get::<MatchedPath>().is_none());
    assert!(extensions.get::<NestedPath>().is_none());
    assert!(extensions.get::<OriginalUri>().is_none());
    let (mut parts, ()) = HttpRequest::new(()).into_parts();
    parts.extensions = extensions.clone();
    assert!(matches!(
        RawPathParams::from_request_parts(&mut parts, &()).await,
        Err(RawPathParamsRejection::MissingPathParams(_))
    ));
    extensions
}

#[tokio::test]
async fn the_request_client_refuses_a_map_with_only_the_adapters_records() {
    let (handle, approval) = ShutdownHandle::new_with_readiness_approval();
    approval.approve();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let routes = GuardedRouter::new().route(
        "/inspect",
        get(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            async { "inspected" }
        }),
    );
    let client = wrapped_boundary_app(routes, &handle).await;
    let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9)), 41_000);
    let mut reused = HttpRequest::builder()
        .uri("/inspect")
        .header(header::AUTHORIZATION, "Bearer fake-token")
        .body(Body::empty())
        .unwrap();
    *reused.extensions_mut() = batter_records_alone().await;

    let refused = client.request(reused, peer).await;
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(refused.extensions().get::<InProcessRefusal>().is_some());
    assert!(refused.headers().get("x-request-id").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the handler ran");
}
