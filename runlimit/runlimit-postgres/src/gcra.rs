//! Opt-in durable GCRA storage; independent from the fixed-window protocol.

use std::{fmt, future::Future, sync::Arc, time::Instant as WallInstant};

use runlimit_core::{
    AdmissionObservation, BatchDecision, Check, CleanupObservation, CleanupOutcome, Decision,
    Denial, GcraPolicy, Limiter, Observation, Observer, QuotaMode, gcra, observe_safely,
    validate_batch,
};
use sqlx::{
    Acquire, PgPool, Row,
    migrate::{MigrateError, Migrator},
    postgres::PgRow,
};
use tokio::time::Instant;

use crate::{
    BatchCheckError, CheckError, CheckPhase, CleanupPhase, ConnectionCancellationGuard,
    ConnectionOutcome, MaintenanceError, PostgresConfig,
    admission::{
        Admission, CheckTransaction, acquire_check_connection, batch_decision, single_decision,
    },
    maintenance::{
        acquire_maintenance_connection, commit_maintenance, maintenance_before_commit,
        set_maintenance_server_timeouts,
    },
    protocol::{CAPACITY_SHARD_COUNT, capacity_shard},
};

/// Independent, opt-in GCRA storage migration. Strict host migrators may vendor
/// this SQL into their own migration stream; no fixed-window tables are needed.
pub const CREATE_RUNLIMIT_GCRA_SQL: &str =
    include_str!("../gcra-migrations/20260922000000_create_runlimit_gcra.sql");

/// Additive shard/expiry index for bounded cleanup under regressing clocks.
/// Strict host migrators apply this after [`CREATE_RUNLIMIT_GCRA_SQL`].
pub const INDEX_RUNLIMIT_GCRA_SHARD_EXPIRY_SQL: &str =
    include_str!("../gcra-migrations/20260922000001_index_runlimit_gcra_shard_expiry.sql");

/// Raw GCRA migrator with `SQLx`'s strict defaults. Prefer
/// [`PostgresGcraLimiter::migrate`] for cancellation-safe pooled migration.
pub static GCRA_MIGRATOR: Migrator = sqlx::migrate!("./gcra-migrations");

const LOCK_SHARDS: &str = "SELECT capacity_shard, row_count, observed_at_ms FROM runlimit_gcra_shards WHERE capacity_shard = ANY($1) ORDER BY capacity_shard FOR UPDATE";
const SAMPLE_TIME: &str =
    "SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT AS now_ms";
const READ_COUNTERS: &str = r"
SELECT input.ordinality - 1 AS input_index, stored.tat_scaled::TEXT AS tat_scaled,
    stored.expires_at_ms
FROM unnest($1::BYTEA[], $2::BYTEA[]) WITH ORDINALITY AS input(fingerprint, subject, ordinality)
LEFT JOIN runlimit_gcra AS stored ON stored.config_fingerprint = input.fingerprint
    AND stored.subject_key = input.subject
ORDER BY input.ordinality";
const UPSERT: &str = r"
INSERT INTO runlimit_gcra(config_fingerprint, subject_key, tat_scaled, expires_at_ms)
SELECT fingerprint, subject, tat::NUMERIC, expires
FROM unnest($1::BYTEA[], $2::BYTEA[], $3::TEXT[], $4::BIGINT[]) AS input(fingerprint, subject, tat, expires)
ON CONFLICT (config_fingerprint, subject_key) DO UPDATE
SET tat_scaled = EXCLUDED.tat_scaled, expires_at_ms = EXCLUDED.expires_at_ms";
const CLEANUP_LOCKS: &str = r"
WITH sample AS MATERIALIZED (
    SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT AS now_ms
), candidates AS MATERIALIZED (
    -- At most 256 indexed probes, each returning at most one candidate. Using
    -- the same shard clock as admission makes logically expired slots
    -- reclaimable even while the physical clock remains behind its watermark.
    SELECT shard.capacity_shard FROM runlimit_gcra_shards AS shard
    CROSS JOIN LATERAL (
        SELECT 1 FROM runlimit_gcra AS counter
        WHERE counter.capacity_shard = shard.capacity_shard
            AND counter.expires_at_ms <= greatest((SELECT now_ms FROM sample), shard.observed_at_ms)
        ORDER BY counter.expires_at_ms LIMIT 1
    ) AS expired
)
SELECT capacity.capacity_shard FROM runlimit_gcra_shards AS capacity
WHERE capacity.capacity_shard IN (SELECT capacity_shard FROM candidates)
ORDER BY capacity.capacity_shard FOR UPDATE OF capacity SKIP LOCKED";
const CLEANUP_ADVANCE_TIME: &str = r"
WITH sample AS MATERIALIZED (
    SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp()) * 1000)::BIGINT AS now_ms
), clamped AS MATERIALIZED (
    SELECT greatest((SELECT now_ms FROM sample), coalesce(max(observed_at_ms), 0)) AS now_ms
    FROM runlimit_gcra_shards WHERE capacity_shard = ANY($1)
)
UPDATE runlimit_gcra_shards SET observed_at_ms = (SELECT now_ms FROM clamped)
WHERE capacity_shard = ANY($1)";
const CLEANUP_DELETE: &str = r"
WITH expired AS MATERIALIZED (
    SELECT candidate.config_fingerprint, candidate.subject_key
    FROM runlimit_gcra_shards AS shard
    CROSS JOIN LATERAL (
        SELECT config_fingerprint, subject_key, expires_at_ms FROM runlimit_gcra AS counter
        WHERE counter.capacity_shard = shard.capacity_shard
            AND counter.expires_at_ms <= shard.observed_at_ms
        ORDER BY counter.expires_at_ms LIMIT $2
    ) AS candidate
    WHERE shard.capacity_shard = ANY($1)
    ORDER BY candidate.expires_at_ms LIMIT $2
)
DELETE FROM runlimit_gcra AS stored USING expired
WHERE stored.config_fingerprint = expired.config_fingerprint AND stored.subject_key = expired.subject_key";

