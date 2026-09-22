//! Durable outcome-aware attempts. Exactly one verification lease may be active
//! per policy/subject; crashed or abandoned attempts advance failure state.
//! Standalone calls own bounded transactions. Host transactions use the explicit
//! [`low_level`] seam and must publish results only after acknowledged commit.
use crate::{
    CheckError, CheckPhase, ConnectionCancellationGuard, ConnectionOutcome, PostgresConfig,
    admission::{CheckTransaction, acquire_check_connection},
};
use runlimit_core::{
    Delay,
    attempts::{
        AttemptAdmission, AttemptCompletion, AttemptCompletionResult, AttemptDenial,
        AttemptObservation, AttemptObserver, AttemptOutcome, AttemptPolicy, AttemptSubject,
        StagedAttemptCompletion, observe_attempt_safely,
    },
};
use sqlx::{
    PgPool, Postgres, Row,
    migrate::{MigrateError, Migrator},
    postgres::{PgArguments, PgRow},
    query::Query,
};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

/// Isolated migration history; install only when opting into attempt policies.
pub static ATTEMPTS_MIGRATOR: Migrator = sqlx::migrate!("./attempts-migrations");
/// SQL suitable for a new forward-only application-owned migration.
pub const CREATE_RUNLIMIT_ATTEMPTS_SQL: &str =
    include_str!("../attempts-migrations/20260921000001_create_runlimit_attempts.sql");
const COMPLETE_SQL: &str = include_str!("attempt_complete.sql");
const CLEANUP_SQL: &str = include_str!("attempt_cleanup.sql");
const PHASE: CheckPhase = CheckPhase::AcquiringCounterRowLocks;
// Stable protocol: a distinct two-integer namespace and 256 capacity shards.
const LOCK_NAMESPACE: i32 = 0x524c_4154;

/// Consuming opaque lease; no clone, serialization, public constructor, or raw token access.
pub struct PgAttemptReceipt {
    policy: AttemptPolicy,
    subject: [u8; 32],
    token: String,
}
/// A live receipt fenced to the exact PostgreSQL transaction that locked it.
/// Completion may outlive its admission lease while the transaction retains
/// the row lock. Claiming transactionally rotates its private token, so rolling
/// back a containing savepoint invalidates the escaped claim even when the
/// outer transaction ID stays the same. The original reservation then expires.
pub struct PgAttemptClaim {
    receipt: PgAttemptReceipt,
    transaction_id: String,
}
impl std::fmt::Debug for PgAttemptClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgAttemptClaim([REDACTED])")
    }
}
/// A claim must succeed before its owner executes application writes.
#[derive(Debug)]
pub enum PgAttemptClaimResult {
    /// This transaction holds the live reservation's row lock.
    Claimed(PgAttemptClaim),
    /// No live reservation matched; rollback without running application work.
    Stale,
}
impl std::fmt::Debug for PgAttemptReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PgAttemptReceipt([REDACTED])")
    }
}

