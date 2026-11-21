//! Arrival-shaped requests, including the supported migration that copies
//! application extensions by type, still route as fresh arrivals, and
//! ordinary nested routing inside one dispatch keeps accumulating captures.

use super::{
    capture::Capture,
    reused_routing::{Fixture, Label, REFUSAL_BODY, get_request, reusing},
    support::{SECOND, body_text, completions, count, ready_handle, request_policy},
};
use axum::{
    Router,
    extract::{ConnectInfo, MatchedPath, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use batter_axum::{CorrelationId, GuardedRouter, HttpBoundary, InProcessRefusal, RouteInventory};
use std::net::SocketAddr;
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn a_fresh_request_keeps_its_application_extensions() {
    let fixture = Fixture::new().await;
    let peer: SocketAddr = "192.0.2.10:4100".parse().unwrap();
    let mut fresh = get_request("/view/9");
    fresh.extensions_mut().insert(Label("caller"));
    fresh.extensions_mut().insert(ConnectInfo(peer));
    let response = fixture.client.request(fresh).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.extensions().get::<InProcessRefusal>().is_none());
    assert!(response.headers().contains_key("x-request-id"));
    assert_eq!(
        body_text(response).await,
        format!("id=9;route=/view/{{id}};label=Some(\"caller\");peer=Some({peer})")
    );
    assert_eq!(count(&fixture.views), 1);
}

#[tokio::test]
async fn a_boundary_built_internal_failure_carries_no_refusal_marker() {
    let fixture = Fixture::new().await;
    let response = fixture.client.request(get_request("/broken")).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.extensions().get::<InProcessRefusal>().is_none());
    assert!(response.headers().contains_key("x-request-id"));
    // The same sanitized body: only the marker separates a refusal from it.
    assert_eq!(body_text(response).await, REFUSAL_BODY);
}

#[tokio::test]
async fn a_dispatched_response_drops_a_refusal_marker_the_application_attached() {
    let inner = Fixture::new().await;
    let captured = inner.captured_at("/source/7").await;
    let refused = inner
        .client
        .request(reusing("/view/9", captured.clone()))
        .await;
    let marker = *refused.extensions().get::<InProcessRefusal>().unwrap();

    let forwarding = inner.client.clone();
    let routes = GuardedRouter::new()
        .route(
            "/copied",
            get(move || async move {
                // A marker read from an earlier refusal, beside the
                // application's own response extension.
                let mut response = "copied".into_response();
                response.extensions_mut().insert(marker);
                response.extensions_mut().insert(Label("kept"));
                response
            }),
        )
        .route(
            "/forwarded",
            get(move || {
                let (client, reused) = (forwarding.clone(), captured.clone());
                // The inner client's refusal, returned whole.
                async move { client.request(reusing("/view/9", reused)).await }
            }),
        );
    let outer = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
        .assemble(routes)
        .await
        .unwrap()
        .in_process();

    let copied = outer.request(get_request("/copied")).await;
    assert_eq!(copied.status(), StatusCode::OK);
    assert!(copied.extensions().get::<InProcessRefusal>().is_none());
    let kept = copied.extensions().get::<Label>().map(|label| label.0);
    assert_eq!(kept, Some("kept"), "other response extensions stay");
    assert!(copied.headers().contains_key("x-request-id"));
    assert_eq!(body_text(copied).await, "copied");

    // The handler ran, so this is a response the outer boundary built: it
    // has the inner refusal's status and body, a correlation header, and no
    // marker.
    let forwarded = outer.request(get_request("/forwarded")).await;
    assert_eq!(forwarded.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(forwarded.extensions().get::<InProcessRefusal>().is_none());
    assert!(forwarded.headers().contains_key("x-request-id"));
    assert_eq!(body_text(forwarded).await, REFUSAL_BODY);
}

#[test]
fn copying_application_extensions_by_type_routes_as_a_fresh_arrival() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    let (source_id, response_id, body) = ambient.block_on(async {
        let fixture = Fixture::new().await;
        let mut source = get_request("/source/7");
        source.extensions_mut().insert(Label("carried"));
        let captured = fixture.captured(source).await;
        let source_id = captured.get::<CorrelationId>().unwrap().to_string();

        // The supported migration: a fresh request with the application's
        // own extension copied by type, never the whole map.
        let mut fresh = get_request("/view/9");
        fresh
            .extensions_mut()
            .insert(captured.get::<Label>().cloned().unwrap());
        let response = fixture
            .client
            .request(fresh)
            .with_subscriber(scoped.dispatch.clone())
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.extensions().get::<InProcessRefusal>().is_none());
        let response_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(count(&fixture.views), 1);
        (source_id, response_id, body_text(response).await)
    });
    // The destination's own capture and pattern, not the source's.
    assert_eq!(
        body,
        "id=9;route=/view/{id};label=Some(\"carried\");peer=None"
    );
    assert_ne!(source_id, response_id);
    let text = scoped.text();
    let events = completions(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert!(events[0].contains("route=\"/view/{id}\""), "{text}");
    assert!(events[0].contains("status=200"), "{text}");
    assert!(
        events[0].contains(&format!("request_id=\"{response_id}\"")),
        "{text}"
    );
    assert!(!text.contains(&source_id), "{text}");
    assert!(!text.contains("ERROR"), "{text}");
}

