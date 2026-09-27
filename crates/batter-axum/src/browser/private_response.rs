use axum::{
    extract::Request,
    http::{self, HeaderMap, HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tower::{Layer, Service};

const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const NO_REFERRER: &str = "no-referrer";
const SAME_ORIGIN: &str = "same-origin";

/// Private-response headers with one typed referrer choice.
///
/// Every value overwrites `Cache-Control: no-store` and
/// `X-Content-Type-Options: nosniff`. Only `Referrer-Policy` varies, and only
/// between the two policies that never send referrer information to another
/// origin; weaker, raw, empty and comma-listed policies are unrepresentable.
/// The policy is itself a Tower layer: one `layer` call applies all three
/// headers to every inner response. Choose it deliberately per route group:
///
/// ```
/// use axum::{Router, middleware, routing::get};
/// use batter_axum::{browser::PrivateResponsePolicy, observe_http};
///
/// // These pages submit same-origin HTML forms to an exact-origin mutation check.
/// let forms = Router::new()
///     .route("/account", get(|| async { "form page" }).post(|| async { "saved" }))
///     .layer(PrivateResponsePolicy::SameOriginReferrer);
/// // These URLs must not reach even same-origin logs; pages mutate through `fetch`.
/// let reports = Router::new()
///     .route("/reports", get(|| async { "report" }))
///     .fallback(|| async { "private fallback" })
///     .layer(PrivateResponsePolicy::NoReferrer);
/// let app: Router = forms.merge(reports).layer(middleware::from_fn(observe_http));
/// # let _ = app;
/// ```
///
/// Apply the layer after assembling the private routes and fallback. Put
/// application mutation-rejection middleware inside it so rejections receive
/// the headers; put [`crate::observe_http`] outside it if those statuses should
/// be observed. An outer middleware that short-circuits without calling the
/// inner service cannot be changed by this layer. Inner service errors pass
/// through without headers; Axum routers are infallible.
///
/// # Referrer choice
///
/// `Referrer-Policy` is a response field that browsers apply to later request
/// fields; it adds no request check. It becomes the initial policy of a
/// document or worker created from this response and replaces the policy for
/// the next request of a redirect. Stylesheets may also apply it to their own
/// subresource requests. Other responses, such as JSON, are unaffected, and the
/// request that fetched this response already followed its initiator's policy.
///
/// The document's policy selects `Referer` for the navigations and subresource
/// requests it starts. `Origin` is a separate request field, and CORS-mode
/// requests, the default for script `fetch`, never consult the referrer policy
/// for it. Only requests outside CORS mode with a method other than `GET` or
/// `HEAD` do, notably HTML form submissions: they carry `Origin: null` whenever
/// the policy withholds the origin. `Sec-Fetch-Site` never depends on this
/// policy.
///
/// - [`Self::SameOriginReferrer`] sends the page URL, without fragment, as
///   `Referer` only to its own origin. Same-origin form posts keep the
///   serialized origin that [`MutationPolicy::exact_origin`] requires; a form
///   posting to another origin carries `Origin: null` and cannot satisfy that
///   target's exact-origin policy.
/// - [`Self::NoReferrer`] omits `Referer` everywhere. Same-origin navigations
///   lose it too, and destination documents see an empty `document.referrer`.
///   Every form post, including a same-origin one, carries `Origin: null`,
///   which [`MutationPolicy::exact_origin`] rejects. Such pages need CORS-mode
///   `fetch` mutations, which keep the serialized origin, or a custom-marker
///   [`MutationPolicy`].
///
/// The field sets only the document's initial policy. A `<meta name="referrer">`
/// element can change the document's policy, and `referrerpolicy` attributes,
/// `rel="noreferrer"` and `fetch` options can select another policy for
/// individual requests; Batter cannot constrain page content. This policy also
/// does not protect against malicious caches or configure CSP, HSTS, CORS, or
/// browser history.
///
/// # Overwrite and nesting
///
/// Existing `Cache-Control`, `X-Content-Type-Options` and `Referrer-Policy`
/// fields are replaced by one field each, with one exception: if
/// `Referrer-Policy` is present and every field is exactly `no-referrer`, the
/// strictest value, [`Self::SameOriginReferrer`] keeps one `no-referrer` field
/// instead of weakening it. An outer private-response layer therefore cannot
/// weaken an inner [`Self::NoReferrer`] selection unless middleware between them
/// rewrites the field.
///
/// Weaker policies have no representation:
///
/// ```compile_fail,E0599
/// let weaker = batter_axum::browser::PrivateResponsePolicy::StrictOriginWhenCrossOrigin;
/// ```
///
/// [`MutationPolicy`]: super::MutationPolicy
/// [`MutationPolicy::exact_origin`]: super::MutationPolicy::exact_origin
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrivateResponsePolicy {
    /// `Referrer-Policy: same-origin`, the default.
    ///
    /// It withholds referrers from every other origin while keeping exact-origin
    /// HTML form mutations working. [`private_response`] and
    /// [`apply_private_response_headers`] apply this value.
    #[default]
    SameOriginReferrer,
    /// `Referrer-Policy: no-referrer`, which also withholds same-origin referrers
    /// and makes every HTML form post carry `Origin: null`.
    NoReferrer,
}

impl PrivateResponsePolicy {
    /// Overwrite the private-response headers in one header map.
    ///
    /// Use this for a response assembled outside a layered router; the layer
    /// applies the same replacement.
    ///
    /// ```
    /// use axum::http::{HeaderMap, HeaderValue, header};
    /// use batter_axum::browser::PrivateResponsePolicy;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public"));
    /// headers.insert("referrer-policy", HeaderValue::from_static("unsafe-url"));
    /// PrivateResponsePolicy::NoReferrer.apply(&mut headers);
    /// assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    /// assert_eq!(headers["referrer-policy"], "no-referrer");
    /// assert_eq!(headers["x-content-type-options"], "nosniff");
    ///
    /// // A later default application keeps the stricter existing value.
    /// PrivateResponsePolicy::SameOriginReferrer.apply(&mut headers);
    /// assert_eq!(headers["referrer-policy"], "no-referrer");
    /// ```
    pub fn apply(self, headers: &mut HeaderMap) {
        let referrer = match self {
            Self::NoReferrer => NO_REFERRER,
            Self::SameOriginReferrer if only_no_referrer(headers) => NO_REFERRER,
            Self::SameOriginReferrer => SAME_ORIGIN,
        };
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(REFERRER_POLICY, HeaderValue::from_static(referrer));
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    }
}

fn only_no_referrer(headers: &HeaderMap) -> bool {
    let mut fields = headers.get_all(REFERRER_POLICY).iter();
    fields.next().is_some_and(is_no_referrer) && fields.all(is_no_referrer)
}

fn is_no_referrer(field: &HeaderValue) -> bool {
    field.as_bytes() == NO_REFERRER.as_bytes()
}

impl<S> Layer<S> for PrivateResponsePolicy {
    type Service = PrivateResponseService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PrivateResponseService {
            inner,
            policy: *self,
        }
    }
}

/// Service produced by layering a [`PrivateResponsePolicy`].
///
/// It applies [`PrivateResponsePolicy::apply`] to every inner response and
/// returns inner service errors unchanged.
#[derive(Clone, Debug)]
pub struct PrivateResponseService<S> {
    inner: S,
    policy: PrivateResponsePolicy,
}

impl<S, R, B> Service<R> for PrivateResponseService<S>
where
    S: Service<R, Response = http::Response<B>>,
{
    type Response = http::Response<B>;
    type Error = S::Error;
    type Future = PrivateResponseFuture<S::Future>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: R) -> Self::Future {
        PrivateResponseFuture {
            inner: self.inner.call(request),
            policy: self.policy,
        }
    }
}

