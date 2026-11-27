//! The pool, runtime bounds and observer every `PostgreSQL` limiter owns, and
//! the one way each of them migrates, reaches a connection and reports. Both
//! quota limiters also time, validate, clean up and observe through the
//! wrappers here rather than through copies of their own.

use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use runlimit_core::{
    AdmissionObservation, BatchDecision, Check, CleanupObservation, CleanupOutcome, Decision,
    Observation, Observer, RateLimitPolicy, observe_safely, validate_batch,
};
use sqlx::{
    PgPool,
    migrate::{MigrateError, Migrator},
};

use crate::{
    BatchCheckError, CheckError, MaintenanceError, PostgresConfig,
    transaction::{
        self, BoundedConnection, ConnectionCancellationGuard, ConnectionOutcome, Maintenance,
    },
};

/// The state shared by every limiter, generic over its observer trait object.
pub(crate) struct PgLimiterBase<O: ?Sized> {
    pool: PgPool,
    config: PostgresConfig,
    observer: Option<Arc<O>>,
}

impl<O: ?Sized> Clone for PgLimiterBase<O> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            config: self.config,
            observer: self.observer.clone(),
        }
    }
}

impl<O: ?Sized> PgLimiterBase<O> {
    pub(crate) const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            config: PostgresConfig::new(),
            observer: None,
        }
    }

    pub(crate) const fn set_config(&mut self, config: PostgresConfig) {
        self.config = config;
    }

    pub(crate) fn set_observer(&mut self, observer: Arc<O>) {
        self.observer = Some(observer);
    }

    pub(crate) const fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) const fn config(&self) -> PostgresConfig {
        self.config
    }

    pub(crate) fn observer(&self) -> Option<&O> {
        self.observer.as_deref()
    }

    /// Formats the limiter called `name`. The observer is reported only as
    /// present or absent, so the output is marked non-exhaustive.
    pub(crate) fn fmt_debug(&self, name: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(name)
            .field("pool", &self.pool)
            .field("config", &self.config)
            .field("has_observer", &self.observer.is_some())
            .finish_non_exhaustive()
    }

    /// Applies one bundled migration `history`, ignoring unrelated versions
    /// in a shared history.
    ///
    /// Migration locking is session scoped. The connection returns to the pool
    /// only after a successful run and unlock; an error or cancellation closes
    /// it instead.
    ///
    /// This is the one place a migrator runs. It selects the bundled
    /// migrator itself and runs a fresh one over its migrations, so only
    /// published SQL runs, under the default history table and no schema.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the bundled migrations run"
    )]
    pub(crate) async fn migrate(&self, history: BundledHistory) -> Result<(), MigrateError> {
        let bundled = match history {
            BundledHistory::FixedWindow => &crate::MIGRATOR,
            BundledHistory::Gcra => &crate::GCRA_MIGRATOR,
            BundledHistory::Attempts => &crate::attempts::ATTEMPTS_MIGRATOR,
        };
        let mut connection = ConnectionCancellationGuard::new(self.pool.acquire().await?);
        let mut migrator = Migrator::with_migrations(bundled.iter().cloned().collect());
        migrator.set_ignore_missing(true);
        let result = migrator
            .run_direct(None, &mut **connection.connection(), false)
            .await;
        connection.release(match result {
            Ok(()) => ConnectionOutcome::Reusable(Ok(())),
            Err(error) => ConnectionOutcome::MustClose(Err(error)),
        })
    }

    pub(crate) async fn check_connection(
        &self,
    ) -> Result<BoundedConnection<transaction::Check>, CheckError> {
        BoundedConnection::acquire(&self.pool, self.config).await
    }

    /// Runs one bounded cleanup `transaction` that removes at most
    /// `maximum_rows` rows. A zero limit performs no database work.
    async fn cleanup(
        &self,
        maximum_rows: u32,
        transaction: impl AsyncFnOnce(
            &mut BoundedConnection<Maintenance>,
            u32,
        ) -> Result<u64, ConnectionOutcome<MaintenanceError>>,
    ) -> Result<u64, MaintenanceError> {
        if maximum_rows == 0 {
            return Ok(0);
        }
        let mut connection =
            BoundedConnection::<Maintenance>::acquire(&self.pool, self.config).await?;
        let result = transaction(&mut connection, maximum_rows).await;
        connection.finish(result.map(ConnectionOutcome::Reusable))
    }
}

