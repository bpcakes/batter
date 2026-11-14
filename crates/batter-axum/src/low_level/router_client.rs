//! The in-process transport over a caller-composed router, and the
//! arrival-shaped entry check it shares with the protected client.

use crate::{HttpFailure, ReusedRoutingState, render_infrastructure_failure};
use axum::{
    Router,
    body::Body,
    extract::{
        FromRequestParts, MatchedPath, NestedPath, OriginalUri, RawPathParams, Request,
        rejection::RawPathParamsRejection,
    },
    http::request::Parts,
    response::Response,
};
use tower::ServiceExt;

/// An in-process request transport over a caller-composed [`Router`].
///
/// This is the low-level counterpart of the protected
/// [`InProcessClient`](crate::InProcessClient), for a composition that
/// [`HttpBoundary`](crate::HttpBoundary) cannot express, such as
/// `batter-runlimit`'s authenticated quota assembly. It is not equivalent to
/// that client. It establishes nothing about the router it dispatches:
/// correlation, the single observer, admission, probe placement, route-group
/// policy and authentication are present only if the caller composed them into
/// the router, in the order the helpers of this module document. Prefer
/// [`AssembledHttp::in_process`](crate::AssembledHttp::in_process), whose
/// client proves that composition. A `RouterClient` cannot become an
/// `InProcessClient`:
///
/// ```compile_fail,E0277
/// fn cannot_claim_the_protected_client(
///     client: batter_axum::low_level::RouterClient,
/// ) -> batter_axum::InProcessClient {
///     client.into()
/// }
/// ```
///
/// What it owns is the transport's entry contract, which
/// [`InProcessClient::request`](crate::InProcessClient::request) delegates to:
///
/// - The router is prepared once, at construction, with the
///   `Router::with_state(())` preparation that pinned Axum 0.8.9 performs
///   inside its make-service conversions, so a layer wrapping a lazily built
///   endpoint is constructed here and never again per request.
/// - [`request`](Self::request) accepts only an arrival-shaped request. It
///   refuses one whose extensions carry an Axum routing record, or one of
///   Batter's own private request records, from an earlier dispatch before
///   the router is polled, with the sanitized internal failure carrying
///   [`ReusedRoutingState`].
/// - That marker is removed from every dispatched response, so a response
///   carries it only when the same call refused the request.
/// - Nothing is spawned: the request is polled, and its future destroyed,
///   inside the caller's task.
///
/// Like the protected client, it is a request transport, not a server, and
/// establishes no accepted connection, listener, TLS handshake, peer, body
/// streaming or stopped detached descendant. It keeps its router private, is
/// not a Tower service and cannot be served:
///
/// ```compile_fail,E0277
/// async fn cannot_serve(
///     listener: tokio::net::TcpListener,
///     client: batter_axum::low_level::RouterClient,
/// ) {
///     axum::serve(listener, client).await.unwrap();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn requires_a_request_service<S: tower::Service<axum::extract::Request>>() {}
///
/// fn cannot_be_a_request_service() {
///     requires_a_request_service::<batter_axum::low_level::RouterClient>();
/// }
/// ```
///
/// ```compile_fail,E0616
/// fn cannot_read_the_prepared_field(client: batter_axum::low_level::RouterClient) {
///     let _ = client.prepared;
/// }
/// ```
///
/// Each conversion and accessor that could hand the router back has its own
/// control:
///
/// ```compile_fail,E0599
/// fn cannot_take_the_router(client: batter_axum::low_level::RouterClient) {
///     let _ = client.into_router();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn converts_into_a_router<T: Into<axum::Router>>() {}
///
/// fn cannot_convert_into_a_router() {
///     converts_into_a_router::<batter_axum::low_level::RouterClient>();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn borrows_a_router<T: AsRef<axum::Router>>() {}
///
/// fn cannot_borrow_a_router() {
///     borrows_a_router::<batter_axum::low_level::RouterClient>();
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn dereferences_to_a_router<T: std::ops::Deref<Target = axum::Router>>() {}
///
/// fn cannot_dereference_to_a_router() {
///     dereferences_to_a_router::<batter_axum::low_level::RouterClient>();
/// }
/// ```
///
/// A bare router has none of the boundary's layers, yet a reused routing
/// record is still refused before it runs:
///
/// ```
/// use axum::{
///     Router,
///     body::Body,
///     extract::{OriginalUri, Request},
///     http::{StatusCode, Uri},
///     routing::get,
/// };
/// use batter_axum::{ReusedRoutingState, low_level::RouterClient};
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let client = RouterClient::new(Router::new().route("/work", get(|| async { "ok" })));
///
/// let fresh = Request::builder().uri("/work").body(Body::empty())?;
/// let served = client.request(fresh).await;
/// assert_eq!(served.status(), StatusCode::OK);
/// assert!(served.extensions().get::<ReusedRoutingState>().is_none());
///
/// let mut reused = Request::builder().uri("/work").body(Body::empty())?;
/// reused
///     .extensions_mut()
///     .insert(OriginalUri(Uri::from_static("/earlier")));
/// let refused = client.request(reused).await;
/// assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
/// assert!(refused.extensions().get::<ReusedRoutingState>().is_some());
/// # Ok(()) }
/// ```
#[derive(Clone)]
#[must_use = "issue requests through the client"]
pub struct RouterClient {
    prepared: Router,
}

