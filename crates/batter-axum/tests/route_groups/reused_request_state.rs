//! Batter's own request records, and a live request another router already
//! dispatched, are refused at client entry like a reused routing map.

use super::{
    reused_routing::{Fixture, Label, assert_refusal, get_request, raw_params, reusing},
    support::{SECOND, body_text, count, ready_handle, request_policy},
};
use axum::{
    Router,
    body::Body,
    extract::{MatchedPath, NestedPath, OriginalUri, Request, rejection::RawPathParamsRejection},
    http::{Extensions, Method, StatusCode, Version},
    middleware::{from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::any,
};
use batter_axum::{CorrelationId, InProcessRefusal, RequestInterruptionResponder, low_level};
use batter_core::operation::OperationContext;
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
    assert!(response.extensions().get::<InProcessRefusal>().is_none());
    let generated = response.headers()["x-request-id"].to_str().unwrap();
    assert_ne!(generated, earlier.as_str());
    assert_eq!(count(&fixture.views), 1);
}

/// The documented migration for a request another router hands on: a fresh
/// request from the arrival's own parts, with each application extension
/// copied by type.
fn rebuild(arrival: Request) -> Request {
    let (parts, body) = arrival.into_parts();
    let mut request = Request::builder()
        .method(parts.method)
        .version(parts.version)
        .uri(parts.uri)
        .body(body)
        .unwrap();
    *request.headers_mut() = parts.headers;
    if let Some(label) = parts.extensions.get::<Label>() {
        request.extensions_mut().insert(label.clone());
    }
    request
}

/// A request carrying a method, version, header, body and application
/// extension that the destination's `/echo` route reports.
fn arrival(path: &str) -> Request {
    let mut request = Request::builder()
        .method(Method::POST)
        .version(Version::HTTP_2)
        .uri(path)
        .header("x-scenario", "forwarded")
        .body(Body::from("payload"))
        .unwrap();
    request.extensions_mut().insert(Label("carried"));
    request
}

#[tokio::test]
async fn a_live_request_forwarded_by_an_outer_router_is_refused_and_a_rebuilt_one_is_served() {
    let fixture = Fixture::new().await;
    let (forwarding, rebuilding, mounted) = (
        fixture.client.clone(),
        fixture.client.clone(),
        fixture.client.clone(),
    );
    let outer = Router::new()
        .route(
            "/echo",
            any(move |request: Request| {
                let client = rebuilding.clone();
                async move { client.request(rebuild(request)).await }
            }),
        )
        .nest_service(
            "/mounted",
            tower::service_fn(move |request: Request| {
                let client = mounted.clone();
                async move { Ok::<_, Infallible>(client.request(rebuild(request)).await) }
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

    // The rebuilt request keeps the arrival's method, version, target,
    // headers, body and copied application extension.
    let served = outer.clone().oneshot(arrival("/echo")).await.unwrap();
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(
        body_text(served).await,
        r#"POST HTTP/2.0 /echo scenario=Some("forwarded") label=Some("carried") body=payload"#
    );

    // Under `nest_service` the arrival's target is the path below the mount,
    // so the rebuilt request is dispatched there.
    let served = outer.oneshot(arrival("/mounted/echo")).await.unwrap();
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(
        body_text(served).await,
        r#"POST HTTP/2.0 /echo scenario=Some("forwarded") label=Some("carried") body=payload"#
    );
}

/// Send `request` through `wrapper`, whose response the caller inspects.
async fn through<S>(wrapper: S, request: Request) -> Response
where
    S: tower::Service<Request, Response = Response, Error = Infallible>,
{
    wrapper.oneshot(request).await.unwrap()
}

#[tokio::test]
async fn a_live_request_forwarded_from_inside_a_low_level_wrapper_is_refused() {
    let fixture = Fixture::new().await;
    let client = fixture.client.clone();
    let forward = tower::service_fn(move |request: Request| {
        let client = client.clone();
        async move { Ok::<_, Infallible>(client.request(request).await) }
    });
    let policy = request_policy(&ready_handle(), SECOND);
    let entered = fixture.entered();

    // No router ran, so each wrapper's own record is what refuses it. The
    // plain observer passes the refusal through untouched.
    let observed = from_fn(low_level::observe_http).layer(forward.clone());
    assert_refusal(through(observed, get_request("/view/9")).await).await;

    // The other wrappers rewrite the refused response on the way out, but
    // the marker survives each of them.
    let refused = [
        through(
            from_fn(low_level::operational_http).layer(forward.clone()),
            get_request("/view/9"),
        )
        .await,
        through(
            from_fn(low_level::operational_http_with_quota).layer(forward.clone()),
            get_request("/view/9"),
        )
        .await,
        through(
            from_fn_with_state(policy, low_level::request_scope).layer(forward.clone()),
            get_request("/view/9"),
        )
        .await,
    ];
    for (index, response) in refused.into_iter().enumerate() {
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{index}"
        );
        assert!(
            response.extensions().get::<InProcessRefusal>().is_some(),
            "{index}"
        );
    }
    assert_eq!(fixture.entered(), entered, "an application layer ran");
    assert_eq!(count(&fixture.views), 0);

    // A rebuilt request passes the same wrapper.
    let observed = from_fn(low_level::observe_http).layer(tower::service_fn({
        let client = fixture.client.clone();
        move |request: Request| {
            let client = client.clone();
            async move { Ok::<_, Infallible>(client.request(rebuild(request)).await) }
        }
    }));
    let served = through(observed, get_request("/view/9")).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(count(&fixture.views), 1);
}

#[tokio::test]
async fn a_map_captured_behind_request_admission_alone_carries_only_public_values() {
    let fixture = Fixture::new().await;
    let slot = Arc::new(Mutex::new(None));
    let capture = slot.clone();
    let service = tower::service_fn(move |request: Request| {
        *capture.lock().unwrap() = Some(request.extensions().clone());
        async { Ok::<_, Infallible>(StatusCode::OK.into_response()) }
    });
    // Without the operational wrapper, admission records no admission and
    // inserts only its public context and interruption responder.
    let admitted = from_fn_with_state(
        request_policy(&ready_handle(), SECOND),
        low_level::request_admission,
    )
    .layer(service);
    admitted.oneshot(get_request("/anything")).await.unwrap();
    let captured = slot.lock().unwrap().take().unwrap();
    assert_no_routing_record(&captured).await;
    // The earlier request's context was cancelled when its response was built.
    assert!(captured.get::<OperationContext>().unwrap().check().is_err());
    assert!(captured.get::<RequestInterruptionResponder>().is_some());

    // Neither is a private record, so the request is served, and the
    // boundary's own admission gives the admitted route a fresh context.
    let response = fixture.client.request(reusing("/context", captured)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.extensions().get::<InProcessRefusal>().is_none());
    assert_eq!(body_text(response).await, "live");
}
