use std::str::FromStr;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use cron::Schedule;
use runledger_postgres::jobs::{self, JobEnqueue};
use serde_json::{Value, json};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::config::JobsConfig;
use crate::shutdown;
use crate::{Result, RuntimeLoopExit, SchedulerError};

mod coalescing;
mod wait;

const FAILED_SCHEDULE_RETRY_DELAY_SECONDS: i64 = 30;
const SCHEDULE_STALE_CATCHUP_JITTER_SEARCH_LIMIT: usize =
    jobs::JOB_SCHEDULE_MAX_JITTER_SECONDS as usize + 2;
const CREATE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL: &str = "SAVEPOINT materialize_due_schedule";
const ROLLBACK_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL: &str =
    "ROLLBACK TO SAVEPOINT materialize_due_schedule";
const RELEASE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL: &str =
    "RELEASE SAVEPOINT materialize_due_schedule";

/// Runs cron schedule materialization until shutdown is requested.
///
/// Each pass claims up to [`JobsConfig::claim_batch_size`] due schedules in one
/// transaction, materializes each into a job and advances its next UTC fire
/// cursor. A committed pass that claimed a whole batch is followed by another
/// pass after a yield and a shutdown check, so a backlog drains without waiting
/// for the poll interval. Every schedule such a pass committed now fires after
/// that pass's clock, so the next pass claims other rows. After a smaller pass
/// the loop waits for shutdown or until the earliest active fire, but at least
/// 500 ms and at most [`JobsConfig::schedule_poll_interval`]; the cap bounds how
/// late a schedule written by another process is noticed. A failed pass, or a
/// failed read of the earliest fire, waits the whole poll interval. Individual
/// materialization failures are logged and deferred for 30 seconds so one bad
/// schedule does not permanently starve other due schedules.
///
/// When the first fire after a claimed cursor is already due, the scheduler
/// has lagged by at least one cron period: after downtime, behind a backlog, or
/// whenever passes run late. One missed fire is materialized with its original
/// `scheduled_for` metadata and the cursor is then coalesced to the first future
/// fire after the scheduler's current clock. The occurrences in between are
/// dropped, not replayed; a warning names the schedule and counts them, capped
/// at 1000. This bounds catch-up instead of replaying every missed cron tick.
/// The next cursor is computed from the jittered fire time, so a
/// `max_jitter_seconds` window at least as large as the gap between cron
/// occurrences can also skip occurrences.
///
/// Shutdown is requested by sending `true` on `shutdown` or by dropping the
/// watch sender. The signal is checked between passes; a pass already running
/// is completed, not cancelled. This function returns
/// [`RuntimeLoopExit::Shutdown`] after the shutdown signal is observed.
///
/// This lower-level loop remains public for custom runtime orchestration.
/// Prefer [`crate::Supervisor`] for ordinary worker processes so scheduler,
/// worker, and reaper tasks are started, monitored, and shut down together.
pub async fn run_scheduler_loop(
    pool: runledger_postgres::DbPool,
    config: JobsConfig,
    shutdown: watch::Receiver<bool>,
) -> RuntimeLoopExit {
    run_scheduler_loop_initialized(pool, config, shutdown, None).await
}

pub(crate) async fn run_scheduler_loop_initialized(
    pool: runledger_postgres::DbPool,
    config: JobsConfig,
    mut shutdown: watch::Receiver<bool>,
    mut startup: Option<crate::startup::LoopStartup>,
) -> RuntimeLoopExit {
    if let Err(error) = config.validate_scheduler_loop() {
        warn!(%error, "invalid jobs config; stopping scheduler loop");
        return RuntimeLoopExit::InvalidConfig(error);
    }

    crate::startup::acknowledge(&mut startup);

    loop {
        if shutdown::is_requested_or_closed(&shutdown) {
            return scheduler_shutdown_complete();
        }

        let wait = match materialize_pass(&pool, &config).await {
            wait::NextPass::Immediately => {
                tokio::task::yield_now().await;
                continue;
            }
            wait::NextPass::After(wait) => wait,
        };

        if shutdown::wait_for_request_or_timeout(&mut shutdown, wait).await {
            return scheduler_shutdown_complete();
        }
    }
}

/// Runs one pass to completion and chooses when the next one starts.
async fn materialize_pass(
    pool: &runledger_postgres::DbPool,
    config: &JobsConfig,
) -> wait::NextPass {
    match materialize_due_schedules(pool, config.claim_batch_size).await {
        Ok(claimed) if wait::batch_was_full(claimed, config.claim_batch_size) => {
            wait::NextPass::Immediately
        }
        Ok(_) => wait::NextPass::After(
            wait::until_next_due_fire(pool, config.schedule_poll_interval).await,
        ),
        Err(error) => {
            warn!(%error, "schedule materialization failed");
            wait::NextPass::After(config.schedule_poll_interval)
        }
    }
}