/// Replica-safe GCRA quotas using the same exact arithmetic as the memory store.
///
/// Each opaque policy/subject key stores a scaled theoretical arrival time.
/// Time comes from PostgreSQL and is clamped to the latest committed observation
/// in each affected shard. Full replenishment releases quota continuously; a
/// scheduled [`Self::cleanup_expired`] releases storage slots.
///
/// The independent GCRA v1 protocol locks its 256 persistent capacity shards in
/// ascending order before observing logical keys. The fingerprint/subject XOR
/// shard derivation, row-lock order, and 65,536-row hard ceiling must not change
/// during rolling deployment. Shards deliberately serialize their admission
/// transactions, including different keys in the same shard.
///
/// Commit errors have the existing [`CheckError::consumption`] semantics and
/// are never replayed. Cancellation may race with commit; dropping a future
/// returns no evidence of consumption and closes its connection. Prefer the
/// configured deadlines over a shorter outer timeout. Observer callbacks run
/// after releasing database connections.
#[derive(Clone)]
pub struct PostgresGcraLimiter {
    pool: PgPool,
    config: PostgresConfig,
    observer: Option<Arc<dyn Observer>>,
}

impl fmt::Debug for PostgresGcraLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresGcraLimiter")
            .field("pool", &self.pool)
            .field("config", &self.config)
            .field("has_observer", &self.observer.is_some())
            .finish()
    }
}

