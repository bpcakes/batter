use axum::{Router, body::Body, http::Request, middleware};
use batter_axum::{RequestPolicy, observe_http, request_admission, request_scope};

#[derive(Clone, Copy, Debug)]
pub enum Boundary {
    Observation,
    Admission,
    Split,
    Combined,
}

impl Boundary {
    pub fn apply(self, router: Router, policy: RequestPolicy) -> Router {
        match self {
            Self::Observation => router.layer(middleware::from_fn(observe_http)),
            Self::Admission => {
                router.route_layer(middleware::from_fn_with_state(policy, request_admission))
            }
            Self::Split => router
                .route_layer(middleware::from_fn_with_state(policy, request_admission))
                .layer(middleware::from_fn(observe_http)),
            Self::Combined => {
                router.route_layer(middleware::from_fn_with_state(policy, request_scope))
            }
        }
    }
}

pub fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", "secret-authorization")
        .header("x-request-id", "secret-client-id")
        .body(Body::from("secret-request-body"))
        .unwrap()
}

pub fn completions(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.contains("HTTP response boundary finished"))
        .collect()
}

pub fn assert_completion(text: &str, status: u16, outcome: &str, route: &str, method: &str) {
    let events = completions(text);
    assert_eq!(events.len(), 1, "{text}");
    let (_, event) = events[0]
        .split_once("HTTP response boundary finished")
        .unwrap();
    for field in [
        format!("status={status}"),
        format!("http_outcome=\"{outcome}\""),
        format!("route=\"{route}\""),
        format!("method=\"{method}\""),
        "latency_ms=".into(),
    ] {
        assert!(event.contains(&field), "missing {field}: {}", events[0]);
    }
    assert_no_secrets(text);
}

pub fn assert_no_secrets(text: &str) {
    assert!(!text.contains("secret-"), "{text}");
    assert!(!text.contains("SECRET-METHOD"), "{text}");
}
