use super::PROBE_LEASE_RELEASE_BUDGET;
use sqlx::{Connection, PgConnection};
use std::{fmt, future::Future, sync::Arc};
use tokio::time::{Instant, timeout_at};

/// Server observation of the single acquisition attempted by this control owner.
#[derive(Clone, Debug)]
pub enum UnlockOutcome {
    /// PostgreSQL acknowledged removal of this owner's lock.
    Confirmed,
    /// PostgreSQL confirmed this session held no matching lock.
    NotHeld,
    /// An unusable session could not be asked to unlock.
    Unavailable,
    /// The query failed; remote release remains unconfirmed.
    Failed(Arc<sqlx::Error>),
    /// No answer arrived within the release allowance.
    TimedOut,
}

/// Local client closure; even success is not a backend-exit witness.
#[derive(Clone, Debug)]
pub enum CloseOutcome {
    /// Terminate was sent and the client transport closed.
    Closed,
    /// Closing the client returned its native error.
    Failed(Arc<sqlx::Error>),
    /// The shared release allowance expired before closure completed.
    TimedOut,
}

/// Independent unlock and client-close observations, retained even on timeout.
///
/// ```
/// use batter_example_reference_service::worker::{ProbeLeaseRelease, UnlockOutcome, CloseOutcome};
/// let release = ProbeLeaseRelease { unlock: UnlockOutcome::Confirmed, close: CloseOutcome::Closed };
/// assert!(release.is_success()); // Does not assert that the backend exited.
/// ```
#[derive(Clone)]
pub struct ProbeLeaseRelease {
    /// The server's answer, or why no answer was obtained.
    pub unlock: UnlockOutcome,
    /// The separate local closure result.
    pub close: CloseOutcome,
}

impl ProbeLeaseRelease {
    /// True when this session holds no lock and client closure completed.
    pub fn is_success(&self) -> bool {
        matches!(
            self.unlock,
            UnlockOutcome::Confirmed | UnlockOutcome::NotHeld
        ) && matches!(self.close, CloseOutcome::Closed)
    }
}

impl fmt::Display for ProbeLeaseRelease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("startup control release observations (native details withheld)")
    }
}
impl fmt::Debug for ProbeLeaseRelease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for ProbeLeaseRelease {}

pub(super) async fn release_probe_lease(
    connection: PgConnection,
    usable: bool,
) -> ProbeLeaseRelease {
    release_with(
        connection,
        usable,
        |connection| {
            Box::pin(async move {
                sqlx::query_scalar("SELECT pg_advisory_unlock($1, $2)")
                    .bind(super::STARTUP_LEASE_LOCK_CLASS)
                    .bind(super::STARTUP_LEASE_LOCK_KEY)
                    .fetch_one(connection)
                    .await
            })
        },
        |connection| connection.close(),
    )
    .await
}

// Borrow for unlock, then consume for close. Both phases share one deadline;
// an unlock failure does not bypass a close attempt or erase either outcome.
async fn release_with<C, U, F>(
    connection: C,
    usable: bool,
    unlock: U,
    close: impl FnOnce(C) -> F,
) -> ProbeLeaseRelease
where
    U: for<'a> FnOnce(
        &'a mut C,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<bool, sqlx::Error>> + Send + 'a>,
    >,
    F: Future<Output = Result<(), sqlx::Error>>,
{
    let mut connection = connection;
    let deadline = Instant::now() + PROBE_LEASE_RELEASE_BUDGET;
    let unlock = if usable {
        match timeout_at(deadline, unlock(&mut connection)).await {
            Ok(Ok(true)) => UnlockOutcome::Confirmed,
            Ok(Ok(false)) => UnlockOutcome::NotHeld,
            Ok(Err(error)) => UnlockOutcome::Failed(Arc::new(error)),
            Err(_) => UnlockOutcome::TimedOut,
        }
    } else {
        UnlockOutcome::Unavailable
    };
    let close = match timeout_at(deadline, close(connection)).await {
        Ok(Ok(())) => CloseOutcome::Closed,
        Ok(Err(error)) => CloseOutcome::Failed(Arc::new(error)),
        Err(_) => CloseOutcome::TimedOut,
    };
    ProbeLeaseRelease { unlock, close }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::pending, time::Duration};

    #[tokio::test]
    async fn unlock_and_close_failures_are_both_retained() {
        let release = release_with(
            (),
            true,
            |_| Box::pin(async { Err(sqlx::Error::Protocol("unlock-marker".into())) }),
            |()| async { Err(sqlx::Error::Protocol("close-marker".into())) },
        )
        .await;
        let UnlockOutcome::Failed(unlock) = &release.unlock else {
            panic!("lost unlock failure")
        };
        let CloseOutcome::Failed(close) = &release.close else {
            panic!("lost close failure")
        };
        assert!(unlock.to_string().contains("unlock-marker"));
        assert!(close.to_string().contains("close-marker"));
        assert!(!release.is_success());
        assert!(!format!("{release:?}").contains("marker"));
    }

    #[tokio::test(start_paused = true)]
    async fn close_timeout_preserves_confirmed_unlock() {
        let release =
            release_with((), true, |_| Box::pin(async { Ok(true) }), |()| pending()).await;
        assert!(matches!(release.unlock, UnlockOutcome::Confirmed));
        assert!(matches!(release.close, CloseOutcome::TimedOut));
        assert!(!release.is_success());
    }

    #[tokio::test(start_paused = true)]
    async fn unlock_timeout_still_attempts_close_with_same_deadline() {
        let started = Instant::now();
        let release = release_with((), true, |_| Box::pin(pending()), |()| async { Ok(()) }).await;
        assert!(matches!(release.unlock, UnlockOutcome::TimedOut));
        assert!(matches!(release.close, CloseOutcome::Closed));
        assert_eq!(started.elapsed(), PROBE_LEASE_RELEASE_BUDGET);
        assert!(!release.is_success());
    }

    #[tokio::test(start_paused = true)]
    async fn composed_stop_includes_abort_drain_and_full_release_attempt() {
        let result = tokio::time::timeout(super::super::WORKER_SHUTDOWN_ALLOWANCE, async {
            // Exercise the release implementation after the pinned native API's
            // maximum cooperative plus abort-drain allowance has elapsed.
            tokio::time::sleep(
                super::super::UPSTREAM_SHUTDOWN_BUDGET + super::super::UPSTREAM_ABORT_DRAIN,
            )
            .await;
            release_with(
                (),
                true,
                |_| {
                    Box::pin(async {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        Err(sqlx::Error::PoolClosed)
                    })
                },
                |()| pending(),
            )
            .await
        })
        .await
        .expect("outer grace must retain the release outcome");
        assert!(matches!(result.unlock, UnlockOutcome::Failed(_)));
        assert!(matches!(result.close, CloseOutcome::TimedOut));
    }

    #[tokio::test]
    async fn absent_lock_and_unusable_session_are_distinct() {
        let absent = release_with(
            (),
            true,
            |_| Box::pin(async { Ok(false) }),
            |()| async { Ok(()) },
        )
        .await;
        assert!(matches!(absent.unlock, UnlockOutcome::NotHeld));
        assert!(absent.is_success());
        let unavailable = release_with(
            (),
            false,
            |_| panic!("must not query unusable session"),
            |()| async { Ok(()) },
        )
        .await;
        assert!(matches!(unavailable.unlock, UnlockOutcome::Unavailable));
        assert!(!unavailable.is_success());
    }
}
