//! Direct-job worker execution, promotion, catalog sync and scheduled dispatch.

use super::policy::{Composition, PublicPolicy};
use super::schema;
use super::support::{Fixture, Result, quote, require, require_denied, require_within_policy};
use batter::runledger::RunledgerDatabase;
use batter::runledger::grants::RunledgerOperation;
use batter::runledger::native::core::jobs::{JobFailureKind, JobType, JobTypeName};
use batter::runledger::native::postgres::jobs::{
    self, JobCompletionUpdate, JobDefinitionUpsert, JobEnqueue, JobFailureUpdate, JobLeaseIdentity,
    JobOrdinaryProgressUpdate, JobRunningUpdate,
};
use serde_json::Value;
use std::time::Duration;

const JOB_TYPE: &str = "batter.grants.direct";
const RESOURCE_JOB_TYPE: &str = "batter.grants.resource";

fn definition(job_type: &str) -> JobDefinitionUpsert<'_> {
    JobDefinitionUpsert {
        job_type: JobType::new(job_type),
        version: 1,
        max_attempts: 2,
        default_timeout_seconds: 60,
        default_priority: 100,
        is_enabled: true,
    }
}

fn submission<'a>(job_type: &'a str, payload: &'a Value) -> JobEnqueue<'a> {
    JobEnqueue {
        job_type: JobType::new(job_type),
        organization_id: None,
        payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: None,
        stage: None,
    }
}