impl PostgresGcraLimiter {
    /// Creates a limiter with default operational bounds. Migrate before use.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            config: PostgresConfig::new(),
            observer: None,
        }
    }

    /// Uses explicit operational bounds without changing policy semantics.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.config = config;
        self
    }

    /// Installs a low-cardinality observer; callback panics are isolated.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Returns the underlying pool.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Returns the runtime configuration.
    pub const fn config(&self) -> PostgresConfig {
        self.config
    }

    /// Applies only the independent GCRA migrations.
    ///
    /// Every migrator sharing `SQLx` history must ignore unrelated versions.
    /// Cancellation/error closes the session rather than leaking migration locks.
    ///
    /// # Errors
    /// Returns `SQLx` migration or acquisition failures.
    pub async fn migrate(&self) -> Result<(), MigrateError> {
        let mut connection = ConnectionCancellationGuard::new(self.pool.acquire().await?);
        let mut migrator = sqlx::migrate!("./gcra-migrations");
        migrator.set_ignore_missing(true);
        let result = migrator
            .run_direct(None, &mut **connection.connection(), false)
            .await;
        if result.is_ok() {
            connection.reuse();
        }
        result
    }

    /// Checks and consumes one GCRA quota. Denials consume nothing.
    ///
    /// # Errors
    /// Returns classified pre-commit or uncertain-commit failures.
    pub async fn check(&self, check: &Check<'_, GcraPolicy>) -> Result<Decision, CheckError> {
        let started = WallInstant::now();
        let result = self.run(std::slice::from_ref(check), single_decision).await;
        if let Some(observer) = &self.observer {
            let observation = match &result {
                Ok(decision) => {
                    AdmissionObservation::from_check(check, decision, started.elapsed())
                }
                Err(error) => AdmissionObservation::failed_check(
                    check,
                    error.consumption(),
                    started.elapsed(),
                ),
            };
            observe_safely(observer.as_ref(), &Observation::Admission(observation));
        }
        result
    }

    /// Consumes a validated nonempty batch atomically, preserving input order.
    /// Shadow-denied batches consume no member. Mixed modes and duplicate keys
    /// are rejected before acquiring a connection.
    ///
    /// # Errors
    /// Returns structural batch failures or classified database failures.
    pub async fn check_all(
        &self,
        checks: &[Check<'_, GcraPolicy>],
    ) -> Result<BatchDecision, BatchCheckError> {
        let started = WallInstant::now();
        let result = match validate_batch(checks, self.config.max_batch_size()) {
            Ok(()) => self
                .run(checks, |admission| batch_decision(checks.len(), admission))
                .await
                .map_err(BatchCheckError::from),
            Err(error) => Err(error.into()),
        };
        if let Some(observer) = &self.observer {
            let observation = match &result {
                Ok(decision) => {
                    AdmissionObservation::from_batch(checks, decision, started.elapsed())
                }
                Err(error) => AdmissionObservation::failed_batch(
                    checks,
                    error.consumption(),
                    started.elapsed(),
                ),
            };
            observe_safely(observer.as_ref(), &Observation::Admission(observation));
        }
        result
    }

    async fn run<D>(
        &self,
        checks: &[Check<'_, GcraPolicy>],
        build: impl FnOnce(Admission) -> Result<D, CheckError>,
    ) -> Result<D, CheckError> {
        let connection =
            acquire_check_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        let mut guarded = ConnectionCancellationGuard::new(connection);
        let deadline = Instant::now() + self.config.operation_timeout();
        let result =
            run_transaction(guarded.connection(), checks, self.config, deadline, build).await;
        guarded.finish(match result {
            Ok(result) => result.map(Ok),
            Err(error) => error.map(Err),
        })
    }

    /// Removes at most `maximum_rows` expired counters and releases their slots.
    /// Busy shards are skipped. A zero limit performs no database work.
    ///
    /// # Errors
    /// Returns bounded maintenance failures; an uncertain commit may have deleted rows.
    pub async fn cleanup_expired(&self, maximum_rows: u32) -> Result<u64, MaintenanceError> {
        let started = WallInstant::now();
        let result = self.cleanup(maximum_rows).await;
        if let Some(observer) = &self.observer {
            let outcome = match &result {
                Ok(removed) => CleanupOutcome::Confirmed { removed: *removed },
                Err(error) if error.may_have_removed_rows() => CleanupOutcome::Unknown,
                Err(_) => CleanupOutcome::NoEffect,
            };
            observe_safely(
                observer.as_ref(),
                &Observation::Cleanup(CleanupObservation::new(
                    usize::try_from(maximum_rows).unwrap_or(usize::MAX),
                    outcome,
                    started.elapsed(),
                )),
            );
        }
        result
    }

    async fn cleanup(&self, maximum_rows: u32) -> Result<u64, MaintenanceError> {
        if maximum_rows == 0 {
            return Ok(0);
        }
        let connection =
            acquire_maintenance_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        let mut guarded = ConnectionCancellationGuard::new(connection);
        let deadline = Instant::now() + self.config.operation_timeout();
        let result = cleanup_transaction(guarded.connection(), maximum_rows, deadline).await;
        guarded.finish(match result {
            Ok(rows) => ConnectionOutcome::Reusable(Ok(rows)),
            Err(error) => error.map(Err),
        })
    }
}

impl Limiter for PostgresGcraLimiter {
    type Policy = GcraPolicy;
    type CheckError = CheckError;
    type CheckAllError = BatchCheckError;

    fn check(
        &self,
        check: &Check<'_, GcraPolicy>,
    ) -> impl Future<Output = Result<Decision, CheckError>> + Send {
        Self::check(self, check)
    }

    fn check_all(
        &self,
        checks: &[Check<'_, GcraPolicy>],
    ) -> impl Future<Output = Result<BatchDecision, BatchCheckError>> + Send {
        Self::check_all(self, checks)
    }
}

type Failure = ConnectionOutcome<CheckError>;

fn invariant(detail: &'static str) -> Failure {
    ConnectionOutcome::Reusable(CheckError::storage_invariant(detail))
}

fn field<'r, T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>>(
    row: &'r PgRow,
    name: &str,
) -> Result<T, Failure> {
    row.try_get(name).map_err(|error| {
        ConnectionOutcome::Reusable(CheckError::storage_decode_invariant(
            "invalid GCRA database response",
            error,
        ))
    })
}

async fn run_transaction<D>(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    checks: &[Check<'_, GcraPolicy>],
    config: PostgresConfig,
    deadline: Instant,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> Result<ConnectionOutcome<D>, Failure> {
    let mut transaction = CheckTransaction::begin(connection, deadline).await?;
    let mut shards = checks
        .iter()
        .map(|check| capacity_shard(check.counter_key()))
        .collect::<Vec<_>>();
    shards.sort_unstable();
    shards.dedup();
    let locked = transaction
        .fetch_all(
            CheckPhase::AcquiringCapacityShardLocks,
            sqlx::query(LOCK_SHARDS).bind(&shards),
        )
        .await?;
    if locked.len() != shards.len() {
        return Err(invariant("GCRA capacity ledger is missing a shard"));
    }
    let mut counts = [0_i64; CAPACITY_SHARD_COUNT];
    let sample = transaction
        .fetch_one(CheckPhase::PreflightingCounters, sqlx::query(SAMPLE_TIME))
        .await?;
    let mut now: i64 = field(&sample, "now_ms")?;
    for (row, expected) in locked.iter().zip(&shards) {
        let shard: i16 = field(row, "capacity_shard")?;
        let count: i64 = field(row, "row_count")?;
        let observed: i64 = field(row, "observed_at_ms")?;
        if shard != *expected
            || !(0..=i64::from(crate::HARD_MAX_ROWS_PER_SHARD)).contains(&count)
            || observed < 0
        {
            return Err(invariant("invalid GCRA capacity ledger"));
        }
        counts[usize::try_from(shard).expect("shard came from validated key")] = count;
        now = now.max(observed);
    }
    let now_u128 = u128::try_from(now).map_err(|_| invariant("negative GCRA database time"))?;
    let fingerprints = checks
        .iter()
        .map(|check| check.counter_key().fingerprint().as_bytes().to_vec())
        .collect::<Vec<_>>();
    let subjects = checks
        .iter()
        .map(|check| check.subject().as_bytes().to_vec())
        .collect::<Vec<_>>();
    let rows = transaction
        .fetch_all(
            CheckPhase::PreflightingCounters,
            sqlx::query(READ_COUNTERS)
                .bind(&fingerprints)
                .bind(&subjects),
        )
        .await?;
    let pending = evaluate(
        checks,
        &rows,
        &mut counts,
        config.maximum_rows_per_shard(),
        now_u128,
    )?;
    match pending {
        Pending::Denied(admission) => {
            let decision = build(admission).map_err(ConnectionOutcome::Reusable)?;
            Ok(transaction.deny(decision).await)
        }
        Pending::Allowed(allowances) => {
            let tats = allowances
                .iter()
                .map(|allowance| allowance.tat_scaled.to_string())
                .collect::<Vec<_>>();
            let expiries = allowances
                .iter()
                .map(|allowance| {
                    i64::try_from(allowance.expires_at_millis)
                        .map_err(|_| invariant("GCRA expiry exceeds database range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let decision = build(Admission::Allowed(
                allowances.into_iter().map(|item| item.allowance).collect(),
            ))
            .map_err(ConnectionOutcome::Reusable)?;
            let updated = transaction
                .execute(
                    CheckPhase::UpdatingCounters,
                    sqlx::query(UPSERT)
                        .bind(&fingerprints)
                        .bind(&subjects)
                        .bind(tats)
                        .bind(expiries),
                )
                .await?;
            if updated.rows_affected() != checks.len() as u64 {
                return Err(invariant("GCRA upsert count disagrees with the batch"));
            }
            transaction.execute(CheckPhase::UpdatingCounters, sqlx::query("UPDATE runlimit_gcra_shards SET observed_at_ms = $2 WHERE capacity_shard = ANY($1)").bind(&shards).bind(now)).await?;
            transaction.commit().await?;
            Ok(ConnectionOutcome::Reusable(decision))
        }
    }
}

enum Pending {
    Allowed(Vec<gcra::PendingAllowance>),
    Denied(Admission),
}

fn evaluate(
    checks: &[Check<'_, GcraPolicy>],
    rows: &[PgRow],
    counts: &mut [i64; CAPACITY_SHARD_COUNT],
    maximum: u32,
    now: u128,
) -> Result<Pending, Failure> {
    if rows.len() != checks.len() {
        return Err(invariant("GCRA preflight count disagrees with the batch"));
    }
    let mut pending = Vec::with_capacity(checks.len());
    let mut denial = None;
    for (index, (check, row)) in checks.iter().zip(rows).enumerate() {
        let returned_index: i64 = field(row, "input_index")?;
        if usize::try_from(returned_index).ok() != Some(index) {
            return Err(invariant("GCRA preflight changed input order"));
        }
        let tat: Option<String> = field(row, "tat_scaled")?;
        let expires: Option<i64> = field(row, "expires_at_ms")?;
        let stored = match (tat, expires) {
            (None, None) => None,
            (Some(tat), Some(expires)) if expires >= 0 => Some((
                tat.parse::<u128>()
                    .map_err(|_| invariant("invalid GCRA theoretical arrival time"))?,
                u128::try_from(expires).expect("nonnegative expiry"),
            )),
            _ => return Err(invariant("inconsistent GCRA stored counter")),
        };
        if let Some((tat, expires)) = stored
            && tat.div_ceil(u128::from(check.policy().quota().get())) != expires
        {
            return Err(invariant(
                "GCRA expiry disagrees with theoretical arrival time",
            ));
        }
        let active_tat = stored
            .filter(|(_, expires)| *expires > now)
            .map(|(tat, _)| tat);
        let evaluated = gcra::evaluate(check, now, active_tat)
            .map_err(|_| invariant("GCRA arithmetic exceeded exact range"))?;
        match evaluated {
            gcra::Evaluation::Denied(quota) => {
                denial.get_or_insert_with(|| {
                    if check.policy().quota_mode() == QuotaMode::Shadow {
                        Admission::ShadowDenied {
                            index,
                            denial: quota,
                        }
                    } else {
                        Admission::Denied {
                            index,
                            denial: quota.into(),
                        }
                    }
                });
            }
            gcra::Evaluation::Allowed(allowance) => {
                if stored.is_none() {
                    let shard = usize::try_from(capacity_shard(check.counter_key()))
                        .expect("stable shard fits usize");
                    if counts[shard] >= i64::from(maximum) {
                        denial.get_or_insert(Admission::Denied {
                            index,
                            denial: Denial::StorageCapacity { retry_after: None },
                        });
                    }
                    counts[shard] += 1;
                }
                pending.push(allowance);
            }
        }
    }
    Ok(match denial {
        Some(denial) => Pending::Denied(denial),
        None => Pending::Allowed(pending),
    })
}

async fn cleanup_transaction(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    maximum: u32,
    deadline: Instant,
) -> Result<u64, ConnectionOutcome<MaintenanceError>> {
    let mut transaction = maintenance_before_commit(
        deadline,
        CleanupPhase::BeginningTransaction,
        connection.begin(),
    )
    .await?;
    set_maintenance_server_timeouts(
        &mut transaction,
        deadline,
        CleanupPhase::ConfiguringTimeouts,
    )
    .await?;
    let rows = maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query(CLEANUP_LOCKS).fetch_all(&mut *transaction),
    )
    .await?;
    let shards = rows
        .iter()
        .map(|row| row.try_get::<i16, _>("capacity_shard"))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ConnectionOutcome::Reusable(MaintenanceError::Database(error)))?;
    set_maintenance_server_timeouts(
        &mut transaction,
        deadline,
        CleanupPhase::ConfiguringTimeouts,
    )
    .await?;
    // Cleanup forgets theoretical arrival times, so its clock observation must
    // survive the deleted rows. Keep that observation monotonic under the same
    // shard locks as admission, and persist it in the deletion transaction.
    // Use a separate statement: the deletion triggers also update ledger rows,
    // and PostgreSQL does not support updating a row twice in one command.
    maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query(CLEANUP_ADVANCE_TIME)
            .bind(&shards)
            .execute(&mut *transaction),
    )
    .await?;
    set_maintenance_server_timeouts(
        &mut transaction,
        deadline,
        CleanupPhase::ConfiguringTimeouts,
    )
    .await?;
    let deleted = maintenance_before_commit(
        deadline,
        CleanupPhase::DeletingExpiredWindows,
        sqlx::query(CLEANUP_DELETE)
            .bind(shards)
            .bind(i64::from(maximum))
            .execute(&mut *transaction),
    )
    .await?;
    commit_maintenance(deadline, transaction).await?;
    Ok(deleted.rows_affected())
}