#[test]
fn nested_routing_inside_one_dispatch_keeps_every_capture() {
    let capture = Capture::new();
    let bodies = capture.block_on(async {
        let item = GuardedRouter::new().route(
            "/item/{item_id}",
            get(
                |Path((outer, inner, item)): Path<(String, String, String)>,
                 matched: MatchedPath| async move {
                    format!("{outer}/{inner}/{item};{}", matched.as_str())
                },
            ),
        );
        let nested = GuardedRouter::new().nest(
            "/outer/{outer_id}",
            GuardedRouter::new().nest("/inner/{inner_id}", item),
        );
        // An admitted router is dispatched again after the native root
        // fallback has already written an empty capture list.
        let admitted = GuardedRouter::from_router(
            Router::new().route(
                "/declared/{declared_id}",
                get(|Path(id): Path<String>, matched: MatchedPath| async move {
                    format!("{id};{}", matched.as_str())
                }),
            ),
            RouteInventory::new(["/declared/{declared_id}"]).unwrap(),
        );
        let client = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
            .assemble(nested.merge(admitted))
            .await
            .unwrap()
            .in_process();
        let mut bodies = Vec::new();
        for path in ["/outer/a/inner/b/item/c", "/declared/d"] {
            let response = client.request(get_request(path)).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            bodies.push(body_text(response).await);
        }
        bodies
    });
    assert_eq!(
        bodies,
        [
            "a/b/c;/outer/{outer_id}/inner/{inner_id}/item/{item_id}",
            "d;/declared/{declared_id}",
        ]
    );
    let text = capture.text();
    let events = completions(&text);
    assert_eq!(events.len(), 2, "{text}");
    assert!(
        events[0].contains("route=\"/outer/{outer_id}/inner/{inner_id}/item/{item_id}\""),
        "{text}"
    );
    assert!(!text.contains("ERROR"), "{text}");
}

/// Send `body` to `path` through a service behind `limit` that hands each
/// request to `client` whole.
async fn forwarded_behind(
    limit: axum::extract::DefaultBodyLimit,
    client: &batter_axum::InProcessClient,
    path: &str,
    body: Vec<u8>,
) -> axum::response::Response {
    let client = client.clone();
    let limited = tower::Layer::layer(
        &limit,
        tower::service_fn(move |request: axum::extract::Request| {
            let client = client.clone();
            async move { Ok::<_, std::convert::Infallible>(client.request(request).await) }
        }),
    );
    tower::ServiceExt::oneshot(limited, sized(path, body))
        .await
        .unwrap()
}

fn sized(path: &str, body: Vec<u8>) -> axum::extract::Request {
    axum::extract::Request::builder()
        .method("POST")
        .uri(path)
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// Axum settings that a layer outside the client writes into the request are
/// not routing records. They reach the router like application extensions,
/// a limit set inside the boundary replaces them, and the documented rebuild
/// drops them.
#[tokio::test]
async fn a_body_limit_chosen_outside_the_client_reaches_the_router() {
    use axum::extract::DefaultBodyLimit;
    let size =
        || axum::routing::post(|body: axum::body::Bytes| async move { body.len().to_string() });
    let client = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
        .assemble(
            GuardedRouter::new()
                .route("/size", size())
                .route("/own-limit", size().layer(DefaultBodyLimit::max(1_000))),
        )
        .await
        .unwrap()
        .in_process();
    let ten = || b"ten bytes!".to_vec();
    // Axum's default limit is 2 MiB.
    let large = || vec![b'x'; 2 * 1024 * 1024 + 1];

    let outer = forwarded_behind(DefaultBodyLimit::max(4), &client, "/size", ten()).await;
    assert_eq!(outer.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(outer.extensions().get::<InProcessRefusal>().is_none());
    // A limit set inside the boundary overwrites the outer one.
    let inner = forwarded_behind(DefaultBodyLimit::max(4), &client, "/own-limit", ten()).await;
    assert_eq!(body_text(inner).await, "10");
    // An outer `disable()` removes Axum's default limit for the boundary's
    // extractors, which a served request would hit.
    let disabled = forwarded_behind(DefaultBodyLimit::disable(), &client, "/size", large()).await;
    assert_eq!(body_text(disabled).await, (2 * 1024 * 1024 + 1).to_string());

    // A request built fresh, as the rebuild migration does, carries no such
    // setting and gets the default limit.
    assert_eq!(
        body_text(client.request(sized("/size", ten())).await).await,
        "10"
    );
    let fresh = client.request(sized("/size", large())).await;
    assert_eq!(fresh.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