/// Register the two job definitions as the schema owner.
async fn register_definitions(owner: &RunledgerDatabase) -> Result {
    let mut transaction = owner.pool().begin().await?;
    for job_type in [JOB_TYPE, RESOURCE_JOB_TYPE] {
        jobs::upsert_job_definition_tx(&mut transaction, &definition(job_type)).await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn the_direct_job_worker_runs_every_selected_lifecycle_path() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs_schema = fixture.names.jobs.clone();
        let owner = fixture.names.owner.clone();
        let owner_database = schema::runledger_database(fixture, &owner, &jobs_schema, 2)?;
        register_definitions(&owner_database).await?;

        let role = Composition::new(PublicPolicy::Deny)
            .with_jobs(&jobs_schema, &[RunledgerOperation::DirectJobExecution])
            .compile()?;
        let login = fixture.login("worker", &role).await?;
        let probe = fixture.pool(&login, &[&jobs_schema]).await?;
        require_within_policy(&probe, &role).await?;
        let worker_database = schema::runledger_database(fixture, &login, &jobs_schema, 4)?;
        let worker = worker_database.pool();
        let payload = serde_json::json!({"direct": true});

        // Success: claim, start, heartbeat, progress, checkpoint, finish.
        let success_id = jobs::enqueue_job(owner_database.pool(), &submission(JOB_TYPE, &payload))
            .await?;
        let claimed = jobs::claim_jobs(worker, "grants-worker", 60, 10).await?;
        let claim = claimed
            .iter()
            .find(|record| record.id == success_id)
            .ok_or_else(|| super::support::fail("the worker could not claim its own job"))?;
        jobs::mark_job_running(
            worker,
            claim.id,
            claim.run_number,
            claim.attempt,
            "grants-worker",
            &JobRunningUpdate {
                progress_done: Some(0),
                progress_total: Some(2),
                checkpoint: Some(&payload),
            },
        )
        .await?;
        jobs::heartbeat_job(worker, claim.id, claim.run_number, claim.attempt, "grants-worker", 60)
            .await?;
        jobs::update_job_ordinary_progress(
            worker,
            claim.id,
            claim.run_number,
            claim.attempt,
            "grants-worker",
            &JobOrdinaryProgressUpdate {
                progress_done: Some(1),
                progress_total: Some(2),
                checkpoint: Some(&payload),
            },
        )
        .await?;
        jobs::complete_job_success(
            worker,
            claim.id,
            claim.run_number,
            claim.attempt,
            "grants-worker",
            Some(&JobCompletionUpdate {
                progress_done: Some(2),
                progress_total: Some(2),
                checkpoint: None,
                output: Some(&payload),
            }),
        )
        .await?;
        require_status(&probe, &jobs_schema, success_id, "SUCCEEDED").await?;

        // Retry then terminal failure with its dead-letter write.
        let failing_id = jobs::enqueue_job(owner_database.pool(), &submission(JOB_TYPE, &payload))
            .await?;
        for expected in ["PENDING", "DEAD_LETTERED"] {
            let claimed = jobs::claim_jobs(worker, "grants-worker", 60, 10).await?;
            let claim = claimed
                .iter()
                .find(|record| record.id == failing_id)
                .ok_or_else(|| super::support::fail("the failing job was not claimable"))?;
            jobs::complete_job_failure(
                worker,
                claim.id,
                claim.run_number,
                claim.attempt,
                "grants-worker",
                &JobFailureUpdate::new(
                    JobFailureKind::Retryable,
                    "grants.failed",
                    "retained failure",
                    Some(1),
                ),
            )
            .await?;
            require_status(&probe, &jobs_schema, failing_id, expected).await?;
        }
        let dead_lettered: i64 = scalar(
            &probe,
            format!(
                "SELECT count(*) FROM {}.job_dead_letters WHERE job_id = '{failing_id}'",
                quote(&jobs_schema)
            ),
        )
        .await?;
        require(dead_lettered == 1, "no dead-letter row was written")?;

        // Releasing an unstarted claim deletes its attempt row.
        let released_id = jobs::enqueue_job(owner_database.pool(), &submission(JOB_TYPE, &payload))
            .await?;
        // Only a worker prestart claim can be released as unstarted, which is the
        // path that follows a failed RUNNING persistence.
        let claimed = jobs::claim_prestart_jobs(worker, "grants-worker", 60, 10).await?;
        let claim = claimed
            .iter()
            .find(|record| record.id == released_id)
            .ok_or_else(|| super::support::fail("the releasable job was not claimable"))?;
        jobs::release_unstarted_job_claim(
            worker,
            JobLeaseIdentity::new(claim.id, claim.run_number, claim.attempt, "grants-worker"),
            "grants.release",
            1,
        )
        .await?;
        let attempts: i64 = scalar(
            &probe,
            format!(
                "SELECT count(*) FROM {}.job_attempts WHERE job_id = '{released_id}'",
                quote(&jobs_schema)
            ),
        )
        .await?;
        require(attempts == 0, "the unstarted claim's attempt row survived")?;

        // Execution resource claims are acquired and released by the trigger.
        let resource_id = jobs::enqueue_job_with_execution_resource(
            owner_database.pool(),
            &submission(RESOURCE_JOB_TYPE, &payload),
            "grants-resource",
        )
        .await?
        .job_id;
        let claimed = jobs::claim_jobs(worker, "grants-worker", 60, 10).await?;
        let claim = claimed
            .iter()
            .find(|record| record.id == resource_id)
            .ok_or_else(|| super::support::fail("the resource job was not claimable"))?;
        let held: i64 = scalar(
            &probe,
            format!(
                "SELECT count(*) FROM {}.job_execution_resource_claims WHERE resource_key = 'grants-resource'",
                quote(&jobs_schema)
            ),
        )
        .await?;
        require(held == 1, "no execution resource claim was recorded")?;
        jobs::complete_job_success(
            worker,
            claim.id,
            claim.run_number,
            claim.attempt,
            "grants-worker",
            None,
        )
        .await?;
        let released: i64 = scalar(
            &probe,
            format!(
                "SELECT count(*) FROM {}.job_execution_resource_claims WHERE resource_key = 'grants-resource'",
                quote(&jobs_schema)
            ),
        )
        .await?;
        require(
            released == 0,
            "the installed release trigger could not delete the resource claim",
        )?;

        // Reaping an expired lease is the worker's own authority.
        let reaped_id = jobs::enqueue_job(owner_database.pool(), &submission(JOB_TYPE, &payload))
            .await?;
        let claimed = jobs::claim_jobs(worker, "grants-reaper", 1, 10).await?;
        require(
            claimed.iter().any(|record| record.id == reaped_id),
            "the reapable job was not claimable",
        )?;
        tokio::time::sleep(Duration::from_millis(1_300)).await;
        // Reaping always runs its bounded coordination cleanup, whose failures are
        // diagnostics rather than errors, so assert on the detailed result.
        let reaped = jobs::reap_expired_leases_with_diagnostics(worker, 10, 1).await?;
        require(
            reaped.summary.processed >= 1,
            "the reaper processed no expired lease",
        )?;
        require(
            reaped.cleanup_errors.is_empty(),
            &format!("the reaper retained cleanup errors: {:?}", reaped.cleanup_errors),
        )?;
        require(
            reaped.deferred_row_errors.is_empty(),
            &format!("the reaper deferred rows: {:?}", reaped.deferred_row_errors),
        )?;

        // Authority the reviewed worker inventory excludes.
        for forbidden in [
            format!("DELETE FROM {}.job_events", quote(&jobs_schema)),
            format!("DELETE FROM {}.job_queue", quote(&jobs_schema)),
            format!("SELECT 1 FROM {}.job_logs", quote(&jobs_schema)),
            format!("DELETE FROM {}.job_logs", quote(&jobs_schema)),
            format!(
                "INSERT INTO {}.workflow_runs (workflow_type) VALUES ('x')",
                quote(&jobs_schema)
            ),
            format!(
                "INSERT INTO {}.job_definitions (job_type) VALUES ('x')",
                quote(&jobs_schema)
            ),
            format!("SELECT enqueue_request FROM {}.job_queue", quote(&jobs_schema)),
        ] {
            require_denied(&probe, &forbidden).await?;
        }
        fixture.track(owner_database.pool().clone());
        fixture.track(worker.clone());
        Ok(())
    })
    .await
}

async fn require_status(
    pool: &sqlx::PgPool,
    schema: &str,
    job: uuid::Uuid,
    expected: &str,
) -> Result {
    let observed: String = scalar(
        pool,
        format!(
            "SELECT status::text FROM {}.job_queue WHERE id = '{job}'",
            quote(schema)
        ),
    )
    .await?;
    require(
        observed == expected,
        &format!("job {job} is {observed}, expected {expected}"),
    )
}

