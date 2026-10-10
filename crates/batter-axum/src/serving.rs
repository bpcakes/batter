use axum::{
    Router,
    serve::{Listener, ListenerExt},
};
use batter_core::{
    RegistrationError,
    lifecycle::Supervisor,
    registration::{Registration, RegistrationTarget},
};
use std::fmt::Debug;

/// Register an already-bound native Axum server as a supervised critical task.
///
/// `listener` is any bound [`axum::serve::Listener`]: [`tokio::net::TcpListener`],
/// [`tokio::net::UnixListener`], or an application-owned listener such as one
/// that completes TLS handshakes before yielding a connection. Batter owns no
/// transport policy. Binding, certificates, private keys, protocol versions,
/// ALPN, client-certificate rules and handshake concurrency stay with the
/// application's listener. Axum 0.8.9 additionally requires the listener's
/// address type to implement [`Debug`].
///
/// Bind the listener and finish router/resource initialization in [`batter_core::startup::Startup`]
/// before calling this. Registration transfers listener ownership, does not poll
/// the server, and releases it on registration failure or abandoned startup.
/// Cancelling a borrowed startup waiter leaves the owner intact. Dropping the
/// startup owner requests drain; the live coordinator releases the registered
/// listener before awaiting dependent startup cleanup. Observers can await that
/// cleanup report without retaining running ownership.
/// The task acknowledges startup when it runs, then serves until native drain.
/// Application approval and a running supervisor are still required for readiness.
/// After startup transfers the running owner, use
/// [`batter_core::lifecycle::RunningSupervisor::wait_checked`] to propagate an
/// unsuccessful process report; registration alone proves no clean shutdown.
///
/// Axum internally spawns connection and graceful-signal tasks. A graceful return
/// waits for native connections, but abortion/panic of the direct wrapper does
/// not prove its descendants stopped; the supervisor conservatively skips cleanup.
/// Request deadlines/observations do not bound streaming bodies or WebSockets.
/// The native server retries accept errors; it does not report them as task exits.
/// No bind policy, signals, budgets, subscriber or runtime are installed here.
///
/// A custom listener's `accept` is polled only inside this registered task, and
/// Axum's trait requires it to handle and retry its own accept errors rather
/// than return them, so accept failures never reach the component exit. When
/// drain arrives, the accept in progress — including a handshake driven inside
/// it — is dropped without being awaited, and the listener is released before
/// connection completion is awaited. Work the listener spawned onto the runtime
/// instead of polling inside `accept` is a detached descendant: exactly like
/// Axum's own connection tasks, registration proves nothing about its
/// termination, and a wrapper abort or panic still conservatively skips cleanup.
///
/// This helper does not install [`axum::extract::ConnectInfo`]. Use
/// [`register_http_with_connect_info_in`] when middleware or handlers need the
/// listener's own peer address.
/// See the runnable `http_service` example for owned startup and signal composition.
/// Prefer [`register_http_in`] inside canonical protected startup; this signature
/// remains for lower-level direct-supervisor composition.
///
/// Migration: `batter_axum::register_http` moved to
/// `batter_axum::low_level::register_http` with the same caller obligations.
pub fn register_http(
    supervisor: &mut Supervisor,
    name: &'static str,
    listener: impl Listener<Addr: Debug>,
    application: Router,
) -> Result<(), RegistrationError> {
    register_http_impl(supervisor.registration(), name, listener, application)
}

/// Register an already-bound Axum server through constrained registration authority.
///
/// This is the canonical companion to [`batter_core::startup::Startup::scoped`]. It
/// has the same runtime, listener and native descendant limits as
/// [`register_http`], while preventing the adapter from receiving process-start
/// or cleanup-extraction authority.
/// It does not install connection metadata; use [`register_http_with_connect_info_in`]
/// for the listener's peer address.
/// The listener type is inferred, preserving existing explicit target arguments
/// such as `register_http_in::<Supervisor>(...)`.
///
/// ```no_run
/// use axum::{Router, routing::get};
/// use batter_core::startup::ProtectedStartupScope;
/// use batter_axum::low_level::register_http_in;
///
/// async fn register(scope: &mut ProtectedStartupScope) -> Result<(), batter_core::BoxError> {
///     let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
///     let app = Router::new().route("/live", get(batter_axum::low_level::liveness));
///     register_http_in::<ProtectedStartupScope>(scope, "http", listener, app)?;
///     Ok(())
/// }
///
/// async fn finish(running: &batter_core::lifecycle::RunningSupervisor)
///     -> Result<(), batter_core::BoxError>
/// {
///     running.wait_checked().await?;
///     Ok(())
/// }
/// ```
///
/// Any other bound listener registers through the same helper. An application
/// that owns a TLS listener writes the accept and handshake policy itself and
/// keeps Batter's lifecycle contract:
///
/// ```no_run
/// use axum::{Router, routing::get, serve::Listener};
/// use batter_core::registration::RegistrationTarget;
/// use batter_axum::low_level::register_http_in;
/// use std::fmt::Debug;
///
/// // `listener` may be a TcpListener, a UnixListener, or an application-owned
/// // listener that completes its TLS handshakes inside `accept`.
/// fn register<T, L>(scope: &mut T, listener: L) -> Result<(), batter_core::BoxError>
/// where
///     T: RegistrationTarget + ?Sized,
///     L: Listener,
///     L::Addr: Debug,
/// {
///     let app = Router::new().route("/live", get(batter_axum::low_level::liveness));
///     register_http_in(scope, "http", listener, app)?;
///     Ok(())
/// }
/// ```
///
/// Migration: `batter_axum::register_http_in` moved to
/// `batter_axum::low_level::register_http_in` with the same caller obligations.
pub fn register_http_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    name: &'static str,
    listener: impl Listener<Addr: Debug>,
    application: Router,
) -> Result<(), RegistrationError> {
    register_http_impl(target.registration(), name, listener, application)
}

