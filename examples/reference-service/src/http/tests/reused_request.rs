//! The application's request client inherits the adapter client's entry
//! check: it only adds the synthetic peer before delegating.

use super::*;
use batter::axum::ReusedRoutingState;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    assert!(served.extensions().get::<ReusedRoutingState>().is_none());
    let earlier = captured.lock().unwrap().take().unwrap();

    // The whole map of that dispatch, on an otherwise identical request.
    let mut reused = fresh();
    *reused.extensions_mut() = earlier;
    let refused = client.request(reused, peer).await;
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(refused.extensions().get::<ReusedRoutingState>().is_some());
    assert!(refused.headers().get("x-request-id").is_none());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the handler ran for the reused map"
    );
}
