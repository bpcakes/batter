use axum::Router;
use batter::{
    RegistrationError,
    lifecycle::Supervisor,
    registration::{Registration, RegistrationTarget},
};
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// Register an already-bound native Axum server as a supervised critical task.
///
/// Bind the listener and finish router/resource initialization in [`batter::startup::Startup`]
/// before calling this. Registration transfers listener ownership, does not poll
/// the server, and releases it on registration failure or abandoned startup.
/// Cancelling a borrowed startup waiter leaves the owner intact. Dropping the
/// startup owner requests drain; the live coordinator releases the registered
/// listener before awaiting dependent startup cleanup. Observers can await that
/// cleanup report without retaining running ownership.
/// The task acknowledges startup when it runs, then serves until native drain.
/// Application approval and a running supervisor are still required for readiness.
///
/// Axum internally spawns connection and graceful-signal tasks. A graceful return
/// waits for native connections, but abortion/panic of the direct wrapper does
/// not prove its descendants stopped; the supervisor conservatively skips cleanup.
/// Request deadlines/observations do not bound streaming bodies or WebSockets.
/// The native server retries accept errors; it does not report them as task exits.
/// No bind policy, signals, budgets, subscriber or runtime are installed here.
///
/// This helper does not install [`axum::extract::ConnectInfo`]. Use
/// [`register_http_with_connect_info_in`] when middleware or handlers need the
/// direct TCP peer address.
/// See the runnable `http_service` example for owned startup and signal composition.
/// Prefer [`register_http_in`] inside canonical protected startup; this signature
/// remains for lower-level direct-supervisor composition.
pub fn register_http(
    supervisor: &mut Supervisor,
    name: &'static str,
    listener: TcpListener,
    application: Router,
) -> Result<(), RegistrationError> {
    register_http_impl(
        supervisor.registration(),
        name,
        listener,
        application,
        false,
    )
}

/// Register an already-bound Axum server through constrained registration authority.
///
/// This is the canonical companion to [`batter::startup::Startup::scoped`]. It
/// has the same runtime and native descendant limits as [`register_http`], while
/// preventing the adapter from receiving process-start or cleanup-extraction authority.
/// It does not install connection metadata; use [`register_http_with_connect_info_in`]
/// for the direct TCP peer.
///
/// ```no_run
/// use axum::{Router, routing::get};
/// use batter::startup::ProtectedStartupScope;
/// use batter_axum::register_http_in;
///
/// async fn register(scope: &mut ProtectedStartupScope) -> Result<(), batter::BoxError> {
///     let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
///     let app = Router::new().route("/live", get(batter_axum::liveness));
///     register_http_in(scope, "http", listener, app)?;
///     Ok(())
/// }
/// ```
pub fn register_http_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    name: &'static str,
    listener: TcpListener,
    application: Router,
) -> Result<(), RegistrationError> {
    register_http_impl(target.registration(), name, listener, application, false)
}

/// Register a native Axum server with the direct TCP peer available to the router.
///
/// This opt-in companion to [`register_http_in`] installs
/// [`axum::extract::ConnectInfo<SocketAddr>`] using Axum's native make-service
/// conversion. Middleware and handlers receive the accepted socket's remote
/// address, including its port. Behind a proxy this is the proxy's socket address;
/// forwarded headers are not interpreted. Authentication, proxy trust and any
/// application middleware that replaces extensions remain application-owned.
///
/// Listener transfer, startup acknowledgement, graceful drain and conservative
/// cleanup after wrapper abortion have the same contract as [`register_http`].
/// No custom connection-info type, listener or make-service is accepted.
///
/// ```no_run
/// use axum::{Router, extract::ConnectInfo, routing::get};
/// use batter::registration::RegistrationTarget;
/// use batter_axum::register_http_with_connect_info_in;
/// use std::net::SocketAddr;
///
/// // Call from Startup::scoped after application resources are initialized.
/// async fn register<T: RegistrationTarget + ?Sized>(scope: &mut T)
///     -> Result<(), batter::BoxError>
/// {
///     let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
///     let router = Router::new().route("/peer", get(
///         |ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() },
///     ));
///     register_http_with_connect_info_in(scope, "http", listener, router)?;
///     Ok(())
/// }
/// ```
pub fn register_http_with_connect_info_in<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    name: &'static str,
    listener: TcpListener,
    application: Router,
) -> Result<(), RegistrationError> {
    register_http_impl(target.registration(), name, listener, application, true)
}

fn register_http_impl(
    mut registration: Registration<'_>,
    name: &'static str,
    listener: TcpListener,
    application: Router,
    direct_peer: bool,
) -> Result<(), RegistrationError> {
    registration.register(name, move |shutdown| async move {
        shutdown.mark_started();
        let draining = async move { shutdown.draining().await };
        if direct_peer {
            axum::serve(
                listener,
                application.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(draining)
            .await?;
        } else {
            axum::serve(listener, application)
                .with_graceful_shutdown(draining)
                .await?;
        }
        Ok(())
    })
}