/// Replica-safe attempt limiter with bounded acquisition and transaction budgets.
#[derive(Clone)]
pub struct PostgresAttemptLimiter {
    pool: PgPool,
    config: PostgresConfig,
    observer: Option<Arc<dyn AttemptObserver>>,
}
impl PostgresAttemptLimiter {
    /// Creates a limiter; install its isolated migration before serving requests.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            config: PostgresConfig::new(),
            observer: None,
        }
    }
    /// Sets timeouts and the operational per-shard row bound.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.config = config;
        self
    }
    /// Installs a panic-isolated lifecycle observer.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn AttemptObserver>) -> Self {
        self.observer = Some(observer);
        self
    }
    /// Pool that transaction-owning application adapters must use for completion.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
    /// Installs attempt tables. Shared application migrators must also ignore
    /// missing migrations, or vendor [`CREATE_RUNLIMIT_ATTEMPTS_SQL`] instead.
    ///
    /// # Errors
    /// Returns database or migration errors. The migration connection is always
    /// retired so cancellation cannot leak migration advisory locks to the pool.
    pub async fn migrate(&self) -> Result<(), MigrateError> {
        let mut connection = self.pool.acquire().await?;
        connection.close_on_drop();
        let mut migrator = Migrator {
            migrations: ATTEMPTS_MIGRATOR.migrations.clone(),
            ..Migrator::DEFAULT
        };
        migrator.set_ignore_missing(true);
        migrator.run(&mut *connection).await
    }
    fn observe(&self, event: AttemptObservation) {
        if let Some(observer) = &self.observer {
            observe_attempt_safely(observer.as_ref(), event);
        }
    }
    /// Admits one bounded verification lease, using database time after locks.
    /// An unpolled future performs no work. Dropping a receipt leaves its lease
    /// to expire as a failure; it never refunds or resets the subject.
    /// A capacity denial may still commit cleanup of at most 16 already-expired
    /// rows, allowing repeated calls to converge after an operational cap is
    /// lowered. It creates no reservation or failure transition. That denial
    /// remains authoritative even when cleanup's commit acknowledgement is lost;
    /// the uncertain connection is retired and a later call can repeat cleanup.
    ///
    /// # Errors
    /// [`CheckError`] distinguishes no committed effect from uncertain commit.
    /// Never automatically retry an uncertain admission.
    pub async fn admit(
        &self,
        subject: AttemptSubject<'_>,
    ) -> Result<AttemptAdmission<PgAttemptReceipt>, CheckError> {
        let connection =
            acquire_check_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        let mut guard = ConnectionCancellationGuard::new(connection);
        let deadline = Instant::now() + self.config.operation_timeout();
        let result = admit_transaction(
            guard.connection(),
            subject,
            self.config.maximum_rows_per_shard(),
            deadline,
        )
        .await;
        let result = guard.finish(match result {
            Ok(value) => value.map(Ok),
            Err(error) => error.map(Err),
        });
        match &result {
            Ok(AttemptAdmission::Admitted(_)) => self.observe(AttemptObservation::Admitted),
            Ok(AttemptAdmission::Denied(denial)) => {
                self.observe(AttemptObservation::Denied(*denial));
            }
            Err(error)
                if error.consumption() == runlimit_core::ConsumptionStatus::PossiblyConsumed =>
            {
                self.observe(AttemptObservation::CommitUncertain);
            }
            Err(_) => {}
        }
        result
    }
    /// Completes a receipt in an owned transaction, returning acknowledged state.
    /// The distinct result from [`low_level::complete_in`] remains staged.
    /// A stale receipt never resets newer failure state.
    ///
    /// # Errors
    /// Returns bounded database failures, explicitly distinguishing uncertain commit.
    pub async fn complete(
        &self,
        receipt: PgAttemptReceipt,
        outcome: AttemptOutcome,
    ) -> Result<AttemptCompletionResult, CheckError> {
        let connection =
            acquire_check_connection(&self.pool, self.config.pool_acquire_timeout()).await?;
        let mut guard = ConnectionCancellationGuard::new(connection);
        let deadline = Instant::now() + self.config.operation_timeout();
        let result = async {
            let mut transaction = CheckTransaction::begin(guard.connection(), deadline).await?;
            let rows = transaction
                .fetch_all(PHASE, completion_query(&receipt, outcome, None))
                .await?;
            let result = completion_result(&rows, outcome).map_err(ConnectionOutcome::Reusable)?;
            transaction.commit().await?;
            Ok::<_, ConnectionOutcome<CheckError>>(match result {
                StagedAttemptCompletion::Applied(done) => AttemptCompletionResult::Applied(done),
                StagedAttemptCompletion::Stale => AttemptCompletionResult::Stale,
            })
        }
        .await;
        let result = guard.finish(match result {
            Ok(value) => ConnectionOutcome::Reusable(Ok(value)),
            Err(error) => error.map(Err),
        });
        match &result {
            Ok(AttemptCompletionResult::Applied(completion)) => {
                self.observe(AttemptObservation::Completed(*completion));
            }
            Ok(AttemptCompletionResult::Stale) => self.observe(AttemptObservation::Stale),
            Err(error)
                if error.consumption() == runlimit_core::ConsumptionStatus::PossiblyConsumed =>
            {
                self.observe(AttemptObservation::CommitUncertain);
            }
            Err(_) => {}
        }
        result
    }
}