fn scheduler_shutdown_complete() -> RuntimeLoopExit {
    info!("scheduler shutdown complete");
    RuntimeLoopExit::Shutdown
}

/// Commits one pass and returns how many schedules it claimed, including
/// claimed schedules whose failed materialization was deferred.
#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn materialize_due_schedules(
    pool: &runledger_postgres::DbPool,
    batch_size: i64,
) -> Result<usize> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| SchedulerError::BeginTransaction {
            source: runledger_postgres::Error::ConnectionError(error.to_string()),
        })?;

    let now = Utc::now();
    let pass = materialize_due_schedules_tx(&mut tx, now, batch_size).await?;

    tx.commit()
        .await
        .map_err(|error| SchedulerError::CommitTransaction {
            source: runledger_postgres::Error::commit_unconfirmed(
                "commit scheduler transaction",
                error,
            ),
        })?;
    // Coalescing drops occurrences only once its cursor advance is durable.
    for coalesced in &pass.coalesced {
        coalesced.warn();
    }
    Ok(pass.claimed)
}

/// Schedules claimed by one pass, and those whose cursor was coalesced.
#[derive(Debug)]
struct MaterializationPass {
    claimed: usize,
    coalesced: Vec<coalescing::CoalescedSchedule>,
}

fn savepoint_error_variant(
    statement: &'static str,
    source: runledger_postgres::Error,
) -> SchedulerError {
    match statement {
        CREATE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            SchedulerError::SavepointCreate { statement, source }
        }
        ROLLBACK_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            SchedulerError::SavepointRollback { statement, source }
        }
        RELEASE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            SchedulerError::SavepointRelease { statement, source }
        }
        _ => unreachable!("unexpected savepoint statement: {statement}"),
    }
}

#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn execute_savepoint_sql_tx(
    tx: &mut runledger_postgres::DbTx<'_>,
    statement: &'static str,
) -> Result<()> {
    match statement {
        CREATE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            sqlx::query!("SAVEPOINT materialize_due_schedule")
                .execute(&mut **tx)
                .await
                .map_err(|error| {
                    savepoint_error_variant(
                        statement,
                        runledger_postgres::Error::ConnectionError(error.to_string()),
                    )
                })?;
        }
        ROLLBACK_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            sqlx::query!("ROLLBACK TO SAVEPOINT materialize_due_schedule")
                .execute(&mut **tx)
                .await
                .map_err(|error| {
                    savepoint_error_variant(
                        statement,
                        runledger_postgres::Error::ConnectionError(error.to_string()),
                    )
                })?;
        }
        RELEASE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL => {
            sqlx::query!("RELEASE SAVEPOINT materialize_due_schedule")
                .execute(&mut **tx)
                .await
                .map_err(|error| {
                    savepoint_error_variant(
                        statement,
                        runledger_postgres::Error::ConnectionError(error.to_string()),
                    )
                })?;
        }
        _ => unreachable!("unexpected savepoint statement: {statement}"),
    }

    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn materialize_due_schedules_tx(
    tx: &mut runledger_postgres::DbTx<'_>,
    now: DateTime<Utc>,
    batch_size: i64,
) -> Result<MaterializationPass> {
    let schedules = jobs::claim_due_schedules_tx(tx, now, batch_size)
        .await
        .map_err(|source| SchedulerError::ClaimDueSchedules { source })?;
    materialize_claimed_schedules_tx(tx, now, schedules).await
}

#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn materialize_claimed_schedules_tx(
    tx: &mut runledger_postgres::DbTx<'_>,
    now: DateTime<Utc>,
    schedules: Vec<jobs::JobScheduleRecord>,
) -> Result<MaterializationPass> {
    let mut pass = MaterializationPass {
        claimed: schedules.len(),
        coalesced: Vec::new(),
    };
    for schedule in schedules {
        execute_savepoint_sql_tx(tx, CREATE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL).await?;

        let coalesced = match materialize_schedule_tx(tx, &schedule, now).await {
            Ok(coalesced) => coalesced,
            Err(error) => {
                warn!(
                    %error,
                    schedule_id=%schedule.id,
                    schedule_name=%schedule.name,
                    "schedule materialization failed; skipping"
                );
                execute_savepoint_sql_tx(tx, ROLLBACK_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL)
                    .await?;
                execute_savepoint_sql_tx(tx, RELEASE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL)
                    .await?;

                if matches!(
                    error,
                    crate::Error::Scheduler(SchedulerError::ClaimedScheduleMissing { .. })
                ) {
                    return Err(error);
                }

                // Push failed schedules out of the immediate due window to avoid
                // repeatedly selecting the same failing rows and starving valid
                // schedules. This also keeps a full batch of failures from making
                // the loop re-poll the same rows.
                let retry_at = failed_schedule_retry_at(now);
                defer_failed_schedule_tx(tx, schedule.id, retry_at).await?;
                continue;
            }
        };

        execute_savepoint_sql_tx(tx, RELEASE_MATERIALIZE_DUE_SCHEDULE_SAVEPOINT_SQL).await?;
        pass.coalesced.extend(coalesced);
    }

    Ok(pass)
}

