//! The explicit synthetic test transport of a prepared quota boundary.

use axum::{
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
    /// Built only by `PreparedHttp::in_process`.
    pub(super) client: RouterClient,
}

impl TestClient {
    /// Dispatches with the supplied synthetic peer, replacing any request peer.
    ///
    /// The request must be arrival-shaped. The entry contract is
    /// `batter_axum::low_level::RouterClient::request`, the one that
    /// `batter_axum::InProcessClient::request` also delegates to, which lists
    /// the detected Axum routing records and the refusal: a request built
    /// from a whole extension map cloned inside an earlier dispatch is
    /// refused whole, nothing is stripped, and the response carries the
    /// `batter_axum::ReusedRoutingState` marker; a dispatched response never
    /// does. Here that refusal happens at entry, before lifecycle admission,
    /// authentication, the native quota check, public probes or any handler
    /// run, so it consumes no quota and has no completion event or
    /// `x-request-id` header. Application extensions reach the assembly
    /// untouched.
    ///
    /// The check also covers the adapter's own private request records. A
    /// map that carries this boundary's observation and operational records
    /// but no Axum record, such as one captured in a protected custom
    /// fallback for an `OPTIONS *` request after the caller removed its
    /// `OriginalUri`, is refused the same way, before authentication and the
    /// native quota check.
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
