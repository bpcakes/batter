//! Batter's own request records, and a live request another router already
//! dispatched, are refused at client entry like a reused routing map.

use super::{
    reused_routing::{Fixture, Label, assert_refusal, get_request, raw_params, reusing},
    support::{body_text, count},
};
use axum::{
    Router,
    body::Body,
    extract::{MatchedPath, NestedPath, OriginalUri, Request, rejection::RawPathParamsRejection},
    http::{Extensions, Method, StatusCode},
    middleware::from_fn,
    response::IntoResponse,
    routing::any,
};
use batter_axum::{CorrelationId, ReusedRoutingState, low_level};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tower::{Layer, ServiceExt};

/// No record that Axum routing writes is left in the map.
async fn assert_no_routing_record(extensions: &Extensions) {
    assert!(extensions.get::<MatchedPath>().is_none());
    assert!(extensions.get::<NestedPath>().is_none());
    assert!(extensions.get::<OriginalUri>().is_none());
    assert!(matches!(
        raw_params(extensions).await,
        Err(RawPathParamsRejection::MissingPathParams(_))
    ));
}

#[tokio::test]
async fn a_map_with_only_batter_records_is_refused() {
    let fixture = Fixture::new().await;
    // A path no route pattern can match reaches the boundary's catch-all
    // fallback with the original URI as its only routing record.
    let asterisk = Request::builder()
        .method(Method::OPTIONS)
        .uri("*")
        .body(Body::empty())
        .unwrap();
    let mut captured = fixture.captured(asterisk).await;
    captured.remove::<OriginalUri>();
    assert_no_routing_record(&captured).await;
    // What remains is the boundary's own state for the earlier request.
    assert!(captured.get::<CorrelationId>().is_some());

    fixture.assert_refused(reusing("/view/9", captured)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_map_captured_behind_a_low_level_wrapper_is_refused() {
    let fixture = Fixture::new().await;
    let slot = Arc::new(Mutex::new(None));
    let capture = slot.clone();
    // A service that is not a `Router` writes no routing record at all.
    let service = tower::service_fn(move |request: Request| {
        *capture.lock().unwrap() = Some(request.extensions().clone());
        async { Ok::<_, Infallible>(StatusCode::OK.into_response()) }
    });

    // The observer alone leaves only its observation record.
    let observed = from_fn(low_level::observe_http).layer(service.clone());
    observed.oneshot(get_request("/anything")).await.unwrap();
    let captured = slot.lock().unwrap().take().unwrap();
    assert_no_routing_record(&captured).await;
    assert!(captured.get::<CorrelationId>().is_none());
    fixture.assert_refused(reusing("/view/9", captured)).await;

    // The operational wrapper adds its marker beside the observer's record.
    let operational = from_fn(low_level::operational_http).layer(service);
    operational.oneshot(get_request("/anything")).await.unwrap();
    let captured = slot.lock().unwrap().take().unwrap();
    assert_no_routing_record(&captured).await;
    assert!(captured.get::<CorrelationId>().is_some());
    fixture.assert_refused(reusing("/view/9", captured)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_public_correlation_copied_by_type_is_replaced_not_refused() {
    let fixture = Fixture::new().await;
    let captured = fixture.captured_at("/source/7").await;
    let earlier = captured.get::<CorrelationId>().cloned().unwrap();

    // The public correlation value is application-visible metadata, not one
    // of the boundary's private records.
    let mut fresh = get_request("/view/9");
    fresh.extensions_mut().insert(earlier.clone());
    let response = fixture.client.request(fresh).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.extensions().get::<ReusedRoutingState>().is_none());
    let generated = response.headers()["x-request-id"].to_str().unwrap();
    assert_ne!(generated, earlier.as_str());
    assert_eq!(count(&fixture.views), 1);
}

#[tokio::test]
async fn a_live_request_forwarded_by_an_outer_router_is_refused_and_a_rebuilt_one_is_served() {
    let fixture = Fixture::new().await;
    let forwarding = fixture.client.clone();
    let rebuilding = fixture.client.clone();
    let outer = Router::new()
        .route(
            "/rebuilt",
            any(move |request: Request| {
                let client = rebuilding.clone();
                async move {
                    // The supported shape: a fresh request from the arrival's
                    // method, headers and body, with each application
                    // extension copied by type.
                    let (parts, body) = request.into_parts();
                    let mut fresh = Request::builder()
                        .method(parts.method)
                        .uri("/view/9")
                        .body(body)
                        .unwrap();
                    *fresh.headers_mut() = parts.headers;
                    if let Some(label) = parts.extensions.get::<Label>() {
                        fresh.extensions_mut().insert(label.clone());
                    }
                    client.request(fresh).await
                }
            }),
        )
        .fallback(move |request: Request| {
            let client = forwarding.clone();
            // The outer router's own dispatch already wrote routing records.
            async move { client.request(request).await }
        });

    let entered = fixture.entered();
    let refused = outer.clone().oneshot(get_request("/view/9")).await.unwrap();
    // The outer router is not a client, so the forwarded marker is visible.
    assert_refusal(refused).await;
    assert_eq!(fixture.entered(), entered, "an application layer ran");
    assert_eq!(count(&fixture.views), 0);

    let mut arrival = get_request("/rebuilt");
    arrival.extensions_mut().insert(Label("carried"));
    let served = outer.oneshot(arrival).await.unwrap();
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(
        body_text(served).await,
        "id=9;route=/view/{id};label=Some(\"carried\");peer=None"
    );
    assert_eq!(count(&fixture.views), 1);
}
