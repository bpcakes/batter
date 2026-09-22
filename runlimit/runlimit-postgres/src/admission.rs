use std::{future::Future, time::Duration};

use runlimit_core::{
    Allowance, BatchDecision, Capacity, Check, Decision, Denial, QuotaDenial, QuotaMode,
};
use sqlx::{
    Acquire, PgPool, Postgres, Row, Transaction,
    pool::PoolConnection,
    postgres::{PgArguments, PgQueryResult, PgRow, types::PgInterval},
    query::Query,
    types::chrono::{DateTime, Utc},
};
use tokio::time::{Instant, timeout, timeout_at};

use crate::{
    CheckError, CheckPhase, ConnectionOutcome,
    protocol::{
        BATCH_ADVISORY_LOCK_SQL, BATCH_CAPACITY_LOCK_SQL, BATCH_PREFLIGHT_SQL, BATCH_ROW_LOCK_SQL,
        BATCH_UPSERT_SQL, CAPACITY_SHARD_COUNT, SET_LOCAL_TIMEOUTS_SQL, advisory_lock_id,
        capacity_shard, is_server_timeout, remaining_server_timeout_settings,
    },
};

const STORED_USAGE_EXCEEDS_POLICY_LIMIT: &str = "stored usage exceeds its policy limit";
const BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE: &str = "batch update returned undecodable usage";

/// The finalized outcome of one transaction, before it is shaped into a
/// single-check or batch decision.
///
/// Allowances are in caller order with one entry per submitted check, and a
/// denial index names a submitted check; both are validated against the
/// submitted batch before commit or rollback.
#[derive(Debug)]
pub(crate) enum Admission {
    Allowed(Vec<Allowance>),
    Denied { index: usize, denial: Denial },
    ShadowDenied { index: usize, denial: QuotaDenial },
}

/// Shapes a single check's admission into its decision.
///
/// The transaction evaluated exactly one check, so the admission carries one
/// allowance or names index zero. Anything else is a protocol invariant
/// failure, reported before the transaction is finalized.
pub(crate) fn single_decision(admission: Admission) -> Result<Decision, CheckError> {
    match admission {
        Admission::Allowed(allowances) => match <[Allowance; 1]>::try_from(allowances) {
            Ok([allowance]) => Ok(Decision::allowed(allowance)),
            Err(_) => Err(CheckError::storage_invariant(
                "single check returned a different number of allowances",
            )),
        },
        Admission::Denied { index: 0, denial } => Ok(Decision::denied(denial)),
        Admission::ShadowDenied { index: 0, denial } => Ok(Decision::shadow_denied(denial)),
        Admission::Denied { .. } | Admission::ShadowDenied { .. } => Err(
            CheckError::storage_invariant("single check denied an input other than its only check"),
        ),
    }
}

/// Shapes a batch admission into its decision for a batch of `batch_size`.
pub(crate) fn batch_decision(
    batch_size: usize,
    admission: Admission,
) -> Result<BatchDecision, CheckError> {
    if matches!(&admission, Admission::Allowed(allowances) if allowances.len() != batch_size) {
        return Err(CheckError::storage_invariant(
            "allowed batch returned a different number of allowances than submitted checks",
        ));
    }
    match admission {
        Admission::Allowed(allowances) => BatchDecision::allowed(allowances),
        Admission::Denied { index, denial } => BatchDecision::denied(index, batch_size, denial),
        Admission::ShadowDenied { index, denial } => {
            BatchDecision::shadow_denied(index, batch_size, denial)
        }
    }
    .map_err(|_| CheckError::storage_invariant("batch decision did not match the submitted batch"))
}

#[derive(Debug)]
enum PendingBatchOutcome {
    Allowed(Vec<PendingAllowance>),
    Denied { index: usize, denial: PendingDenial },
}

#[derive(Debug)]
pub(crate) struct BatchSqlInput {
    policy_ids: Vec<String>,
    scope_ids: Vec<String>,
    fingerprints: Vec<Vec<u8>>,
    subjects: Vec<Vec<u8>>,
    capacity_shards: Vec<i16>,
    pub(crate) lock_input_positions: Vec<i64>,
    advisory_lock_ids: Vec<i64>,
    windows: Vec<PgInterval>,
    costs: Vec<i64>,
    limits: Vec<i64>,
}

