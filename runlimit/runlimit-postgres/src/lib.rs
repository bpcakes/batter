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

use std::{fmt, future::Future, sync::Arc};

use runlimit_core::{BatchDecision, Check, Decision, FixedWindowPolicy, Limiter, Observer};
use sqlx::{
    PgPool,
    migrate::{MigrateError, Migrator},
};

mod admission;
pub mod attempts;
mod base;
#[cfg(test)]
mod catalog_guard;
mod gcra;
pub use gcra::{
    CREATE_RUNLIMIT_GCRA_SQL, GCRA_MIGRATOR, INDEX_RUNLIMIT_GCRA_SHARD_EXPIRY_SQL,
    PostgresGcraLimiter,
};
mod config;
mod errors;
mod maintenance;
mod pending;
mod protocol;
mod statements;
mod transaction;

pub use config::{PostgresConfig, PostgresConfigError};
pub use errors::{
    BatchCheckError, CheckError, CheckPhase, CleanupPhase, MaintenanceError, StorageInvariantError,
};
pub use protocol::{CAPACITY_SHARD_COUNT, HARD_MAX_ROWS_PER_SHARD};

use admission::{Admission, BatchSqlInput, batch_decision, run_check_transaction, single_decision};
use base::{BundledHistory, PgLimiterBase};
use maintenance::run_cleanup_transaction;

#[cfg(test)]
use admission::interval;
#[cfg(test)]
use pending::PendingAllowance;
#[cfg(test)]
use protocol::{advisory_lock_id, capacity_shard};
#[cfg(test)]
use sqlx::postgres::types::PgInterval;
#[cfg(test)]
use statements::fixed_window::CLEANUP_SQL;
#[cfg(test)]
use tokio::time::Instant;
#[cfg(test)]
use transaction::{ConnectionOutcome, finish_denied_transaction};

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
#[expect(
    clippy::disallowed_types,
    reason = "sqlx::migrate! embeds each migration as a sqlx::SqlStr"
)]
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// A replica-safe rate limiter backed by a `PostgreSQL` pool.
///
/// Cloning this value is cheap because [`PgPool`] is internally reference
/// counted.
#[derive(Clone)]
pub struct PostgresLimiter {
    base: PgLimiterBase<dyn Observer>,
}

impl fmt::Debug for PostgresLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.base.fmt_debug("PostgresLimiter", formatter)
    }
}