/// Register a native Axum server with the listener's own peer available to the router.
///
/// This opt-in companion to [`register_http_in`] installs
/// [`axum::extract::ConnectInfo<L::Addr>`](axum::extract::ConnectInfo) using
/// Axum's native make-service conversion. For a [`tokio::net::TcpListener`] the
/// extension stays `ConnectInfo<SocketAddr>`: middleware and handlers receive
/// the accepted socket's remote address, including its port. For another
/// listener it carries whatever that listener reports as the accepted peer, so
/// a TLS listener over TCP still yields the direct TCP peer. Behind a proxy
/// this is the proxy's address; forwarded headers are not interpreted.
/// Authentication, proxy trust and any application middleware that replaces
/// extensions remain application-owned.
///
/// Pinned Axum 0.8.9 implements `Connected` for a plain listener only for
/// `TcpListener`, and generically for [`axum::serve::TapIo`]. This helper
/// therefore wraps the listener with an empty
/// [`tap_io`](axum::serve::ListenerExt::tap_io), which changes no accepted
/// connection and no reported address. Applications need no such wrapper of
/// their own, and the extra address bounds are Axum's `Connected` requirements.
///
/// Listener transfer, startup acknowledgement, graceful drain, released pending
/// accepts and conservative cleanup after wrapper abortion have the same
/// contract as [`register_http`]. No custom connection-info type or
/// make-service is accepted.
///
/// ```no_run
/// use axum::{Router, extract::ConnectInfo, routing::get};
/// use batter_core::registration::RegistrationTarget;
/// use batter_axum::low_level::register_http_with_connect_info_in;
/// use std::net::SocketAddr;
///
/// // Call from Startup::scoped after application resources are initialized.
/// async fn register<T: RegistrationTarget + ?Sized>(scope: &mut T)
///     -> Result<(), batter_core::BoxError>
/// {
///     let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
///     let router = Router::new().route("/peer", get(
///         |ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() },
///     ));
///     register_http_with_connect_info_in::<T>(scope, "http", listener, router)?;
///     Ok(())
/// }
/// ```
///
/// An application-owned TLS listener that reports the accepted TCP socket keeps
/// the same `ConnectInfo<SocketAddr>` handlers:
///
/// ```no_run
/// use axum::{Router, extract::ConnectInfo, routing::get, serve::Listener};
/// use batter_core::registration::RegistrationTarget;
/// use batter_axum::low_level::register_http_with_connect_info_in;
/// use std::net::SocketAddr;
///
/// fn register<T, L>(scope: &mut T, listener: L) -> Result<(), batter_core::BoxError>
/// where
///     T: RegistrationTarget + ?Sized,
///     L: Listener<Addr = SocketAddr>,
/// {
///     let router = Router::new().route("/peer", get(
///         |ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() },
///     ));
///     register_http_with_connect_info_in(scope, "http", listener, router)?;
///     Ok(())
/// }
/// ```
///
/// Migration: `batter_axum::register_http_with_connect_info_in` moved to
/// `batter_axum::low_level::register_http_with_connect_info_in` with the same caller obligations.
pub fn register_http_with_connect_info_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    name: &'static str,
    listener: impl Listener<Addr: Clone + Debug + Sync + 'static>,
    application: Router,
) -> Result<(), RegistrationError> {
    register_peer_http_impl(target.registration(), name, listener, application)
}

fn register_http_impl<L>(
    mut registration: Registration<'_>,
    name: &'static str,
    listener: L,
    application: Router,
) -> Result<(), RegistrationError>
where
    L: Listener,
    L::Addr: Debug,
{
    registration.register(name, move |startup| async move {
        let running = startup.acknowledge_started();
        let signal = running.signal();
        let draining = async move { signal.draining().await };
        axum::serve(listener, application)
            .with_graceful_shutdown(draining)
            .await?;
        Ok(running.stopped())
    })
}

fn register_peer_http_impl<L>(
    mut registration: Registration<'_>,
    name: &'static str,
    listener: L,
    application: Router,
) -> Result<(), RegistrationError>
where
    L: Listener,
    L::Addr: Clone + Debug + Sync + 'static,
{
    registration.register(name, move |startup| async move {
        let running = startup.acknowledge_started();
        let signal = running.signal();
        let draining = async move { signal.draining().await };
        axum::serve(
            listener.tap_io(|_: &mut L::Io| {}),
            application.into_make_service_with_connect_info::<L::Addr>(),
        )
        .with_graceful_shutdown(draining)
        .await?;
        Ok(running.stopped())
    })
}
