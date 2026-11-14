//! A whole extension map from an earlier dispatch is refused at client entry,
//! before the router, any layer or any handler runs.

use super::{
    capture::Capture,
    support::{SECOND, body_text, count, ready_handle, request, request_policy},
};
use axum::{
    Router,
    body::Body,
    extract::{
        ConnectInfo, FromRequestParts, MatchedPath, NestedPath, OriginalUri, Path, RawPathParams,
        Request, rejection::RawPathParamsRejection,
    },
    http::{Extensions, Method, StatusCode, Uri},
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::{MethodRouter, any, get},
};
use batter_axum::{
    GuardedRouter, HttpBoundary, HttpFailure, InProcessClient, ReusedRoutingState,
    render_infrastructure_failure,
};
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

/// The extension map a handler or service captured from the request it served.
pub type Slot = Arc<Mutex<Option<Extensions>>>;

/// An application extension that a caller attaches to its requests.
#[derive(Clone, Debug)]
pub struct Label(pub &'static str);

pub const REFUSAL_BODY: &str =
    r#"{"code":"internal_error","message":"An internal error occurred","request_id":null}"#;

pub struct Fixture {
    pub client: InProcessClient,
    slot: Slot,
    /// Requests that reached the application's own layer.
    entered: Arc<AtomicUsize>,
    /// Calls of the `/view/{id}` handler.
    pub views: Arc<AtomicUsize>,
}

impl Fixture {
    pub async fn new() -> Self {
        let slot = Slot::default();
        let entered = Arc::new(AtomicUsize::new(0));
        let views = Arc::new(AtomicUsize::new(0));
        let counted = views.clone();
        let fallback = slot.clone();
        let layer = entered.clone();
        let routes = GuardedRouter::new()
            .route("/source/{source_id}", capturing(&slot))
            .route("/source", capturing(&slot))
            .nest(
                "/outer/{outer_id}",
                GuardedRouter::new().route("/source/{source_id}", capturing(&slot)),
            )
            .route(
                "/view/{id}",
                get(
                    move |Path(id): Path<u64>, matched: MatchedPath, request: Request| {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let extensions = request.extensions();
                        let label = extensions.get::<Label>().map(|label| label.0);
                        let peer = extensions
                            .get::<ConnectInfo<SocketAddr>>()
                            .map(|ConnectInfo(peer)| *peer);
                        let route = matched.as_str().to_owned();
                        async move { format!("id={id};route={route};label={label:?};peer={peer:?}") }
                    },
                ),
            )
            .route(
                "/broken",
                get(|| async { render_infrastructure_failure(HttpFailure::Internal, None) }),
            )
            .fallback(move |request: Request| {
                *fallback.lock().unwrap() = Some(request.extensions().clone());
                async { StatusCode::NOT_FOUND }
            })
            .layer(from_fn(move |request: Request, next: Next| {
                layer.fetch_add(1, Ordering::SeqCst);
                next.run(request)
            }));
        let client = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
            .assemble(routes)
            .await
            .unwrap()
            .in_process();
        Self {
            client,
            slot,
            entered,
            views,
        }
    }

    /// Dispatch `request` and return the extension map its handler captured.
    pub async fn captured(&self, request: Request) -> Extensions {
        let _ = self.client.request(request).await;
        self.slot.lock().unwrap().take().unwrap()
    }

    pub async fn captured_at(&self, path: &str) -> Extensions {
        self.captured(get_request(path)).await
    }

    /// Send `request`, which the client must refuse before any application
    /// layer or handler runs.
    pub async fn assert_refused(&self, request: Request) {
        let (entered, views) = (count(&self.entered), count(&self.views));
        let response = self.client.request(request).await;
        assert_refusal(response).await;
        assert_eq!(count(&self.entered), entered, "an application layer ran");
        assert_eq!(count(&self.views), views, "the destination handler ran");
    }
}

fn capturing(slot: &Slot) -> MethodRouter {
    let slot = slot.clone();
    any(move |request: Request| {
        *slot.lock().unwrap() = Some(request.extensions().clone());
        async { "captured" }
    })
}

pub fn get_request(path: &str) -> Request {
    request(Method::GET, path)
}

/// A new request to `path` whose extension map is `extensions`.
pub fn reusing(path: &str, extensions: Extensions) -> Request {
    let mut request = get_request(path);
    *request.extensions_mut() = extensions;
    request
}

pub async fn assert_refusal(response: Response) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.extensions().get::<ReusedRoutingState>().is_some(),
        "the refusal marker is missing"
    );
    assert!(!response.headers().contains_key("x-request-id"));
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(body_text(response).await, REFUSAL_BODY);
}

