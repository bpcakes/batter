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
    browser::{PrivateResponsePolicy, apply_private_response_headers, private_response},
    observe_http,
};
use tower::{Layer, ServiceExt, service_fn};

const REFERRER_POLICY: &str = "referrer-policy";

const POLICIES: [(PrivateResponsePolicy, &str); 2] = [
    (PrivateResponsePolicy::SameOriginReferrer, "same-origin"),
    (PrivateResponsePolicy::NoReferrer, "no-referrer"),
];

/// Each public way of applying private headers to a router.
#[derive(Clone, Copy, Debug)]
enum Layering {
    Compatibility,
    Policy(PrivateResponsePolicy),
}

impl Layering {
    fn wrap(self, router: Router) -> Router {
        match self {
            Self::Compatibility => router.layer(middleware::from_fn(private_response)),
            Self::Policy(policy) => router.layer(policy),
        }
    }
}

const LAYERINGS: [(Layering, &str); 3] = [
    (Layering::Compatibility, "same-origin"),
    (
        Layering::Policy(PrivateResponsePolicy::SameOriginReferrer),
        "same-origin",
    ),
    (
        Layering::Policy(PrivateResponsePolicy::NoReferrer),
        "no-referrer",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResponseMarker;

fn request(path: &str) -> Request {
    Request::builder()
        .uri(path)
        .header("origin", "secret-origin-value")
        .body(Body::empty())
        .unwrap()
}

fn assert_private(headers: &HeaderMap, referrer: &str) {
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{referrer}");
    assert_eq!(headers[REFERRER_POLICY], referrer);
    assert_eq!(headers["x-content-type-options"], "nosniff", "{referrer}");
    for name in [
        header::CACHE_CONTROL.as_str(),
        REFERRER_POLICY,
        "x-content-type-options",
    ] {
        let count = headers.get_all(name).iter().count();
        assert_eq!(count, 1, "{name} under {referrer}");
    }
}

fn add_weaker_application_headers(headers: &mut HeaderMap) {
    headers.append(header::CACHE_CONTROL, HeaderValue::from_static("public"));
    headers.append(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=60"),
    );
    headers.append(REFERRER_POLICY, HeaderValue::from_static("unsafe-url"));
    headers.append(REFERRER_POLICY, HeaderValue::from_static("origin"));
    headers.insert("x-content-type-options", HeaderValue::from_static("other"));
    headers.insert("x-application", HeaderValue::from_static("preserved"));
}

#[test]
fn direct_policies_overwrite_only_the_selected_singletons() {
    for (policy, referrer) in POLICIES {
        let mut headers = HeaderMap::new();
        add_weaker_application_headers(&mut headers);
        policy.apply(&mut headers);
        assert_private(&headers, referrer);
        assert_eq!(headers["x-application"], "preserved");
        assert_eq!(headers.len(), 4, "{referrer}");
    }
    let mut compatibility = HeaderMap::new();
    add_weaker_application_headers(&mut compatibility);
    apply_private_response_headers(&mut compatibility);
    assert_private(&compatibility, "same-origin");
    assert_eq!(compatibility.len(), 4);
    assert_eq!(
        PrivateResponsePolicy::default(),
        PrivateResponsePolicy::SameOriginReferrer
    );
}

fn referrer_after(policy: PrivateResponsePolicy, existing: &[&'static str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in existing {
        headers.append(REFERRER_POLICY, HeaderValue::from_static(value));
    }
    policy.apply(&mut headers);
    headers
}

#[test]
fn referrer_selection_never_weakens_an_exact_no_referrer() {
    for existing in [&["no-referrer"][..], &["no-referrer", "no-referrer"]] {
        for (policy, _) in POLICIES {
            assert_private(&referrer_after(policy, existing), "no-referrer");
        }
    }
    for existing in [
        &["same-origin"][..],
        &["No-Referrer"],
        &[" no-referrer"],
        &["no-referrer, unsafe-url"],
        &["no-referrer", "unsafe-url"],
        &["unsafe-url", "no-referrer"],
        &[""],
    ] {
        let headers = referrer_after(PrivateResponsePolicy::SameOriginReferrer, existing);
        assert_private(&headers, "same-origin");
        let headers = referrer_after(PrivateResponsePolicy::NoReferrer, existing);
        assert_private(&headers, "no-referrer");
    }
}

async fn weaker_application_response() -> Response {
    let mut response = (StatusCode::CREATED, "body").into_response();
    add_weaker_application_headers(response.headers_mut());
    response.extensions_mut().insert(ResponseMarker);
    response
}

#[tokio::test]
async fn layers_overwrite_handler_headers_and_preserve_response_data() {
    for (layering, referrer) in LAYERINGS {
        let route = Router::new().route("/weaker", get(weaker_application_response));
        let response = layering
            .wrap(route)
            .oneshot(request("/weaker"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "{layering:?}");
        assert_private(response.headers(), referrer);
        assert_eq!(response.headers()["x-application"], "preserved");
        assert_eq!(
            response.extensions().get::<ResponseMarker>(),
            Some(&ResponseMarker)
        );
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(bytes, "body", "{layering:?}");
    }
}

async fn application_rejection(_request: Request, _next: Next) -> Response {
    (StatusCode::FORBIDDEN, "application rejection").into_response()
}

async fn extraction_rejection(_request: Request, _next: Next) -> Response {
    (StatusCode::UNSUPPORTED_MEDIA_TYPE, "extractor rejection").into_response()
}

fn private_router(layering: Layering) -> Router {
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
    layering.wrap(
        ordinary
            .merge(application_rejection)
            .merge(extraction)
            .fallback(|| async { (StatusCode::NOT_FOUND, "private fallback") }),
    )
}

#[tokio::test]
async fn middleware_covers_success_application_errors_inner_rejections_and_fallback() {
    for (layering, referrer) in LAYERINGS {
        let router = private_router(layering);
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
            assert_eq!(response.status(), status, "{path} under {layering:?}");
            assert_private(response.headers(), referrer);
            if path == "/ok" {
                assert_eq!(response.headers()["x-application"], "yes");
                assert_eq!(
                    response.extensions().get::<ResponseMarker>(),
                    Some(&ResponseMarker)
                );
            }
            let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
            assert_eq!(bytes, body, "{path} under {layering:?}");
        }
    }
}

async fn rewrite_referrer(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let weaker = HeaderValue::from_static("unsafe-url");
    response.headers_mut().insert(REFERRER_POLICY, weaker);
    response
}

#[tokio::test]
async fn nested_layers_keep_the_strictest_referrer_in_either_order() {
    let same_origin = Layering::Policy(PrivateResponsePolicy::SameOriginReferrer);
    let no_referrer = Layering::Policy(PrivateResponsePolicy::NoReferrer);
    let compatibility = Layering::Compatibility;
    for (inner, outer, referrer) in [
        (no_referrer, same_origin, "no-referrer"),
        (same_origin, no_referrer, "no-referrer"),
        (no_referrer, compatibility, "no-referrer"),
        (compatibility, no_referrer, "no-referrer"),
        (no_referrer, no_referrer, "no-referrer"),
        (same_origin, compatibility, "same-origin"),
    ] {
        let route = Router::new().route("/ok", get(|| async { StatusCode::NO_CONTENT }));
        let router = outer.wrap(inner.wrap(route));
        let response = router.oneshot(request("/ok")).await.unwrap();
        assert_private(response.headers(), referrer);
    }
    // A rewrite between the layers leaves only the outer selection.
    let route = Router::new().route("/ok", get(|| async { StatusCode::NO_CONTENT }));
    let rewritten = no_referrer
        .wrap(route)
        .layer(middleware::from_fn(rewrite_referrer));
    let response = same_origin
        .wrap(rewritten)
        .oneshot(request("/ok"))
        .await
        .unwrap();
    assert_private(response.headers(), "same-origin");
}

#[tokio::test]
async fn outer_short_circuit_cannot_receive_inner_private_headers() {
    async fn outer(_request: Request, _next: Next) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
    for (layering, _) in LAYERINGS {
        let router = private_router(layering).layer(middleware::from_fn(outer));
        let response = router.oneshot(request("/ok")).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!response.headers().contains_key(header::CACHE_CONTROL));
        assert!(!response.headers().contains_key(REFERRER_POLICY));
        assert!(!response.headers().contains_key("x-content-type-options"));
    }
}

#[tokio::test]
async fn inner_service_errors_pass_through_without_headers() {
    for (policy, _) in POLICIES {
        let failing = service_fn(|_request: Request| async {
            Err::<Response, &'static str>("inner failure")
        });
        let outcome = policy.layer(failing).oneshot(request("/")).await;
        assert_eq!(outcome.unwrap_err(), "inner failure", "{policy:?}");
    }
}

#[test]
fn private_headers_compose_inside_one_http_observation_without_value_leaks() {
    for (layering, referrer) in LAYERINGS {
        let capture = Capture::new();
        let router = private_router(layering).layer(middleware::from_fn(observe_http));
        let response = capture
            .block_on(router.oneshot(request("/mutation")))
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_private(response.headers(), referrer);
        let text = capture.text();
        assert_eq!(
            text.matches("HTTP response boundary finished").count(),
            1,
            "{text}"
        );
        assert!(text.contains("status=403"), "{text}");
        assert!(!text.contains("secret-origin-value"), "{text}");
    }
}
