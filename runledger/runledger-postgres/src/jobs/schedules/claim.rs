use chrono::{DateTime, Utc};
use sqlx::types::Uuid;

use crate::{DbPool, DbTx, Error, Result};

use super::super::types::JobScheduleRecord;
use super::locking::lock_job_schedules_for_due_schedule_claim_tx;
use super::row::{JobScheduleRow, job_schedule_from_row};

/// Claims due schedules for runtime materialization inside an existing transaction.
///
/// This is a low-level runtime helper used by `runledger-runtime`'s scheduler
/// loop. It selects active schedules with `next_fire_at <= now`, ordered by
/// `next_fire_at`, using `FOR UPDATE SKIP LOCKED` so concurrent scheduler loops
/// do not materialize the same schedule row.
///
/// The claim takes the scheduler's `ROW EXCLUSIVE` table lock before row locks.
/// That lock is compatible with other scheduler workers, but conflicts with
/// exact schedule sync's table lock so claims wait before holding claimed rows.
///
/// Most applications should create schedules with [`crate::jobs::upsert_job_schedule`] and
/// run schedule materialization through `runledger_runtime::Supervisor` instead
/// of calling this helper directly.
///
/// # Errors
/// Returns an error if PostgreSQL rejects the claim query or if a claimed row
/// cannot be decoded into [`JobScheduleRecord`].
pub async fn claim_due_schedules_tx(
    tx: &mut DbTx<'_>,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<JobScheduleRecord>> {
    lock_job_schedules_for_due_schedule_claim_tx(tx).await?;

    let rows = sqlx::query!(
        "SELECT
            id,
            name,
            job_type,
            organization_id,
            payload_template,
            cron_expr,
            max_jitter_seconds,
            next_fire_at
         FROM job_schedules
         WHERE is_active = true
           AND next_fire_at <= $1
         ORDER BY next_fire_at ASC
         FOR UPDATE SKIP LOCKED
         LIMIT $2",
        now,
        limit,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| Error::from_query_sqlx_with_context("claim due schedules", error))?;

    rows.into_iter()
        .map(|row| {
            job_schedule_from_row(JobScheduleRow {
                id: row.id,
                name: row.name,
                job_type: row.job_type,
                organization_id: row.organization_id,
                payload_template: row.payload_template,
                cron_expr: row.cron_expr,
                is_active: true,
                max_jitter_seconds: row.max_jitter_seconds,
                next_fire_at: row.next_fire_at,
            })
        })
        .collect::<Result<Vec<_>>>()
}

/// Records a successful schedule materialization inside an existing transaction.
///
/// This is a low-level runtime helper used by `runledger-runtime` after a due
/// schedule has produced its job. It updates `last_fired_at` and advances
/// `next_fire_at` to the caller-computed UTC cursor.
///
/// Pass the [`JobScheduleRecord::id`] returned by [`claim_due_schedules_tx`].
/// Returns `true` when that schedule row still existed and was updated, and
/// `false` when no row matched `schedule_id`.
///
/// Most applications should let `runledger_runtime::Supervisor` call this as
/// part of the scheduler loop instead of calling it directly.
///
/// # Errors
/// Returns an error if PostgreSQL rejects the update. A missing schedule row is
/// reported as `Ok(false)`, not as an error.
pub async fn mark_schedule_fired_tx(
    tx: &mut DbTx<'_>,
    schedule_id: Uuid,
    fired_at: DateTime<Utc>,
    next_fire_at: DateTime<Utc>,
) -> Result<bool> {
    let result = sqlx::query!(
        "UPDATE job_schedules
         SET last_fired_at = $2,
             next_fire_at = $3,
             updated_at = now()
         WHERE id = $1",
        schedule_id,
        fired_at,
        next_fire_at,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| Error::from_query_sqlx_with_context("mark schedule fired", error))?;

    Ok(result.rows_affected() > 0)
}

/// Reads the earliest `next_fire_at` among active schedules.
///
/// This is a low-level runtime helper used by `runledger-runtime`'s scheduler
/// loop to sleep until the next due fire instead of a whole poll interval. It
/// reads the same active-schedule set that [`claim_due_schedules_tx`] claims
/// from, through the partial index on active `next_fire_at`, without taking row
/// locks. The value is therefore only a hint: the row may already be claimed by
/// another scheduler, and a later write may create an earlier fire. Returns
/// `None` when no schedule is active.
///
/// The read owns a transaction with a one-second `statement_timeout` cap,
/// preserving a stricter caller setting. It awaits rollback on success and
/// failure, restoring pooled-session settings before returning. Pool acquisition
/// retains the pool's configured timeout; this is not a transport deadline.
///
/// # Errors
/// Returns an error if acquisition, the bounded read or rollback fails. When
/// both the read and rollback fail, both errors are retained.
///
/// # Example
///
/// ```no_run
/// # async fn example(pool: &runledger_postgres::DbPool) -> runledger_postgres::Result<()> {
/// use runledger_postgres::jobs::earliest_active_schedule_fire_at;
///
/// let poll_interval = std::time::Duration::from_secs(30);
/// let wait = match earliest_active_schedule_fire_at(pool).await? {
///     Some(fire_at) => (fire_at - chrono::Utc::now())
///         .to_std()
///         .unwrap_or_default()
///         .min(poll_interval),
///     None => poll_interval,
/// };
/// tokio::time::sleep(wait).await;
/// # Ok(()) }
/// ```
pub async fn earliest_active_schedule_fire_at(pool: &DbPool) -> Result<Option<DateTime<Utc>>> {
    let mut tx = pool.begin().await.map_err(|error| {
        Error::from_query_sqlx_with_context("begin earliest schedule fire read", error)
    })?;
    let read = async {
        super::super::transaction_settings::cap_local_statement_timeout_tx(
            &mut tx,
            "1s",
            1_000,
            "cap earliest schedule fire read",
        )
        .await?;
        sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT min(next_fire_at)
             FROM job_schedules
             WHERE is_active",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| {
            Error::from_query_sqlx_with_context("read earliest active schedule fire", error)
        })
    }
    .await;
    match (read, tx.rollback().await) {
        (result, Ok(())) => result,
        (Err(operation), Err(rollback)) => {
            Err(Error::RollbackFailure(Box::new(crate::RollbackFailure {
                operation,
                rollback,
            })))
        }
        (Ok(_), Err(error)) => Err(Error::from_query_sqlx_with_context(
            "rollback earliest schedule fire read",
            error,
        )),
    }
}