async fn admit_transaction(
    connection: &mut sqlx::pool::PoolConnection<Postgres>,
    subject: AttemptSubject<'_>,
    maximum: u32,
    deadline: Instant,
) -> Result<ConnectionOutcome<AttemptAdmission<PgAttemptReceipt>>, ConnectionOutcome<CheckError>> {
    let policy = subject.policy().clone();
    let fingerprint = policy.fingerprint().into_bytes();
    let subject = subject.into_unbound_subject_key().into_bytes();
    let shard = i16::from(fingerprint[0] ^ subject[0]);
    let mut tx = CheckTransaction::begin(connection, deadline).await?;
    tx.execute(
        PHASE,
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1, $2)")
            .bind(LOCK_NAMESPACE)
            .bind(i32::from(shard)),
    )
    .await?;
    tx.execute(PHASE, sqlx::query(CLEANUP_SQL).bind(shard))
        .await?;
    let rows = tx.fetch_all(PHASE, sqlx::query("SELECT failures,last_failure_ms,retry_at_ms,lease_until_ms FROM runlimit_attempts WHERE config_fingerprint=$1 AND subject_key=$2 FOR UPDATE").bind(fingerprint.as_slice()).bind(subject.as_slice())).await?;
    let time = tx.fetch_one(PHASE, sqlx::query("SELECT floor(extract(epoch FROM pg_catalog.clock_timestamp())*1000)::bigint AS now_ms")).await?;
    let decode = |error| {
        ConnectionOutcome::Reusable(CheckError::storage_decode_invariant(
            "decode attempt admission response",
            error,
        ))
    };
    let now: i64 = time.try_get("now_ms").map_err(decode)?;
    let mut failures = 0_u32;
    let mut last_failure = now;
    if let Some(row) = rows.first() {
        failures =
            u32::try_from(row.try_get::<i64, _>("failures").map_err(decode)?).map_err(|_| {
                ConnectionOutcome::Reusable(CheckError::storage_invariant(
                    "invalid attempt failure count",
                ))
            })?;
        last_failure = row.try_get("last_failure_ms").map_err(decode)?;
        let mut retry: i64 = row.try_get("retry_at_ms").map_err(decode)?;
        let lease: Option<i64> = row.try_get("lease_until_ms").map_err(decode)?;
        if let Some(until) = lease {
            if now < until {
                return finish_denial(
                    tx,
                    AttemptDenial::Busy {
                        retry_after: delay(until - now),
                    },
                )
                .await;
            }
            failures = failures.saturating_add(1);
            last_failure = until;
            retry = until.saturating_add(millis(policy.delay(failures)));
        }
        if now >= last_failure.saturating_add(integer(policy.quiet_period().millis())) {
            failures = 0;
            retry = now;
        }
        if now < retry {
            return finish_denial(
                tx,
                AttemptDenial::Backoff {
                    retry_after: delay(retry - now),
                },
            )
            .await;
        }
    }
    let token_row = if rows.is_empty() {
        // A lower runtime bound counts every retained row, including slots
        // allocated by replicas using a higher bound. A hole in the new slot
        // interval is not proof that total shard occupancy is below the cap.
        let occupancy = tx
            .fetch_one(
                PHASE,
                sqlx::query(
                    "SELECT count(*) AS row_count FROM runlimit_attempts WHERE capacity_shard=$1",
                )
                .bind(shard),
            )
            .await?;
        let row_count: i64 = occupancy.try_get("row_count").map_err(decode)?;
        if row_count >= i64::from(maximum) {
            return Ok(finish_capacity_denial(tx).await);
        }
        let slot = tx.fetch_one(PHASE, sqlx::query("SELECT min(slot)::integer AS slot FROM generate_series(0, $2::integer-1) AS slot WHERE NOT EXISTS (SELECT 1 FROM runlimit_attempts WHERE capacity_shard=$1 AND capacity_slot=slot)").bind(shard).bind(i64::from(maximum))).await?;
        let Some(slot) = slot.try_get::<Option<i32>, _>("slot").map_err(decode)? else {
            return Ok(finish_capacity_denial(tx).await);
        };
        tx.fetch_one(PHASE, sqlx::query("INSERT INTO runlimit_attempts (config_fingerprint,subject_key,capacity_slot,failures,last_failure_ms,retry_at_ms,quiet_ms,lease_until_ms,lease_token) VALUES ($1,$2,$3,0,$4,$4,$5,$6,pg_catalog.gen_random_uuid()::text) RETURNING lease_token").bind(fingerprint.as_slice()).bind(subject.as_slice()).bind(slot).bind(now).bind(integer(policy.quiet_period().millis())).bind(now.saturating_add(integer(policy.lease().millis())))).await?
    } else {
        tx.fetch_one(PHASE, sqlx::query("UPDATE runlimit_attempts SET failures=$3,last_failure_ms=$4,retry_at_ms=$5,lease_until_ms=$6,lease_token=pg_catalog.gen_random_uuid()::text WHERE config_fingerprint=$1 AND subject_key=$2 RETURNING lease_token").bind(fingerprint.as_slice()).bind(subject.as_slice()).bind(i64::from(failures)).bind(last_failure).bind(now).bind(now.saturating_add(integer(policy.lease().millis())))).await?
    };
    let token = token_row.try_get("lease_token").map_err(decode)?;
    tx.commit().await?;
    Ok(ConnectionOutcome::Reusable(AttemptAdmission::Admitted(
        PgAttemptReceipt {
            policy,
            subject,
            token,
        },
    )))
}
async fn finish_denial(
    tx: CheckTransaction<'_>,
    denial: AttemptDenial,
) -> Result<ConnectionOutcome<AttemptAdmission<PgAttemptReceipt>>, ConnectionOutcome<CheckError>> {
    // Cleanup may have removed expired rows but denial owns no reservation.
    // A successful rollback restores cleanup too. A rollback failure must close
    // the connection before reporting the otherwise valid denial.
    Ok(tx.deny(AttemptAdmission::Denied(denial)).await)
}
async fn finish_capacity_denial(
    tx: CheckTransaction<'_>,
) -> ConnectionOutcome<AttemptAdmission<PgAttemptReceipt>> {
    // No reservation was inserted. Preserve bounded expired-row cleanup even
    // when a lowered cap still denies this call; otherwise rolling back every
    // batch can make an over-cap shard permanently unreclaimable. The admission
    // denial remains certain even if cleanup's commit acknowledgement is lost.
    let denial = AttemptAdmission::Denied(AttemptDenial::StorageCapacity);
    match tx.commit().await {
        Ok(()) => ConnectionOutcome::Reusable(denial),
        Err(outcome) => outcome.map(|_| denial),
    }
}
fn integer(value: u64) -> i64 {
    i64::try_from(value).expect("validated policy duration fits i64")
}
fn millis(value: Duration) -> i64 {
    i64::try_from(value.as_millis()).expect("validated delay fits i64")
}
fn delay(value: i64) -> Delay {
    Delay::new(Duration::from_millis(value.unsigned_abs()))
}
fn completion_query<'a>(
    receipt: &'a PgAttemptReceipt,
    outcome: AttemptOutcome,
    transaction_id: Option<&'a str>,
) -> Query<'a, Postgres, PgArguments> {
    sqlx::query(COMPLETE_SQL)
        .bind(receipt.policy.fingerprint().into_bytes().to_vec())
        .bind(receipt.subject.to_vec())
        .bind(&receipt.token)
        .bind(outcome == AttemptOutcome::Success)
        .bind(integer(receipt.policy.initial_delay().millis()))
        .bind(integer(receipt.policy.maximum_delay().millis()))
        .bind(transaction_id)
}
fn completion_result(
    rows: &[PgRow],
    outcome: AttemptOutcome,
) -> Result<StagedAttemptCompletion, CheckError> {
    let Some(row) = rows.first() else {
        return Ok(StagedAttemptCompletion::Stale);
    };
    let decode =
        |error| CheckError::storage_decode_invariant("decode attempt completion response", error);
    let failures = u32::try_from(row.try_get::<i64, _>("failures").map_err(decode)?)
        .map_err(|_| CheckError::storage_invariant("invalid attempt failure count"))?;
    let retry: i64 = row.try_get("delay_ms").map_err(decode)?;
    if retry < 0 {
        return Err(CheckError::storage_invariant("invalid attempt delay"));
    }
    AttemptCompletion::new(outcome, failures, delay(retry))
        .map(StagedAttemptCompletion::Applied)
        .map_err(|_| {
            CheckError::storage_invariant("attempt completion metadata contradicts its outcome")
        })
}