pin_project_lite::pin_project! {
    /// Response future of [`PrivateResponseService`].
    pub struct PrivateResponseFuture<F> {
        #[pin]
        inner: F,
        policy: PrivateResponsePolicy,
    }
}

impl<F, B, E> Future for PrivateResponseFuture<F>
where
    F: Future<Output = Result<http::Response<B>, E>>,
{
    type Output = Result<http::Response<B>, E>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut response = ready!(this.inner.poll(context))?;
        this.policy.apply(response.headers_mut());
        Poll::Ready(Ok(response))
    }
}

/// Overwrite the private-response headers with the default referrer choice.
///
/// This compatibility function is exactly
/// [`PrivateResponsePolicy::SameOriginReferrer`] applied with
/// [`PrivateResponsePolicy::apply`]; see that type for the referrer
/// consequences. Call the policy directly to state the choice.
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
    PrivateResponsePolicy::SameOriginReferrer.apply(headers);
}

/// Axum middleware applying the default private-response headers.
///
/// This compatibility middleware behaves like
/// `.layer(PrivateResponsePolicy::SameOriginReferrer)`. It ignores router
/// state, so it cannot select [`PrivateResponsePolicy::NoReferrer`], even when
/// a policy is passed through `middleware::from_fn_with_state`. New
/// compositions should layer a [`PrivateResponsePolicy`] to state the referrer
/// choice; the same placement rules apply.
///
/// ```
/// use axum::{Router, middleware, routing::get};
/// use batter_axum::{browser::private_response, observe_http};
///
/// // Existing composition that deliberately keeps the same-origin default.
/// let private = Router::new()
///     .route("/session", get(|| async { "private" }))
///     .fallback(|| async { "private fallback" })
///     .layer(middleware::from_fn(private_response));
/// let app: Router = private.layer(middleware::from_fn(observe_http));
/// # let _ = app;
/// ```
pub async fn private_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    PrivateResponsePolicy::SameOriginReferrer.apply(response.headers_mut());
    response
}
