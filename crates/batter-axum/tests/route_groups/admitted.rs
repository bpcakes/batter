//! Routers built outside `GuardedRouter` serve only their declared routes,
//! inside their group's policy.

use super::{
    capture::Capture,
    support::{
        ORIGIN, SECOND, assert_not_private, assert_private, body_text, browser_group, count,
        exact_origin, from_origin, ready_handle, request, request_policy,
    },
};
use axum::{
    Router,
    extract::{MatchedPath, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware,
    response::Response,
    routing::{MethodRouter, get, put},
};
use batter_axum::{
    GuardedRouter, HttpBoundary, ProbePath, RouteGroup, RouteInventory,
    browser::PrivateResponsePolicy,
};
use batter_core::lifecycle::ShutdownHandle;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::time::Instant;
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

#[derive(Clone)]
struct Catalog(&'static str);

fn undeclared_route(calls: &Arc<AtomicUsize>) -> MethodRouter<Catalog> {
    let calls = calls.clone();
    get(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { "undeclared" }
    })
}

/// A stateful router as another router builder would produce it, with one
/// undeclared route inside the declared paths, one outside them, and its own
/// fallback. `undeclared` counts calls to everything the inventory omits.
fn catalog(undeclared: &Arc<AtomicUsize>) -> Router<Catalog> {
    let fallback = undeclared.clone();
    Router::new()
        .route(
            "/items",
            get(|State(Catalog(name)): State<Catalog>| async move { name }),
        )
        .route(
            "/items/{id}",
            get(|Path(id): Path<u32>, matched: MatchedPath| async move {
                format!("{id}:{}", matched.as_str())
            })
            .put(std::future::pending::<&'static str>),
        )
        .route("/items/featured", undeclared_route(undeclared))
        .route("/hidden", undeclared_route(undeclared))
        .fallback(move || {
            fallback.fetch_add(1, Ordering::SeqCst);
            async { "catalog fallback" }
        })
}

fn declared_catalog(undeclared: &Arc<AtomicUsize>) -> GuardedRouter {
    GuardedRouter::from_router(
        catalog(undeclared),
        RouteInventory::new(["/items", "/items/{id}"]).unwrap(),
    )
    .with_state(Catalog("catalog"))
}

fn default_routes() -> GuardedRouter {
    GuardedRouter::new()
        .route("/work", get(|| async { "work" }))
        .fallback(|| async { (StatusCode::NOT_FOUND, "default fallback") })
}

async fn with_items_group(handle: &ShutdownHandle, items: GuardedRouter) -> Router {
    let policy = browser_group(
        handle,
        3 * SECOND,
        PrivateResponsePolicy::NoReferrer,
        exact_origin(),
    );
    HttpBoundary::new(request_policy(handle, SECOND))
        .with_liveness(ProbePath::new("/live").unwrap())
        .unwrap()
        .with_group(RouteGroup::new("items", policy, items))
        .unwrap()
        .assemble(default_routes())
        .await
        .unwrap()
        .into_router()
}

async fn call(app: &Router, request: Request) -> (StatusCode, HeaderMap, String) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    (status, headers, body_text(response).await)
}

#[tokio::test]
async fn admitted_router_serves_declared_routes_inside_its_group_policy() {
    let undeclared = Arc::new(AtomicUsize::new(0));
    let handle = ready_handle();
    let app = with_items_group(&handle, declared_catalog(&undeclared)).await;

    let (status, headers, body) = call(&app, request(Method::GET, "/items")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "catalog"));
    assert_private(&headers, "no-referrer");
    // Native routing runs once: one path parameter and one matched pattern.
    let (status, _, body) = call(&app, request(Method::GET, "/items/7")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "7:/items/{id}"));

    // The group's mutation checks and method handling cover the admitted routes.
    let cross_site = from_origin(Method::PUT, "/items/7", "https://attacker.example");
    let (status, headers, body) = call(&app, cross_site).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.starts_with("browser:"), "{body}");
    assert_private(&headers, "no-referrer");
    let (status, headers, _) = call(&app, from_origin(Method::DELETE, "/items/7", ORIGIN)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_private(&headers, "no-referrer");

    // Undeclared routes and the router's own fallback never serve a request.
    for path in ["/items/featured", "/hidden", "/missing"] {
        let (status, headers, body) = call(&app, request(Method::GET, path)).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_FOUND, "default fallback"),
            "{path}"
        );
        assert_not_private(&headers);
    }
    assert_eq!(
        call(&app, request(Method::GET, "/live")).await.0,
        StatusCode::OK
    );
    assert_eq!(count(&undeclared), 0);

    handle.request();
    let (status, headers, _) = call(&app, request(Method::GET, "/items")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_private(&headers, "no-referrer");
}

#[tokio::test(start_paused = true)]
async fn admitted_routes_expire_at_their_group_budget() {
    let undeclared = Arc::new(AtomicUsize::new(0));
    let app = with_items_group(&ready_handle(), declared_catalog(&undeclared)).await;
    let started = Instant::now();
    let (status, headers, body) = call(&app, from_origin(Method::PUT, "/items/7", ORIGIN)).await;
    assert_eq!(started.elapsed(), 3 * SECOND);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("deadline_exceeded"), "{body}");
    assert_private(&headers, "no-referrer");
}

async fn route_tag(mut response: Response) -> Response {
    let tag = HeaderValue::from_static("route");
    response.headers_mut().append("x-layer", tag);
    response
}

async fn router_tag(mut response: Response) -> Response {
    let tag = HeaderValue::from_static("router");
    response.headers_mut().append("x-layer", tag);
    response
}

