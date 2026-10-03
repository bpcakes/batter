//! Supported supervisor configurations over a direct-only workload.
//!
//! Both configurations are judged by the effects of every enabled loop, not by
//! readiness or an empty poll: a retained native permission or cleanup error
//! would leave one of these observations missing or the settlement unclean.

use super::policy::{Composition, PublicPolicy};
use super::schema;
use super::support::{Fixture, Result, quote, require, require_within_policy};
use async_trait::async_trait;
use batter::runledger::grants::RunledgerOperation;
use batter::runledger::native::core::jobs::{
    JobCompletion, JobContext, JobFailure, JobHandler, JobType,
};
use batter::runledger::native::postgres::jobs::{self, JobDefinitionUpsert, JobEnqueue};
use batter::runledger::native::runtime::registry::JobRegistry;
use batter::runledger::native::runtime::{
    RuntimeSettlement, RuntimeShutdownBudget, Supervisor, config::JobsConfig,
};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const DIRECT_TYPE: &str = "batter.grants.supervised";

struct CountingHandler {
    executed: Arc<AtomicUsize>,
}

#[async_trait]
impl JobHandler for CountingHandler {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DIRECT_TYPE)
    }

    async fn execute(
        &self,
        _context: JobContext,
        _payload: Value,
    ) -> std::result::Result<JobCompletion, JobFailure> {
        self.executed.fetch_add(1, Ordering::Relaxed);
        Ok(JobCompletion::success())
    }
}

fn config(worker: &str) -> JobsConfig {
    JobsConfig {
        worker_id: worker.to_owned(),
        poll_interval: Duration::from_millis(50),
        claim_batch_size: 4,
        lease_ttl_seconds: 60,
        max_global_concurrency: 2,
        reaper_interval: Duration::from_millis(50),
        schedule_poll_interval: Duration::from_millis(50),
        reaper_retry_delay_ms: 1,
    }
}