#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn defer_failed_schedule_tx(
    tx: &mut runledger_postgres::DbTx<'_>,
    schedule_id: uuid::Uuid,
    next_fire_at: DateTime<Utc>,
) -> Result<()> {
    let updated = sqlx::query!(
        "UPDATE job_schedules
         SET next_fire_at = $2,
             updated_at = now()
         WHERE id = $1",
        schedule_id,
        next_fire_at,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| SchedulerError::DeferFailedSchedule {
        schedule_id,
        source: runledger_postgres::Error::from_query_sqlx_with_context(
            "defer failed schedule",
            error,
        ),
    })?;

    if updated.rows_affected() == 0 {
        return Err(SchedulerError::ClaimedScheduleMissing {
            schedule_id,
            operation: "deferring failed schedule",
        }
        .into());
    }

    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "preserve concrete native scheduler errors without changing the imported runtime contract"
)]
async fn materialize_schedule_tx(
    tx: &mut runledger_postgres::DbTx<'_>,
    schedule: &jobs::JobScheduleRecord,
    now: DateTime<Utc>,
) -> Result<Option<coalescing::CoalescedSchedule>> {
    let scheduled_for = schedule.next_fire_at;
    let cron = Schedule::from_str(&schedule.cron_expr)
        .map_err(|_| invalid_schedule_cron_error(schedule))?;
    let next_fire = next_fire_after_materialization(
        &cron,
        scheduled_for,
        now,
        schedule.id,
        schedule.max_jitter_seconds,
    )
    .ok_or_else(|| invalid_schedule_cron_error(schedule))?;

    let mut payload = schedule.payload_template.clone();
    merge_schedule_metadata(&mut payload, schedule.id, &schedule.name, scheduled_for);

    let enqueue_payload = JobEnqueue {
        job_type: schedule.job_type.as_borrowed(),
        organization_id: schedule.organization_id,
        payload: &payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: Some(now),
        idempotency_key: None,
        stage: Some(runledger_core::jobs::JobStage::Scheduled),
    };

    jobs::enqueue_job_tx(tx, &enqueue_payload)
        .await
        .map_err(|source| SchedulerError::EnqueueScheduledJob {
            schedule_id: schedule.id,
            job_type: schedule.job_type.to_string(),
            source,
        })?;

    let marked = jobs::mark_schedule_fired_tx(tx, schedule.id, now, next_fire.at())
        .await
        .map_err(|source| SchedulerError::MarkScheduleFired {
            schedule_id: schedule.id,
            source,
        })?;
    if !marked {
        // The enqueue above is still inside the caller's schedule savepoint.
        // Returning an error lets the caller roll it back instead of producing
        // work for a schedule row that is no longer present.
        return Err(SchedulerError::ClaimedScheduleMissing {
            schedule_id: schedule.id,
            operation: "marking schedule as fired",
        }
        .into());
    }
    Ok(match next_fire {
        NextFire::OnSchedule(_) => None,
        NextFire::Coalesced(next_fire_at) => Some(coalescing::CoalescedSchedule::new(
            schedule,
            cron,
            now,
            next_fire_at,
        )),
    })
}

/// Cursor after one materialized fire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NextFire {
    /// The first fire after `scheduled_for` is still in the future.
    OnSchedule(DateTime<Utc>),
    /// That fire was already due, so the cursor skipped to the first future fire.
    Coalesced(DateTime<Utc>),
}

impl NextFire {
    const fn at(self) -> DateTime<Utc> {
        match self {
            Self::OnSchedule(at) | Self::Coalesced(at) => at,
        }
    }
}

