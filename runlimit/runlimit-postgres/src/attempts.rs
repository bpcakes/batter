//! Durable outcome-aware attempts. Exactly one verification lease may be active
//! per policy/subject; crashed or abandoned attempts advance failure state.
//! Standalone calls own bounded transactions. Host transactions use the explicit
//! [`low_level`] seam and must publish results only after acknowledged commit.
//!
//! Admission decodes the locked row once into `runlimit_core`'s `AttemptState`
//! and evaluates it with that type's `admit`, the transition the memory backend
//! also calls. Completion has to lock the row, sample the clock and check its
//! fence in one statement, so that statement applies the live-lease case of
//! `AttemptState::complete` in SQL and looks its delay up in the bound
//! `AttemptPolicy::delay_schedule`. Neither path restates expiry, the quiet
//! period or the backoff formula. A [`PgAttemptClaim`] is this backend's one
//! extension of that transition: it validated the lease while taking the row
//! lock, which hides the row from admission until its transaction ends, so its
//! completion applies even after the lease deadline.
//!
//! Every evaluation samples the server clock. A `Busy` or `Backoff` denial
//! rolls back and a stale completion matches no row, so neither changes the
//! subject's row; a `StorageCapacity` denial has no row for the subject and
//! commits only the bounded cleanup of expired rows. Of the two choices
//! `AttemptState` gives a backend for a regressing clock, this one re-evaluates
//! the stored row at the earlier instant: after a backwards step behind a
//! lease deadline, a lease already reported expired is live again until that
//! deadline, for admission and for its own receipt. Earlier releases behave
//! the same; `batter-ctk7.22` tracks evaluating against an instant that never
//! decreases.
use crate::{
    CheckError, CheckPhase, PostgresConfig,
    base::{BundledHistory, PgLimiterBase},
    protocol::{ATTEMPT_LOCK_NAMESPACE, capacity_shard},
    statements::attempts as sql,
    transaction::{BoundedConnection, Check, CheckTransaction, ConnectionOutcome},
};
use row::StoredAttempt;
use runlimit_core::{
    Delay,
    attempts::{
        AdmitEvaluation, AttemptAdmission, AttemptCompletion, AttemptCompletionResult,
        AttemptDenial, AttemptObservation, AttemptObserver, AttemptOutcome, AttemptPolicy,
        AttemptState, AttemptSubject, StagedAttemptCompletion, observe_attempt_safely,
    },
};
use sqlx::{
    PgPool, Postgres, Row,
    migrate::{MigrateError, Migrator},
    postgres::{PgArguments, PgRow},
    query::Query,
};
use std::{fmt, sync::Arc, time::Duration};

pub(crate) mod row;

/// Isolated migration history; install only when opting into attempt policies.
#[expect(
    clippy::disallowed_types,
    reason = "sqlx::migrate! embeds each migration as a sqlx::SqlStr"
)]
pub static ATTEMPTS_MIGRATOR: Migrator = sqlx::migrate!("./attempts-migrations");
/// SQL suitable for a new forward-only application-owned migration.
pub const CREATE_RUNLIMIT_ATTEMPTS_SQL: &str =
    include_str!("../attempts-migrations/20260921000001_create_runlimit_attempts.sql");