impl RouterClient {
    /// Prepare `router` once, exactly as a served router is prepared.
    ///
    /// Pinned Axum 0.8.9 `Router::into_make_service` and
    /// `Router::into_make_service_with_connect_info` both call
    /// `Router::with_state(())` so that endpoints become routes eagerly instead
    /// of once per request. This uses the same preparation and then clones the
    /// prepared router per request. Recheck it whenever the Axum pin changes.
    pub fn new(router: Router) -> Self {
        Self {
            prepared: router.with_state(()),
        }
    }

    /// Dispatch one arrival-shaped request through the router and return its
    /// response.
    ///
    /// This is the entry contract of
    /// [`InProcessClient::request`](crate::InProcessClient::request), which
    /// documents the detected records, the refusal and the migration.
    /// In short: a request whose extensions carry the path-parameter record
    /// that [`RawPathParams`] reads (any result except its missing-parameters
    /// rejection), [`MatchedPath`], [`NestedPath`] or [`OriginalUri`] is
    /// refused whole before the router, any layer or any application code is
    /// polled, and nothing is stripped. The refusal is
    /// [`render_infrastructure_failure`] for [`HttpFailure::Internal`] with no
    /// correlation identifier, carrying [`ReusedRoutingState`], plus one ERROR
    /// event with a fixed message and no request-derived field on the caller's
    /// dispatch. Application extensions, such as a chosen
    /// [`ConnectInfo`](axum::extract::ConnectInfo), are accepted and reach the
    /// router untouched. The marker is removed from every dispatched response.
    ///
    /// A request is refused the same way when its extensions carry one of
    /// the private request records that only this crate's layers write: the
    /// observation state, the operational wrapper's marker, the admission
    /// record, the browser credential record or the quota record. None of
    /// those types can be named or built outside this crate, so one is
    /// present only in a map copied from a request a Batter layer already
    /// processed. That covers a map with no Axum record left, for example
    /// the map of a dispatch no pattern matched, such as `OPTIONS *`, after
    /// the caller removed its `OriginalUri`, or one captured behind a
    /// lower-level wrapper around a service that is not a `Router`. The
    /// refusal precedes everything inside the router, including
    /// authentication or a quota check it composes.
    pub async fn request(&self, request: Request<Body>) -> Response {
        let (mut parts, body) = request.into_parts();
        if carries_routing_record(&mut parts).await
            || crate::correlation::carries_private_record(&parts.extensions)
        {
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

/// Whether the request carries a record that Axum routing writes. Batter's
/// own private records are checked separately, by
/// `correlation::carries_private_record`.
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
        "in-process request refused: its extensions carry state from an earlier dispatch"
    );
    let mut response = render_infrastructure_failure(HttpFailure::Internal, None);
    response.extensions_mut().insert(ReusedRoutingState::new());
    response
}