/// Explicit integration seam for a transaction owner such as Batter.
/// Calling this with an autocommit executor breaks application atomicity.
/// The owner must reject stale completion, rollback on every error/cancellation,
/// and publish application output only after acknowledged commit. Claim before
/// application writes, then finish with their final authentication outcome in
/// the same transaction. No await follows
/// query decoding; transaction disposition belongs exclusively to the caller.
pub mod low_level {
    use super::{
        PgAttemptClaim, PgAttemptClaimResult, PgAttemptReceipt, completion_query, completion_result,
    };
    use runlimit_core::attempts::{AttemptOutcome, StagedAttemptCompletion};
    use sqlx::{Executor, Postgres, Row};
    /// Locks and fences a live reservation before application writes. The lease
    /// is validated after acquiring the lock, against authoritative server time.
    /// Hold the same transaction through [`finish_in`] and final commit. An
    /// autocommit executor produces a claim unusable in subsequent transactions.
    /// Rolling back the savepoint containing this operation invalidates the
    /// claim by reverting its private token rotation; releasing that savepoint
    /// retains the lock and the claim remains valid in the outer transaction.
    ///
    /// # Errors
    /// Returns database/decode failures; the transaction owner must rollback.
    pub async fn claim_in<'e, E: Executor<'e, Database = Postgres>>(
        executor: E,
        mut receipt: PgAttemptReceipt,
    ) -> Result<PgAttemptClaimResult, sqlx::Error> {
        let row = sqlx::query("WITH locked AS MATERIALIZED (SELECT config_fingerprint,subject_key,lease_token,lease_until_ms FROM runlimit_attempts WHERE config_fingerprint=$1 AND subject_key=$2 FOR UPDATE), sampled AS MATERIALIZED (SELECT *, floor(extract(epoch FROM pg_catalog.clock_timestamp())*1000)::bigint AS now_ms FROM locked) UPDATE runlimit_attempts AS attempts SET lease_token=pg_catalog.gen_random_uuid()::text FROM sampled WHERE attempts.config_fingerprint=sampled.config_fingerprint AND attempts.subject_key=sampled.subject_key AND sampled.lease_token=$3 AND sampled.lease_until_ms>sampled.now_ms RETURNING attempts.lease_token,pg_catalog.pg_current_xact_id()::text AS transaction_id")
            .bind(receipt.policy.fingerprint().into_bytes().to_vec()).bind(receipt.subject.to_vec()).bind(&receipt.token).fetch_optional(executor).await?;
        let decode = |error| {
            sqlx::Error::Decode(Box::new(crate::CheckError::storage_decode_invariant(
                "decode attempt claim response",
                error,
            )))
        };
        match row {
            Some(row) => {
                receipt.token = row.try_get("lease_token").map_err(decode)?;
                Ok(PgAttemptClaimResult::Claimed(PgAttemptClaim {
                    receipt,
                    transaction_id: row.try_get("transaction_id").map_err(decode)?,
                }))
            }
            None => Ok(PgAttemptClaimResult::Stale),
        }
    }
    /// Stages the final outcome after application checks, in the claim's exact
    /// transaction. Time elapsed since claim does not invalidate its held lock.
    /// A different transaction returns `Stale` without changing state. The owner
    /// must rollback on `Stale` and publish only after acknowledged commit.
    ///
    /// # Errors
    /// Returns database/decode failures; the transaction owner must rollback.
    pub async fn finish_in<'e, E: Executor<'e, Database = Postgres>>(
        executor: E,
        claim: PgAttemptClaim,
        outcome: AttemptOutcome,
    ) -> Result<StagedAttemptCompletion, sqlx::Error> {
        let rows = completion_query(&claim.receipt, outcome, Some(&claim.transaction_id))
            .fetch_all(executor)
            .await?;
        completion_result(&rows, outcome).map_err(|error| sqlx::Error::Decode(Box::new(error)))
    }
    /// Stages one fenced completion in a caller-owned transaction, in one SQL statement.
    ///
    /// # Errors
    /// Returns database/decode errors. The transaction owner must rollback;
    /// an error does not authorize retrying or publishing application output.
    pub async fn complete_in<'e, E: Executor<'e, Database = Postgres>>(
        executor: E,
        receipt: PgAttemptReceipt,
        outcome: AttemptOutcome,
    ) -> Result<StagedAttemptCompletion, sqlx::Error> {
        let rows = completion_query(&receipt, outcome, None)
            .fetch_all(executor)
            .await?;
        completion_result(&rows, outcome).map_err(|error| sqlx::Error::Decode(Box::new(error)))
    }
}
