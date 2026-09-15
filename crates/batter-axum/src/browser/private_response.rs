use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};

const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");

/// Overwrite the fixed private-response headers selected by this adapter.
///
/// This applies `Cache-Control: no-store`, `Referrer-Policy: same-origin`, and
/// `X-Content-Type-Options: nosniff`. The referrer policy withholds referrers
/// from cross-origin destinations while preserving the request origin needed
/// by same-origin HTML form mutations. A page posting a non-CORS form to a
/// different origin receives `Origin: null` and cannot satisfy that target's
/// exact-origin policy. This function does not claim protection from malicious
/// caches or configure CSP, HSTS, CORS, or browser history.
///
/// ```
/// use axum::http::{HeaderMap, header};
/// use batter_axum::browser::apply_private_response_headers;
///
/// let mut headers = HeaderMap::new();
/// apply_private_response_headers(&mut headers);
/// assert_eq!(headers[header::CACHE_CONTROL], "no-store");
/// assert_eq!(headers["referrer-policy"], "same-origin");
/// assert_eq!(headers["x-content-type-options"], "nosniff");
/// ```
pub fn apply_private_response_headers(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
}

/// Axum middleware applying private-response headers to every inner response.
///
/// Apply this after assembling the private routes and fallbacks. Put
/// application mutation-rejection middleware inside it so rejections receive
/// the headers; put [`crate::observe_http`] outside it if those statuses should
/// be observed. An outer middleware that short-circuits without calling `next`
/// cannot be changed by this middleware.
///
/// ```
/// use axum::{Router, middleware, routing::get};
/// use batter_axum::{browser::private_response, observe_http};
///
/// let private = Router::new()
///     .route("/session", get(|| async { "private" }))
///     .fallback(|| async { "private fallback" })
///     .layer(middleware::from_fn(private_response));
/// let app: Router = private.layer(middleware::from_fn(observe_http));
/// # let _ = app;
/// ```
pub async fn private_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    apply_private_response_headers(response.headers_mut());
    response
}
