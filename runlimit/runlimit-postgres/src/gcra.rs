//! Opt-in durable GCRA storage; independent from the fixed-window protocol.

use std::{fmt, future::Future, sync::Arc};

use runlimit_core::{BatchDecision, Check, Decision, GcraPolicy, Limiter, Observer, gcra};
use sqlx::{
    PgPool, Row,
    migrate::{MigrateError, Migrator},
    postgres::PgRow,
    types::chrono::{DateTime, Utc},
};

use crate::{
    BatchCheckError, CheckError, CheckPhase, CleanupPhase, MaintenanceError, PostgresConfig,
    admission::{Admission, batch_decision, finalize, single_decision},
    base::{BundledHistory, PgLimiterBase},
    pending::{Pending, PendingDenial, PendingQuotaDenial, authoritative_elapsed},
    protocol::{
        CAPACITY_SHARD_COUNT, HARD_MAX_ROWS_PER_SHARD, capacity_shard, shard_admits_new_row,
    },
    statements::gcra as sql,
    transaction::{self, BoundedConnection, CheckTransaction, ConnectionOutcome},
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
#[expect(
    clippy::disallowed_types,
    reason = "sqlx::migrate! embeds each migration as a sqlx::SqlStr"
)]
pub static GCRA_MIGRATOR: Migrator = sqlx::migrate!("./gcra-migrations");

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
    base: PgLimiterBase<dyn Observer>,
}

impl fmt::Debug for PostgresGcraLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.base.fmt_debug("PostgresGcraLimiter", formatter)
    }
}