// The imported `attempt_complete.sql` stays byte-identical, outside this
// package under `runlimit/provenance`, as a digest-guarded record that no code
// path executes. It restated the backoff formula
// in SQL; this successor reads the delay from the schedule Rust binds.
const DEADLINE_RANGE: &str = "attempt lease expiry exceeds the storage range";
const PHASE: CheckPhase = CheckPhase::AcquiringCounterRowLocks;

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
    base: PgLimiterBase<dyn AttemptObserver>,
}
impl fmt::Debug for PostgresAttemptLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.base.fmt_debug("PostgresAttemptLimiter", formatter)
    }
}
impl PostgresAttemptLimiter {
    /// Creates a limiter; install its isolated migration before serving requests.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            base: PgLimiterBase::new(pool),
        }
    }
    /// Sets timeouts and the operational per-shard row bound.
    #[must_use]
    pub const fn with_config(mut self, config: PostgresConfig) -> Self {
        self.base.set_config(config);
        self
    }
    /// Installs a panic-isolated lifecycle observer.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn AttemptObserver>) -> Self {
        self.base.set_observer(observer);
        self
    }
    /// Pool that transaction-owning application adapters must use for completion.
    pub const fn pool(&self) -> &PgPool {
        self.base.pool()
    }
    /// Returns the runtime configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use runlimit_postgres::{PostgresConfig, attempts::PostgresAttemptLimiter};
    /// use sqlx::postgres::PgPoolOptions;
    ///
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // A lazy pool opens no connection until the limiter is used.
    /// let pool = PgPoolOptions::new().connect_lazy("postgresql://localhost/app")?;
    /// let config = PostgresConfig::new().with_operation_timeout(Duration::from_secs(1))?;
    ///
    /// let limiter = PostgresAttemptLimiter::new(pool).with_config(config);
    ///
    /// assert_eq!(limiter.config(), config);
    /// assert_eq!(limiter.config().operation_timeout(), Duration::from_secs(1));
    /// # Ok(())
    /// # }
    /// ```
    pub const fn config(&self) -> PostgresConfig {
        self.base.config()
    }
    /// Installs attempt tables. Shared application migrators must also ignore
    /// missing migrations, or vendor [`CREATE_RUNLIMIT_ATTEMPTS_SQL`] instead.
    ///
    /// Migration locking is session scoped. The connection returns to the pool
    /// only after a successful run and unlock; an error or cancellation closes
    /// it instead, so migration advisory locks cannot leak to the pool.
    ///
    /// # Errors
    /// Returns database or migration errors.
    pub async fn migrate(&self) -> Result<(), MigrateError> {
        self.base.migrate(BundledHistory::Attempts).await
    }
    fn observe(&self, event: AttemptObservation) {
        if let Some(observer) = self.base.observer() {
            observe_attempt_safely(observer, event);
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
        let mut connection = self.base.check_connection().await?;
        let result = admit_transaction(
            &mut connection,
            subject,
            self.base.config().maximum_rows_per_shard(),
        )
        .await;
        let result = connection.finish(result);
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
        let mut connection = self.base.check_connection().await?;
        let result = async {
            let mut transaction = connection.begin().await?;
            let rows = transaction
                .fetch_all(PHASE, completion_query(&receipt, outcome, None))
                .await?;
            let result = completion_result(&rows, outcome).map_err(ConnectionOutcome::Reusable)?;
            transaction.commit().await?;
            Ok::<_, ConnectionOutcome<CheckError>>(ConnectionOutcome::Reusable(match result {
                StagedAttemptCompletion::Applied(done) => AttemptCompletionResult::Applied(done),
                StagedAttemptCompletion::Stale => AttemptCompletionResult::Stale,
            }))
        }
        .await;
        let result = connection.finish(result);
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
    connection: &mut BoundedConnection<Check>,
    subject: AttemptSubject<'_>,
    maximum: u32,
) -> Result<ConnectionOutcome<AttemptAdmission<PgAttemptReceipt>>, ConnectionOutcome<CheckError>> {
    let policy = subject.policy().clone();
    let key = subject.counter_key();
    let shard = capacity_shard(key);
    let fingerprint = key.fingerprint().into_bytes();
    let subject = key.subject().into_bytes();
    let mut tx = connection.begin().await?;
    tx.execute(
        PHASE,
        sql::SHARD_LOCK_SQL
            .query()
            .bind(ATTEMPT_LOCK_NAMESPACE)
            .bind(i32::from(shard)),
    )
    .await?;
    tx.execute(PHASE, sql::CLEANUP_SQL.query().bind(shard))
        .await?;
    let rows = tx
        .fetch_all(
            PHASE,
            sql::ROW_LOCK_SQL
                .query()
                .bind(fingerprint.as_slice())
                .bind(subject.as_slice()),
        )
        .await?;
    let time = tx.fetch_one(PHASE, sql::SAMPLE_TIME_SQL.query()).await?;
    let decode = |error| {
        ConnectionOutcome::Reusable(CheckError::storage_decode_invariant(
            "decode attempt admission response",
            error,
        ))
    };
    let invariant = |detail| ConnectionOutcome::Reusable(CheckError::storage_invariant(detail));
    let now_ms: i64 = time.try_get("now_ms").map_err(decode)?;
    let now = row::instant(now_ms);
    // `None` is a subject without a row: it is `Clear` and needs a capacity slot.
    let stored = match rows.first() {
        Some(row) => Some(
            StoredAttempt::read(row)
                .map_err(decode)?
                .state()
                .map_err(invariant)?,
        ),
        None => None,
    };
    let pending = match stored.unwrap_or(AttemptState::Clear).admit(&policy, now) {
        AdmitEvaluation::Grant(pending) => pending,
        // The held state is left unstored: see the module's clock note.
        AdmitEvaluation::Denied(denied) => return finish_denial(tx, denied.denial().into()).await,
        // The same refusal as a deadline past the storage range below. A
        // `BIGINT` millisecond clock ends long before the state's time does.
        AdmitEvaluation::DeadlineOverflow(_) => return Err(invariant(DEADLINE_RANGE)),
    };
    let failures = i64::from(pending.failures());
    let until_ms = row::stored_instant(pending.until()).ok_or_else(|| invariant(DEADLINE_RANGE))?;
    // A held lease leaves both failure timestamps unread until it ends, so
    // every reservation stores the admission instant in them.
    let token_row = match stored {
        None => {
            let Some(slot) = free_slot(&mut tx, shard, maximum).await? else {
                return Ok(finish_capacity_denial(tx).await);
            };
            tx.fetch_one(
                PHASE,
                sql::INSERT_SQL
                    .query()
                    .bind(fingerprint.as_slice())
                    .bind(subject.as_slice())
                    .bind(slot)
                    .bind(failures)
                    .bind(now_ms)
                    .bind(integer(policy.quiet_period().millis()))
                    .bind(until_ms),
            )
            .await?
        }
        Some(_) => {
            tx.fetch_one(
                PHASE,
                sql::LEASE_SQL
                    .query()
                    .bind(fingerprint.as_slice())
                    .bind(subject.as_slice())
                    .bind(failures)
                    .bind(now_ms)
                    .bind(until_ms),
            )
            .await?
        }
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
/// The lowest free capacity slot of `shard`, or `None` when the shard is full.
async fn free_slot(
    tx: &mut CheckTransaction<'_>,
    shard: i16,
    maximum: u32,
) -> Result<Option<i32>, ConnectionOutcome<CheckError>> {
    let decode = |error| {
        ConnectionOutcome::Reusable(CheckError::storage_decode_invariant(
            "decode attempt admission response",
            error,
        ))
    };
    // A lower runtime bound counts every retained row, including slots
    // allocated by replicas using a higher bound. A hole in the new slot
    // interval is not proof that total shard occupancy is below the cap.
    let occupancy = tx
        .fetch_one(PHASE, sql::OCCUPANCY_SQL.query().bind(shard))
        .await?;
    let row_count: i64 = occupancy.try_get("row_count").map_err(decode)?;
    if row_count >= i64::from(maximum) {
        return Ok(None);
    }
    let slot = tx
        .fetch_one(
            PHASE,
            sql::FREE_SLOT_SQL
                .query()
                .bind(shard)
                .bind(i64::from(maximum)),
        )
        .await?;
    slot.try_get::<Option<i32>, _>("slot").map_err(decode)
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
fn delay(value: i64) -> Delay {
    Delay::new(Duration::from_millis(value.unsigned_abs()))
}
fn completion_query<'a>(
    receipt: &'a PgAttemptReceipt,
    outcome: AttemptOutcome,
    transaction_id: Option<&'a str>,
) -> Query<'a, Postgres, PgArguments> {
    // A success applies no delay, and the statement never reads the schedule.
    let schedule = match outcome {
        AttemptOutcome::Success => Vec::new(),
        AttemptOutcome::Failure | AttemptOutcome::Abandoned => row::delay_schedule(&receipt.policy),
    };
    sql::COMPLETE_SQL
        .query()
        .bind(receipt.policy.fingerprint().into_bytes().to_vec())
        .bind(receipt.subject.to_vec())
        .bind(&receipt.token)
        .bind(outcome == AttemptOutcome::Success)
        .bind(schedule)
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

pub mod low_level;