/// The migration histories this crate publishes, the only ones
/// [`PgLimiterBase::migrate`] runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BundledHistory {
    FixedWindow,
    Gcra,
    Attempts,
}

/// Quota limiters report through the backend-neutral [`Observer`]. Callbacks
/// run only after the operation has released its connection.
///
/// The `observed_*` wrappers are the one timing and reporting path of both
/// quota limiters: each measures wall-clock time around the whole operation,
/// connection acquisition included, and reports its result once.
impl PgLimiterBase<dyn Observer> {
    /// Runs one check's `admission` and reports it.
    pub(crate) async fn observed_check<P: RateLimitPolicy + ?Sized>(
        &self,
        check: &Check<'_, P>,
        admission: impl Future<Output = Result<Decision, CheckError>>,
    ) -> Result<Decision, CheckError> {
        let started = Instant::now();
        let result = admission.await;
        self.observe_check(check, &result, started.elapsed());
        result
    }

    /// Validates `checks`, runs the batch's `admission` only when they form a
    /// valid batch, and reports the batch either way. A rejected batch drops
    /// `admission` unpolled, so it never acquires a connection.
    pub(crate) async fn observed_batch<P: RateLimitPolicy>(
        &self,
        checks: &[Check<'_, P>],
        admission: impl Future<Output = Result<BatchDecision, CheckError>>,
    ) -> Result<BatchDecision, BatchCheckError> {
        let started = Instant::now();
        let result = match validate_batch(checks, self.config.max_batch_size()) {
            Ok(()) => admission.await.map_err(BatchCheckError::from),
            Err(error) => Err(error.into()),
        };
        self.observe_batch(checks, &result, started.elapsed());
        result
    }

    /// Runs one bounded cleanup `transaction` that removes at most
    /// `maximum_rows` rows, and reports it. A zero limit performs no database
    /// work and is reported as confirmed with nothing removed.
    pub(crate) async fn observed_cleanup(
        &self,
        maximum_rows: u32,
        transaction: impl AsyncFnOnce(
            &mut BoundedConnection<Maintenance>,
            u32,
        ) -> Result<u64, ConnectionOutcome<MaintenanceError>>,
    ) -> Result<u64, MaintenanceError> {
        let started = Instant::now();
        let result = self.cleanup(maximum_rows, transaction).await;
        self.observe_cleanup(maximum_rows, &result, started.elapsed());
        result
    }

    pub(crate) fn observe_check<P: RateLimitPolicy + ?Sized>(
        &self,
        check: &Check<'_, P>,
        result: &Result<Decision, CheckError>,
        elapsed: Duration,
    ) {
        let Some(observer) = self.observer() else {
            return;
        };
        let admission = match result {
            Ok(decision) => AdmissionObservation::from_check(check, decision, elapsed),
            Err(error) => AdmissionObservation::failed_check(check, error.consumption(), elapsed),
        };
        observe_safely(observer, &Observation::Admission(admission));
    }

    pub(crate) fn observe_batch<P: RateLimitPolicy + ?Sized>(
        &self,
        checks: &[Check<'_, P>],
        result: &Result<BatchDecision, BatchCheckError>,
        elapsed: Duration,
    ) {
        let Some(observer) = self.observer() else {
            return;
        };
        let admission = match result {
            Ok(decision) => AdmissionObservation::from_batch(checks, decision, elapsed),
            Err(error) => AdmissionObservation::failed_batch(checks, error.consumption(), elapsed),
        };
        observe_safely(observer, &Observation::Admission(admission));
    }

    pub(crate) fn observe_cleanup(
        &self,
        requested: u32,
        result: &Result<u64, MaintenanceError>,
        elapsed: Duration,
    ) {
        let Some(observer) = self.observer() else {
            return;
        };
        let requested = usize::try_from(requested).unwrap_or(usize::MAX);
        let outcome = match result {
            Ok(removed) => CleanupOutcome::Confirmed { removed: *removed },
            Err(error) if error.may_have_removed_rows() => CleanupOutcome::Unknown,
            Err(_) => CleanupOutcome::NoEffect,
        };
        observe_safely(
            observer,
            &Observation::Cleanup(CleanupObservation::new(requested, outcome, elapsed)),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use runlimit_core::{BatchError, Observation, Observer};
    use sqlx::postgres::PgPoolOptions;

    use crate::{
        BatchCheckError, PostgresConfig, PostgresGcraLimiter, PostgresLimiter,
        attempts::PostgresAttemptLimiter,
    };

    #[derive(Default)]
    struct RecordingObserver(Mutex<Vec<String>>);

    impl Observer for RecordingObserver {
        fn observe(&self, observation: &Observation<'_>) {
            let entry = match observation {
                Observation::Admission(admission) => {
                    format!("{:?} {:?}", admission.outcome(), admission.consumption())
                }
                Observation::Cleanup(cleanup) => {
                    format!("cleanup {} {:?}", cleanup.requested(), cleanup.outcome())
                }
                Observation::Capacity(_) => "capacity".to_owned(),
            };
            self.0.lock().unwrap().push(entry);
        }
    }

    // The pool points at a closed port, so either path reaching for a
    // connection would fail instead of returning these results.
    #[tokio::test]
    async fn both_quota_limiters_validate_skip_and_observe_through_one_path() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let fixed_window_observer = Arc::new(RecordingObserver::default());
        let gcra_observer = Arc::new(RecordingObserver::default());
        let fixed_window =
            PostgresLimiter::new(pool.clone()).with_observer(fixed_window_observer.clone());
        let gcra = PostgresGcraLimiter::new(pool).with_observer(gcra_observer.clone());

        assert!(matches!(
            fixed_window.check_all(&[]).await,
            Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
        ));
        assert!(matches!(
            gcra.check_all(&[]).await,
            Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
        ));
        assert_eq!(fixed_window.cleanup_expired(0).await.unwrap(), 0);
        assert_eq!(gcra.cleanup_expired(0).await.unwrap(), 0);

        let observed = fixed_window_observer.0.lock().unwrap().clone();
        assert_eq!(
            observed,
            ["Failed NotConsumed", "cleanup 0 Confirmed { removed: 0 }"]
        );
        assert_eq!(*gcra_observer.0.lock().unwrap(), observed);
    }

    #[tokio::test]
    async fn every_limiter_exposes_the_same_shell() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let config = PostgresConfig::new()
            .with_operation_timeout(Duration::from_secs(7))
            .expect("valid operation timeout");

        let fixed_window = PostgresLimiter::new(pool.clone()).with_config(config);
        let gcra = PostgresGcraLimiter::new(pool.clone()).with_config(config);
        let attempts = PostgresAttemptLimiter::new(pool).with_config(config);

        assert_eq!(fixed_window.config(), config);
        assert_eq!(gcra.config(), config);
        assert_eq!(attempts.config(), config);
        for (name, debug) in [
            ("PostgresLimiter", format!("{fixed_window:?}")),
            ("PostgresGcraLimiter", format!("{gcra:?}")),
            ("PostgresAttemptLimiter", format!("{attempts:?}")),
        ] {
            assert!(debug.starts_with(&format!("{name} {{ pool: ")), "{debug}");
            assert!(debug.ends_with("has_observer: false, .. }"), "{debug}");
        }
    }
}
