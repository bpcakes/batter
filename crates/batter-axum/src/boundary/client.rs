//! The opaque in-process request client that sealed assembly consumes into.

use crate::{HttpFailure, render_infrastructure_failure};
use axum::{
    body::Body,
    extract::{
        FromRequestParts, MatchedPath, NestedPath, OriginalUri, RawPathParams, Request,
        rejection::RawPathParamsRejection,
    },
    http::request::Parts,
    response::Response,
};
use tower::ServiceExt;

/// An assembled boundary consumed into an in-process request transport.
///
/// [`AssembledHttp::in_process`](super::AssembledHttp::in_process) is the only
/// way to obtain one. The client dispatches a request through the complete
/// boundary — correlation, the single observer, probe placement and every route
/// group's admission — and returns the response it built. Cloning shares the
/// one router prepared at construction, so cloning a client builds no layer.
///
/// This is a request transport, not a server. It establishes nothing about
/// serving: no accepted connection, no listener, no TLS handshake, no peer, no
/// body streaming and no detached descendant. Register the assembled boundary
/// for those. Requests may carry explicitly inserted synthetic extensions, such
/// as a chosen [`ConnectInfo`](axum::extract::ConnectInfo); choosing them, and
/// their meaning, belongs to the caller.
///
/// The client accepts only an arrival-shaped request: one whose extensions
/// carry no Axum routing record from an earlier dispatch. A request built from
/// a whole extension map that a router already dispatched is refused before
/// the boundary runs, with a response carrying [`ReusedRoutingState`]; see
/// [`InProcessClient::request`] for the records, the outcome and the
/// migration.
///
/// The inner router stays private. The client is not a Tower service, cannot
/// become a [`Router`](axum::Router), and cannot be served:
///
/// ```compile_fail,E0308
/// fn cannot_become_a_router(client: batter_axum::InProcessClient) -> axum::Router {
///     client
/// }
/// ```
///
/// ```compile_fail,E0599
/// fn cannot_take_the_router(client: batter_axum::InProcessClient) {
///     let _ = client.into_router();
/// }
/// ```
///
/// Those two reject an identity return and a named taker. Each conversion and
/// accessor trait that could reintroduce the router has its own control, and so
/// does reading the field directly:
///
/// ```compile_fail,E0277
/// fn converts_into_a_router<T: Into<axum::Router>>() {}
///
/// fn cannot_convert_into_a_router() {
///     converts_into_a_router::<batter_axum::InProcessClient>();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn borrows_a_router<T: AsRef<axum::Router>>() {}
///
/// fn cannot_borrow_a_router() {
///     borrows_a_router::<batter_axum::InProcessClient>();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn dereferences_to_a_router<T: std::ops::Deref<Target = axum::Router>>() {}
///
/// fn cannot_dereference_to_a_router() {
///     dereferences_to_a_router::<batter_axum::InProcessClient>();
/// }
/// ```
///
/// ```compile_fail,E0616
/// fn cannot_read_the_prepared_field(client: batter_axum::InProcessClient) {
///     let _ = client.prepared;
/// }
/// ```
///
/// ```compile_fail,E0277
/// async fn cannot_serve(listener: tokio::net::TcpListener, client: batter_axum::InProcessClient) {
///     axum::serve(listener, client).await.unwrap();
/// }
/// ```
///
/// It is also not a request service in its own right, which `axum::serve`
/// alone would not establish because that path needs the make-service
/// conversion:
///
/// ```compile_fail,E0277
/// fn requires_a_request_service<S: tower::Service<axum::extract::Request>>() {}
///
/// fn cannot_be_a_request_service() {
///     requires_a_request_service::<batter_axum::InProcessClient>();
/// }
/// ```
///
/// No route or layer can be added after assembly, so nothing can end up outside
/// the observer:
///
/// ```compile_fail,E0599
/// use axum::routing::get;
///
/// fn cannot_add_a_route(client: batter_axum::InProcessClient) {
///     let _ = client.route("/late", get(|| async { "outside" }));
/// }
/// ```
///
/// ```compile_fail,E0599
/// fn cannot_add_a_layer(client: batter_axum::InProcessClient) {
///     let _ = client.layer(axum::middleware::from_fn(
///         |request: axum::extract::Request, next: axum::middleware::Next| async move {
///             next.run(request).await
///         },
///     ));
/// }
/// ```
#[derive(Clone)]
#[must_use = "issue requests through the client"]
pub struct InProcessClient {
    prepared: axum::Router,
}