#[tokio::test]
async fn nesting_merging_and_layers_carry_the_admitted_router() {
    let undeclared = Arc::new(AtomicUsize::new(0));
    let versioned = GuardedRouter::new()
        .route("/status", get(|| async { "native" }))
        .merge(declared_catalog(&undeclared))
        .route_layer(middleware::map_response(route_tag))
        .layer(middleware::map_response(router_tag));
    let default = default_routes().nest("/v1", versioned);
    let app = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
        .assemble(default)
        .await
        .unwrap()
        .into_router();

    for (path, expected) in [
        ("/v1/items/7", "7:/v1/items/{id}"),
        ("/v1/items", "catalog"),
        ("/v1/status", "native"),
    ] {
        let (status, headers, body) = call(&app, request(Method::GET, path)).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::OK, expected),
            "{path}"
        );
        let layers: Vec<_> = headers.get_all("x-layer").iter().collect();
        assert_eq!(layers, ["route", "router"], "{path}");
    }
    for path in ["/v1/items/featured", "/items/7", "/v1/missing"] {
        let (status, _, body) = call(&app, request(Method::GET, path)).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_FOUND, "default fallback"),
            "{path}"
        );
    }
    assert_eq!(count(&undeclared), 0);

    // A route layer applied to an admitted router alone needs no native route.
    let alone = declared_catalog(&undeclared).route_layer(middleware::map_response(route_tag));
    let app = HttpBoundary::new(request_policy(&ready_handle(), SECOND))
        .assemble(alone)
        .await
        .unwrap()
        .into_router();
    let (status, headers, _) = call(&app, request(Method::GET, "/items")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-layer"], "route");
}

/// A layer that counts how often Axum builds the service it wraps.
#[derive(Clone)]
struct CountBuilds(Arc<AtomicUsize>);

impl<S> tower::Layer<S> for CountBuilds {
    type Service = S;

    fn layer(&self, inner: S) -> S {
        self.0.fetch_add(1, Ordering::SeqCst);
        inner
    }
}

#[tokio::test]
async fn served_requests_reuse_layers_built_before_serving() {
    for admitted in [false, true] {
        let builds = Arc::new(AtomicUsize::new(0));
        let handle = ready_handle();
        let default = GuardedRouter::new()
            .route("/work", get(|| async { "work" }))
            .layer(CountBuilds(builds.clone()));
        let mut boundary = HttpBoundary::new(request_policy(&handle, SECOND));
        if admitted {
            let converted = Router::new()
                .route("/items/{id}", get(|| async { "item" }))
                .layer(CountBuilds(builds.clone()));
            let inventory = RouteInventory::new(["/items/{id}"]).unwrap();
            let items =
                GuardedRouter::from_router(converted, inventory).layer(CountBuilds(builds.clone()));
            let group = RouteGroup::new("items", request_policy(&handle, SECOND), items);
            boundary = boundary.with_group(group).unwrap();
        }
        let assembled = boundary.assemble(default).await.unwrap();
        // Prepare the router once, as registration with the direct peer does.
        let app = assembled
            .into_router()
            .into_make_service()
            .oneshot(())
            .await
            .unwrap();
        let built = count(&builds);
        for path in ["/work", "/items/7", "/work", "/items/8", "/missing"] {
            call(&app, request(Method::GET, path)).await;
        }
        assert_eq!(count(&builds), built, "admitted={admitted}");
    }
}

fn completions(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect()
}

#[test]
fn every_admitted_request_has_one_completion_naming_its_route() {
    let capture = Capture::new();
    let statuses = capture.block_on(async {
        let undeclared = Arc::new(AtomicUsize::new(0));
        let app = with_items_group(&ready_handle(), declared_catalog(&undeclared)).await;
        let mut statuses = Vec::new();
        for path in ["/items/7", "/items/featured", "/work"] {
            statuses.push(call(&app, request(Method::GET, path)).await.0.as_u16());
        }
        statuses
    });
    assert_eq!(statuses, [200, 404, 200]);
    let text = capture.text();
    let events = completions(&text);
    assert_eq!(events.len(), 3, "{text}");
    for (event, route) in events.iter().zip(["/items/{id}", "<unmatched>", "/work"]) {
        assert!(event.contains(&format!("route=\"{route}\"")), "{text}");
    }
}

struct DropTrace;

impl Drop for DropTrace {
    fn drop(&mut self) {
        tracing::info!(resource = "admitted.resource", "captured resource dropped");
    }
}

#[test]
fn aborted_admitted_request_is_destroyed_under_its_first_poll_dispatch() {
    let scoped = Capture::new();
    let ambient = Capture::new();
    ambient.block_on(async {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(Mutex::new(Some(started_tx)));
        let converted = Router::new().route(
            "/items/{id}",
            put(move || {
                let started = started.clone();
                async move {
                    let _resource = DropTrace;
                    started.lock().unwrap().take().unwrap().send(()).unwrap();
                    std::future::pending::<&'static str>().await
                }
            }),
        );
        let items =
            GuardedRouter::from_router(converted, RouteInventory::new(["/items/{id}"]).unwrap());
        let app = with_items_group(&ready_handle(), items).await;
        let task = tokio::spawn(
            app.oneshot(from_origin(Method::PUT, "/items/7", ORIGIN))
                .with_subscriber(scoped.dispatch.clone()),
        );
        started_rx.await.unwrap();
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled(), "{error}");
    });
    let text = scoped.text();
    assert_eq!(
        text.matches("operation boundary finished").count(),
        1,
        "{text}"
    );
    let events = completions(&text);
    assert_eq!(events.len(), 1, "{text}");
    assert!(events[0].contains("http_outcome=\"dropped\""), "{text}");
    assert!(text.contains("admitted.resource"), "{text}");
    assert!(ambient.text().is_empty(), "{}", ambient.text());
}