impl PostgresGcraLimiter {
    /// Creates a limiter with default operational bounds. Migrate before use.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            base: PgLimiterBase::new(pool),
        }
    }

    /// Uses explicit operational bounds without changing policy semantics.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.base.set_config(config);
        self
    }

    /// Installs a low-cardinality observer; callback panics are isolated.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.base.set_observer(observer);
        self
    }

    /// Returns the underlying pool.
    pub const fn pool(&self) -> &PgPool {
        self.base.pool()
    }

    /// Returns the runtime configuration.
    pub const fn config(&self) -> PostgresConfig {
        self.base.config()
    }

    /// Applies only the independent GCRA migrations.
    ///
    /// Every migrator sharing `SQLx` history must ignore unrelated versions.
    /// The connection returns to the pool only after a successful run and
    /// unlock; cancellation or an error closes the session rather than leaking
    /// migration locks.
    ///
    /// # Errors
    /// Returns `SQLx` migration or acquisition failures.
    pub async fn migrate(&self) -> Result<(), MigrateError> {
        self.base.migrate(BundledHistory::Gcra).await
    }

    /// Checks and consumes one GCRA quota. Denials consume nothing.
    ///
    /// # Errors
    /// Returns classified pre-commit or uncertain-commit failures.
    pub async fn check(&self, check: &Check<'_, GcraPolicy>) -> Result<Decision, CheckError> {
        self.base
            .observed_check(check, || {
                self.run(std::slice::from_ref(check), single_decision)
            })
            .await
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
        self.base
            .observed_batch(checks, || {
                self.run(checks, |admission| batch_decision(checks.len(), admission))
            })
            .await
    }

    async fn run<D>(
        &self,
        checks: &[Check<'_, GcraPolicy>],
        build: impl FnOnce(Admission) -> Result<D, CheckError>,
    ) -> Result<D, CheckError> {
        let mut connection = self.base.check_connection().await?;
        let result = run_transaction(&mut connection, checks, self.base.config(), build).await;
        connection.finish(result)
    }

    /// Removes at most `maximum_rows` expired counters and releases their slots.
    /// Busy shards are skipped. A zero limit performs no database work.
    ///
    /// # Errors
    /// Returns bounded maintenance failures; an uncertain commit may have deleted rows.
    pub async fn cleanup_expired(&self, maximum_rows: u32) -> Result<u64, MaintenanceError> {
        self.base
            .observed_cleanup(maximum_rows, cleanup_transaction)
            .await
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
    connection: &mut BoundedConnection<transaction::Check>,
    checks: &[Check<'_, GcraPolicy>],
    config: PostgresConfig,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> Result<ConnectionOutcome<D>, Failure> {
    let mut transaction = connection.begin().await?;
    let mut lock_order = checks
        .iter()
        .map(|check| capacity_shard(check.counter_key()))
        .collect::<Vec<_>>();
    lock_order.sort_unstable();
    lock_order.dedup();
    let shards = lock_order
        .iter()
        .copied()
        .map(i16::from)
        .collect::<Vec<_>>();
    let locked = transaction
        .fetch_all(
            CheckPhase::AcquiringCapacityShardLocks,
            sql::LOCK_SHARDS.query().bind(&shards),
        )
        .await?;
    if locked.len() != shards.len() {
        return Err(invariant("GCRA capacity ledger is missing a shard"));
    }
    let mut stored_rows = [0_u64; CAPACITY_SHARD_COUNT];
    let sample = transaction
        .fetch_one(CheckPhase::PreflightingCounters, sql::SAMPLE_TIME.query())
        .await?;
    let mut now: i64 = field(&sample, "now_ms")?;
    // The watermark clamps evaluation only; elapsed time uses the real sample.
    let database_now: DateTime<Utc> = field(&sample, "database_now")?;
    for (row, &expected) in locked.iter().zip(&lock_order) {
        let shard: i16 = field(row, "capacity_shard")?;
        let count: i64 = field(row, "row_count")?;
        let observed: i64 = field(row, "observed_at_ms")?;
        let count = u64::try_from(count)
            .ok()
            .filter(|count| *count <= u64::from(HARD_MAX_ROWS_PER_SHARD));
        match count {
            Some(count) if shard == i16::from(expected) && observed >= 0 => {
                stored_rows[usize::from(expected)] = count;
            }
            _ => return Err(invariant("invalid GCRA capacity ledger")),
        }
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
            sql::READ_COUNTERS
                .query()
                .bind(&fingerprints)
                .bind(&subjects),
        )
        .await?;
    let pending = evaluate(
        checks,
        &rows,
        &stored_rows,
        config.maximum_rows_per_shard(),
        now_u128,
    )?;
    // Elapsed time runs through the path's last statement: the preflight for a
    // denial, the watermark update for an allowance.
    let response_now = match &pending {
        Pending::Denied { .. } => field(
            rows.last()
                .ok_or_else(|| invariant("missing GCRA response time"))?,
            "response_now",
        )?,
        Pending::Allowed(allowances) => {
            persist_allowances(
                &mut transaction,
                &fingerprints,
                &subjects,
                allowances,
                &shards,
                now,
            )
            .await?
        }
    };
    finalize(
        transaction,
        pending,
        authoritative_elapsed(database_now, response_now),
        build,
    )
    .await
}

async fn persist_allowances(
    transaction: &mut CheckTransaction<'_>,
    fingerprints: &[Vec<u8>],
    subjects: &[Vec<u8>],
    allowances: &[gcra::PendingAllowance],
    shards: &[i16],
    now: i64,
) -> Result<DateTime<Utc>, Failure> {
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
    let updated = transaction
        .execute(
            CheckPhase::UpdatingCounters,
            sql::UPSERT
                .query()
                .bind(fingerprints)
                .bind(subjects)
                .bind(tats)
                .bind(expiries),
        )
        .await?;
    if updated.rows_affected() != allowances.len() as u64 {
        return Err(invariant("GCRA upsert count disagrees with the batch"));
    }
    let response = transaction
        .fetch_one(
            CheckPhase::UpdatingCounters,
            sql::UPDATE_WATERMARKS.query().bind(shards).bind(now),
        )
        .await?;
    let updated_count: i64 = field(&response, "updated_count")?;
    if usize::try_from(updated_count).ok() != Some(shards.len()) {
        return Err(invariant("GCRA watermark count disagrees with the shards"));
    }
    field(&response, "response_now")
}

fn evaluate(
    checks: &[Check<'_, GcraPolicy>],
    rows: &[PgRow],
    stored_rows: &[u64; CAPACITY_SHARD_COUNT],
    maximum: u32,
    now: u128,
) -> Result<Pending<gcra::PendingAllowance>, Failure> {
    if rows.len() != checks.len() {
        return Err(invariant("GCRA preflight count disagrees with the batch"));
    }
    let mut pending_rows = [0_u32; CAPACITY_SHARD_COUNT];
    let mut pending = Vec::with_capacity(checks.len());
    let mut denial = None;
    for (index, (check, row)) in checks.iter().zip(rows).enumerate() {
        let returned_index: i64 = field(row, "input_index")?;
        if usize::try_from(returned_index).ok() != Some(index) {
            return Err(invariant("GCRA preflight changed input order"));
        }
        let tat: Option<String> = field(row, "tat_scaled")?;
        let expires: Option<i64> = field(row, "expires_at_ms")?;
        let stored = match (tat, expires.map(u64::try_from)) {
            (None, None) => None,
            (Some(tat), Some(Ok(expires))) => Some((
                tat.parse::<u128>()
                    .map_err(|_| invariant("invalid GCRA theoretical arrival time"))?,
                u128::from(expires),
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
                denial.get_or_insert((
                    index,
                    PendingDenial::Quota(PendingQuotaDenial {
                        limit: quota.capacity(),
                        retry_from_sample: quota.retry_after().duration(),
                    }),
                ));
            }
            gcra::Evaluation::Allowed(allowance) => {
                if stored.is_none() {
                    let shard = usize::from(capacity_shard(check.counter_key()));
                    if !shard_admits_new_row(stored_rows[shard], pending_rows[shard], maximum) {
                        denial.get_or_insert((index, PendingDenial::StorageCapacity));
                    }
                    pending_rows[shard] += 1;
                }
                pending.push(allowance);
            }
        }
    }
    Ok(match denial {
        Some((index, denial)) => Pending::Denied {
            index,
            denial,
            mode: checks[index].policy().quota_mode(),
        },
        None => Pending::Allowed(pending),
    })
}

async fn cleanup_transaction(
    connection: &mut BoundedConnection<transaction::Maintenance>,
    maximum: u32,
) -> Result<u64, ConnectionOutcome<MaintenanceError>> {
    let mut transaction = connection.begin().await?;
    let rows = transaction
        .fetch_all(
            CleanupPhase::DeletingExpiredWindows,
            sql::CLEANUP_LOCKS.query(),
        )
        .await?;
    let shards = rows
        .iter()
        .map(|row| row.try_get::<i16, _>("capacity_shard"))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ConnectionOutcome::Reusable(MaintenanceError::Database(error)))?;
    // Cleanup forgets theoretical arrival times, so its clock observation must
    // survive the deleted rows. Keep that observation monotonic under the same
    // shard locks as admission, and persist it in the deletion transaction.
    // Use a separate statement: the deletion triggers also update ledger rows,
    // and PostgreSQL does not support updating a row twice in one command.
    transaction
        .execute(
            CleanupPhase::DeletingExpiredWindows,
            sql::CLEANUP_ADVANCE_TIME.query().bind(&shards),
        )
        .await?;
    let deleted = transaction
        .execute(
            CleanupPhase::DeletingExpiredWindows,
            sql::CLEANUP_DELETE
                .query()
                .bind(&shards)
                .bind(i64::from(maximum)),
        )
        .await?;
    transaction.commit().await?;
    Ok(deleted.rows_affected())
}