impl InProcessClient {
    /// Prepare the assembled router once, exactly as a served router is prepared.
    ///
    /// Pinned Axum 0.8.9 `Router::into_make_service` and
    /// `Router::into_make_service_with_connect_info` both call
    /// `Router::with_state(())` so that endpoints become routes eagerly instead
    /// of once per request. This uses the same preparation, so a layer wrapping
    /// a lazily built endpoint is constructed here and never again.
    pub(super) fn new(router: axum::Router) -> Self {
        Self {
            prepared: router.with_state(()),
        }
    }

    /// Dispatch one request through the boundary and return its response.
    ///
    /// The request is polled, and its future destroyed, inside the caller's
    /// task: the boundary's own dispatch and observation ownership covers it,
    /// and nothing is spawned. A dropped `request` future therefore drops the
    /// boundary's future the same way a dropped connection does.
    ///
    /// Every call reuses the router prepared at construction. Application
    /// extensions the caller inserts are kept, while a request carrying an
    /// Axum routing record is refused, as described below. The entry check
    /// looks only for Axum routing records, so it does not refuse a request
    /// for carrying Batter's own observation or operational record. The
    /// observer installs an observation record on a request that carries
    /// none, and the correlation layer installs an operational record on a
    /// request that carries none. Where the request already carries that
    /// kind of record, the layer leaves the inherited record in place and
    /// installs none of that kind; the adapter's contract records that as a
    /// residual the entry check does not cover.
    ///
    /// # Accepted requests
    ///
    /// The request must be arrival-shaped: its extensions carry no record that
    /// Axum routing writes. Four such records are detected, using only public
    /// Axum types: the path-parameter record that
    /// [`RawPathParams`] reads (any result except its missing-parameters
    /// rejection counts, including an empty capture list or an invalid UTF-8
    /// capture), [`MatchedPath`], [`NestedPath`] and [`OriginalUri`]. Every
    /// request an Axum [`Router`](axum::Router) dispatches gains an
    /// `OriginalUri`, and every request it routes to a route, a path fallback
    /// or a service mounted with `nest_service` also gains a path-parameter
    /// record, so cloning the whole extension map of a dispatched request, for
    /// example `request.extensions().clone()` inside a handler, always carries
    /// at least one of them. Application extensions, such as a chosen
    /// [`ConnectInfo`](axum::extract::ConnectInfo) or an application's own
    /// types, are accepted and reach handlers untouched.
    ///
    /// The check runs at entry, before the router, any layer or any
    /// application code is polled, and costs a few type-map lookups.
    ///
    /// # Refusal
    ///
    /// If any one of those records is present, the whole request is refused:
    /// nothing is stripped or repaired, and the request is destroyed unrouted.
    /// The response is the sanitized internal failure that
    /// [`render_infrastructure_failure`] renders for
    /// [`HttpFailure::Internal`] with no correlation identifier, and it carries
    /// the [`ReusedRoutingState`] response extension. The client removes that
    /// extension from every response it dispatched, so a returned response
    /// carries it only when this call refused the request, even if a handler
    /// forwarded a marker from another refusal. One ERROR event with a fixed
    /// message and no request-derived field is emitted on the caller's
    /// dispatch. Because the request never entered the boundary, there is no
    /// completion event, no request span and no `x-request-id` header.
    ///
    /// Without the refusal, pinned Axum 0.8.9 would treat the inherited
    /// records as an outer router's state: path parameters are appended to,
    /// so `Path` extraction fails or reads another route's captures, an
    /// inherited invalid UTF-8 capture rejects a valid path, the matched path
    /// is prefixed with the earlier route's pattern, and the inherited Batter
    /// state suppresses the destination's own observation.
    ///
    /// # Migration
    ///
    /// Build a fresh request and copy each application extension by type,
    /// instead of copying the whole map:
    ///
    /// ```
    /// use axum::{body::Body, extract::Request, http::Extensions};
    ///
    /// #[derive(Clone)]
    /// struct Label(&'static str);
    ///
    /// fn redispatch(earlier: &Extensions) -> Request<Body> {
    ///     let mut request = Request::builder().uri("/view/9").body(Body::empty()).unwrap();
    ///     if let Some(label) = earlier.get::<Label>() {
    ///         request.extensions_mut().insert(label.clone());
    ///     }
    ///     request
    /// }
    /// # let _ = redispatch(&Extensions::new());
    /// ```
    ///
    /// The detection is a run-time check because the records cannot be reset
    /// individually: Axum keeps its path-parameter record and its nested
    /// matched-path record private, and `http` 1.5.0 `Extensions` cannot be
    /// iterated or filtered, so no public facility removes routing state while
    /// keeping unknown application extensions. A private nested matched-path
    /// record is only ever written in the same routing step as the
    /// path-parameter record, so it never arrives without a detected one.
    /// Recheck these facts whenever the Axum or `http` pin changes.
    pub async fn request(&self, request: Request<Body>) -> Response {
        let (mut parts, body) = request.into_parts();
        if carries_routing_record(&mut parts).await {
            return refuse_reused_routing_state();
        }
        let request = Request::from_parts(parts, body);
        match self.prepared.clone().oneshot(request).await {
            Ok(mut response) => {
                // The marker is `Copy`, so application code can put one it
                // read from an earlier refusal on the response it returns.
                // Only the refusal above may leave this method carrying it.
                response.extensions_mut().remove::<ReusedRoutingState>();
                response
            }
            Err(never) => match never {},
        }
    }
}