impl PostgresLimiter {
    /// Creates a limiter using `pool` and the default [`PostgresConfig`].
    ///
    /// Call [`Self::migrate`] during application startup before admitting
    /// requests.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            base: PgLimiterBase::new(pool),
        }
    }

    /// Returns this limiter with an explicit runtime configuration.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.base.set_config(config);
        self
    }

    /// Returns this limiter with an operational observer.
    ///
    /// Observer callbacks run only after a database operation has released its
    /// connection. Callback panics are isolated and cannot change admission or
    /// cleanup results.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.base.set_observer(observer);
        self
    }

    /// Returns the underlying connection pool.
    pub const fn pool(&self) -> &PgPool {
        self.base.pool()
    }

    /// Returns the backend runtime configuration.
    pub const fn config(&self) -> PostgresConfig {
        self.base.config()
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
        self.base.migrate(BundledHistory::FixedWindow).await
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
        self.base
            .observed_check(
                check,
                self.run_checks(std::slice::from_ref(check), single_decision),
            )
            .await
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
        self.base
            .observed_batch(
                checks,
                self.run_checks(checks, |admission| batch_decision(checks.len(), admission)),
            )
            .await
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
        let mut connection = self.base.check_connection().await?;
        let result = run_check_transaction(
            &mut connection,
            &input,
            checks,
            self.base.config().maximum_rows_per_shard(),
            build,
        )
        .await;
        connection.finish(result)
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
        self.base
            .observed_cleanup(maximum_rows, run_cleanup_transaction)
            .await
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

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroUsize,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::*;
    use runlimit_core::{
        AdmissionOperation, AdmissionOutcome, Allowance, BatchDecisionView, BatchError, Capacity,
        CleanupOutcome, ConsumptionStatus, DecisionView, Denial, FixedWindowPolicy, MAX_LIMIT,
        MAX_WINDOW, MAX_WINDOW_MILLIS, Observation, PolicyId, QuotaDenial, QuotaMode, ScopeId,
        SubjectKey,
    };

    fn capacity(value: u64) -> Capacity {
        Capacity::new(value).unwrap()
    }

    fn policy(id: &str, scope: &str) -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new(id).expect("valid policy identifier"),
            ScopeId::new(scope).expect("valid scope identifier"),
            10,
            Duration::from_mins(1),
        )
        .expect("valid policy")
    }

    fn key(byte: u8) -> SubjectKey {
        SubjectKey::from_digest([byte; 32])
    }

    #[derive(Default)]
    struct RecordingObserver {
        events: Mutex<Vec<(usize, AdmissionOutcome, ConsumptionStatus)>>,
        cleanups: Mutex<Vec<(usize, CleanupOutcome)>>,
    }

    impl Observer for RecordingObserver {
        fn observe(&self, observation: &Observation<'_>) {
            match observation {
                Observation::Admission(admission) => self.events.lock().unwrap().push((
                    match admission.operation() {
                        AdmissionOperation::Check { .. } => 1,
                        AdmissionOperation::Batch { batch_size, .. } => batch_size,
                    },
                    admission.outcome(),
                    admission.consumption(),
                )),
                Observation::Cleanup(cleanup) => self
                    .cleanups
                    .lock()
                    .unwrap()
                    .push((cleanup.requested(), cleanup.outcome())),
                // The PostgreSQL backend has no local capacity to report.
                Observation::Capacity(_) => {}
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedOperation {
        Check,
        Batch { batch_size: usize, has_policy: bool },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct RecordedAdmissionMetadata {
        operation: RecordedOperation,
        consumption: ConsumptionStatus,
    }

    #[derive(Default)]
    struct AdmissionMetadataObserver {
        admissions: Mutex<Vec<RecordedAdmissionMetadata>>,
    }

    impl Observer for AdmissionMetadataObserver {
        fn observe(&self, observation: &Observation<'_>) {
            let Observation::Admission(admission) = observation else {
                return;
            };
            self.admissions
                .lock()
                .unwrap()
                .push(RecordedAdmissionMetadata {
                    operation: match admission.operation() {
                        AdmissionOperation::Check { .. } => RecordedOperation::Check,
                        AdmissionOperation::Batch { batch_size, policy } => {
                            RecordedOperation::Batch {
                                batch_size,
                                has_policy: policy.is_some(),
                            }
                        }
                    },
                    consumption: admission.consumption(),
                });
        }
    }

    struct PanickingObserver;

    impl Observer for PanickingObserver {
        fn observe(&self, _: &Observation<'_>) {
            panic!("injected observer panic");
        }
    }

    #[test]
    fn portable_core_boundaries_fit_database_values_exactly() {
        let policy = FixedWindowPolicy::new(
            PolicyId::new("portable.maximum").unwrap(),
            ScopeId::new("subject").unwrap(),
            MAX_LIMIT,
            MAX_WINDOW,
        )
        .unwrap();
        let check = Check::new(key(1).bind(&policy))
            .with_cost(MAX_LIMIT)
            .unwrap();

        assert_eq!(policy.limit().get_i64(), i64::MAX);
        assert_eq!(check.cost_i64(), i64::MAX);
        let window = interval(policy.window());
        assert_eq!(
            window,
            PgInterval::try_from(policy.window().duration()).unwrap()
        );
        assert_eq!(window.months, 0);
        assert_eq!(window.days, 0);
        assert_eq!(
            window.microseconds,
            i64::try_from(MAX_WINDOW_MILLIS * 1_000).unwrap()
        );
    }

    #[test]
    fn row_lock_permutation_orders_complete_counter_keys() {
        let alpha_client = policy("alpha", "client");
        let alpha_identity = policy("alpha", "identity");
        let beta_client = policy("beta", "client");
        let checks = [
            Check::new(key(0).bind(&beta_client)),
            Check::new(key(0).bind(&alpha_identity)),
            Check::new(key(1).bind(&alpha_client)),
            Check::new(key(0).bind(&alpha_client)),
        ];

        let input = BatchSqlInput::from_checks(&checks);
        let locked_keys = input
            .lock_input_positions
            .iter()
            .map(|position| checks[usize::try_from(*position - 1).unwrap()].counter_key())
            .collect::<Vec<_>>();

        assert!(locked_keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn advisory_lock_protocol_has_a_golden_vector() {
        let policy = policy("login", "client");
        let check = Check::new(key(7).bind(&policy));

        assert_eq!(
            advisory_lock_id(check.counter_key()),
            4_549_358_434_381_602_087
        );
    }

    #[test]
    fn capacity_shard_protocol_has_a_golden_vector() {
        let policy = policy("login", "client");
        let check = Check::new(key(7).bind(&policy));

        assert_eq!(capacity_shard(check.counter_key()), 141);
    }

    #[tokio::test]
    async fn observer_reports_connection_free_operations_and_isolates_panics() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let observer = Arc::new(RecordingObserver::default());
        let limiter = PostgresLimiter::new(pool.clone()).with_observer(observer.clone());

        assert!(matches!(
            limiter.check_all(&[]).await,
            Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
        ));
        assert_eq!(limiter.cleanup_expired(0).await.unwrap(), 0);
        assert_eq!(
            observer.events.lock().unwrap().as_slice(),
            [(0, AdmissionOutcome::Failed, ConsumptionStatus::NotConsumed,)]
        );
        assert_eq!(
            observer.cleanups.lock().unwrap().as_slice(),
            [(0, CleanupOutcome::Confirmed { removed: 0 })]
        );

        let panicking = PostgresLimiter::new(pool).with_observer(Arc::new(PanickingObserver));
        assert!(matches!(
            panicking.check_all(&[]).await,
            Err(BatchCheckError::InvalidBatch(BatchError::EmptyBatch))
        ));
        assert_eq!(panicking.cleanup_expired(0).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn failed_postgres_batches_preserve_one_item_policy_metadata() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let observer = Arc::new(AdmissionMetadataObserver::default());
        let limiter = PostgresLimiter::new(pool).with_observer(observer.clone());
        let first_policy = policy("observed.failure.first", "client");
        let second_policy = policy("observed.failure.second", "client");
        let first = Check::new(key(1).bind(&first_policy));
        let second = Check::new(key(2).bind(&second_policy));
        let elapsed = Duration::from_millis(5);

        limiter
            .base
            .observe_check(&first, &Err(CheckError::CommitTimedOut), elapsed);
        limiter.base.observe_batch(
            &[first],
            &Err(BatchCheckError::Check(CheckError::CommitTimedOut)),
            elapsed,
        );
        limiter.base.observe_batch(
            &[first, second],
            &Err(BatchCheckError::Check(CheckError::CommitTimedOut)),
            elapsed,
        );

        assert_eq!(
            observer.admissions.lock().unwrap().as_slice(),
            [
                RecordedAdmissionMetadata {
                    operation: RecordedOperation::Check,
                    consumption: ConsumptionStatus::PossiblyConsumed,
                },
                RecordedAdmissionMetadata {
                    operation: RecordedOperation::Batch {
                        batch_size: 1,
                        has_policy: true,
                    },
                    consumption: ConsumptionStatus::PossiblyConsumed,
                },
                RecordedAdmissionMetadata {
                    operation: RecordedOperation::Batch {
                        batch_size: 2,
                        has_policy: false,
                    },
                    consumption: ConsumptionStatus::PossiblyConsumed,
                },
            ]
        );
    }

    #[tokio::test]
    async fn cleanup_observations_map_every_backend_effect_class() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let observer = Arc::new(RecordingObserver::default());
        let limiter = PostgresLimiter::new(pool).with_observer(observer.clone());
        let elapsed = Duration::from_millis(5);

        limiter.base.observe_cleanup(7, &Ok(3), elapsed);
        limiter.base.observe_cleanup(
            7,
            &Err(MaintenanceError::Database(sqlx::Error::RowNotFound)),
            elapsed,
        );
        limiter.base.observe_cleanup(
            7,
            &Err(MaintenanceError::TimedOutBeforeCommit {
                phase: CleanupPhase::DeletingExpiredWindows,
            }),
            elapsed,
        );
        limiter.base.observe_cleanup(
            7,
            &Err(MaintenanceError::CommitOutcomeUnknown(
                sqlx::Error::RowNotFound,
            )),
            elapsed,
        );
        limiter
            .base
            .observe_cleanup(7, &Err(MaintenanceError::CommitTimedOut), elapsed);

        assert_eq!(
            observer.cleanups.lock().unwrap().as_slice(),
            [
                (7, CleanupOutcome::Confirmed { removed: 3 }),
                (7, CleanupOutcome::NoEffect),
                (7, CleanupOutcome::NoEffect),
                (7, CleanupOutcome::Unknown),
                (7, CleanupOutcome::Unknown),
            ]
        );
    }

    #[test]
    fn stored_usage_above_the_limit_cannot_construct_a_pending_allowance() {
        let malformed = PendingAllowance::new(capacity(10), 11, Duration::from_millis(250))
            .expect_err("usage above the limit cannot become pending allowance state");

        assert!(matches!(malformed, CheckError::StorageInvariant(_)));
        assert_eq!(malformed.consumption(), ConsumptionStatus::NotConsumed);
    }

    #[tokio::test]
    async fn denied_decision_survives_rollback_failure_and_discards_connection() {
        let quota = QuotaDenial::new(capacity(10), Duration::from_millis(130));
        let outcome = finish_denied_transaction(
            Instant::now() + Duration::from_secs(1),
            BatchDecision::denied(2, 3, quota).unwrap(),
            std::future::ready(Err(sqlx::Error::Protocol(
                "injected rollback failure".to_owned(),
            ))),
        )
        .await;

        let ConnectionOutcome::MustClose(decision) = outcome else {
            panic!("rollback failure must close the connection")
        };
        assert_eq!(
            decision.view(),
            BatchDecisionView::Denied {
                index: 2,
                batch_size: NonZeroUsize::new(3).unwrap(),
                denial: Denial::QuotaExceeded(quota),
            }
        );
    }

    #[tokio::test]
    async fn denied_decision_survives_rollback_deadline_and_discards_connection() {
        let quota = QuotaDenial::new(capacity(1), Duration::from_millis(50));
        let outcome = finish_denied_transaction(
            Instant::now(),
            Decision::denied(quota),
            std::future::pending(),
        )
        .await;

        let ConnectionOutcome::MustClose(decision) = outcome else {
            panic!("rollback deadline must close the connection")
        };
        assert_eq!(
            decision.view(),
            DecisionView::Denied {
                denial: Denial::QuotaExceeded(quota),
            }
        );
    }

    #[tokio::test]
    async fn shadow_quota_denial_is_reported_after_rollback() {
        let quota = QuotaDenial::new(capacity(1), Duration::from_secs(1));
        let outcome = finish_denied_transaction(
            Instant::now() + Duration::from_secs(1),
            Decision::shadow_denied(quota),
            std::future::ready(Ok(())),
        )
        .await;

        let ConnectionOutcome::Reusable(decision) = outcome else {
            panic!("successful rollback must reuse the connection")
        };
        assert_eq!(
            decision.view(),
            DecisionView::ShadowDenied { denial: quota }
        );
    }

    #[test]
    fn single_decisions_are_shaped_before_the_transaction_is_finalized() {
        let allowance = Allowance::new(capacity(10), 4, Duration::from_secs(1)).unwrap();
        let quota = QuotaDenial::new(capacity(10), Duration::from_secs(1));

        assert_eq!(
            single_decision(Admission::Allowed(vec![allowance])).unwrap(),
            Decision::allowed(allowance)
        );
        assert_eq!(
            single_decision(Admission::Denied {
                index: 0,
                denial: Denial::QuotaExceeded(quota),
            })
            .unwrap(),
            Decision::denied(quota)
        );
        assert_eq!(
            single_decision(Admission::ShadowDenied {
                index: 0,
                denial: quota,
            })
            .unwrap(),
            Decision::shadow_denied(quota)
        );

        for malformed in [
            single_decision(Admission::Allowed(vec![allowance, allowance])),
            single_decision(Admission::Denied {
                index: 1,
                denial: Denial::QuotaExceeded(quota),
            }),
            single_decision(Admission::ShadowDenied {
                index: 1,
                denial: quota,
            }),
        ] {
            let error = malformed.expect_err("a malformed single response is a failure");
            assert!(matches!(error, CheckError::StorageInvariant(_)));
            assert_eq!(error.consumption(), ConsumptionStatus::NotConsumed);
        }
    }

    #[test]
    fn batch_decisions_are_shaped_against_the_submitted_batch() {
        // The stable classification, spelled out so the constant cannot drift.
        const BATCH_MISMATCH: &str = "batch decision did not match the submitted batch";

        let allowance = Allowance::new(capacity(10), 4, Duration::from_secs(1)).unwrap();
        let quota = QuotaDenial::new(capacity(10), Duration::from_secs(1));

        assert_eq!(
            batch_decision(2, Admission::Allowed(vec![allowance, allowance])).unwrap(),
            BatchDecision::allowed(vec![allowance, allowance]).unwrap()
        );
        assert!(matches!(
            batch_decision(2, Admission::Allowed(vec![allowance])),
            Err(CheckError::StorageInvariant(_))
        ));
        assert_eq!(
            batch_decision(
                2,
                Admission::Denied {
                    index: 1,
                    denial: Denial::QuotaExceeded(quota),
                }
            )
            .unwrap(),
            BatchDecision::denied(1, 2, quota).unwrap()
        );
        assert_eq!(
            batch_decision(
                2,
                Admission::ShadowDenied {
                    index: 1,
                    denial: quota,
                }
            )
            .unwrap(),
            BatchDecision::shadow_denied(1, 2, quota).unwrap()
        );
        let out_of_range = batch_decision(
            2,
            Admission::Denied {
                index: 2,
                denial: Denial::QuotaExceeded(quota),
            },
        )
        .expect_err("a denial outside the batch is a failure");
        assert!(
            matches!(&out_of_range, CheckError::StorageInvariant(error) if error.detail() == BATCH_MISMATCH),
            "{out_of_range:?}"
        );
        let shadow_out_of_range = batch_decision(
            2,
            Admission::ShadowDenied {
                index: 2,
                denial: quota,
            },
        )
        .expect_err("a shadow denial outside the batch is a failure");
        assert!(
            matches!(&shadow_out_of_range, CheckError::StorageInvariant(error) if error.detail() == BATCH_MISMATCH),
            "{shadow_out_of_range:?}"
        );
    }

    #[test]
    fn configuration_bounds_batches_and_deadlines() {
        let defaults = PostgresConfig::new();
        assert_eq!(defaults.maximum_rows_per_shard(), 4_096);
        assert_eq!(defaults.max_batch_size(), 32);
        assert_eq!(defaults.pool_acquire_timeout(), Duration::from_secs(3));
        assert_eq!(defaults.operation_timeout(), Duration::from_secs(3));
        assert_eq!(
            defaults.with_maximum_rows_per_shard(0),
            Err(PostgresConfigError::ZeroMaximumRowsPerShard)
        );
        assert_eq!(
            defaults.with_maximum_rows_per_shard(HARD_MAX_ROWS_PER_SHARD + 1),
            Err(PostgresConfigError::MaximumRowsPerShardTooLarge {
                actual: HARD_MAX_ROWS_PER_SHARD + 1,
                maximum: HARD_MAX_ROWS_PER_SHARD,
            })
        );
        assert_eq!(
            defaults.with_max_batch_size(0),
            Err(PostgresConfigError::ZeroBatchSize)
        );
        assert_eq!(
            defaults.with_pool_acquire_timeout(Duration::ZERO),
            Err(PostgresConfigError::ZeroPoolAcquireTimeout)
        );
        assert_eq!(
            defaults.with_pool_acquire_timeout(Duration::from_secs(61)),
            Err(PostgresConfigError::PoolAcquireTimeoutTooLong {
                actual: Duration::from_secs(61),
                maximum: Duration::from_mins(1),
            })
        );
        assert_eq!(
            defaults.with_operation_timeout(Duration::ZERO),
            Err(PostgresConfigError::ZeroOperationTimeout)
        );
        assert_eq!(
            defaults.with_operation_timeout(Duration::from_secs(61)),
            Err(PostgresConfigError::OperationTimeoutTooLong {
                actual: Duration::from_secs(61),
                maximum: Duration::from_mins(1),
            })
        );
    }

    #[tokio::test]
    async fn duplicate_storage_keys_fail_before_connecting() {
        let alpha = policy("alpha", "client");
        let beta = policy("beta", "client");
        let checks = [
            Check::new(key(7).bind(&beta)),
            Check::new(key(7).bind(&beta)),
            Check::new(key(8).bind(&alpha)),
            Check::new(key(8).bind(&alpha)),
        ];
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let limiter = PostgresLimiter::new(pool);

        let error = limiter
            .check_all(&checks)
            .await
            .expect_err("duplicate keys must be rejected");

        assert!(matches!(
            error,
            BatchCheckError::InvalidBatch(BatchError::DuplicateKey {
                first_index: 0,
                duplicate_index: 1,
            })
        ));
    }

    #[tokio::test]
    async fn oversized_batch_fails_before_connecting() {
        let first_policy = policy("login", "client");
        let second_policy = policy("signup", "client");
        let checks = [
            Check::new(key(1).bind(&first_policy)),
            Check::new(key(2).bind(&second_policy)),
        ];
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let config = PostgresConfig::new()
            .with_max_batch_size(1)
            .expect("one is a valid batch limit");
        let limiter = PostgresLimiter::new(pool).with_config(config);

        let error = limiter
            .check_all(&checks)
            .await
            .expect_err("oversized batch must be rejected");

        assert!(matches!(
            error,
            BatchCheckError::InvalidBatch(BatchError::BatchTooLarge {
                actual: 2,
                maximum: 1,
            })
        ));
    }

    #[tokio::test]
    async fn mixed_quota_modes_fail_before_connecting() {
        let enforced = policy("enforced", "client");
        let shadow = policy("shadow", "client").with_quota_mode(QuotaMode::Shadow);
        let checks = [
            Check::new(key(1).bind(&enforced)),
            Check::new(key(2).bind(&shadow)),
        ];
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://runlimit:runlimit@127.0.0.1:1/runlimit")
            .expect("syntactically valid database URL");
        let limiter = PostgresLimiter::new(pool);

        let error = limiter
            .check_all(&checks)
            .await
            .expect_err("mixed quota modes must be rejected");

        assert!(matches!(
            error,
            BatchCheckError::InvalidBatch(BatchError::MixedQuotaModes {
                first: QuotaMode::Enforce,
                index: 1,
                actual: QuotaMode::Shadow,
            })
        ));
    }

    #[test]
    fn error_reports_consumption_certainty() {
        let definite = CheckError::DefinitelyNotConsumed(sqlx::Error::RowNotFound);
        let unknown = CheckError::CommitOutcomeUnknown(sqlx::Error::RowNotFound);
        let timed_out = CheckError::TimedOutBeforeCommit {
            phase: CheckPhase::AcquiringCounterRowLocks,
        };
        let invariant = CheckError::storage_invariant("injected");
        let decode_invariant =
            CheckError::storage_decode_invariant("decode injected", sqlx::Error::RowNotFound);
        let cleanup_definite = MaintenanceError::Database(sqlx::Error::RowNotFound);
        let cleanup_unknown = MaintenanceError::CommitOutcomeUnknown(sqlx::Error::RowNotFound);

        assert_eq!(definite.consumption(), ConsumptionStatus::NotConsumed);
        assert_eq!(timed_out.consumption(), ConsumptionStatus::NotConsumed);
        assert_eq!(invariant.consumption(), ConsumptionStatus::NotConsumed);
        assert_eq!(
            decode_invariant.consumption(),
            ConsumptionStatus::NotConsumed
        );
        assert_eq!(unknown.consumption(), ConsumptionStatus::PossiblyConsumed);
        assert_eq!(
            CheckError::CommitTimedOut.consumption(),
            ConsumptionStatus::PossiblyConsumed
        );
        assert_eq!(
            BatchCheckError::InvalidBatch(BatchError::EmptyBatch).consumption(),
            ConsumptionStatus::NotConsumed
        );
        assert_eq!(
            BatchCheckError::Check(CheckError::CommitTimedOut).consumption(),
            ConsumptionStatus::PossiblyConsumed
        );
        assert_eq!(
            timed_out.to_string(),
            "PostgreSQL rate-limit check timed out while acquiring counter row lock; quota was \
             not consumed"
        );
        let CheckError::StorageInvariant(semantic) = &invariant else {
            panic!("expected a storage invariant");
        };
        assert_eq!(semantic.detail(), "injected");
        assert!(std::error::Error::source(semantic).is_none());

        let CheckError::StorageInvariant(decode) = &decode_invariant else {
            panic!("expected a decode storage invariant");
        };
        assert_eq!(decode.detail(), "decode injected");
        assert!(
            std::error::Error::source(decode)
                .is_some_and(|source| source.downcast_ref::<sqlx::Error>().is_some())
        );
        assert!(!cleanup_definite.may_have_removed_rows());
        assert!(cleanup_unknown.may_have_removed_rows());
        assert!(MaintenanceError::CommitTimedOut.may_have_removed_rows());
    }

    #[test]
    fn cleanup_query_is_bounded_and_nonblocking() {
        assert_eq!(
            CLEANUP_SQL
                .sql()
                .matches("pg_catalog.clock_timestamp()")
                .count(),
            1
        );
        assert!(CLEANUP_SQL.sql().contains("AS MATERIALIZED"));
        assert!(
            CLEANUP_SQL
                .sql()
                .contains("SELECT sampled_at\n        FROM authoritative_time")
        );
        assert!(
            !CLEANUP_SQL
                .sql()
                .contains("window_expires_at <= pg_catalog.clock_timestamp()")
        );
        assert!(CLEANUP_SQL.sql().contains("LIMIT $1"));
        assert!(CLEANUP_SQL.sql().contains("FOR UPDATE SKIP LOCKED"));
        assert!(
            CLEANUP_SQL
                .sql()
                .contains("ORDER BY capacity.capacity_shard")
        );
        assert!(CLEANUP_SQL.sql().contains("FOR UPDATE OF capacity"));
    }
}
