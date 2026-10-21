//! Each route group's browser policy decides whether its requests may present
//! ambient browser credentials through `BrowserCredentials`.

use super::support::{
    ORIGIN, SECOND, assert_private, body_text, browser_group, exact_origin, ready_handle, request,
    request_policy,
};
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{any, get},
};
use batter_axum::{
    BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary, InProcessClient, MutationCheckWaiver,
    RouteGroup, RouteInventory,
    browser::{
        BrowserCredentials, BrowserCredentialsRejection, CookieHeaderPolicy, CookieName,
        PrivateResponsePolicy,
    },
    low_level::{operational_http, request_admission},
};
use batter_core::lifecycle::ShutdownHandle;
use tower::ServiceExt;

const PROBLEM: &str = r#"{"type":"about:blank","title":"Internal Server Error","status":500,"code":"internal_error"}"#;

/// Report what the request's group let the handler read.
async fn session(credentials: Result<BrowserCredentials, BrowserCredentialsRejection>) -> String {
    let name = CookieName::new("session").unwrap();
    match credentials {
        Ok(credentials) => match credentials.cookie(&name, CookieHeaderPolicy::TargetOnly) {
            Ok(Some(value)) => format!("session:{}", value.as_str()),
            Ok(None) => "session:absent".to_owned(),
            Err(error) => format!("cookie:{error:?}"),
        },
        Err(rejection) => format!("refused:{rejection:?}"),
    }
}

fn with_cookie(method: Method, path: &str, cookie: &'static str) -> Request<Body> {
    let mut request = request(method, path);
    request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    request
}

fn from_site(method: Method, path: &str, origin: &str, cookie: &'static str) -> Request<Body> {
    let mut request = with_cookie(method, path, cookie);
    request
        .headers_mut()
        .insert(header::ORIGIN, origin.parse().unwrap());
    request
}

fn waived(handle: &ShutdownHandle, waiver: MutationCheckWaiver) -> GroupPolicy {
    GroupPolicy::browser(
        request_policy(handle, SECOND),
        BrowserPolicy::without_mutation_checks(PrivateResponsePolicy::NoReferrer, waiver),
    )
}

async fn app(policy: GroupPolicy, routes: GuardedRouter) -> InProcessClient {
    let handle = ready_handle();
    HttpBoundary::new(request_policy(&handle, SECOND))
        .with_group(RouteGroup::new("browser", policy, routes))
        .unwrap()
        .assemble(GuardedRouter::new().route("/default", any(session)))
        .await
        .unwrap()
        .in_process()
}

async fn read(app: &InProcessClient, request: Request<Body>) -> (StatusCode, String) {
    let response = app.request(request).await;
    (response.status(), body_text(response).await)
}

#[tokio::test]
async fn a_checked_group_presents_cookies_only_after_its_mutation_checks() {
    let handle = ready_handle();
    let policy = browser_group(
        &handle,
        SECOND,
        PrivateResponsePolicy::SameOriginReferrer,
        exact_origin(),
    );
    let app = app(policy, GuardedRouter::new().route("/account", any(session))).await;
    let ok = |body: &str| (StatusCode::OK, body.to_owned());
    // Safe methods are not checked and present cookies.
    let viewed = read(&app, with_cookie(Method::GET, "/account", "session=a")).await;
    assert_eq!(viewed, ok("session:a"));
    for method in [Method::POST, Method::DELETE] {
        let saved = from_site(method, "/account", ORIGIN, "other=x; session=a");
        assert_eq!(read(&app, saved).await, ok("session:a"));
    }
    // A failed check renders the rejection; the handler never reads the cookie.
    let forged = from_site(
        Method::POST,
        "/account",
        "https://attacker.example",
        "session=a",
    );
    let (status, body) = read(&app, forged).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.starts_with("browser:"), "{body}");
    // The presented fields keep the parser's ambiguity rules.
    let duplicate = from_site(Method::POST, "/account", ORIGIN, "session=a; session=b");
    assert_eq!(read(&app, duplicate).await, ok("cookie:DuplicateTarget"));
    let absent = from_site(Method::POST, "/account", ORIGIN, "other=x");
    assert_eq!(read(&app, absent).await, ok("session:absent"));

    // The extractor must preserve every Cookie field before parsing.
    for (first, later, expected) in [
        (
            "other=x",
            HeaderValue::from_static("session=late"),
            "session:late",
        ),
        (
            "session=a",
            HeaderValue::from_static("session=b"),
            "cookie:DuplicateTarget",
        ),
        (
            "session=a",
            HeaderValue::from_bytes(b"\xff").unwrap(),
            "cookie:NonUtf8",
        ),
    ] {
        let mut request = from_site(Method::POST, "/account", ORIGIN, first);
        request.headers_mut().append(header::COOKIE, later);
        assert_eq!(read(&app, request).await, ok(expected), "{expected}");
    }
}