fn submission<'a>(payload: &'a Value) -> JobEnqueue<'a> {
    JobEnqueue {
        job_type: JobType::new(DIRECT_TYPE),
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

/// Wait until `probe` observes `expected`, or report the last observation.
async fn await_count(pool: &sqlx::PgPool, sql: String, expected: i64, label: &str) -> Result<bool> {
    for _ in 0..100 {
        let observed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.clone()))
            .fetch_one(pool)
            .await?;
        if observed >= expected {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let observed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(pool)
        .await?;
    Err(super::support::fail(&format!(
        "{label}: observed {observed}, expected at least {expected}"
    )))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a dedicated disposable PostgreSQL 18 cluster through BATTER_SQLX_ADMIN_URL"]
#[allow(clippy::too_many_lines)]
async fn scheduler_disabled_and_default_loop_supervisors_run_a_direct_workload() -> Result {
    Fixture::run(async |fixture| {
        schema::install(fixture).await?;
        let jobs_schema = fixture.names.jobs.clone();
        let owner = fixture.names.owner.clone();
        let owner_database = schema::runledger_database(fixture, &owner, &jobs_schema, 2)?;
        let mut transaction = owner_database.pool().begin().await?;
        jobs::upsert_job_definition_tx(
            &mut transaction,
            &JobDefinitionUpsert {
                job_type: JobType::new(DIRECT_TYPE),
                version: 1,
                max_attempts: 3,
                default_timeout_seconds: 60,
                default_priority: 100,
                is_enabled: true,
            },
        )
        .await?;
        transaction.commit().await?;
        let payload = serde_json::json!({"supervised": true});

        // The scheduler-disabled composition selects no scheduled dispatch.
        let narrow_role = Composition::new(PublicPolicy::Deny)
            .with_jobs(
                &jobs_schema,
                &[
                    RunledgerOperation::DirectJobExecution,
                    RunledgerOperation::IntentPromotion,
                ],
            )
            .compile()?;
        let narrow_login = fixture.login("supervisor_narrow", &narrow_role).await?;
        let narrow_probe = fixture.pool(&narrow_login, &[&jobs_schema]).await?;
        require_within_policy(&narrow_probe, &narrow_role).await?;

        // Give the reaper a real expired lease to recover, and the promoter a
        // real intent to promote, before the supervisor starts.
        let reapable = jobs::enqueue_job(owner_database.pool(), &submission(&payload)).await?;
        let claimed = jobs::claim_jobs(owner_database.pool(), "grants-stale", 1, 10).await?;
        require(
            claimed.iter().any(|record| record.id == reapable),
            "the reapable job was not claimable by the owner",
        )?;
        let mut owner_transaction = owner_database.pool().begin().await?;
        let _recorded = jobs::record_job_enqueue_intent_tx(
            &mut owner_transaction,
            &batter::runledger::native::postgres::jobs::JobEnqueueIntent::new(
                JobType::new(DIRECT_TYPE),
                &payload,
                "supervised-intent",
            ),
        )
        .await?;
        owner_transaction.commit().await?;
        tokio::time::sleep(Duration::from_millis(1_200)).await;

        let executed = Arc::new(AtomicUsize::new(0));
        let narrow_database = schema::runledger_database(fixture, &narrow_login, &jobs_schema, 6)?;
        let mut registry = JobRegistry::new();
        registry.register(CountingHandler {
            executed: Arc::clone(&executed),
        });
        let supervisor = Supervisor::builder(narrow_database.pool(), config("grants-narrow"))?
            .with_registry(registry)
            .disable_scheduler()
            .build()?;
        supervisor
            .startup_observer()
            .wait_initialized()
            .await
            .map_err(|error| super::support::fail(&format!("{error:?}")))?;

        let succeeded = format!(
            "SELECT count(*) FROM {}.job_queue WHERE status = 'SUCCEEDED'",
            quote(&jobs_schema)
        );
        // The worker, promoter and reaper loops each have to do real work: the
        // reapable job, the promoted intent and nothing else can reach SUCCEEDED.
        await_count(
            &narrow_probe,
            succeeded.clone(),
            2,
            "scheduler-disabled loops",
        )
        .await?;
        // The scheduler loop is disabled, so no schedule may be claimed here.
        let report = supervisor
            .shutdown_report(RuntimeShutdownBudget::new(
                Duration::from_secs(10),
                Duration::from_secs(2),
            )?)
            .await;
        let settlement = report.classify();
        require(
            matches!(settlement, RuntimeSettlement::Clean(_)),
            &format!("the scheduler-disabled supervisor did not settle cleanly: {settlement:?}"),
        )?;
        require(
            executed.load(Ordering::Relaxed) >= 2,
            "the scheduler-disabled worker executed no handler",
        )?;
        fixture.track(narrow_database.pool().clone());

        // The default-loop composition additionally selects scheduled dispatch
        // and catalog synchronization.
        let wide_role = Composition::new(PublicPolicy::Deny)
            .with_jobs(
                &jobs_schema,
                &[
                    RunledgerOperation::CatalogSync,
                    RunledgerOperation::DirectJobExecution,
                    RunledgerOperation::IntentPromotion,
                    RunledgerOperation::ScheduledDispatch,
                ],
            )
            .compile()?;
        let wide_login = fixture.login("supervisor_default", &wide_role).await?;
        let wide_probe = fixture.pool(&wide_login, &[&jobs_schema]).await?;
        require_within_policy(&wide_probe, &wide_role).await?;
        let due: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT now()")
            .fetch_one(&wide_probe)
            .await?;
        jobs::upsert_job_schedule(
            owner_database.pool(),
            &jobs::JobScheduleUpsert {
                name: "grants-supervised-schedule",
                job_type: JobType::new(DIRECT_TYPE),
                organization_id: None,
                payload_template: &payload,
                cron_expr: "0 * * * * *",
                is_active: true,
                next_fire_at: due,
                max_jitter_seconds: 0,
            },
        )
        .await?;

        let scheduled_executed = Arc::new(AtomicUsize::new(0));
        let wide_database = schema::runledger_database(fixture, &wide_login, &jobs_schema, 6)?;
        let mut registry = JobRegistry::new();
        registry.register(CountingHandler {
            executed: Arc::clone(&scheduled_executed),
        });
        let supervisor = Supervisor::builder(wide_database.pool(), config("grants-default"))?
            .with_registry(registry)
            .build()?;
        supervisor
            .startup_observer()
            .wait_initialized()
            .await
            .map_err(|error| super::support::fail(&format!("{error:?}")))?;
        // The scheduler loop materializes the due schedule and records the fire;
        // the worker loop then executes the job the scheduler created.
        // The scheduler needs no read of `last_fired_at`, so this observation
        // deliberately uses the owning connection rather than widening the
        // serving login's reviewed inventory.
        await_count(
            owner_database.pool(),
            format!(
                "SELECT count(*) FROM {}.job_schedules \
                 WHERE name = 'grants-supervised-schedule' AND last_fired_at IS NOT NULL",
                quote(&jobs_schema)
            ),
            1,
            "default-loop schedule fire",
        )
        .await?;
        await_count(&wide_probe, succeeded, 3, "default-loop scheduled dispatch").await?;
        let report = supervisor
            .shutdown_report(RuntimeShutdownBudget::new(
                Duration::from_secs(10),
                Duration::from_secs(2),
            )?)
            .await;
        let settlement = report.classify();
        require(
            matches!(settlement, RuntimeSettlement::Clean(_)),
            &format!("the default-loop supervisor did not settle cleanly: {settlement:?}"),
        )?;
        require(
            scheduled_executed.load(Ordering::Relaxed) >= 1,
            "the default-loop worker executed no scheduled handler",
        )?;

        // Every installed native trigger stayed trusted while no serving login
        // and no PUBLIC role held EXECUTE on its function.
        let executable: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM pg_catalog.pg_proc AS routine
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace
             WHERE namespace.nspname = '{}'
               AND (
                 pg_catalog.has_function_privilege('{}', routine.oid, 'EXECUTE')
                 OR pg_catalog.has_function_privilege('public', routine.oid, 'EXECUTE')
               )",
            jobs_schema.replace('\'', "''"),
            wide_login.replace('\'', "''"),
        )))
        .fetch_one(&wide_probe)
        .await?;
        require(
            executable == 0,
            &format!("{executable} native routines remained executable by a serving login"),
        )?;
        let triggers: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM pg_catalog.pg_trigger AS trigger_row
             JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger_row.tgrelid
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
             WHERE namespace.nspname = '{}' AND NOT trigger_row.tgisinternal",
            jobs_schema.replace('\'', "''"),
        )))
        .fetch_one(&wide_probe)
        .await?;
        require(triggers > 0, "the native triggers were not installed")?;
        fixture.track(owner_database.pool().clone());
        fixture.track(wide_database.pool().clone());
        Ok(())
    })
    .await
}
