use std::{error::Error, fmt, time::Duration};

use batter_sqlx::test_support::{CleanupPhase, FixtureReport, FixtureRun, SessionObserver};
use sqlx::PgPool;

use super::fixture_diagnostics::ProbeError;

/// Consumer completion policy. A pending error retains this same run and control.
/// Dropping it detaches the driver; runtime destruction may invoke native lease Drop.
/// A deadline does not authorize lease deletion. A caller that panics after
/// receiving pending can destroy the runtime and trigger native destructive Drop.
#[must_use]
pub struct ObservedRun<T, E> {
    pub run: FixtureRun<T, E>,
    observer: SessionObserver,
    session_pools: Vec<PgPool>,
    pub diagnostic_pool: PgPool,
    driver_failed: Option<bool>,
}

impl<T: Send + Sync + 'static, E: Send + Sync + 'static> ObservedRun<T, E> {
    pub fn new(
        run: FixtureRun<T, E>,
        observer: SessionObserver,
        session_pool: PgPool,
        diagnostic_pool: PgPool,
    ) -> Self {
        Self {
            run,
            observer,
            session_pools: vec![session_pool],
            diagnostic_pool,
            driver_failed: None,
        }
    }

    /// Keep every replacement until the driver finishes; closing an old pool
    /// immediately could disrupt an observation still using it.
    /// Replacing a pool does not repair the sessions it owns. If one targets a
    /// disposable database, use `observer_pools` to close it and witness backend
    /// exit before requesting a corrected retry.
    pub fn retry_with(&mut self, pool: PgPool) {
        self.session_pools.push(pool.clone());
        self.observer.retry_with(pool);
    }

    /// Native pools retained for explicit repair after a pending finish.
    /// Close a misplaced pool through this shared handle before retrying; do not
    /// close pools still serving active attempts or other owners without coordination.
    pub fn observer_pools(&self) -> &[PgPool] {
        &self.session_pools
    }

    /// Bound driver observation and administrative pool closure together.
    /// No retry is automatic. Downcast a pending cause to recover the run/control.
    /// Success leaves the caller's diagnostic clone open for catalog checks and
    /// explicit close; the caller must retain that clone. A terminal driver error
    /// closes both capacities here, before callers can propagate that error.
    pub async fn finish(mut self, bound: Duration) -> Result<FixtureReport<T, E>, ProbeError> {
        let completion = tokio::time::timeout(bound, async {
            self.driver_failed = Some(self.run.wait().await.is_err());
            let sessions = async {
                futures_util::future::join_all(self.session_pools.iter().map(PgPool::close)).await;
            };
            let diagnostics = async {
                if self.driver_failed == Some(true) {
                    self.diagnostic_pool.close().await;
                }
            };
            tokio::join!(sessions, diagnostics);
        })
        .await;
        if completion.is_err() {
            let summary = self.to_string();
            return Err(ProbeError::pending(Box::new(self), summary));
        }
        // wait cached the report or original JoinError. A timed-out close never
        // consumes it, so recovering a pending owner resumes the same outcome.
        self.run
            .into_report()
            .await
            .map_err(|error| ProbeError::new(Box::new(error)))
    }
}

impl<T, E> fmt::Display for ObservedRun<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let progress = self.run.cleanup_progress();
        let phases = [
            CleanupPhase::ClosingPools,
            CleanupPhase::ObservingSessions,
            CleanupPhase::AwaitingRetry,
            CleanupPhase::CleaningLease,
            CleanupPhase::Complete,
        ];
        f.write_str("fixture completion pending; cleanup is not confirmed")?;
        write!(
            f,
            "; driver_joined={}; driver_failed={}",
            self.driver_failed.is_some(),
            self.driver_failed == Some(true)
        )?;
        for phase in phases {
            let count = progress.iter().filter(|item| item.phase == phase).count();
            write!(f, "; {phase:?}={count}")?;
        }
        let failures: usize = progress
            .iter()
            .map(|item| item.observation_failures.len())
            .sum();
        write!(f, "; observation_failures={failures}")
    }
}
impl<T, E> fmt::Debug for ObservedRun<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl<T: 'static, E: 'static> Error for ObservedRun<T, E> {}
