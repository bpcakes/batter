//! Replica-safe `PostgreSQL` storage for Runlimit quotas and outcome-aware attempts.
//! [`PostgresGcraLimiter`] and [`attempts::PostgresAttemptLimiter`] each use
//! independently opt-in migrations; existing fixed-window consumers need no
//! new schema or workflow changes.
//!
//! [`PostgresLimiter`] implements the same anchored fixed-window model as the
//! in-memory backend: the first admitted check starts a window, and subsequent
//! checks share that window until it expires. `PostgreSQL`'s clock is the sole
//! time authority, so callers on different hosts do not need synchronized
//! clocks. Atomic batches use set-based lock, preflight, and update phases, so
//! their number of SQL phases does not grow with the batch size. Single checks
//! use the same separate logical-key, counter-row, and capacity-shard lock
//! phases so every statement that tests row absence starts after the logical
//! key lock has been acquired.
//! [`PostgresLimiter`] implements [`runlimit_core::Limiter`] for async generic
//! adapters.
//! The optional `serde` feature enables validated [`PostgresConfig`] loading
//! and the corresponding `runlimit-core` policy and response metadata.
//!
//! # Installation
//!
//! Call [`PostgresLimiter::migrate`] before serving traffic. It can share
//! `SQLx`'s `_sqlx_migrations` table with application migrations only when every
//! participating [`Migrator`] configures
//! [`Migrator::set_ignore_missing`] to `true`.
//! A strict application migrator will reject Runlimit's migration versions as
//! unknown. The first migration creates `runlimit_fixed_windows` in the
//! connection's default schema.
//!
//! [`MIGRATOR`] is the raw bundled migrator with `SQLx`'s strict defaults. It is
//! intended for an exclusively managed migration history and connection. It
//! does not add [`PostgresLimiter::migrate`]'s protection against returning a
//! possibly session-locked connection to a pool after an error or cancellation.
//! Applications that retain a strict host migrator should vendor the bundled
//! Runlimit SQL as application-owned migrations, in order, using
//! [`CREATE_RUNLIMIT_FIXED_WINDOWS_SQL`] and
//! [`SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL`] followed by
//! [`BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL`] instead of invoking either
//! Runlimit migrator against the shared history.
//!
//! Runtime processes can call [`PostgresLimiter::validate_installation`] with
//! [`MigrationHistory::Bundled`] or [`MigrationHistory::ApplicationManaged`]
//! before serving traffic. Both inspect the installed schema and effective
//! runtime grants in a read-only transaction without applying migrations,
//! probing writes, or consuming quota. The application-managed mode supports
//! vendored SQL under host migration versions without reading a migration ledger.
//!
//! # Storage capacity
//!
//! Every counter belongs to one of [`CAPACITY_SHARD_COUNT`] persistent shards.
//! The migration-maintained ledger enforces
//! [`HARD_MAX_ROWS_PER_SHARD`] even for older replicas, while
//! [`PostgresConfig::maximum_rows_per_shard`] lets current replicas fail closed
//! at a lower operational bound. A full shard denies only new storage keys;
//! existing rows remain usable and active rows are never evicted. Expiry alone
//! does not release a row. Capacity becomes reusable only after
//! [`PostgresLimiter::cleanup_expired`] commits its deletion.
//!
//! # Failure semantics
//!
//! Pre-commit database failures and pre-commit timeouts mean the transaction
//! did not commit. A
//! [`CheckError::CommitOutcomeUnknown`] or [`CheckError::CommitTimedOut`] error
//! means `PostgreSQL` may have committed even though the client did not receive
//! confirmation. Fail closed in both cases, and do not blindly retry a check
//! that may already have consumed quota; [`CheckError::consumption`] names the
//! certainty. A decision is always built from the database response before the
//! transaction is finalized, so a malformed response is reported as a
//! pre-commit failure and never as a possibly committed one.
//! Once the database has produced a denial, an explicit rollback failure or
//! deadline cannot replace that valid decision; the connection is discarded so
//! closing it rolls back the non-mutating denied transaction.
//! Dropping a check or cleanup future prevents the backend from reporting its
//! outcome; if cancellation races with commit, callers must likewise treat the
//! outcome as unknown. Prefer this backend's configured acquisition and
//! operation budgets to a shorter outer timeout.
//!
//! The advisory-lock `v1` domain and derivation are a cross-replica protocol.
//! They must not be changed in place: replicas from a rolling deployment must
//! calculate identical lock IDs for the same persistent counter. `PostgreSQL`'s
//! advisory-lock namespace is shared by every application in the same database.
//! An unrelated or deliberately held matching `bigint` lock therefore blocks
//! the affected Runlimit key until the operation deadline; use a trusted
//! database boundary and restrict advisory-lock privileges for unrelated roles.
//! The persisted capacity-shard derivation is likewise a cross-replica
//! protocol: it XORs the leading bytes of the policy fingerprint and opaque
//! subject key. Changing the derivation or shard count requires a new storage
//! protocol and migration.
//!
//! # Integration tests
//!
//! The database tests are opt-in:
//!
//! ```text
//! RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql:///runlimit_test \
//! cargo test -p runlimit-postgres --test postgres -- --ignored
//! ```

