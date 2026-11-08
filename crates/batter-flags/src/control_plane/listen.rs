//! The NOTIFY wake: a listener that wakes the refresh when an audited write
//! commits, with the poll as the backstop.
//!
//! The listener is a component that publishes nothing. It holds one session
//! on a private pool, LISTENs on the channel the store names, wakes the
//! refresh once after each LISTEN to catch what it missed while not
//! listening, and wakes it again on every notification. It never reads a
//! payload. A lost session, an error, a failed idle ping or a standby ends the
//! session, and the next one rebuilds and resyncs. At drain it unlistens
//! before it drops its session; SQLx issues a detached `UNLISTEN *` when a
//! listener's connection returns to its pool, which is disclosed, not relied on.

use std::{fmt, time::Duration};

use batter_core::{
    BoxError, ConfigurationError, RegistrationError, lifecycle::RunningComponent,
    periodic::PeriodicWake, registration::RegistrationTarget,
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgListener, PgPoolOptions},
};

/// How long a listening session may be idle before it is pinged.
const IDLE_PING: Duration = Duration::from_secs(30);
/// How long a ping, or the setup of a session, may take.
const SESSION_STEP: Duration = Duration::from_secs(5);
/// How long to wait before rebuilding a lost session.
const REBUILD_DELAY: Duration = Duration::from_secs(1);

/// Where the listener connects, and how closely wakes may follow each other.
///
/// The options are the root's own, never `PgConnectOptions::new()`'s ambient
/// defaults. Formatting never shows them.
///
/// ```
/// use std::time::Duration;
/// use batter_flags::control_plane::ListenEndpoint;
/// use sqlx::postgres::PgConnectOptions;
///
/// let options = PgConnectOptions::new_without_pgpass().host("127.0.0.1").username("flags").database("flags");
/// let endpoint = ListenEndpoint::new(options, Duration::from_secs(1)).unwrap();
/// assert_eq!(format!("{endpoint:?}"), "ListenEndpoint([redacted])");
/// ```
pub struct ListenEndpoint {
    options: PgConnectOptions,
    wake_spacing: Duration,
}

impl ListenEndpoint {
    /// The listener's connection options and its wake spacing: no refresh
    /// starts sooner than this after the previous one.
    pub fn new(
        options: PgConnectOptions,
        wake_spacing: Duration,
    ) -> Result<Self, ConfigurationError> {
        if wake_spacing.is_zero() {
            return Err(ConfigurationError::Zero("flags wake spacing"));
        }
        Ok(Self {
            options,
            wake_spacing,
        })
    }

    pub(crate) fn wake_spacing(&self) -> Duration {
        self.wake_spacing
    }
}

impl fmt::Debug for ListenEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ListenEndpoint([redacted])")
    }
}

impl Clone for ListenEndpoint {
    fn clone(&self) -> Self {
        Self {
            options: self.options.clone(),
            wake_spacing: self.wake_spacing,
        }
    }
}

/// The application name every listener session reports, so its backends can
/// be told apart in `pg_stat_activity`.
pub(crate) const APPLICATION_NAME: &str = "batter-flags-listener";

/// One session's private pool: two connections, each with its own statement
/// timeout. Each session gets a new pool, closed within a bound when the
/// session ends, so a connection a dead peer keeps busy, such as the one
/// SQLx's detached `UNLISTEN *` holds after a lost session, never blocks the
/// next session.
fn session_pool(endpoint: &ListenEndpoint) -> PgPool {
    let options = endpoint
        .options
        .clone()
        .application_name(APPLICATION_NAME)
        .options([("statement_timeout", "5s")]);
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(SESSION_STEP)
        .connect_lazy_with(options)
}

/// Closes a session's pool, or gives up on it after a bound.
async fn retire(pool: PgPool) {
    let _ = tokio::time::timeout(SESSION_STEP, pool.close()).await;
}

/// Registers `flags.listen`. Its sessions' pools are its own: it closes each
/// when the session ends, and the last before it reports stopped.
pub(crate) fn register<T: RegistrationTarget + ?Sized>(
    target: &mut T,
    endpoint: ListenEndpoint,
    wake: PeriodicWake,
) -> Result<(), RegistrationError> {
    target
        .registration()
        .register("flags.listen", move |startup| async move {
            let running = startup.acknowledge_started();
            let mut health = Health::Starting;
            loop {
                let pool = session_pool(&endpoint);
                let ended = session(&pool, &wake, &running, &mut health).await;
                retire(pool).await;
                match ended {
                    Ended::Draining => break,
                    Ended::Lost(cause) => {
                        health = health.lost(cause);
                        tokio::select! {
                            () = running.draining() => break,
                            () = tokio::time::sleep(REBUILD_DELAY) => {}
                        }
                    }
                }
            }
            Ok::<_, BoxError>(running.stopped())
        })
}