impl BatchSqlInput {
    pub(crate) fn from_checks(checks: &[Check<'_>]) -> Self {
        let counter_keys = checks.iter().map(Check::counter_key).collect::<Vec<_>>();
        let mut ordered_indices = (0..checks.len()).collect::<Vec<_>>();
        ordered_indices.sort_unstable_by_key(|index| counter_keys[*index]);

        let mut input = Self {
            policy_ids: Vec::with_capacity(checks.len()),
            scope_ids: Vec::with_capacity(checks.len()),
            fingerprints: Vec::with_capacity(checks.len()),
            subjects: Vec::with_capacity(checks.len()),
            capacity_shards: Vec::with_capacity(checks.len()),
            lock_input_positions: ordered_indices
                .into_iter()
                .map(|index| {
                    i64::try_from(index + 1)
                        .expect("bounded batch input positions fit PostgreSQL BIGINT")
                })
                .collect(),
            advisory_lock_ids: counter_keys.iter().copied().map(advisory_lock_id).collect(),
            windows: Vec::with_capacity(checks.len()),
            costs: Vec::with_capacity(checks.len()),
            limits: Vec::with_capacity(checks.len()),
        };
        input.advisory_lock_ids.sort_unstable();
        input.advisory_lock_ids.dedup();

        for (check, counter_key) in checks.iter().zip(counter_keys) {
            let policy = check.policy();
            input.policy_ids.push(policy.id().as_str().to_owned());
            input.scope_ids.push(policy.scope().as_str().to_owned());
            input
                .fingerprints
                .push(counter_key.fingerprint().as_bytes().to_vec());
            input
                .subjects
                .push(counter_key.subject().as_bytes().to_vec());
            input.capacity_shards.push(capacity_shard(counter_key));
            input.windows.push(
                PgInterval::try_from(policy.window().duration())
                    .expect("core policy windows fit PostgreSQL INTERVAL exactly"),
            );
            input.costs.push(database_integer(check.cost()));
            input.limits.push(database_integer(policy.limit().get()));
        }
        input
    }
}

#[derive(Debug)]
struct BatchPreflight {
    database_now: DateTime<Utc>,
    response_now: DateTime<Utc>,
    denial: Option<(usize, PendingDenial)>,
}

#[derive(Debug)]
pub(crate) struct PendingAllowance {
    allowance: Allowance,
}

impl PendingAllowance {
    pub(crate) fn new(
        limit: Capacity,
        used: u64,
        reset_from_sample: Duration,
    ) -> Result<Self, CheckError> {
        let remaining = limit
            .get()
            .checked_sub(used)
            .ok_or(CheckError::storage_invariant(
                STORED_USAGE_EXCEEDS_POLICY_LIMIT,
            ))?;
        let allowance = Allowance::new(limit, remaining, reset_from_sample)
            .map_err(|_| CheckError::storage_invariant(STORED_USAGE_EXCEEDS_POLICY_LIMIT))?;
        Ok(Self { allowance })
    }