pub async fn raw_params(extensions: &Extensions) -> Result<Vec<String>, RawPathParamsRejection> {
    let (mut parts, _body) = reusing("/", extensions.clone()).into_parts();
    let params = RawPathParams::from_request_parts(&mut parts, &()).await?;
    Ok(params.iter().map(|(_, value)| value.to_owned()).collect())
}

#[tokio::test]
async fn a_map_from_a_captured_route_is_refused_for_another_captured_route() {
    let fixture = Fixture::new().await;
    let captured = fixture.captured_at("/source/7").await;
    assert_eq!(raw_params(&captured).await.unwrap(), ["7"]);
    assert_eq!(
        captured.get::<MatchedPath>().unwrap().as_str(),
        "/source/{source_id}"
    );
    fixture.assert_refused(reusing("/view/9", captured)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_map_from_a_nested_route_is_refused() {
    let fixture = Fixture::new().await;
    let captured = fixture.captured_at("/outer/3/source/7").await;
    assert_eq!(raw_params(&captured).await.unwrap(), ["3", "7"]);
    assert_eq!(
        captured.get::<NestedPath>().unwrap().as_str(),
        "/outer/{outer_id}"
    );
    fixture.assert_refused(reusing("/view/9", captured)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_map_with_an_invalid_utf8_capture_is_refused_for_a_valid_path() {
    let fixture = Fixture::new().await;
    let captured = fixture.captured_at("/source/%ff").await;
    assert!(matches!(
        raw_params(&captured).await,
        Err(RawPathParamsRejection::InvalidUtf8InPathParam(_))
    ));
    fixture
        .assert_refused(reusing("/view/9", captured.clone()))
        .await;

    // Without the nameable records the extractor's rejection is the only
    // evidence left, and it alone refuses the request.
    let mut alone = captured;
    alone.remove::<MatchedPath>();
    alone.remove::<NestedPath>();
    alone.remove::<OriginalUri>();
    assert!(matches!(
        raw_params(&alone).await,
        Err(RawPathParamsRejection::InvalidUtf8InPathParam(_))
    ));
    fixture.assert_refused(reusing("/view/9", alone)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_map_captured_in_a_fallback_is_refused() {
    let fixture = Fixture::new().await;
    let captured = fixture.captured_at("/missing/path").await;
    // A fallback dispatch writes only the original URI and an empty capture list.
    assert!(captured.get::<MatchedPath>().is_none());
    assert!(captured.get::<NestedPath>().is_none());
    assert_eq!(
        captured.get::<OriginalUri>().unwrap().0.path(),
        "/missing/path"
    );
    assert!(raw_params(&captured).await.unwrap().is_empty());
    fixture.assert_refused(reusing("/view/9", captured)).await;

    // A path no route pattern can match reaches the catch-all fallback
    // without a path-parameter record, but still with the original URI.
    let asterisk = Request::builder()
        .method(Method::OPTIONS)
        .uri("*")
        .body(Body::empty())
        .unwrap();
    let captured = fixture.captured(asterisk).await;
    assert!(matches!(
        raw_params(&captured).await,
        Err(RawPathParamsRejection::MissingPathParams(_))
    ));
    assert_eq!(captured.get::<OriginalUri>().unwrap().0, "*");
    fixture.assert_refused(reusing("/view/9", captured)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn a_map_from_a_native_nested_service_is_refused_with_its_private_record() {
    let slot = Slot::default();
    let inner = slot.clone();
    let native = Router::new().nest_service(
        "/mounted",
        tower::service_fn(move |request: Request| {
            *inner.lock().unwrap() = Some(request.extensions().clone());
            async { Ok::<_, Infallible>(StatusCode::OK.into_response()) }
        }),
    );
    native.oneshot(get_request("/mounted/inner")).await.unwrap();
    let captured = slot.lock().unwrap().take().unwrap();
    // A nested service receives Axum's private nested matched-path record
    // instead of a `MatchedPath`.
    assert!(captured.get::<MatchedPath>().is_none());
    assert_eq!(captured.get::<NestedPath>().unwrap().as_str(), "/mounted");
    assert!(raw_params(&captured).await.unwrap().is_empty());

    let mut isolated = captured.clone();
    isolated.remove::<NestedPath>();
    isolated.remove::<OriginalUri>();
    // The private record is present: native routing prefixes its pattern.
    let probe = Router::new().route(
        "/plain",
        get(|matched: MatchedPath| async move { matched.as_str().to_owned() }),
    );
    let prefixed = probe
        .oneshot(reusing("/plain", isolated.clone()))
        .await
        .unwrap();
    assert_eq!(body_text(prefixed).await, "/mounted/plain");

    let fixture = Fixture::new().await;
    fixture.assert_refused(reusing("/view/9", captured)).await;
    // Without any nameable record, the path-parameter record still refuses it.
    fixture.assert_refused(reusing("/view/9", isolated)).await;
    assert_eq!(count(&fixture.views), 0);
}

#[tokio::test]
async fn each_routing_record_alone_is_refused() {
    let fixture = Fixture::new().await;
    let routed = fixture.captured_at("/source/7").await;
    let nested = fixture.captured_at("/outer/3/source/7").await;
    let unmatched = fixture.captured_at("/missing").await;
    // A static route has no capture, and its match still writes a list.
    let fixed = fixture.captured_at("/source").await;
    assert_eq!(fixed.get::<MatchedPath>().unwrap().as_str(), "/source");
    assert!(raw_params(&fixed).await.unwrap().is_empty());

    let mut alone = Extensions::new();
    alone.insert(routed.get::<MatchedPath>().cloned().unwrap());
    fixture.assert_refused(reusing("/view/9", alone)).await;

    let mut alone = Extensions::new();
    alone.insert(nested.get::<NestedPath>().cloned().unwrap());
    fixture.assert_refused(reusing("/view/9", alone)).await;

    let mut alone = Extensions::new();
    alone.insert(OriginalUri(Uri::from_static("/elsewhere")));
    fixture.assert_refused(reusing("/view/9", alone)).await;

    // The path-parameter record has no public type; remove every nameable
    // record around it, with captures and with the empty list of a fallback
    // and of a static route.
    for mut parameters in [routed, unmatched, fixed] {
        parameters.remove::<MatchedPath>();
        parameters.remove::<NestedPath>();
        parameters.remove::<OriginalUri>();
        assert!(raw_params(&parameters).await.is_ok());
        fixture.assert_refused(reusing("/view/9", parameters)).await;
    }
    assert_eq!(count(&fixture.views), 0);
}

#[test]
fn a_refusal_emits_one_error_and_no_completion_or_request_fields() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let fixture = Fixture::new().await;
        let captured = fixture.captured_at("/source/4242424242").await;
        let response = fixture
            .client
            .request(reusing("/view/5151515151", captured))
            .with_subscriber(scoped.dispatch.clone())
            .await;
        assert_refusal(response).await;
    });
    let text = scoped.text();
    assert!(!text.contains("HTTP response boundary finished"), "{text}");
    assert!(!text.contains("batter.request"), "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert_eq!(
        text.lines().filter(|line| line.contains("ERROR")).count(),
        1,
        "{text}"
    );
    assert!(
        text.contains(
            "in-process request refused: its extensions carry routing state from an earlier dispatch"
        ),
        "{text}"
    );
    for leaked in ["4242424242", "5151515151", "/view", "/source", "source_id"] {
        assert!(!text.contains(leaked), "{leaked}: {text}");
    }
    // The source dispatch itself completed on the ambient dispatch.
    assert!(
        ambient.text().contains("HTTP response boundary finished"),
        "{}",
        ambient.text()
    );
}
