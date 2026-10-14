//! Several admitted routers, each offered only the paths its declared patterns
//! match.

use super::support::{SECOND, body_text, count, counted, ready_handle, request, request_policy};
use axum::{
    Router,
    extract::Path,
    http::{Method, StatusCode},
    routing::get,
};
use batter_axum::{GuardedRouter, HttpBoundary, RouteGroup, RouteInventory};
use std::sync::{Arc, atomic::AtomicUsize};

/// A router declaring only `declared`, whose handler names `name` and the
/// captured value, beside counted undeclared routes.
fn admitted(
    name: &'static str,
    declared: &'static str,
    undeclared: &[&'static str],
    calls: &Arc<AtomicUsize>,
) -> GuardedRouter {
    let router = undeclared.iter().fold(
        Router::new().route(
            declared,
            get(move |Path(value): Path<String>| async move { format!("{name}:{value}") }),
        ),
        |router, path| router.route(path, counted(calls, "undeclared")),
    );
    GuardedRouter::from_router(router, RouteInventory::new([declared]).unwrap())
}

#[tokio::test]
async fn each_admitted_router_serves_only_the_paths_it_declares() {
    let undeclared = Arc::new(AtomicUsize::new(0));
    let handle = ready_handle();
    // The first router would itself route the second router's declared path
    // to an undeclared route, and its own literal precedes its declared capture.
    let first = admitted("first", "/a/{id}", &["/a/special", "/b/{id}"], &undeclared);
    let second = admitted("second", "/b/{id}", &[], &undeclared);
    let reports = admitted("reports", "/c/{*rest}", &["/hidden"], &undeclared);
    let default = GuardedRouter::new()
        .route("/native", get(|| async { "native" }))
        .merge(first)
        .merge(second)
        .fallback(|| async { (StatusCode::NOT_FOUND, "default fallback") });
    let group = RouteGroup::new("reports", request_policy(&handle, SECOND), reports);
    let app = HttpBoundary::new(request_policy(&handle, SECOND))
        .with_group(group)
        .unwrap()
        .assemble(default)
        .await
        .unwrap()
        .in_process();

    for (path, status, body) in [
        ("/native", StatusCode::OK, "native"),
        ("/a/7", StatusCode::OK, "first:7"),
        ("/b/7", StatusCode::OK, "second:7"),
        ("/c/x/y", StatusCode::OK, "reports:x/y"),
        ("/a/special", StatusCode::NOT_FOUND, "default fallback"),
        ("/hidden", StatusCode::NOT_FOUND, "default fallback"),
        ("/c/", StatusCode::NOT_FOUND, "default fallback"),
        ("/missing", StatusCode::NOT_FOUND, "default fallback"),
    ] {
        let response = app.request(request(Method::GET, path)).await;
        let actual = response.status();
        assert_eq!(
            (actual, body_text(response).await.as_str()),
            (status, body),
            "{path}"
        );
    }
    assert_eq!(count(&undeclared), 0);
}
