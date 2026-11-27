use std::time::Duration;

use runlimit_core::{Allowance, BatchDecision, Check, Decision, Denial, QuotaDenial, QuotaPeriod};
use sqlx::{
    Row,
    postgres::{PgRow, types::PgInterval},
    types::chrono::{DateTime, Utc},
};

use crate::{
    CheckError, CheckPhase,
    pending::{
        Pending, PendingAllowance, PendingDenial, PendingQuotaDenial, SampledAllowance,
        authoritative_elapsed,
    },
    protocol::{CAPACITY_SHARD_COUNT, advisory_lock_id, capacity_shard},
    statements::fixed_window as sql,
    transaction::{self, BoundedConnection, CheckTransaction, ConnectionOutcome},
};

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
    let out_of_range = |_| CheckError::storage_invariant("denied batch index is out of range");
    match admission {
        Admission::Allowed(allowances) => BatchDecision::allowed(allowances)
            .map_err(|_| CheckError::storage_invariant("allowed batch returned no allowances")),
        Admission::Denied { index, denial } => {
            BatchDecision::denied(index, batch_size, denial).map_err(out_of_range)
        }
        Admission::ShadowDenied { index, denial } => {
            BatchDecision::shadow_denied(index, batch_size, denial).map_err(out_of_range)
        }
    }
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
        // One-based input positions, in lock (counter key) order.
        let mut lock_order = counter_keys.iter().zip(1_i64..).collect::<Vec<_>>();
        lock_order.sort_unstable_by_key(|(key, _)| *key);

        let mut input = Self {
            policy_ids: Vec::with_capacity(checks.len()),
            scope_ids: Vec::with_capacity(checks.len()),
            fingerprints: Vec::with_capacity(checks.len()),
            subjects: Vec::with_capacity(checks.len()),
            capacity_shards: Vec::with_capacity(checks.len()),
            lock_input_positions: lock_order
                .into_iter()
                .map(|(_, position)| position)
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
            input
                .capacity_shards
                .push(i16::from(capacity_shard(counter_key)));
            input.windows.push(interval(policy.window()));
            input.costs.push(check.cost_i64());
            input.limits.push(policy.limit().get_i64());
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

/// Shapes the evaluated outcome with `build`, then finalizes the transaction:
/// an allowance commits and a denial rolls back.
///
/// `build` runs before the transaction is finalized, so a malformed response
/// is reported as a pre-commit failure and never as a decision that may
/// already have consumed quota.
pub(crate) async fn finalize<A, D>(
    transaction: CheckTransaction<'_>,
    pending: Pending<A>,
    authoritative_elapsed: Duration,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> Result<ConnectionOutcome<D>, ConnectionOutcome<CheckError>>
where
    A: SampledAllowance,
{
    match pending {
        allowed @ Pending::Allowed(_) => {
            let decision = build(allowed.finish(authoritative_elapsed))
                .map_err(ConnectionOutcome::Reusable)?;
            transaction.commit().await?;
            Ok(ConnectionOutcome::Reusable(decision))
        }
        denied @ Pending::Denied { .. } => {
            let decision =
                build(denied.finish(authoritative_elapsed)).map_err(ConnectionOutcome::Reusable)?;
            Ok(transaction.deny(decision).await)
        }
    }
}

/// Runs one fixed-window admission transaction and shapes its outcome with
/// `build`.
pub(crate) async fn run_check_transaction<D>(
    connection: &mut BoundedConnection<transaction::Check>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    maximum_rows_per_shard: u32,
    build: impl FnOnce(Admission) -> Result<D, CheckError>,
) -> Result<ConnectionOutcome<D>, ConnectionOutcome<CheckError>> {
    let mut transaction = connection.begin().await?;

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

    finalize(transaction, pending, authoritative_elapsed, build).await
}

const fn reusable_check_error(error: CheckError) -> ConnectionOutcome<CheckError> {
    ConnectionOutcome::Reusable(error)
}

async fn acquire_advisory_locks(
    transaction: &mut CheckTransaction<'_>,
    advisory_lock_ids: &[i64],
) -> Result<(), ConnectionOutcome<CheckError>> {
    transaction
        .execute(
            CheckPhase::AcquiringLogicalKeyLocks,
            sql::BATCH_ADVISORY_LOCK_SQL.query().bind(advisory_lock_ids),
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
            sql::BATCH_ROW_LOCK_SQL
                .query()
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
            sql::BATCH_CAPACITY_LOCK_SQL
                .query()
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

async fn execute_batch(
    transaction: &mut CheckTransaction<'_>,
    input: &BatchSqlInput,
    checks: &[Check<'_>],
    maximum_rows_per_shard: u32,
) -> Result<(Pending<PendingAllowance>, Duration), ConnectionOutcome<CheckError>> {
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
            Pending::Denied {
                index,
                denial,
                // Batch validation rejects mixed quota modes before a
                // connection is acquired, so the first policy owns the mode
                // of every quota denial.
                mode: checks[0].policy().quota_mode(),
            },
            authoritative_elapsed(preflight.database_now, preflight.response_now),
        ));
    }
    let (allowances, response_now) =
        upsert_batch(transaction, input, checks, preflight.database_now).await?;
    Ok((
        Pending::Allowed(allowances),
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
            sql::BATCH_PREFLIGHT_SQL
                .query()
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
            sql::BATCH_UPSERT_SQL
                .query()
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

/// A policy window as an exact `INTERVAL` of microseconds. Every
/// [`QuotaPeriod`]'s microsecond count fits `BIGINT`, which `runlimit-core`
/// checks at compile time.
pub(crate) const fn interval(window: QuotaPeriod) -> PgInterval {
    PgInterval {
        months: 0,
        days: 0,
        microseconds: window.millis_i64() * 1_000,
    }
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
    use super::BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE;

    #[test]
    fn stable_storage_invariant_classification_is_pinned_without_a_database() {
        assert_eq!(
            BATCH_UPDATE_RETURNED_UNDECODABLE_USAGE,
            "batch update returned undecodable usage"
        );
    }
}
