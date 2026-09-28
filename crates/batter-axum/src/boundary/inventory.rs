//! Inert route inventories that validate assembly without polling application code.

use super::{
    ProbePath,
    pattern::{SharedPath, shared_path},
};
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use tower::ServiceExt;

#[derive(Clone, Copy)]
struct GuardedRouteMatch;

async fn report_guarded_route_match() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.extensions_mut().insert(GuardedRouteMatch);
    response
}

/// Build a library-owned router that only reports whether a pattern matched.
fn inventory<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Router {
    patterns
        .into_iter()
        .fold(Router::new(), |inventory, pattern| {
            inventory.route(pattern, any(report_guarded_route_match))
        })
}

async fn reports_match(inventory: &Router, request: Request) -> bool {
    let response = inventory
        .clone()
        .oneshot(request)
        .await
        .expect("Axum routers are infallible services");
    response.extensions().get::<GuardedRouteMatch>().is_some()
}

fn inspection(path: &str) -> Result<Request, axum::http::Error> {
    Request::builder()
        .method(Method::OPTIONS)
        .uri(path)
        .body(Body::empty())
}

/// Whether any guarded route pattern can match a reserved probe path.
pub(super) async fn claims_probe(patterns: &[String], probes: &[ProbePath]) -> bool {
    let inventory = inventory(patterns);
    for path in probes {
        let request =
            inspection(path.as_str()).expect("validated probe path forms an HTTP request");
        if reports_match(&inventory, request).await {
            return true;
        }
    }
    false
}

/// Return the first pair of groups, in declaration order, sharing a request path.
pub(super) async fn overlapping_groups(
    groups: &[(&'static str, &[String])],
) -> Option<(&'static str, &'static str)> {
    for (index, &(first_name, first)) in groups.iter().enumerate() {
        for &(second_name, second) in &groups[index + 1..] {
            if share_path(first, second).await {
                return Some((first_name, second_name));
            }
        }
    }
    None
}

async fn share_path(first: &[String], second: &[String]) -> bool {
    for first in first {
        for second in second {
            let shared = match shared_path(first, second) {
                SharedPath::Disjoint => false,
                // Axum accepted both patterns, so this is unreachable; fail closed.
                SharedPath::Unanalyzed => true,
                SharedPath::Witness(path) => witnessed(first, second, &path).await,
            };
            if shared {
                return true;
            }
        }
    }
    false
}

/// Confirm a constructed path with native routing of each pattern alone.
///
/// Fail closed when a request URI cannot preserve the witness path: native
/// route registration accepts such literals, and merging them can still panic.
async fn witnessed(first: &String, second: &String, path: &str) -> bool {
    let (Ok(first_request), Ok(second_request)) = (inspection(path), inspection(path)) else {
        return true;
    };
    if first_request.uri().path() != path {
        return true;
    }
    reports_match(&inventory([first]), first_request).await
        && reports_match(&inventory([second]), second_request).await
}