    pub(crate) fn finish(self, authoritative_elapsed: Duration) -> Allowance {
        Allowance::new(
            self.allowance.capacity(),
            self.allowance.available(),
            self.allowance
                .replenishes_after()
                .duration()
                .saturating_sub(authoritative_elapsed),
        )
        .expect("PendingAllowance stores an already-validated capacity and availability")
    }
}

#[derive(Debug)]
pub(crate) struct PendingQuotaDenial {
    pub(crate) limit: Capacity,
    pub(crate) retry_from_sample: Duration,
}

impl PendingQuotaDenial {
    fn finish(self, authoritative_elapsed: Duration) -> QuotaDenial {
        QuotaDenial::new(
            self.limit,
            self.retry_from_sample.saturating_sub(authoritative_elapsed),
        )
    }
}

#[derive(Debug)]
pub(crate) enum PendingDenial {
    Quota(PendingQuotaDenial),
    StorageCapacity,
}

impl PendingDenial {
    pub(crate) fn finish(self, authoritative_elapsed: Duration) -> Denial {
        match self {
            Self::Quota(denial) => Denial::QuotaExceeded(denial.finish(authoritative_elapsed)),
            Self::StorageCapacity => Denial::StorageCapacity { retry_after: None },
        }
    }
}

pub(crate) async fn acquire_check_connection(
    pool: &PgPool,
    acquire_timeout: Duration,
) -> Result<PoolConnection<Postgres>, CheckError> {
    timeout(acquire_timeout, pool.acquire())
        .await
        .map_err(|_| CheckError::TimedOutBeforeCommit {
            phase: CheckPhase::AcquiringConnection,
        })?
        .map_err(CheckError::DefinitelyNotConsumed)
}

/// Runs one admission transaction and shapes its outcome with `build`.
///
/// `build` runs before the transaction is finalized: an allowed decision is
/// constructed before commit and a denial before rollback, so a malformed
/// response is reported as a pre-commit failure and never as a decision that
/// may already have consumed quota.
pub(crate) async fn run_check_transaction<D>(
    connection: &mut PoolConnection<Postgres>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    maximum_rows_per_shard: u32,
    deadline: Instant,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> ConnectionOutcome<Result<D, CheckError>> {
    match run_check_transaction_inner(
        connection,
        input,
        checks,
        maximum_rows_per_shard,
        deadline,
        build,
    )
    .await
    {
        Ok(outcome) => outcome.map(Ok),
        Err(outcome) => outcome.map(Err),
    }
}

/// Keeps every SQL phase on the same remaining operation budget. `PostgreSQL`
/// timeouts are per statement/lock, so a value set once at BEGIN is stale after
/// an earlier phase waits. No query method exposes the underlying transaction.
pub(crate) struct CheckTransaction<'c> {
    inner: Transaction<'c, Postgres>,
    deadline: Instant,
}

impl<'c> CheckTransaction<'c> {
    pub(crate) async fn begin(
        connection: &'c mut PoolConnection<Postgres>,
        deadline: Instant,
    ) -> Result<Self, ConnectionOutcome<CheckError>> {
        let inner = check_before_commit(
            deadline,
            CheckPhase::BeginningTransaction,
            connection.begin(),
        )
        .await?;
        Ok(Self { inner, deadline })
    }

    async fn prepare(&mut self, phase: CheckPhase) -> Result<(), ConnectionOutcome<CheckError>> {
        set_check_server_timeouts(&mut self.inner, self.deadline, phase).await
    }

