//! The test transport accepts only arrival-shaped requests, exactly as the
//! Axum adapter's in-process client does: a whole extension map from an
//! earlier dispatch is refused at entry, before lifecycle admission,
//! authentication, the native quota check or any handler runs.

use super::*;
use axum::{
    extract::{ConnectInfo, MatchedPath, OriginalUri, Path},
    http::{Extensions, HeaderValue, Uri},
    response::{IntoResponse, Response},
};
use batter_axum::ReusedRoutingState;
use batter_runlimit::http::TestClient;

const REFUSAL_BODY: &str =
    r#"{"code":"internal_error","message":"An internal error occurred","request_id":null}"#;

/// An application extension that a caller attaches to its requests.
#[derive(Clone, Debug)]
struct Label(&'static str);

/// Counts of protected work: authentication calls, native batch checks and
/// destination handler calls.
type Counts = (usize, usize, usize);

struct Fixture {
    client: TestClient,
    captured: Arc<Mutex<Option<Extensions>>>,
    forwarded: Arc<Mutex<Option<ReusedRoutingState>>>,
    authentications: Arc<AtomicUsize>,
    backend: Backend,
    views: Arc<AtomicUsize>,
}

impl Fixture {
    fn new(running: &batter_core::lifecycle::RunningSupervisor) -> Self {
        let captured = Arc::new(Mutex::new(None));
        let forwarded = Arc::new(Mutex::new(None::<ReusedRoutingState>));
        let authentications = Arc::new(AtomicUsize::new(0));
        let backend = Backend::new(Mode::Return(allowed()));
        let views = Arc::new(AtomicUsize::new(0));
        let (slot, marker, counted, calls) = (
            captured.clone(),
            forwarded.clone(),
            views.clone(),
            authentications.clone(),
        );
        let routes = Router::new()
            .route(
                "/source/{source_id}",
                post(move |request: Request| {
                    *slot.lock().unwrap() = Some(request.extensions().clone());
                    async { "captured" }
                }),
            )
            .route(
                "/view/{id}",
                post(
                    move |Path(id): Path<u64>,
                          matched: MatchedPath,
                          principal: Authenticated<Principal>,
                          request: Request| {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let extensions = request.extensions();
                        let label = extensions.get::<Label>().map(|label| label.0);
                        let peer = extensions
                            .get::<ConnectInfo<SocketAddr>>()
                            .map(|ConnectInfo(peer)| *peer);
                        let (route, owner) = (matched.as_str().to_owned(), principal.principal().0);
                        async move {
                            format!(
                                "id={id};route={route};owner={owner};label={label:?};peer={peer:?}"
                            )
                        }
                    },
                ),
            )
            .route(
                "/forward",
                post(move || {
                    // A handler that puts a marker it read from an earlier
                    // refusal, and an extension of its own, on its response.
                    let copied = *marker.lock().unwrap();
                    async move {
                        let mut response = "forwarded".into_response();
                        response.extensions_mut().insert(copied.unwrap());
                        response.extensions_mut().insert(Label("kept"));
                        response
                    }
                }),
            );
        let hasher = KeyHasher::new([9; 32]).unwrap();
        let client = HttpQuota::new(
            Quota::new(backend.clone()),
            vec![policy("owner", 10)],
            move |input: AuthInput| {
                calls.fetch_add(1, Ordering::SeqCst);
                let authorized = input.headers.get(header::AUTHORIZATION)
                    == Some(&HeaderValue::from_static("Bearer secret-a"));
                async move {
                    if authorized {
                        Ok(Principal("owner-a"))
                    } else {
                        Err(AuthError)
                    }
                }
            },
            move |principal: &Principal, _: DirectPeer, policy: &FixedWindowPolicy| {
                hasher
                    .hash_for(policy, principal.0)
                    .into_unbound_subject_key()
            },
        )
        .unwrap()
        .prepare(request_policy(running, 1000), routes)
        .in_process();
        Self {
            client,
            captured,
            forwarded,
            authentications,
            backend,
            views,
        }
    }

    fn counts(&self) -> Counts {
        (
            self.authentications.load(Ordering::SeqCst),
            self.backend.calls.load(Ordering::SeqCst),
            self.views.load(Ordering::SeqCst),
        )
    }

    /// Serve a labelled request at a captured route and return the whole
    /// extension map its handler saw.
    async fn captured_map(&self) -> Extensions {
        let mut source = request("/source/7", "Bearer secret-a");
        source.extensions_mut().insert(Label("chosen"));
        let response = self.client.request(source, peer()).await;
        assert_eq!(response.status(), StatusCode::OK);
        self.captured.lock().unwrap().take().unwrap()
    }
}

async fn text(response: Response) -> String {
    String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn a_reused_captured_route_map_is_refused_before_the_protected_assembly() {
    let running = running().await;
    let fixture = Fixture::new(&running);
    let captured = fixture.captured_map().await;
    // The map a handler sees carries the router's routing records and the
    // assembly's own authentication evidence.
    assert!(captured.get::<MatchedPath>().is_some());
    assert!(captured.get::<OriginalUri>().is_some());
    assert!(captured.get::<Authenticated<Principal>>().is_some());
    let before = fixture.counts();

    let capture = Capture::new("info");
    let mut reused = request("/view/9", "Bearer secret-a");
    *reused.extensions_mut() = captured;
    let refused = capture.run(fixture.client.request(reused, peer())).await;

    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(refused.extensions().get::<ReusedRoutingState>().is_some());
    assert!(refused.headers().get("x-request-id").is_none());
    assert_eq!(refused.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(text(refused).await, REFUSAL_BODY);
    assert_eq!(
        fixture.counts(),
        before,
        "authentication, the quota check or the destination handler ran"
    );
    let output = capture.text();
    assert_eq!(
        output.matches("in-process request refused").count(),
        1,
        "{output}"
    );
    assert!(output.contains("ERROR"), "{output}");
    assert!(capture.events().is_empty(), "{output}");
    for secret in [
        "/view/9",
        "/source",
        "source_id",
        "secret-a",
        "owner-a",
        "chosen",
    ] {
        assert!(!output.contains(secret), "{secret} in {output}");
    }
    finish(running).await;
}

#[tokio::test]
async fn a_fresh_request_with_application_extensions_reaches_the_assembly_untouched() {
    let running = running().await;
    let fixture = Fixture::new(&running);
    let captured = fixture.captured_map().await;
    let before = fixture.counts();

    // The migration: a fresh request with each application extension copied
    // by type. A caller-chosen peer is replaced by the transport's own.
    let capture = Capture::new("info");
    let mut fresh = request("/view/9", "Bearer secret-a");
    fresh
        .extensions_mut()
        .insert(captured.get::<Label>().unwrap().clone());
    fresh
        .extensions_mut()
        .insert(ConnectInfo::<SocketAddr>("10.0.0.1:1".parse().unwrap()));
    let response = capture.run(fixture.client.request(fresh, peer())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.extensions().get::<ReusedRoutingState>().is_none());
    assert!(response.headers().contains_key("x-request-id"));
    assert_eq!(
        text(response).await,
        r#"id=9;route=/view/{id};owner=owner-a;label=Some("chosen");peer=Some(127.0.0.1:12345)"#
    );
    assert_eq!(fixture.counts(), (before.0 + 1, before.1 + 1, before.2 + 1));
    let events = capture.events();
    assert_eq!(events.len(), 1, "{}", capture.text());
    assert!(
        events[0].contains("route=\"/view/{id}\""),
        "{}",
        capture.text()
    );
    assert!(!capture.text().contains("in-process request refused"));
    finish(running).await;
}

#[tokio::test]
async fn a_dispatched_response_drops_a_refusal_marker_the_application_attached() {
    let running = running().await;
    let fixture = Fixture::new(&running);
    let mut alone = request("/forward", "Bearer secret-a");
    alone
        .extensions_mut()
        .insert(OriginalUri(Uri::from_static("/earlier")));
    let refused = fixture.client.request(alone, peer()).await;
    let marker = refused.extensions().get::<ReusedRoutingState>().copied();
    assert!(marker.is_some());
    *fixture.forwarded.lock().unwrap() = marker;

    let response = fixture
        .client
        .request(request("/forward", "Bearer secret-a"), peer())
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.extensions().get::<ReusedRoutingState>().is_none());
    assert_eq!(
        response.extensions().get::<Label>().map(|label| label.0),
        Some("kept")
    );
    assert_eq!(text(response).await, "forwarded");
    finish(running).await;
}
