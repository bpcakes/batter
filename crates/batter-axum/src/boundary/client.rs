//! The opaque in-process request client that sealed assembly consumes into.

use axum::{body::Body, extract::Request, response::Response};
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
    /// Every call reuses the router prepared at construction. Request
    /// extensions the caller inserts are kept; the boundary's own private
    /// state is still installed by its own layers.
    pub async fn request(&self, request: Request<Body>) -> Response {
        match self.prepared.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }
}