async fn scalar<T>(pool: &sqlx::PgPool, sql: String) -> Result<T>
where
    T: for<'a> sqlx::Decode<'a, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(pool)
        .await?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn promotion_catalog_disable_and_due_scheduled_dispatch_run_under_their_selections() -> Result
{
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs_schema = fixture.names.jobs.clone();
        let owner = fixture.names.owner.clone();
        let owner_database = schema::runledger_database(fixture, &owner, &jobs_schema, 2)?;
        register_definitions(&owner_database).await?;

        let role = Composition::new(PublicPolicy::Deny)
            .with_jobs(
                &jobs_schema,
                &[
                    RunledgerOperation::CatalogSync,
                    RunledgerOperation::IntentPromotion,
                    RunledgerOperation::ScheduledDispatch,
                ],
            )
            .compile()?;
        let login = fixture.login("orchestrator", &role).await?;
        let probe = fixture.pool(&login, &[&jobs_schema]).await?;
        require_within_policy(&probe, &role).await?;
        let database = schema::runledger_database(fixture, &login, &jobs_schema, 2)?;
        let payload = serde_json::json!({"orchestrated": true});

        // Catalog synchronization registers the catalog and disables exactly the
        // definitions absent from it.
        let mut transaction = database.pool().begin().await?;
        let report = jobs::sync_catalog_job_definitions_exact_tx(
            &mut transaction,
            &[definition(JOB_TYPE)],
            &[
                JobTypeName::new(JOB_TYPE)?,
                JobTypeName::new(RESOURCE_JOB_TYPE)?,
            ],
        )
        .await?;
        transaction.commit().await?;
        require(
            report.disabled_absent_job_types.len() == 1,
            &format!("exact catalog disable did not report one job type: {report:?}"),
        )?;
        let enabled: bool = scalar(
            &probe,
            format!(
                "SELECT is_enabled FROM {}.job_definitions WHERE job_type = '{RESOURCE_JOB_TYPE}'",
                quote(&jobs_schema)
            ),
        )
        .await?;
        require(!enabled, "the absent definition was not disabled")?;

        // Durable promotion of an intent recorded by another login.
        let mut owner_transaction = owner_database.pool().begin().await?;
        let intent = batter::runledger::native::postgres::jobs::JobEnqueueIntent::new(
            JobType::new(JOB_TYPE),
            &payload,
            "promoted-by-grants",
        );
        let _recorded = jobs::record_job_enqueue_intent_tx(&mut owner_transaction, &intent).await?;
        owner_transaction.commit().await?;
        let promotion = jobs::promote_job_enqueue_intents_for_types(
            database.pool(),
            &[JobType::new(JOB_TYPE)],
            10,
        )
        .await?;
        require(
            promotion.total_promoted == 1,
            &format!("promotion did not enqueue the intent: {promotion:?}"),
        )?;

        // A due schedule is claimed, materialized and recorded as fired.
        let now = chrono_now(&probe).await?;
        let schedule = jobs::upsert_job_schedule(
            owner_database.pool(),
            &jobs::JobScheduleUpsert {
                name: "grants-schedule",
                job_type: JobType::new(JOB_TYPE),
                organization_id: None,
                payload_template: &payload,
                cron_expr: "0 * * * * *",
                is_active: true,
                next_fire_at: now,
                max_jitter_seconds: 0,
            },
        )
        .await?;
        let mut transaction = database.pool().begin().await?;
        let due = jobs::claim_due_schedules_tx(&mut transaction, now, 10).await?;
        require(
            due.iter().any(|record| record.id == schedule.id),
            "the due schedule was not claimable",
        )?;
        jobs::enqueue_job_tx(&mut transaction, &submission(JOB_TYPE, &payload)).await?;
        let marked = jobs::mark_schedule_fired_tx(
            &mut transaction,
            schedule.id,
            now,
            now + chrono::Duration::minutes(1),
        )
        .await?;
        transaction.commit().await?;
        require(marked, "the fired schedule was not recorded")?;

        // Authority outside the reviewed orchestration inventory.
        for forbidden in [
            format!("DELETE FROM {}.job_queue", quote(&jobs_schema)),
            format!("DELETE FROM {}.job_definitions", quote(&jobs_schema)),
            format!("DELETE FROM {}.job_schedules", quote(&jobs_schema)),
            format!("SELECT 1 FROM {}.job_attempts", quote(&jobs_schema)),
            format!("SELECT 1 FROM {}.job_dead_letters", quote(&jobs_schema)),
        ] {
            require_denied(&probe, &forbidden).await?;
        }
        fixture.track(owner_database.pool().clone());
        fixture.track(database.pool().clone());
        Ok(())
    })
    .await
}

async fn chrono_now(pool: &sqlx::PgPool) -> Result<chrono::DateTime<chrono::Utc>> {
    Ok(sqlx::query_scalar("SELECT now()").fetch_one(pool).await?)
}