use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant as WallClockInstant},
};

use runlimit_core::{
    AdmissionObservation, BatchDecision, Check, CleanupObservation, CleanupOutcome, Decision,
    FixedWindowPolicy, Limiter, Observation, Observer, observe_safely, validate_batch,
};
use sqlx::{
    PgPool, Postgres,
    migrate::{MigrateError, Migrator},
    pool::PoolConnection,
};
use tokio::time::Instant;

#[derive(Debug)]
pub(crate) enum ConnectionOutcome<T> {
    Reusable(T),
    MustClose(T),
}

impl<T> ConnectionOutcome<T> {
    fn map<U>(self, map: impl FnOnce(T) -> U) -> ConnectionOutcome<U> {
        match self {
            Self::Reusable(value) => ConnectionOutcome::Reusable(map(value)),
            Self::MustClose(value) => ConnectionOutcome::MustClose(map(value)),
        }
    }
}

mod admission;
pub mod attempts;
mod gcra;
pub use gcra::{
    CREATE_RUNLIMIT_GCRA_SQL, GCRA_MIGRATOR, INDEX_RUNLIMIT_GCRA_SHARD_EXPIRY_SQL,
    PostgresGcraLimiter,
};
mod config;
mod errors;
mod installation;
mod maintenance;
mod protocol;

pub use config::{PostgresConfig, PostgresConfigError};
pub use errors::{
    BatchCheckError, CheckError, CheckPhase, CleanupPhase, MaintenanceError, StorageInvariantError,
};
pub use installation::{InstallationError, InstallationIssue, MigrationHistory};
pub use protocol::{CAPACITY_SHARD_COUNT, HARD_MAX_ROWS_PER_SHARD};

use admission::{
    Admission, BatchSqlInput, acquire_check_connection, batch_decision, run_check_transaction,
    single_decision,
};
use maintenance::{acquire_maintenance_connection, run_cleanup_transaction};

#[cfg(test)]
use admission::{
    PendingAllowance, PendingDenial, PendingQuotaDenial, database_integer,
    finish_denied_transaction,
};
#[cfg(test)]
use protocol::{
    BATCH_PREFLIGHT_SQL, BATCH_UPSERT_SQL, CLEANUP_SQL, advisory_lock_id, capacity_shard,
};
#[cfg(test)]
use sqlx::postgres::types::PgInterval;

/// SQL for the immutable `20260723000000_create_runlimit_fixed_windows`
/// migration.
///
/// Strict host migrators can copy this SQL into their application-owned
/// migration stream instead of running [`MIGRATOR`] against a shared
/// `_sqlx_migrations` history. Apply it before
/// [`SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL`].
pub const CREATE_RUNLIMIT_FIXED_WINDOWS_SQL: &str =
    include_str!("../migrations/20260723000000_create_runlimit_fixed_windows.sql");

/// SQL for the additive
/// `20260725000000_set_runlimit_fixed_windows_fillfactor` migration.
///
/// Strict host migrators can copy this SQL into their application-owned
/// migration stream after [`CREATE_RUNLIMIT_FIXED_WINDOWS_SQL`]. The
/// application remains responsible for assigning versions that fit its own
/// migration history.
pub const SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL: &str =
    include_str!("../migrations/20260725000000_set_runlimit_fixed_windows_fillfactor.sql");