    pub(crate) async fn execute(
        &mut self,
        phase: CheckPhase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<PgQueryResult, ConnectionOutcome<CheckError>> {
        self.prepare(phase).await?;
        check_before_commit(self.deadline, phase, query.execute(&mut *self.inner)).await
    }

    pub(crate) async fn fetch_all(
        &mut self,
        phase: CheckPhase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<Vec<PgRow>, ConnectionOutcome<CheckError>> {
        self.prepare(phase).await?;
        check_before_commit(self.deadline, phase, query.fetch_all(&mut *self.inner)).await
    }

    pub(crate) async fn fetch_one(
        &mut self,
        phase: CheckPhase,
        query: Query<'_, Postgres, PgArguments>,
    ) -> Result<PgRow, ConnectionOutcome<CheckError>> {
        self.prepare(phase).await?;
        check_before_commit(self.deadline, phase, query.fetch_one(&mut *self.inner)).await
    }

    pub(crate) async fn commit(mut self) -> Result<(), ConnectionOutcome<CheckError>> {
        self.prepare(CheckPhase::PreparingCommit).await?;
        commit_check(self.deadline, self.inner).await
    }

    pub(crate) async fn deny<D>(self, decision: D) -> ConnectionOutcome<D> {
        finish_denied_transaction(self.deadline, decision, self.inner.rollback()).await
    }
}

async fn run_check_transaction_inner<D>(
    connection: &mut PoolConnection<Postgres>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    maximum_rows_per_shard: u32,
    deadline: Instant,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> Result<ConnectionOutcome<D>, ConnectionOutcome<CheckError>> {
    let mut transaction = CheckTransaction::begin(connection, deadline).await?;

    // Advisory locks cover logical keys that do not have rows yet. Their
    // stable numeric IDs are sorted independently from exact storage keys so
    // deliberately colliding batches cannot acquire them in opposite orders.
    // Singles deliberately use this separate statement too: a statement
    // snapshot taken before an advisory-lock wait cannot safely decide whether
    // a capacity slot is needed.
    acquire_advisory_locks(&mut transaction, &input.advisory_lock_ids).await?;

    // Existing rows may be held by cleanup or a transaction predating the
    // advisory-lock protocol. Wait for all row locks before sampling database
    // time or deciding which keys need capacity.
    acquire_existing_row_locks(&mut transaction, input).await?;

    let (pending, authoritative_elapsed) =
        execute_batch(&mut transaction, input, checks, maximum_rows_per_shard).await?;

    match pending {
        PendingBatchOutcome::Denied { index, denial } => {
            // Batch validation rejects mixed quota modes before a connection is
            // acquired, so the first policy owns the mode of every quota denial.
            let admission = match (denial, checks[0].policy().quota_mode()) {
                (PendingDenial::Quota(denial), QuotaMode::Shadow) => Admission::ShadowDenied {
                    index,
                    denial: denial.finish(authoritative_elapsed),
                },
                (denial, _) => Admission::Denied {
                    index,
                    denial: denial.finish(authoritative_elapsed),
                },
            };
            let decision = build(admission).map_err(reusable_check_error)?;
            Ok(finish_denied_transaction(deadline, decision, transaction.inner.rollback()).await)
        }
        PendingBatchOutcome::Allowed(allowances) => {
            let allowances = allowances
                .into_iter()
                .map(|allowance| allowance.finish(authoritative_elapsed))
                .collect::<Vec<_>>();
            let decision = build(Admission::Allowed(allowances)).map_err(reusable_check_error)?;
            transaction.commit().await?;
            Ok(ConnectionOutcome::Reusable(decision))
        }
    }
}

/// Rolls back a denied transaction and reports whether the connection can be
/// reused.
///
/// The denial is already a valid decision, so a rollback failure or deadline
/// never replaces it; the connection is discarded instead so closing it rolls
/// the non-mutating transaction back.
pub(crate) async fn finish_denied_transaction<D, F>(
    deadline: Instant,
    decision: D,
    rollback: F,
) -> ConnectionOutcome<D>
where
    F: Future<Output = Result<(), sqlx::Error>>,
{
    if denied_rollback_succeeded(deadline, rollback).await {
        ConnectionOutcome::Reusable(decision)
    } else {
        ConnectionOutcome::MustClose(decision)
    }
}

async fn denied_rollback_succeeded<F>(deadline: Instant, rollback: F) -> bool
where
    F: Future<Output = Result<(), sqlx::Error>>,
{
    matches!(timeout_at(deadline, rollback).await, Ok(Ok(())))
}

async fn check_before_commit<T, F>(
    deadline: Instant,
    phase: CheckPhase,
    future: F,
) -> Result<T, ConnectionOutcome<CheckError>>
where
    F: Future<Output = Result<T, sqlx::Error>>,
{
    timeout_at(deadline, future)
        .await
        .map_err(|_| ConnectionOutcome::MustClose(CheckError::TimedOutBeforeCommit { phase }))?
        .map_err(|error| ConnectionOutcome::Reusable(map_check_database_error(error, phase)))
}

async fn commit_check(
    deadline: Instant,
    transaction: Transaction<'_, Postgres>,
) -> Result<(), ConnectionOutcome<CheckError>> {
    if Instant::now() >= deadline {
        return Err(ConnectionOutcome::Reusable(
            CheckError::TimedOutBeforeCommit {
                phase: CheckPhase::StartingCommit,
            },
        ));
    }

    timeout_at(deadline, transaction.commit())
        .await
        .map_err(|_| ConnectionOutcome::MustClose(CheckError::CommitTimedOut))?
        .map_err(|error| ConnectionOutcome::Reusable(CheckError::CommitOutcomeUnknown(error)))
}

fn map_check_database_error(error: sqlx::Error, phase: CheckPhase) -> CheckError {
    if is_server_timeout(&error) {
        CheckError::TimedOutBeforeCommit { phase }
    } else {
        CheckError::DefinitelyNotConsumed(error)
    }
}

const fn reusable_check_error(error: CheckError) -> ConnectionOutcome<CheckError> {
    ConnectionOutcome::Reusable(error)
}

async fn set_check_server_timeouts(
    transaction: &mut Transaction<'_, Postgres>,
    deadline: Instant,
    phase: CheckPhase,
) -> Result<(), ConnectionOutcome<CheckError>> {
    let (statement_timeout, lock_timeout) = remaining_server_timeout_settings(deadline).ok_or(
        ConnectionOutcome::Reusable(CheckError::TimedOutBeforeCommit { phase }),
    )?;
    check_before_commit(
        deadline,
        phase,
        sqlx::query(SET_LOCAL_TIMEOUTS_SQL)
            .bind(statement_timeout)
            .bind(lock_timeout)
            .execute(&mut **transaction),
    )
    .await
    .map(|_| ())
}

async fn acquire_advisory_locks(
    transaction: &mut CheckTransaction<'_>,
    advisory_lock_ids: &[i64],
) -> Result<(), ConnectionOutcome<CheckError>> {
    transaction
        .execute(
            CheckPhase::AcquiringLogicalKeyLocks,
            sqlx::query(BATCH_ADVISORY_LOCK_SQL).bind(advisory_lock_ids),
        )
        .await
        .map(|_| ())
}

async fn acquire_existing_row_locks(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
) -> Result<(), ConnectionOutcome<CheckError>> {
    transaction
        .execute(
            CheckPhase::AcquiringCounterRowLocks,
            sqlx::query(BATCH_ROW_LOCK_SQL)
                .bind(input.fingerprints.as_slice())
                .bind(input.subjects.as_slice())
                .bind(input.lock_input_positions.as_slice()),
        )
        .await
        .map(|_| ())
}

async fn first_capacity_denial(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
    maximum_rows_per_shard: u32,
) -> Result<Option<(usize, PendingDenial)>, ConnectionOutcome<CheckError>> {
    let rows = transaction
        .fetch_all(
            CheckPhase::AcquiringCapacityShardLocks,
            sqlx::query(BATCH_CAPACITY_LOCK_SQL)
                .bind(input.fingerprints.as_slice())
                .bind(input.subjects.as_slice())
                .bind(input.capacity_shards.as_slice()),
        )
        .await?;

    let mut pending_insertions = [0_u32; CAPACITY_SHARD_COUNT];
    for row in rows {
        let input_index: i64 = row
            .try_get("input_index")
            .map_err(|error| {
                CheckError::storage_decode_invariant(
                    "capacity preflight returned an undecodable input index",
                    error,
                )
            })
            .map_err(reusable_check_error)?;
        let input_index = usize::try_from(input_index)
            .map_err(|_| {
                CheckError::storage_invariant("capacity preflight returned an invalid input index")
            })
            .map_err(reusable_check_error)?;
        let expected_shard = input
            .capacity_shards
            .get(input_index)
            .ok_or(CheckError::storage_invariant(
                "capacity preflight returned an out-of-range input index",
            ))
            .map_err(reusable_check_error)?;
        let returned_shard: i16 = row
            .try_get("capacity_shard")
            .map_err(|error| {
                CheckError::storage_decode_invariant(
                    "capacity preflight returned an undecodable shard",
                    error,
                )
            })
            .map_err(reusable_check_error)?;
        if &returned_shard != expected_shard {
            return Err(reusable_check_error(CheckError::storage_invariant(
                "capacity preflight returned a mismatched shard",
            )));
        }
        let shard_index = usize::try_from(returned_shard)
            .map_err(|_| {
                CheckError::storage_invariant("capacity preflight returned a negative shard")
            })
            .map_err(reusable_check_error)?;
        let pending = pending_insertions
            .get_mut(shard_index)
            .ok_or(CheckError::storage_invariant(
                "capacity preflight returned an out-of-range shard",
            ))
            .map_err(reusable_check_error)?;
        let stored_rows: Option<i64> = row
            .try_get("row_count")
            .map_err(|error| {
                CheckError::storage_decode_invariant(
                    "capacity preflight returned an undecodable ledger count",
                    error,
                )
            })
            .map_err(reusable_check_error)?;
        let stored_rows = stored_rows
            .ok_or(CheckError::storage_invariant(
                "capacity shard ledger row is missing",
            ))
            .map_err(reusable_check_error)?;
        let stored_rows = u64::try_from(stored_rows)
            .map_err(|_| CheckError::storage_invariant("capacity shard ledger count is negative"))
            .map_err(reusable_check_error)?;
        let projected_rows = stored_rows
            .checked_add(u64::from(*pending))
            .and_then(|rows| rows.checked_add(1))
            .ok_or(CheckError::storage_invariant(
                "capacity shard ledger count overflowed",
            ))
            .map_err(reusable_check_error)?;
        if projected_rows > u64::from(maximum_rows_per_shard) {
            return Ok(Some((input_index, PendingDenial::StorageCapacity)));
        }
        *pending += 1;
    }

    Ok(None)
}
fn authoritative_elapsed(start: DateTime<Utc>, end: DateTime<Utc>) -> Duration {
    end.signed_duration_since(start)
        .to_std()
        .unwrap_or(Duration::ZERO)
}

async fn execute_batch(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    maximum_rows_per_shard: u32,
) -> Result<(PendingBatchOutcome, Duration), ConnectionOutcome<CheckError>> {
    let capacity_denial = first_capacity_denial(transaction, input, maximum_rows_per_shard).await?;
    let preflight = preflight_batch(transaction, input, checks).await?;
    let first_denial = match (capacity_denial, preflight.denial) {
        (Some(capacity), Some(quota)) => Some(if capacity.0 < quota.0 {
            capacity
        } else {
            quota
        }),
        (Some(capacity), None) => Some(capacity),
        (None, Some(quota)) => Some(quota),
        (None, None) => None,
    };
    if let Some((index, denial)) = first_denial {
        return Ok((
            PendingBatchOutcome::Denied { index, denial },
            authoritative_elapsed(preflight.database_now, preflight.response_now),
        ));
    }
    let (allowances, response_now) =
        upsert_batch(transaction, input, checks, preflight.database_now).await?;
    Ok((
        PendingBatchOutcome::Allowed(allowances),
        authoritative_elapsed(preflight.database_now, response_now),
    ))
}

async fn preflight_batch(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
) -> Result<BatchPreflight, ConnectionOutcome<CheckError>> {
    let preflight_row = transaction
        .fetch_one(
            CheckPhase::PreflightingCounters,
            sqlx::query(BATCH_PREFLIGHT_SQL)
                .bind(input.fingerprints.as_slice())
                .bind(input.subjects.as_slice())
                .bind(input.costs.as_slice())
                .bind(input.limits.as_slice()),
        )
        .await?;

    let database_now: DateTime<Utc> = preflight_row
        .try_get("database_now")
        .map_err(|error| {
            CheckError::storage_decode_invariant(
                "batch preflight returned an undecodable database time",
                error,
            )
        })
        .map_err(reusable_check_error)?;
    let preflight_response_now: DateTime<Utc> = preflight_row
        .try_get("response_now")
        .map_err(|error| {
            CheckError::storage_decode_invariant(
                "batch preflight returned an undecodable response time",
                error,
            )
        })
        .map_err(reusable_check_error)?;
    let denied_index: Option<i64> = preflight_row
        .try_get("input_index")
        .map_err(|error| {
            CheckError::storage_decode_invariant(
                "batch preflight returned an undecodable denial index",
                error,
            )
        })
        .map_err(reusable_check_error)?;
    let denied_expiry: Option<DateTime<Utc>> = preflight_row
        .try_get("window_expires_at")
        .map_err(|error| {
            CheckError::storage_decode_invariant(
                "batch preflight returned an undecodable denial expiry",
                error,
            )
        })
        .map_err(reusable_check_error)?;

    let denial = match (denied_index, denied_expiry) {
        (Some(input_index), Some(expires_at)) => {
            let input_index = usize::try_from(input_index)
                .map_err(|_| {
                    CheckError::storage_invariant("batch preflight returned an invalid input index")
                })
                .map_err(reusable_check_error)?;
            let check = checks
                .get(input_index)
                .ok_or(CheckError::storage_invariant(
                    "batch preflight returned an out-of-range input index",
                ))
                .map_err(reusable_check_error)?;
            Some((
                input_index,
                PendingDenial::Quota(PendingQuotaDenial {
                    limit: check.policy().limit(),
                    retry_from_sample: duration_until(expires_at, database_now)
                        .map_err(reusable_check_error)?,
                }),
            ))
        }
        (None, None) => None,
        _ => {
            return Err(reusable_check_error(CheckError::storage_invariant(
                "batch preflight returned an incomplete denial",
            )));
        }
    };

    Ok(BatchPreflight {
        database_now,
        response_now: preflight_response_now,
        denial,
    })
}

async fn upsert_batch(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    database_now: DateTime<Utc>,
) -> Result<(Vec<PendingAllowance>, DateTime<Utc>), ConnectionOutcome<CheckError>> {
    let rows = transaction
        .fetch_all(
            CheckPhase::UpdatingCounters,
            sqlx::query(BATCH_UPSERT_SQL)
                .bind(input.policy_ids.as_slice())
                .bind(input.scope_ids.as_slice())
                .bind(input.fingerprints.as_slice())
                .bind(input.subjects.as_slice())
                .bind(input.windows.as_slice())
                .bind(input.costs.as_slice())
                .bind(input.limits.as_slice())
                .bind(database_now),
        )
        .await?;

    if rows.is_empty() {
        return Err(reusable_check_error(CheckError::storage_invariant(
            "allowed batch update returned no decisions",
        )));
    }
    let response_now: DateTime<Utc> = rows[0]
        .try_get("response_now")
        .map_err(|error| {
            CheckError::storage_decode_invariant(
                "batch update returned an undecodable response time",
                error,
            )
        })
        .map_err(reusable_check_error)?;

    let mut allowances = Vec::with_capacity(checks.len());
    for (output_position, row) in rows.iter().enumerate() {
        let input_index: i64 = row
            .try_get("input_index")
            .map_err(|error| {
                CheckError::storage_decode_invariant(
                    "batch update returned an undecodable input index",
                    error,
                )
            })
            .map_err(reusable_check_error)?;
        let input_index = usize::try_from(input_index)
            .map_err(|_| {
                CheckError::storage_invariant("batch evaluation returned an invalid input index")
            })
            .map_err(reusable_check_error)?;
        let Some(check) = checks.get(input_index) else {
            return Err(reusable_check_error(CheckError::storage_invariant(
                "batch evaluation returned an out-of-range input index",
            )));
        };
        let expires_at = read_expiry(row).map_err(reusable_check_error)?;
        let remaining_from_sample =
            duration_until(expires_at, database_now).map_err(reusable_check_error)?;

        if input_index != output_position {
            return Err(reusable_check_error(CheckError::storage_invariant(
                "allowed batch decisions were not returned in caller order",
            )));
        }
        let used = read_used(row).map_err(reusable_check_error)?;
        let limit = check.policy().limit();
        allowances.push(
            PendingAllowance::new(limit, used, remaining_from_sample)
                .map_err(reusable_check_error)?,
        );
    }

    if allowances.len() != checks.len() {
        return Err(reusable_check_error(CheckError::storage_invariant(
            "allowed batch returned an incomplete decision set",
        )));
    }
    Ok((allowances, response_now))
}

pub(crate) fn database_integer(value: u64) -> i64 {
    i64::try_from(value)
        .expect("core policy and check validation keep database integers within i64")
}

fn read_used(row: &PgRow) -> Result<u64, CheckError> {
    let used: i64 = row.try_get("used").map_err(|error| {
        CheckError::storage_decode_invariant(BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE, error)
    })?;
    u64::try_from(used).map_err(|_| CheckError::storage_invariant("stored usage is negative"))
}

fn read_expiry(row: &PgRow) -> Result<DateTime<Utc>, CheckError> {
    row.try_get("window_expires_at").map_err(|error| {
        CheckError::storage_decode_invariant("batch update returned undecodable expiry", error)
    })
}

fn duration_until(
    expires_at: DateTime<Utc>,
    database_now: DateTime<Utc>,
) -> Result<Duration, CheckError> {
    expires_at
        .signed_duration_since(database_now)
        .to_std()
        .map_err(|_| CheckError::storage_invariant("stored window is already expired"))
}

#[cfg(test)]
mod tests {
    use super::{BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE, STORED_USAGE_EXCEEDS_POLICY_LIMIT};

    #[test]
    fn stable_storage_invariant_classifications_are_pinned_without_a_database() {
        assert_eq!(
            STORED_USAGE_EXCEEDS_POLICY_LIMIT,
            "stored usage exceeds its policy limit"
        );
        assert_eq!(
            BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE,
            "batch update returned undecodable usage"
        );
    }
}
