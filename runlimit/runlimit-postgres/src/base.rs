//! The pool, runtime bounds and observer every `PostgreSQL` limiter owns, and
//! the one way each of them migrates, reaches a connection and reports.

use std::{fmt, sync::Arc, time::Duration};

use runlimit_core::{
    AdmissionObservation, BatchDecision, Check, CleanupObservation, CleanupOutcome, Decision,
    Observation, Observer, RateLimitPolicy, observe_safely,
};
use sqlx::{
    PgPool,
    migrate::{MigrateError, Migrator},
};

use crate::{
    BatchCheckError, CheckError, MaintenanceError, PostgresConfig,
    transaction::{self, BoundedConnection, ConnectionCancellationGuard, ConnectionOutcome},
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

    /// Applies the `bundled` migrations, ignoring unrelated versions in a
    /// shared history.
    ///
    /// Migration locking is session scoped. The connection returns to the pool
    /// only after a successful run and unlock; an error or cancellation closes
    /// it instead.
    pub(crate) async fn migrate(&self, bundled: &Migrator) -> Result<(), MigrateError> {
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

    pub(crate) async fn maintenance_connection(
        &self,
    ) -> Result<BoundedConnection<transaction::Maintenance>, MaintenanceError> {
        BoundedConnection::acquire(&self.pool, self.config).await
    }
}

/// Quota limiters report through the backend-neutral [`Observer`]. Callbacks
/// run only after the operation has released its connection.
impl PgLimiterBase<dyn Observer> {
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
    use std::time::Duration;

    use sqlx::postgres::PgPoolOptions;

    use crate::{
        PostgresConfig, PostgresGcraLimiter, PostgresLimiter, attempts::PostgresAttemptLimiter,
    };

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