/// SQL for the additive
/// `20260726000000_bound_runlimit_fixed_window_cardinality` migration.
///
/// This migration assigns every persisted counter to one of 256 stable
/// capacity shards and installs a trigger-maintained row-count ledger. Apply it
/// after [`SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL`].
pub const BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL: &str =
    include_str!("../migrations/20260726000000_bound_runlimit_fixed_window_cardinality.sql");

/// Raw bundled migrations required by [`PostgresLimiter`].
///
/// This value retains `SQLx`'s strict missing-version behavior and is suitable
/// only for an exclusively managed migration history and connection. For a
/// pooled application that shares `_sqlx_migrations`, prefer
/// [`PostgresLimiter::migrate`] and configure every other participating
/// [`Migrator`] to set [`Migrator::set_ignore_missing`] to `true`.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// A replica-safe rate limiter backed by a `PostgreSQL` pool.
///
/// Cloning this value is cheap because [`PgPool`] is internally reference
/// counted.
#[derive(Clone)]
pub struct PostgresLimiter {
    pool: PgPool,
    config: PostgresConfig,
    observer: Option<Arc<dyn Observer>>,
}

impl fmt::Debug for PostgresLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresLimiter")
            .field("pool", &self.pool)
            .field("config", &self.config)
            .field("has_observer", &self.observer.is_some())
            .finish_non_exhaustive()
    }
}

