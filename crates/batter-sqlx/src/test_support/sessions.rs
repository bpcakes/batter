use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use sqlx::PgPool;
use tokio::sync::watch;

use super::{DatabaseCleanup, DatabaseFixture, FixtureError};

/// A separately budgeted admin pool and bounded session observation policy.
///
/// Install with `FixtureSuite::with_session_observer` before starting a run. The
/// pool must target the same server, outside every disposable database and every
/// closing application pool. The caller owns its capacity and eventual close.
/// Stop all producers of new database connections before body exit. Observation
/// is a point-in-time absence witness, not a fence against future connections.
///
/// On failure, cleanup retains the lease and waits for [`Self::retry_with`].
/// Each explicitly requested attempt retains its failure in progress and the
/// final report. Dropping this control does not release parked leases; a live
/// driver can remain pending indefinitely. Runtime death has no completion guarantee.
/// A missing database is an unresolved target, not proof of session absence on
/// the intended server. Use separate controls for independently recovered runs;
/// share one only when their retry and observer-pool lifetime policies are shared.
/// Server identity is a caller precondition, not inferred from database names:
/// an identically named database on another cluster could pass the catalog query.
/// Use the native PostgreSQL catalog with normal session-identity visibility.
/// Parked leases retain upstream admission permits. If they exhaust a shared
/// harness's capacity, new runs can wait indefinitely for acquisition; arrange
/// recovery before waiting for more leases. Retention is not a liveness guarantee.
///
/// ```no_run
/// # async fn example(suite: batter_sqlx::test_support::FixtureSuite, admin: sqlx::PgPool) {
/// use batter_sqlx::test_support::SessionObserver;
/// use std::time::Duration;
/// let observer = SessionObserver::new(admin.clone(), Duration::from_secs(2)).unwrap();
/// let mut run = suite.with_session_observer(observer.clone())
///     .start(|_| Box::pin(async { Ok::<_, ()>(()) }));
/// if run.wait_for(Duration::from_secs(3)).await.unwrap().is_none() {
///     // Inspect progress and repair/release the actual resource before retrying.
///     let _progress = run.cleanup_progress();
///     observer.retry_with(admin);
/// }
/// // This consuming wait has no deadline: a failed retry parks again. A caller
/// // needing bounded, recoverable observation must keep using run.wait_for(...)
/// // and retain the run/control while deciding whether another retry is warranted.
/// let report = run.into_report().await.unwrap();
/// // Recovered observation failures still make report.is_ok() false.
/// assert!(report.databases.iter().all(|database| database.result.is_ok()));
/// let _retained_failures: Vec<_> = report.databases.iter()
///     .flat_map(|database| &database.observation_failures).collect();
/// # }
/// ```
#[derive(Clone)]
pub struct SessionObserver {
    pool: watch::Sender<PgPool>,
    bound: Duration,
}

impl SessionObserver {
    /// Set the independent pool and positive budget for each full absence attempt.
    pub fn new(pool: PgPool, bound: Duration) -> Result<Self, FixtureError> {
        if bound.is_zero() || tokio::time::Instant::now().checked_add(bound).is_none() {
            return Err(FixtureError::InvalidBudget);
        }
        let (pool, _) = watch::channel(pool);
        Ok(Self { pool, bound })
    }

    /// Retry parked observations with this admin pool after repairing their cause.
    /// Replacing a pool does not close its sessions. If an earlier observer was
    /// inside a disposable database, close that pool and witness backend exit
    /// before retrying with a correctly targeted admin pool.
    /// This broadcasts to every database using this control, including other runs.
    /// Requests not yet consumed coalesce. A request during an active attempt does
    /// not cancel it: if it fails, the latest request authorizes another attempt
    /// immediately. A successful attempt needs no retry. A database starting its
    /// first attempt uses the latest pool without an extra queued attempt. Each
    /// database still requires its own positive absence witness before deletion.
    pub fn retry_with(&self, pool: PgPool) {
        self.pool.send_replace(pool);
    }
}

/// Current cleanup phase, not evidence of database reusability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupPhase {
    /// Native tracked connections are still being closed.
    ClosingPools,
    /// Tracked pools closed; independently observing remaining server sessions.
    ObservingSessions,
    /// Observation failed; the lease is retained until an explicit retry.
    AwaitingRetry,
    /// Quiescence requirement met; consuming native lease cleanup is running.
    CleaningLease,
    /// The database cleanup result is recorded; inspect the final report.
    Complete,
}

/// Historical per-database progress. Clones retain errors, never resource ownership.
/// Debug prints only phase and failure count; identities and causes stay inspectable
/// through the fields, without being formatted automatically.
#[derive(Clone)]
pub struct DatabaseProgress {
    /// Disposable database identity.
    pub database_name: String,
    /// Last published phase.
    pub phase: CleanupPhase,
    /// Every failed explicit observation attempt, including recovered failures.
    pub observation_failures: Vec<Arc<FixtureError>>,
}

impl std::fmt::Debug for DatabaseProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseProgress")
            .field("phase", &self.phase)
            .field("observation_failures", &self.observation_failures.len())
            .finish_non_exhaustive()
    }
}

pub(super) type Progress = Arc<Mutex<Vec<DatabaseProgress>>>;

pub(super) async fn cleanup(
    fixture: DatabaseFixture,
    observer: Option<SessionObserver>,
    progress: Progress,
    index: usize,
) -> DatabaseCleanup {
    let name = fixture.database_name().to_owned();
    futures_util::future::join_all(fixture.pools.iter().map(PgPool::close)).await;
    if let Some(observer) = observer {
        let mut retry = observer.pool.subscribe();
        loop {
            set_phase(&progress, index, CleanupPhase::ObservingSessions);
            let pool = retry.borrow_and_update().clone();
            match observe(&pool, &name, observer.bound).await {
                Ok(()) => break,
                Err(error) => {
                    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
                    state[index].observation_failures.push(Arc::new(error));
                    state[index].phase = CleanupPhase::AwaitingRetry;
                }
            }
            // The retained sender keeps this pending even if the caller loses
            // its retry handle. Owner loss must never authorize deletion.
            let _ = retry.changed().await;
        }
    }
    set_phase(&progress, index, CleanupPhase::CleaningLease);
    let result = fixture.lease.cleanup().await.map_err(FixtureError::Harness);
    let mut state = progress.lock().unwrap_or_else(|error| error.into_inner());
    state[index].phase = CleanupPhase::Complete;
    DatabaseCleanup {
        database_name: name,
        result,
        pool_failures: fixture.pool_failures,
        observation_failures: state[index].observation_failures.clone(),
    }
}

fn set_phase(progress: &Progress, index: usize, phase: CleanupPhase) {
    progress.lock().unwrap_or_else(|error| error.into_inner())[index].phase = phase;
}

async fn observe(pool: &PgPool, name: &str, bound: Duration) -> Result<(), FixtureError> {
    let work = async {
        loop {
            // Each query is a separate auto-commit transaction: no cached
            // pg_stat_activity snapshot spans attempts. Check database presence
            // too: a missing target cannot prove quiescence. Name presence does
            // not authenticate cluster identity; same-server targeting is required.
            let (outside, exists, active): (bool, bool, bool) = sqlx::query_as(
                "SELECT current_database() <> $1, \
                 EXISTS(SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1), \
                 EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname = $1)",
            )
            .bind(name)
            .fetch_one(pool)
            .await
            .map_err(FixtureError::Observe)?;
            if !outside || !exists {
                return Err(FixtureError::ObservationTarget);
            }
            if !active {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(bound, work)
        .await
        .map_err(|_| FixtureError::ObservationTimeout)?
}
