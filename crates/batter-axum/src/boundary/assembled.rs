//! The sealed outcome of assembling the canonical HTTP boundary.

use super::InProcessClient;
use crate::serving;
use axum::{Router, serve::Listener};
use batter_core::{RegistrationError, registration::RegistrationTarget};
use std::fmt;

/// A router with the boundary applied, sealed against recovering that router.
///
/// Assembly has exactly two outcomes: registration for protected serving, or
/// consumption into an opaque [`InProcessClient`]. No layer or route can be
/// added outside the observer, because the router cannot be recovered at all.
/// A bare [`Router`] cannot be passed where this is expected:
///
/// ```compile_fail,E0308
/// fn serve(assembled: batter_axum::AssembledHttp) -> axum::Router {
///     assembled
/// }
/// ```
///
/// Each of the following is a separate control, so one returning escape cannot
/// hide behind another that is still absent. The pre-cutover `into_router`
/// escape is gone:
///
/// ```compile_fail,E0599
/// fn cannot_take_the_router(assembled: batter_axum::AssembledHttp) {
///     let _ = assembled.into_router();
/// }
/// ```
///
/// A route cannot be appended to the assembly either, which the control above
/// would not establish on its own:
///
/// ```compile_fail,E0599
/// use axum::routing::get;
///
/// fn cannot_append_a_late_route(assembled: batter_axum::AssembledHttp) {
///     let _ = assembled.route("/late", get(|| async { "outside the boundary" }));
/// }
/// ```
///
/// Neither can a second observer, or any other layer, be wrapped around it:
///
/// ```compile_fail,E0599
/// fn cannot_wrap_the_assembly(assembled: batter_axum::AssembledHttp) {
///     let _ = assembled.layer(axum::middleware::from_fn(
///         |request: axum::extract::Request, next: axum::middleware::Next| async move {
///             next.run(request).await
///         },
///     ));
/// }
/// ```
///
/// It cannot be handed to `axum::serve`:
///
/// ```compile_fail,E0277
/// async fn cannot_serve_directly(
///     listener: tokio::net::TcpListener,
///     assembled: batter_axum::AssembledHttp,
/// ) {
///     axum::serve(listener, assembled).await.unwrap();
/// }
/// ```
///
/// And it is not a request service of its own, which the `axum::serve` control
/// does not establish because that path needs the make-service conversion:
///
/// ```compile_fail,E0277
/// fn requires_a_request_service<S: tower::Service<axum::extract::Request>>() {}
///
/// fn cannot_be_a_request_service() {
///     requires_a_request_service::<batter_axum::AssembledHttp>();
/// }
/// ```
#[must_use = "register the assembled boundary or consume it into a request client"]
pub struct AssembledHttp {
    pub(super) router: Router,
}

impl AssembledHttp {
    /// Register the assembled server through constrained registration authority.
    ///
    /// `listener` is any bound [`axum::serve::Listener`]: a
    /// [`tokio::net::TcpListener`], a Unix listener, or an application-owned
    /// listener that completes its own TLS handshakes. Certificates, protocol
    /// versions and handshake policy stay with that listener; the boundary,
    /// listener transfer, startup acknowledgement, graceful drain and
    /// conservative cleanup after wrapper abortion are unchanged.
    /// See [`crate::low_level::register_http_in`] for the serving contract.
    ///
    /// ```no_run
    /// use axum::serve::Listener;
    /// use batter_axum::AssembledHttp;
    /// use batter_core::registration::RegistrationTarget;
    /// use std::fmt::Debug;
    ///
    /// fn serve<T, L>(assembled: AssembledHttp, scope: &mut T, listener: L)
    ///     -> Result<(), batter_core::BoxError>
    /// where
    ///     T: RegistrationTarget + ?Sized,
    ///     L: Listener,
    ///     L::Addr: Debug,
    /// {
    ///     assembled.register_in::<T>(scope, "http", listener)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn register_in<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        name: &'static str,
        listener: impl Listener<Addr: fmt::Debug>,
    ) -> Result<(), RegistrationError> {
        serving::register_http_in(target, name, listener, self.router)
    }

    /// Register with the listener's own direct peer available to handlers.
    ///
    /// A [`tokio::net::TcpListener`] and a TLS listener over TCP both supply
    /// [`ConnectInfo<SocketAddr>`](axum::extract::ConnectInfo) holding the
    /// accepted socket's address and port; another listener supplies its own
    /// address type. Forwarded headers are never interpreted.
    /// See [`crate::low_level::register_http_with_connect_info_in`] for the contract and
    /// for a worked generic-listener signature.
    pub fn register_with_connect_info_in<T: RegistrationTarget + ?Sized>(
        self,
        target: &mut T,
        name: &'static str,
        listener: impl Listener<Addr: Clone + fmt::Debug + Sync + 'static>,
    ) -> Result<(), RegistrationError> {
        serving::register_http_with_connect_info_in(target, name, listener, self.router)
    }

    /// Consume the assembly into an opaque in-process request client.
    ///
    /// The client dispatches requests through this exact boundary without a
    /// listener, connection or peer. It keeps the router private, so this is
    /// not a way to recover one: see [`InProcessClient`] for what it refuses.
    /// An in-process response proves response construction, never serving
    /// compatibility; keep real socket tests for that.
    ///
    /// ```
    /// use axum::{body::Body, extract::Request, http::StatusCode, routing::get};
    /// use batter_core::lifecycle::ShutdownHandle;
    /// use batter_axum::{
    ///     GuardedRouter, HttpBoundary, RequestPolicy, ResponseConstructionBudget,
    /// };
    /// use std::time::Duration;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
    /// approval.approve();
    /// let budget = ResponseConstructionBudget::new(Duration::from_secs(1))?;
    /// let client = HttpBoundary::new(RequestPolicy::new(control.operation_admission(), budget))
    ///     .assemble(GuardedRouter::new().route("/work", get(|| async { "ok" })))
    ///     .await?
    ///     .in_process();
    /// let request = Request::builder().uri("/work").body(Body::empty())?;
    /// assert_eq!(client.request(request).await.status(), StatusCode::OK);
    /// # Ok(()) }
    /// ```
    pub fn in_process(self) -> InProcessClient {
        InProcessClient::new(self.router)
    }
}