impl PostgresLimiter {
    /// Creates a limiter using `pool` and the default [`PostgresConfig`].
    ///
    /// Call [`Self::migrate`] during application startup before admitting
    /// requests.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            config: PostgresConfig::new(),
            observer: None,
        }
    }

    /// Returns this limiter with an explicit runtime configuration.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.config = config;
        self
    }

    /// Returns this limiter with an operational observer.
    ///
    /// Observer callbacks run only after a database operation has released its
    /// connection. Callback panics are isolated and cannot change admission or
    /// cleanup results.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Returns the underlying connection pool.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Returns the backend runtime configuration.
    pub const fn config(&self) -> PostgresConfig {
        self.config
    }

    /// Applies the bundled Runlimit database migrations.
    ///
    /// Unrelated successful versions in `_sqlx_migrations` are ignored, which
    /// permits a shared migration history only when every other participating
    /// `SQLx` migrator also sets [`Migrator::set_ignore_missing`] to `true`.
    /// Known Runlimit versions still receive `SQLx`'s checksum validation.
    /// Applications that keep a strict host migrator should instead vendor the
    /// bundled SQL into their application-owned migration stream.
    ///
    /// Migration locking is session scoped. An acquired connection is returned
    /// to the pool only after a successful run and unlock; an error or
    /// cancellation closes it instead.
    ///
    /// # Errors
    ///
    /// Returns an error if migration metadata cannot be read or `PostgreSQL`
    /// rejects a migration.
    pub async fn migrate(&self) -> Result<(), MigrateError> {
        let connection = self.pool.acquire().await?;
        let mut guarded_connection = ConnectionCancellationGuard::new(connection);
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.set_ignore_missing(true);

        let result = migrator
            .run_direct(None, &mut **guarded_connection.connection(), false)
            .await;
        if result.is_ok() {
            guarded_connection.reuse();
        }
        result
    }

    /// Checks and, when allowed, consumes one fixed-window quota.
    ///
    /// A denied decision does not change the stored counter. A single check
    /// has no batch structure to validate, so it goes straight to the
    /// database and its error type carries no batch-only variants.
    ///
    /// # Errors
    ///
    /// Pool-acquisition or operation timeout before commit is
    /// [`CheckError::TimedOutBeforeCommit`]. Other database failures before
    /// commit are [`CheckError::DefinitelyNotConsumed`]. Any commit error is
    /// conservatively classified as [`CheckError::CommitOutcomeUnknown`].
    pub async fn check(&self, check: &Check<'_>) -> Result<Decision, CheckError> {
        let started = WallClockInstant::now();
        let result = self
            .run_checks(std::slice::from_ref(check), single_decision)
            .await;
        self.observe_single_admission(check, &result, started.elapsed());
        result
    }

    /// Atomically checks and consumes every quota in the nonempty `checks`.
    ///
    /// Rows are acquired in a deterministic storage-key order to avoid
    /// deadlocks between overlapping batches. Returned allowed decisions retain
    /// the caller's input order. An empty batch, repeated occurrences of the
    /// same storage key, and mixed quota modes are rejected before a
    /// connection is acquired.
    ///
    /// New keys lock their capacity-ledger rows in shard order. If the earliest
    /// caller-ordered failure is the configured shard bound, the batch returns
    /// [`runlimit_core::Denial::StorageCapacity`] with no retry delay and
    /// changes neither quota nor storage. Existing and expired target rows
    /// need no new slot.
    ///
    /// If any check is denied, the backend attempts an explicit rollback and
    /// the returned denial index is the earliest denied item in caller order.
    /// A rollback error or deadline does not replace the denial because no
    /// counter was changed; instead, the connection is discarded so socket
    /// closure rolls the transaction back.
    ///
    /// # Errors
    ///
    /// Structural batch failures are [`BatchCheckError::InvalidBatch`] and
    /// consume nothing. Database failures are [`BatchCheckError::Check`] with
    /// the same classification as [`Self::check`].
    pub async fn check_all(&self, checks: &[Check<'_>]) -> Result<BatchDecision, BatchCheckError> {
        let started = WallClockInstant::now();
        let result = self.check_all_unobserved(checks).await;
        self.observe_batch_admission(checks, &result, started.elapsed());
        result
    }

    async fn check_all_unobserved(
        &self,
        checks: &[Check<'_>],
    ) -> Result<BatchDecision, BatchCheckError> {
        validate_batch(checks, self.config.max_batch_size())?;
        let batch_size = checks.len();
        Ok(self
            .run_checks(checks, move |admission| {
                batch_decision(batch_size, admission)
            })
            .await?)
    }

    /// Runs one admission transaction for a validated, nonempty slice of
    /// checks and shapes its outcome with `build`.
    async fn run_checks<D>(
        &self,
        checks: &[Check<'_>],
        build: impl FnOnce(Admission) -> Result<D, CheckError>,
    ) -> Result<D, CheckError> {
        // Materialize every SQL array and both lock orders before reserving a
        // pool slot. Every database phase then consumes the exact same key
        // arrays, preventing identity drift between locking and mutation.
        let input = BatchSqlInput::from_checks(checks);
        let connection =
            acquire_check_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        // Pool contention does not consume the database-work budget. A
        // survivor receives a fresh deadline for begin, statements, locks,
        // rollback, and commit.
        let deadline = Instant::now() + self.config.operation_timeout();
        let mut guarded_connection = ConnectionCancellationGuard::new(connection);
        let outcome = run_check_transaction(
            guarded_connection.connection(),
            &input,
            checks,
            self.config.maximum_rows_per_shard(),
            deadline,
            build,
        )
        .await;
        guarded_connection.finish(outcome)
    }

    /// Deletes at most `maximum_rows` expired windows and releases their
    /// capacity-ledger slots in the same transaction.
    ///
    /// Cleanup uses `FOR UPDATE SKIP LOCKED`, so it does not wait behind active
    /// checks and cannot remove a row concurrently being renewed. A zero limit
    /// performs no database work.
    ///
    /// # Errors
    ///
    /// Returns an error if `PostgreSQL` rejects the maintenance transaction or
    /// if either [`PostgresConfig::pool_acquire_timeout`] or the fresh
    /// [`PostgresConfig::operation_timeout`] budget is exceeded. A commit error
    /// or commit timeout means rows may have been removed; inspect
    /// [`MaintenanceError::may_have_removed_rows`]. Cleanup only targets
    /// already-expired rows and never consumes quota.
    pub async fn cleanup_expired(&self, maximum_rows: u32) -> Result<u64, MaintenanceError> {
        let started = WallClockInstant::now();
        let result = self.cleanup_expired_unobserved(maximum_rows).await;
        self.observe_cleanup(maximum_rows, &result, started.elapsed());
        result
    }

    async fn cleanup_expired_unobserved(&self, maximum_rows: u32) -> Result<u64, MaintenanceError> {
        if maximum_rows == 0 {
            return Ok(0);
        }

        let connection =
            acquire_maintenance_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        let deadline = Instant::now() + self.config.operation_timeout();
        let mut guarded_connection = ConnectionCancellationGuard::new(connection);
        let outcome =
            run_cleanup_transaction(guarded_connection.connection(), maximum_rows, deadline).await;
        guarded_connection.finish(outcome)
    }

    fn observe_single_admission(
        &self,
        check: &Check<'_>,
        result: &Result<Decision, CheckError>,
        elapsed: Duration,
    ) {
        match result {
            Ok(decision) => self
                .observe_admission(|| AdmissionObservation::from_check(check, decision, elapsed)),
            Err(error) => self.observe_admission(|| {
                AdmissionObservation::failed_check(check, error.consumption(), elapsed)
            }),
        }
    }

    fn observe_batch_admission(
        &self,
        checks: &[Check<'_>],
        result: &Result<BatchDecision, BatchCheckError>,
        elapsed: Duration,
    ) {
        match result {
            Ok(decision) => self
                .observe_admission(|| AdmissionObservation::from_batch(checks, decision, elapsed)),
            Err(error) => {
                self.observe_admission(|| {
                    AdmissionObservation::failed_batch(checks, error.consumption(), elapsed)
                });
            }
        }
    }

    fn observe_admission<'a>(&self, build_observation: impl FnOnce() -> AdmissionObservation<'a>) {
        let Some(observer) = &self.observer else {
            return;
        };
        let admission = build_observation();
        observe_safely(observer.as_ref(), &Observation::Admission(admission));
    }

    fn observe_cleanup(
        &self,
        requested: u32,
        result: &Result<u64, MaintenanceError>,
        elapsed: Duration,
    ) {
        let Some(observer) = &self.observer else {
            return;
        };
        let requested = usize::try_from(requested).unwrap_or(usize::MAX);
        let outcome = match result {
            Ok(removed) => CleanupOutcome::Confirmed { removed: *removed },
            Err(error) if error.may_have_removed_rows() => CleanupOutcome::Unknown,
            Err(_) => CleanupOutcome::NoEffect,
        };
        observe_safely(
            observer.as_ref(),
            &Observation::Cleanup(CleanupObservation::new(requested, outcome, elapsed)),
        );
    }
}

impl Limiter for PostgresLimiter {
    type Policy = FixedWindowPolicy;
    type CheckError = CheckError;
    type CheckAllError = BatchCheckError;

    fn check(
        &self,
        check: &Check<'_>,
    ) -> impl Future<Output = Result<Decision, Self::CheckError>> + Send {
        PostgresLimiter::check(self, check)
    }

    fn check_all(
        &self,
        checks: &[Check<'_>],
    ) -> impl Future<Output = Result<BatchDecision, Self::CheckAllError>> + Send {
        PostgresLimiter::check_all(self, checks)
    }
}

struct ConnectionCancellationGuard {
    connection: PoolConnection<Postgres>,
    armed: bool,
}

impl ConnectionCancellationGuard {
    const fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection,
            armed: true,
        }
    }

    const fn connection(&mut self) -> &mut PoolConnection<Postgres> {
        &mut self.connection
    }

    const fn reuse(&mut self) {
        self.armed = false;
    }

    fn finish<T, E>(mut self, outcome: ConnectionOutcome<Result<T, E>>) -> Result<T, E> {
        match outcome {
            ConnectionOutcome::Reusable(result) => {
                self.reuse();
                result
            }
            ConnectionOutcome::MustClose(result) => result,
        }
    }
}

impl Drop for ConnectionCancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            // Dropping an SQLx query future does not send a PostgreSQL cancel
            // request. Close the socket instead of returning a potentially
            // busy connection to the pool when either Runlimit's deadline or
            // cancellation of the outer limiter future wins the race.
            self.connection.close_on_drop();
        }
    }
}

#[cfg(test)]
mod tests;