/// Why a session ended.
enum Ended {
    Draining,
    Lost(Lost),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lost {
    /// The session could not be set up or was refused.
    Setup,
    /// The pool points at a standby, whose NOTIFY never arrives.
    Standby,
    /// The connection ended or failed while listening.
    Connection,
    /// An idle ping failed or did not return in time.
    Ping,
}

impl Lost {
    fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Standby => "standby",
            Self::Connection => "connection",
            Self::Ping => "ping",
        }
    }
}

/// The listener's health, which decides how loud its events are: a warning
/// when it starts failing or fails differently, nothing more while it keeps
/// failing the same way, and one notice when it listens again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Health {
    Starting,
    Listening,
    Failing(Lost),
}

impl Health {
    fn listening(self) -> Self {
        if let Self::Failing(cause) = self {
            tracing::event!(
                name: "flags.listen",
                target: "batter::flags",
                tracing::Level::INFO,
                batter.flags.listen.recovered_from = cause.as_str(),
                "flags listener is listening again",
            );
        }
        Self::Listening
    }

    fn lost(self, cause: Lost) -> Self {
        if self != Self::Failing(cause) {
            tracing::event!(
                name: "flags.listen",
                target: "batter::flags",
                tracing::Level::WARN,
                batter.flags.listen.cause = cause.as_str(),
                "flags listener lost its session; the poll stays the backstop",
            );
        }
        Self::Failing(cause)
    }
}

/// One listening session, until it is lost or the process drains; at drain
/// it unlistens before it drops its session.
async fn session(
    pool: &PgPool,
    wake: &PeriodicWake,
    running: &RunningComponent,
    health: &mut Health,
) -> Ended {
    let setup = async {
        let standby: bool = sqlx::query_scalar("SELECT pg_is_in_recovery()")
            .fetch_one(pool)
            .await
            .map_err(|_| Lost::Setup)?;
        if standby {
            return Err(Lost::Standby);
        }
        let channel: String =
            sqlx::query_scalar("SELECT channel FROM flags.flag_control WHERE singleton")
                .fetch_one(pool)
                .await
                .map_err(|_| Lost::Setup)?;
        let mut listener = PgListener::connect_with(pool)
            .await
            .map_err(|_| Lost::Setup)?;
        listener.eager_reconnect(false);
        listener.listen(&channel).await.map_err(|_| Lost::Setup)?;
        Ok(listener)
    };
    let mut listener = tokio::select! {
        biased;
        () = running.draining() => return Ended::Draining,
        setup = tokio::time::timeout(SESSION_STEP, setup) => match setup {
            Ok(Ok(listener)) => listener,
            Ok(Err(cause)) => return Ended::Lost(cause),
            Err(_) => return Ended::Lost(Lost::Setup),
        },
    };
    *health = health.listening();
    // Anything written while not listening is caught by one refresh now.
    wake.wake();
    loop {
        let received = tokio::select! {
            biased;
            () = running.draining() => {
                unlisten(&mut listener).await;
                return Ended::Draining;
            }
            received = tokio::time::timeout(IDLE_PING, listener.try_recv()) => received,
        };
        match received {
            Ok(Ok(Some(_))) => wake.wake(),
            // With eager reconnection off, a lost connection is `None`.
            Ok(Ok(None) | Err(_)) => {
                wake.wake();
                return Ended::Lost(Lost::Connection);
            }
            Err(_) => {
                let ping = tokio::select! {
                    biased;
                    () = running.draining() => {
                        unlisten(&mut listener).await;
                        return Ended::Draining;
                    }
                    ping = tokio::time::timeout(SESSION_STEP, sqlx::query("SELECT 1").execute(&mut listener)) => ping,
                };
                if !matches!(ping, Ok(Ok(_))) {
                    wake.wake();
                    return Ended::Lost(Lost::Ping);
                }
            }
        }
    }
}

/// Unlistens before the session is dropped at drain, within a bound.
async fn unlisten(listener: &mut PgListener) {
    let _ = tokio::time::timeout(SESSION_STEP, listener.unlisten_all()).await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::postgres::PgConnectOptions;

    use super::ListenEndpoint;

    #[test]
    fn the_endpoint_never_formats_its_options() {
        let options = PgConnectOptions::new_without_pgpass()
            .host("db.internal")
            .username("flags")
            .password("SENTINEL-PASSWORD")
            .database("flags");
        let endpoint = ListenEndpoint::new(options, Duration::from_secs(1));
        let cloned = endpoint.clone();
        let rendered = format!("{endpoint:?} {cloned:?}");
        assert!(!rendered.contains("SENTINEL-PASSWORD"), "{rendered}");
        assert!(!rendered.contains("db.internal"), "{rendered}");
        let zero = ListenEndpoint::new(PgConnectOptions::new_without_pgpass(), Duration::ZERO);
        assert!(zero.is_err(), "a zero wake spacing is refused");
    }
}
