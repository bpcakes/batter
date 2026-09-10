use axum::Router;
use batter::{RegistrationError, lifecycle::Supervisor};
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
/// ```no_run
/// use axum::{Router, routing::get};
/// use batter::lifecycle::Supervisor;
/// use batter_axum::register_http;
/// # async fn example(supervisor: &mut Supervisor) -> Result<(), batter::BoxError> {
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
/// let app = Router::new().route("/live", get(batter_axum::liveness));
/// register_http(supervisor, "http", listener, app)?;
/// # Ok(()) }
/// ```
/// This helper accepts a plain [`Router`]; it does not install
/// [`axum::extract::ConnectInfo`]. Applications requiring peer addresses keep the
/// native make-service conversion in their own supervised component:
///
/// ```no_run
/// use axum::{Router, extract::ConnectInfo, routing::get};
/// use batter::lifecycle::Supervisor;
/// use std::net::SocketAddr;
/// # async fn example(supervisor: &mut Supervisor) -> Result<(), batter::BoxError> {
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
/// let app = Router::new().route("/peer", get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move {
///     peer.to_string()
/// }));
/// supervisor.register("http", move |shutdown| async move {
///     shutdown.mark_started();
///     axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
///         .with_graceful_shutdown(async move { shutdown.draining().await })
///         .await?;
///     Ok(())
/// })?;
/// # Ok(()) }
/// ```
/// See the runnable `http_service` example for owned startup and signal composition.
pub fn register_http(
    supervisor: &mut Supervisor,
    name: &'static str,
    listener: TcpListener,
    application: Router,
) -> Result<(), RegistrationError> {
    supervisor.register(name, move |shutdown| async move {
        shutdown.mark_started();
        axum::serve(listener, application)
            .with_graceful_shutdown(async move { shutdown.draining().await })
            .await?;
        Ok(())
    })
}