#[tokio::test]
async fn a_group_without_ambient_credentials_refuses_them_on_every_mutation() {
    let handle = ready_handle();
    let routes = GuardedRouter::new()
        .route("/reports", any(session))
        .route(
            "/reports/strict",
            any(|_: BrowserCredentials| async { "read" }),
        )
        .route("/reports/token", any(|| async { "created" }));
    let policy = waived(&handle, MutationCheckWaiver::NoAmbientCredentials);
    let app = app(policy, routes).await;
    for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
        // The plain extractor's status shows the grant even without a body.
        let strict = app
            .request(with_cookie(method.clone(), "/reports/strict", "session=a"))
            .await;
        assert_eq!(strict.status(), StatusCode::OK, "{method}");
        if method != Method::HEAD {
            let read = read(&app, with_cookie(method.clone(), "/reports", "session=a")).await;
            assert_eq!(read, (StatusCode::OK, "session:a".to_owned()), "{method}");
        }
    }
    let extension = Method::from_bytes(b"PURGE").unwrap();
    for method in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        extension,
    ] {
        // Refused from the exact origin, and whether or not a cookie is sent.
        for request in [
            from_site(method.clone(), "/reports", ORIGIN, "session=a"),
            request(method.clone(), "/reports"),
        ] {
            let refused = read(&app, request).await;
            let expected = (StatusCode::OK, "refused:UncheckedMutation".to_owned());
            assert_eq!(refused, expected, "{method}");
        }
        // Routes that read no ambient credential keep serving mutations.
        let token = read(&app, request(method.clone(), "/reports/token")).await;
        assert_eq!(token, (StatusCode::OK, "created".to_owned()), "{method}");
    }
    // The plain extractor answers the sanitized composition-error response
    // inside the group's private-response headers.
    let response = app
        .request(with_cookie(Method::POST, "/reports/strict", "session=a"))
        .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_private(response.headers(), "no-referrer");
    assert_eq!(body_text(response).await, PROBLEM);
}

#[tokio::test]
async fn an_application_checked_group_presents_cookies_on_unchecked_mutations() {
    let handle = ready_handle();
    let policy = waived(&handle, MutationCheckWaiver::ApplicationChecksMutations);
    let app = app(
        policy,
        GuardedRouter::new().route("/callback", any(session)),
    )
    .await;
    // Batter checks nothing: the waiver hands verification to the application.
    let cross_site = from_site(
        Method::POST,
        "/callback",
        "https://identity.example",
        "session=a",
    );
    let read = read(&app, cross_site).await;
    assert_eq!(read, (StatusCode::OK, "session:a".to_owned()));
}

#[tokio::test]
async fn routes_without_a_browser_policy_present_no_cookies() {
    let handle = ready_handle();
    let plain = GroupPolicy::new(request_policy(&handle, SECOND));
    let app = app(plain, GuardedRouter::new().route("/plain", any(session))).await;
    // A named group and the default group, neither with a browser policy.
    for path in ["/plain", "/default"] {
        for method in [Method::GET, Method::POST] {
            let refused = read(&app, with_cookie(method.clone(), path, "session=a")).await;
            let expected = (StatusCode::OK, "refused:NoBrowserPolicy".to_owned());
            assert_eq!(refused, expected, "{method} {path}");
        }
    }
    // Lower-level compositions install no group, and a bare router no layer.
    let lower_level = Router::new()
        .route("/work", get(session))
        .layer(middleware::from_fn_with_state(
            request_policy(&handle, SECOND),
            request_admission,
        ))
        .layer(middleware::from_fn(operational_http));
    let bare = Router::new().route("/work", get(session));
    for router in [lower_level, bare] {
        let response = router
            .oneshot(with_cookie(Method::GET, "/work", "session=a"))
            .await
            .unwrap();
        assert_eq!(body_text(response).await, "refused:NoBrowserPolicy");
    }
}

/// An application layer that swaps GET and POST before the handler, as a
/// method-override layer would.
async fn swap_method(mut request: Request, next: Next) -> Response {
    let swapped = if request.method() == Method::POST {
        Method::GET
    } else {
        Method::POST
    };
    *request.method_mut() = swapped;
    next.run(request).await
}

#[tokio::test]
async fn an_application_layer_cannot_change_the_groups_decision() {
    let handle = ready_handle();
    let routes = GuardedRouter::new()
        .route("/reports", any(session))
        .layer(middleware::from_fn(swap_method));
    let policy = waived(&handle, MutationCheckWaiver::NoAmbientCredentials);
    let app = app(policy, routes).await;
    // The gate decided from the method the request arrived with.
    let posted = read(&app, with_cookie(Method::POST, "/reports", "session=a")).await;
    assert_eq!(posted.1, "refused:UncheckedMutation");
    let viewed = read(&app, with_cookie(Method::GET, "/reports", "session=a")).await;
    assert_eq!(viewed.1, "session:a");
}

#[tokio::test]
async fn admitted_routers_receive_their_groups_decision() {
    let handle = ready_handle();
    let router = Router::new().route("/feeds/{id}", any(session));
    let feeds = GuardedRouter::from_router(router, RouteInventory::new(["/feeds/{id}"]).unwrap());
    let policy = waived(&handle, MutationCheckWaiver::NoAmbientCredentials);
    let app = app(policy, feeds).await;
    let viewed = read(&app, with_cookie(Method::GET, "/feeds/1", "session=a")).await;
    assert_eq!(viewed.1, "session:a");
    let posted = read(&app, with_cookie(Method::POST, "/feeds/1", "session=a")).await;
    assert_eq!(posted.1, "refused:UncheckedMutation");
}
