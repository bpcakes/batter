use super::capture::Capture;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use batter_axum::{
    browser::{apply_private_response_headers, private_response},
    observe_http,
};
use tower::ServiceExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResponseMarker;

fn request(path: &str) -> Request {
    Request::builder()
        .uri(path)
        .header("origin", "secret-origin-value")
        .body(Body::empty())
        .unwrap()
}

fn assert_private(response: &Response) {
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "same-origin");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    for name in [
        header::CACHE_CONTROL.as_str(),
        "referrer-policy",
        "x-content-type-options",
    ] {
        assert_eq!(response.headers().get_all(name).iter().count(), 1, "{name}");
    }
}

#[test]
fn pure_private_headers_overwrite_only_the_selected_singletons() {
    let mut headers = HeaderMap::new();
    headers.append(header::CACHE_CONTROL, HeaderValue::from_static("public"));
    headers.append(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=60"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("origin"));
    headers.insert("x-content-type-options", HeaderValue::from_static("other"));
    headers.insert("x-application", HeaderValue::from_static("preserved"));
    apply_private_response_headers(&mut headers);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers.get_all(header::CACHE_CONTROL).iter().count(), 1);
    assert_eq!(headers["referrer-policy"], "same-origin");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["x-application"], "preserved");
    assert_eq!(headers.len(), 4);
}

async fn application_rejection(_request: Request, _next: Next) -> Response {
    (StatusCode::FORBIDDEN, "application rejection").into_response()
}

async fn extraction_rejection(_request: Request, _next: Next) -> Response {
    (StatusCode::UNSUPPORTED_MEDIA_TYPE, "extractor rejection").into_response()
}

fn private_router() -> Router {
    let ordinary = Router::new()
        .route(
            "/ok",
            get(|| async {
                let mut response =
                    (StatusCode::NO_CONTENT, [("x-application", "yes")]).into_response();
                response.extensions_mut().insert(ResponseMarker);
                response
            }),
        )
        .route(
            "/bad",
            get(|| async { (StatusCode::BAD_REQUEST, "bad request") }),
        );
    let application_rejection = Router::new()
        .route("/mutation", get(|| async { StatusCode::NO_CONTENT }))
        .route_layer(middleware::from_fn(application_rejection));
    let extraction = Router::new()
        .route("/extract", get(|| async { StatusCode::NO_CONTENT }))
        .route_layer(middleware::from_fn(extraction_rejection));
    ordinary
        .merge(application_rejection)
        .merge(extraction)
        .fallback(|| async { (StatusCode::NOT_FOUND, "private fallback") })
        .layer(middleware::from_fn(private_response))
}

#[tokio::test]
async fn middleware_covers_success_application_errors_inner_rejections_and_fallback() {
    let router = private_router();
    for (path, status, body) in [
        ("/ok", StatusCode::NO_CONTENT, ""),
        ("/bad", StatusCode::BAD_REQUEST, "bad request"),
        ("/mutation", StatusCode::FORBIDDEN, "application rejection"),
        (
            "/extract",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "extractor rejection",
        ),
        ("/missing", StatusCode::NOT_FOUND, "private fallback"),
    ] {
        let response = router.clone().oneshot(request(path)).await.unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_private(&response);
        if path == "/ok" {
            assert_eq!(response.headers()["x-application"], "yes");
            assert_eq!(
                response.extensions().get::<ResponseMarker>(),
                Some(&ResponseMarker)
            );
        }
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(bytes, body, "{path}");
    }
}

#[tokio::test]
async fn outer_short_circuit_cannot_receive_inner_private_headers() {
    async fn outer(_request: Request, _next: Next) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
    let router = private_router().layer(middleware::from_fn(outer));
    let response = router.oneshot(request("/ok")).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!response.headers().contains_key(header::CACHE_CONTROL));
    assert!(!response.headers().contains_key("referrer-policy"));
    assert!(!response.headers().contains_key("x-content-type-options"));
}

#[test]
fn private_headers_compose_inside_one_http_observation_without_value_leaks() {
    let capture = Capture::new();
    let router = private_router().layer(middleware::from_fn(observe_http));
    let response = capture
        .block_on(router.oneshot(request("/mutation")))
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_private(&response);
    let text = capture.text();
    assert_eq!(
        text.matches("HTTP response boundary finished").count(),
        1,
        "{text}"
    );
    assert!(text.contains("status=403"), "{text}");
    assert!(!text.contains("secret-origin-value"), "{text}");
}