/// Materializes at most one missed fire for a stale schedule, then coalesces the
/// cursor to the first future fire so an outage cannot create unbounded replay.
fn next_fire_after_materialization(
    schedule: &Schedule,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<NextFire> {
    let next_after_scheduled =
        next_jittered_fire_after_utc(schedule, scheduled_for, schedule_id, max_jitter_seconds)?;
    if next_after_scheduled <= now {
        compute_coalesced_next_fire_after_now_utc(schedule, now, schedule_id, max_jitter_seconds)
            .map(NextFire::Coalesced)
    } else {
        Some(NextFire::OnSchedule(next_after_scheduled))
    }
}

#[cfg(test)]
fn compute_next_fire_at_with_stale_coalescing_utc(
    cron_expr: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    let schedule = Schedule::from_str(cron_expr).ok()?;
    next_fire_after_materialization(
        &schedule,
        scheduled_for,
        now,
        schedule_id,
        max_jitter_seconds,
    )
    .map(NextFire::at)
}

fn invalid_schedule_cron_error(schedule: &jobs::JobScheduleRecord) -> SchedulerError {
    SchedulerError::InvalidCronExpression {
        schedule_id: schedule.id,
        schedule_name: schedule.name.clone(),
        cron_expr: schedule.cron_expr.clone(),
    }
}

fn merge_schedule_metadata(
    payload: &mut Value,
    schedule_id: uuid::Uuid,
    schedule_name: &str,
    scheduled_for: DateTime<Utc>,
) {
    let metadata = json!({
        "schedule_id": schedule_id,
        "schedule_name": schedule_name,
        "scheduled_for": scheduled_for.to_rfc3339_opts(SecondsFormat::AutoSi, true),
    });

    match payload {
        Value::Object(map) => {
            map.insert("_schedule".to_string(), metadata);
        }
        _ => {
            let original_payload = std::mem::take(payload);
            *payload = json!({
                "payload": original_payload,
                "_schedule": metadata,
            });
        }
    }
}

/// Scheduling semantics are UTC-only across the jobs framework.
#[cfg(test)]
fn compute_next_fire_at_utc(
    cron_expr: &str,
    from: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    let schedule = Schedule::from_str(cron_expr).ok()?;
    next_jittered_fire_after_utc(&schedule, from, schedule_id, max_jitter_seconds)
}

fn compute_coalesced_next_fire_after_now_utc(
    schedule: &Schedule,
    now: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    if max_jitter_seconds <= 0 {
        return next_jittered_fire_after_utc(schedule, now, schedule_id, max_jitter_seconds);
    }

    let jitter_window_start = now
        .checked_sub_signed(Duration::seconds(i64::from(max_jitter_seconds) + 1))
        .unwrap_or(now);
    first_jittered_fire_after_utc(
        schedule,
        jitter_window_start,
        now,
        schedule_id,
        max_jitter_seconds,
    )
    .or_else(|| next_jittered_fire_after_utc(schedule, now, schedule_id, max_jitter_seconds))
}

fn first_jittered_fire_after_utc(
    schedule: &Schedule,
    from: DateTime<Utc>,
    after: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    first_jittered_fire_after_candidates_utc(
        schedule
            .after(&from)
            .take(SCHEDULE_STALE_CATCHUP_JITTER_SEARCH_LIMIT),
        after,
        schedule_id,
        max_jitter_seconds,
    )
}

fn first_jittered_fire_after_candidates_utc(
    candidates: impl Iterator<Item = DateTime<Utc>>,
    after: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    let mut best = None;
    for base_fire_at in candidates {
        // Cron bases are strictly increasing and jitter is nonnegative. Once a
        // base reaches the best adjusted fire, no remaining candidate can
        // produce an earlier timestamp; an equal timestamp preserves the same
        // observable tie result.
        if best.is_some_and(|best| base_fire_at >= best) {
            break;
        }

        let jittered_fire_at = apply_schedule_jitter(schedule_id, base_fire_at, max_jitter_seconds);
        if jittered_fire_at > after && best.is_none_or(|best| jittered_fire_at < best) {
            best = Some(jittered_fire_at);
        }
    }
    best
}

fn next_jittered_fire_after_utc(
    schedule: &Schedule,
    from: DateTime<Utc>,
    schedule_id: uuid::Uuid,
    max_jitter_seconds: i32,
) -> Option<DateTime<Utc>> {
    let next = schedule.after(&from).next()?;
    Some(apply_schedule_jitter(schedule_id, next, max_jitter_seconds))
}

fn apply_schedule_jitter(
    schedule_id: uuid::Uuid,
    next_fire_at: DateTime<Utc>,
    max_jitter_seconds: i32,
) -> DateTime<Utc> {
    // Jitter is intentionally non-negative; the stale coalescing search depends
    // on never moving a cron base earlier than its scheduled time.
    next_fire_at
        + Duration::seconds(schedule_jitter_seconds(
            schedule_id,
            next_fire_at,
            max_jitter_seconds,
        ))
}

fn schedule_jitter_seconds(
    schedule_id: uuid::Uuid,
    next_fire_at: DateTime<Utc>,
    max_jitter_seconds: i32,
) -> i64 {
    if max_jitter_seconds <= 0 {
        return 0;
    }

    let max_range = max_jitter_seconds as u128 + 1;
    let next_millis = next_fire_at.timestamp_millis() as u128;
    ((schedule_id.as_u128() ^ next_millis) % max_range) as i64
}

fn failed_schedule_retry_at(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::seconds(FAILED_SCHEDULE_RETRY_DELAY_SECONDS)
}

#[cfg(test)]
mod tests;
