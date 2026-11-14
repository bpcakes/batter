//! The explicit synthetic test transport of a prepared quota boundary.

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request},
    response::Response,
};
use batter_axum::low_level::RouterClient;
use std::net::SocketAddr;

/// Explicit synthetic transport; does not implement Service or expose its Router.
///
/// It dispatches through `batter_axum::low_level::RouterClient`, the transport
/// that `batter_axum::InProcessClient` also delegates to, so it accepts only
/// an arrival-shaped request: [`Self::request`] refuses one whose extensions
/// carry an Axum routing record from an earlier dispatch before the protected
/// assembly runs.
///
/// ```compile_fail
/// async fn cannot_serve(listener: tokio::net::TcpListener, client: batter_runlimit::http::TestClient) {
///     axum::serve(listener, client).await.unwrap();
/// }
/// ```
///
/// It cannot become the adapter's protected boundary client, whose type proves
/// a different composition:
///
/// ```compile_fail,E0308
/// fn cannot_claim_the_boundary_client(
///     client: batter_runlimit::http::TestClient,
/// ) -> batter_axum::InProcessClient {
///     client
/// }
/// ```
#[derive(Clone)]
pub struct TestClient {
    client: RouterClient,
}

impl TestClient {
    /// Prepare the assembled router once, as served registration prepares it.
    ///
    /// `RouterClient::new` performs the `with_state(())` preparation that
    /// `into_make_service_with_connect_info` performs when served; pinned Axum
    /// 0.8.9 would otherwise rebuild lazily layered handler routes on every
    /// call.
    pub(super) fn prepare(router: Router) -> Self {
        Self {
            client: RouterClient::new(router),
        }
    }

    /// Dispatches with the supplied synthetic peer, replacing any request peer.
    ///
    /// The request must be arrival-shaped, exactly as for
    /// `batter_axum::InProcessClient::request`: its extensions may carry
    /// application values, which reach the assembly untouched, but no Axum
    /// routing record, namely the path-parameter record that `RawPathParams`
    /// reads, `MatchedPath`, `NestedPath` or `OriginalUri`. Every request an
    /// Axum `Router` dispatched carries at least one of them, so a request
    /// built from a whole extension map cloned inside an earlier dispatch is
    /// refused whole, and nothing is stripped. The refusal happens at entry,
    /// before lifecycle admission, authentication, the native quota check,
    /// public probes or any handler run, so no quota is consumed and no
    /// completion event or `x-request-id` header is produced. It returns
    /// `batter_axum::render_infrastructure_failure(HttpFailure::Internal,
    /// None)` carrying the `batter_axum::ReusedRoutingState` response
    /// extension, and emits one ERROR event with a fixed message and no
    /// request-derived field. That marker is removed from every dispatched
    /// response, so a returned response carries it only when this call
    /// refused the request.
    ///
    /// Migration: build a fresh request and copy each application extension
    /// by type instead of the whole map:
    ///
    /// ```
    /// use axum::{body::Body, extract::Request, http::Extensions};
    ///
    /// #[derive(Clone)]
    /// struct Label(&'static str);
    ///
    /// fn redispatch(earlier: &Extensions) -> Request<Body> {
    ///     let mut request = Request::builder()
    ///         .method("POST")
    ///         .uri("/work")
    ///         .body(Body::empty())
    ///         .unwrap();
    ///     if let Some(label) = earlier.get::<Label>() {
    ///         request.extensions_mut().insert(label.clone());
    ///     }
    ///     request
    /// }
    /// # let _ = redispatch(&Extensions::new());
    /// ```
    ///
    /// Without the refusal, pinned Axum 0.8.9 would treat the inherited
    /// records as an outer router's state after authentication and the quota
    /// check had already run: `Path` extraction fails or reads another
    /// route's captures, and `MatchedPath` gains the earlier pattern as a
    /// prefix.
    pub async fn request(&self, mut request: Request<Body>, peer: SocketAddr) -> Response {
        request.extensions_mut().insert(ConnectInfo(peer));
        self.client.request(request).await
    }
}