/// Whether the request carries a record that Axum routing writes.
///
/// Only public Axum types and the public extractor are consulted. Any
/// path-parameter result except the missing-parameters rejection counts as
/// present, so a rejection added by a later Axum release fails closed.
async fn carries_routing_record(parts: &mut Parts) -> bool {
    let extensions = &parts.extensions;
    if extensions.get::<MatchedPath>().is_some()
        || extensions.get::<NestedPath>().is_some()
        || extensions.get::<OriginalUri>().is_some()
    {
        return true;
    }
    !matches!(
        RawPathParams::from_request_parts(parts, &()).await,
        Err(RawPathParamsRejection::MissingPathParams(_))
    )
}

fn refuse_reused_routing_state() -> Response {
    tracing::error!(
        target: "batter",
        "in-process request refused: its extensions carry routing state from an earlier dispatch"
    );
    let mut response = render_infrastructure_failure(HttpFailure::Internal, None);
    response
        .extensions_mut()
        .insert(ReusedRoutingState { _private: () });
    response
}

/// Response extension marking a request that [`InProcessClient::request`]
/// refused because its extensions carried Axum routing state from an earlier
/// dispatch.
///
/// The refused response is otherwise the sanitized internal failure of
/// [`render_infrastructure_failure`] for [`HttpFailure::Internal`], so this
/// marker is what tells a client refusal apart from a 500 the boundary built:
/// on a response returned by [`InProcessClient::request`] it means that call
/// refused the request, and no route, layer, renderer or handler of the
/// boundary ran. It has no public constructor and no public field, so an
/// application cannot make one. It is `Copy`, so an application can read one
/// from a refused response and put it on a response of its own, for example
/// by returning an inner client's refusal from a handler; the client removes
/// the marker from every response it dispatched, so such a copy never leaves
/// `request`. It is a response extension, not a header, and is never sent on a
/// served connection.
///
/// ```
/// use axum::{
///     body::Body,
///     extract::{OriginalUri, Request},
///     http::{StatusCode, Uri},
///     routing::get,
/// };
/// use batter_axum::{
///     GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget, ReusedRoutingState,
/// };
/// use batter_core::lifecycle::ShutdownHandle;
/// use std::time::Duration;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(1))?;
/// let client = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
///     .await?
///     .in_process();
///
/// // A routing record from an earlier dispatch: refused before any route runs.
/// let mut reused = Request::builder().uri("/work").body(Body::empty())?;
/// reused
///     .extensions_mut()
///     .insert(OriginalUri(Uri::from_static("/earlier")));
/// let refused = client.request(reused).await;
/// assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
/// assert!(refused.extensions().get::<ReusedRoutingState>().is_some());
///
/// // A response the boundary builds never carries it.
/// let fresh = Request::builder().uri("/work").body(Body::empty())?;
/// let served = client.request(fresh).await;
/// assert_eq!(served.status(), StatusCode::OK);
/// assert!(served.extensions().get::<ReusedRoutingState>().is_none());
/// # Ok(()) }
/// ```
///
/// An application cannot construct it:
///
/// ```compile_fail,E0451
/// fn forge() -> batter_axum::ReusedRoutingState {
///     batter_axum::ReusedRoutingState { _private: () }
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ReusedRoutingState {
    _private: (),
}
