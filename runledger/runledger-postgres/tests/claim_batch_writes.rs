use runledger_core::jobs::{
    JobStage, JobStatus, JobType, StepKey, WorkflowRunEnqueueBuilder, WorkflowStepEnqueueBuilder,
    WorkflowType,
};
use runledger_postgres::jobs::{
    JobQueueRecord, claim_jobs, claim_jobs_for_types, claim_prestart_jobs,
    claim_prestart_jobs_for_types, enqueue_workflow_run,
};
use runledger_postgres::{DbPool, Error};
use runledger_test_support::{setup_ephemeral_pool, teardown_ephemeral_pool};
use serde_json::{Value, json};

mod support;

const JOB_TYPE: &str = "jobs.test.claim_batch";
const WORKER: &str = "claim-batch-worker";

async fn seed_batch(pool: &DbPool) {
    support::register_test_job_definition(pool, JOB_TYPE).await;
    let payload = json!({"batch": true});
    let keys = (0..9).map(|i| format!("step-{i}")).collect::<Vec<_>>();
    let mut builder =
        WorkflowRunEnqueueBuilder::new(WorkflowType::new("workflow.test.claim_batch"), &payload);
    for (index, key) in keys.iter().enumerate() {
        builder = builder.step(
            WorkflowStepEnqueueBuilder::new(StepKey::new(key), JobType::new(JOB_TYPE), &payload)
                .stage(JobStage::Scheduled)
                .priority(if index == 8 { -100 } else { 100 })
                .execution_resource(key)
                .try_build()
                .expect("workflow step"),
        );
    }
    enqueue_workflow_run(pool, &builder.try_build().expect("workflow"))
        .await
        .expect("enqueue workflow");
    for _ in 0..8 {
        support::enqueue_test_job(pool, JOB_TYPE, None, &payload).await;
    }
    // Exercise different run/attempt identities in one batch, including retries.
    sqlx::raw_sql(
        "WITH numbered AS (
            SELECT id, row_number() OVER (ORDER BY id)::int AS n FROM job_queue
         )
         UPDATE job_queue j SET run_number = 1 + n % 3, attempt = n % 2
         FROM numbered WHERE j.id = numbered.id;",
    )
    .execute(pool)
    .await
    .expect("seed varied claim identities");
}

async fn install_statement_counter(pool: &DbPool) {
    sqlx::raw_sql(
        "CREATE TABLE claim_statements (table_name text NOT NULL);
         CREATE FUNCTION count_claim_statement() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN INSERT INTO claim_statements VALUES (TG_TABLE_NAME); RETURN NULL; END $$;
         CREATE TRIGGER count_claim_workflow AFTER UPDATE ON workflow_steps
             FOR EACH STATEMENT EXECUTE FUNCTION count_claim_statement();
         CREATE TRIGGER count_claim_attempt AFTER INSERT ON job_attempts
             FOR EACH STATEMENT EXECUTE FUNCTION count_claim_statement();
         CREATE TRIGGER count_claim_event AFTER INSERT ON job_events
             FOR EACH STATEMENT EXECUTE FUNCTION count_claim_statement();",
    )
    .execute(pool)
    .await
    .expect("install statement counters");
}

async fn assert_statement_count(pool: &DbPool, expected: i64) {
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT table_name, count(*) FROM claim_statements GROUP BY table_name ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .expect("statement counts");
    assert_eq!(
        counts,
        ["job_attempts", "job_events", "workflow_steps"].map(|table| (table.to_owned(), expected))
    );
}

async fn assert_audit(pool: &DbPool, jobs: &[JobQueueRecord], origin: &str) {
    for job in jobs {
        assert_eq!(job.status, JobStatus::Leased);
        assert_eq!(job.worker_id.as_deref(), Some(WORKER));
        let attempts: Vec<(i32, i32, String, bool)> = sqlx::query_as(
            "SELECT run_number, attempt, claim_origin,
                worker_id = $2 AND leased_at = $3 AND started_at = leased_at
                AND execution_started_persisted_at IS NULL AND finished_at IS NULL AS valid
             FROM job_attempts WHERE job_id = $1",
        )
        .bind(job.id)
        .bind(WORKER)
        .bind(job.last_heartbeat_at)
        .fetch_all(pool)
        .await
        .expect("attempt audit");
        assert_eq!(
            attempts,
            [(job.run_number, job.attempt, origin.to_owned(), true)]
        );
        let events: Vec<(i32, i32, String, Value, bool)> = sqlx::query_as(
            "SELECT run_number, attempt, stage, payload, occurred_at = $2
             FROM job_events WHERE job_id = $1 AND event_type = 'LEASED'",
        )
        .bind(job.id)
        .bind(job.last_heartbeat_at)
        .fetch_all(pool)
        .await
        .expect("lease event audit");
        assert_eq!(
            events,
            [(
                job.run_number,
                job.attempt,
                job.stage.as_db_value().to_owned(),
                json!({"worker_id": WORKER, "lease_duration_seconds": 30}),
                true,
            )]
        );
    }
}

async fn claim_variant(pool: &DbPool, prestart: bool, filtered: bool) -> Vec<JobQueueRecord> {
    let types = [JobType::new(JOB_TYPE)];
    match (prestart, filtered) {
        (false, false) => claim_jobs(pool, WORKER, 30, 16).await,
        (false, true) => claim_jobs_for_types(pool, WORKER, 30, 16, &types).await,
        (true, false) => claim_prestart_jobs(pool, WORKER, 30, 16).await,
        (true, true) => claim_prestart_jobs_for_types(pool, WORKER, 30, 16, &types).await,
    }
    .expect("claim batch")
}

#[tokio::test]
async fn batches_preserve_audit_and_use_three_statements_for_all_claim_variants() {
    for (prestart, filtered) in [(false, false), (false, true), (true, false), (true, true)] {
        let (pool, database) = setup_ephemeral_pool("claim_batch_audit", 1).await;
        seed_batch(&pool).await;
        install_statement_counter(&pool).await;
        let jobs = claim_variant(&pool, prestart, filtered).await;
        assert_eq!(jobs.len(), 16);
        assert_audit(
            &pool,
            &jobs,
            if prestart {
                "WORKER_PRESTART"
            } else {
                "DIRECT"
            },
        )
        .await;
        assert_statement_count(&pool, 1).await;
        let state: (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM job_attempts),
                (SELECT count(*) FROM job_events WHERE event_type = 'LEASED'),
                (SELECT count(*) FROM workflow_steps WHERE status = 'RUNNING'),
                (SELECT count(*) FROM workflow_steps WHERE status = 'ENQUEUED'),
                (SELECT count(*) FROM job_execution_resource_claims rc JOIN job_queue j
                    ON j.id = rc.job_id AND j.run_number = rc.run_number
                    AND j.attempt = rc.attempt AND j.worker_id = rc.worker_id
                    AND j.lease_expires_at = rc.lease_expires_at)",
        )
        .fetch_one(&pool)
        .await
        .expect("claim state");
        assert_eq!(state, (16, 16, 8, 1, 8));

        let last = claim_jobs(&pool, WORKER, 30, 16).await.expect("last job");
        assert_eq!(last.len(), 1);
        assert_audit(&pool, &last, "DIRECT").await;
        assert_statement_count(&pool, 2).await;
        assert!(
            claim_jobs(&pool, WORKER, 30, 16)
                .await
                .expect("empty")
                .is_empty()
        );
        assert_statement_count(&pool, 2).await;
        teardown_ephemeral_pool(pool, database).await;
    }
}

#[tokio::test]
async fn batch_workflow_update_preserves_status_guards_and_first_start() {
    let (pool, database) = setup_ephemeral_pool("claim_batch_workflow", 1).await;
    seed_batch(&pool).await;
    // Model pre-existing terminal and blocked steps to exercise the update fence.
    sqlx::raw_sql(
        "UPDATE workflow_steps SET
            status = CASE step_key
                WHEN 'step-0' THEN 'RUNNING'::workflow_step_status
                WHEN 'step-1' THEN 'SUCCEEDED'::workflow_step_status
                WHEN 'step-2' THEN 'FAILED'::workflow_step_status
                WHEN 'step-3' THEN 'CANCELED'::workflow_step_status
                WHEN 'step-4' THEN 'BLOCKED'::workflow_step_status
                ELSE status END,
            started_at = CASE WHEN step_key IN ('step-0', 'step-5')
                THEN '2025-01-01Z'::timestamptz ELSE NULL END,
            output = '{\"prior\": true}'::jsonb;",
    )
    .execute(&pool)
    .await
    .expect("seed workflow states");
    let before: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM workflow_steps s
         WHERE step_key IN ('step-1', 'step-2', 'step-3', 'step-4', 'step-8') ORDER BY step_key",
    )
    .fetch_all(&pool)
    .await
    .expect("guarded steps before");
    assert_eq!(
        claim_jobs(&pool, WORKER, 30, 16)
            .await
            .expect("claim")
            .len(),
        16
    );
    let after: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM workflow_steps s
         WHERE step_key IN ('step-1', 'step-2', 'step-3', 'step-4', 'step-8') ORDER BY step_key",
    )
    .fetch_all(&pool)
    .await
    .expect("guarded steps after");
    assert_eq!(after, before);
    let valid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_steps s JOIN job_queue j ON j.id = s.job_id
         WHERE s.status = 'RUNNING' AND s.output IS NULL
           AND s.updated_at = j.updated_at
           AND s.started_at = CASE WHEN s.step_key IN ('step-0', 'step-5')
               THEN '2025-01-01Z'::timestamptz ELSE j.last_heartbeat_at END",
    )
    .fetch_one(&pool)
    .await
    .expect("updated workflow fields");
    assert_eq!(valid, 4);
    teardown_ephemeral_pool(pool, database).await;
}

async fn durable_snapshot(pool: &DbPool) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'jobs', (SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM job_queue j),
            'steps', (SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM workflow_steps s),
            'attempts', (SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM job_attempts a),
            'events', (SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM job_events e),
            'resources', (SELECT jsonb_agg(to_jsonb(r) ORDER BY resource_key)
                FROM job_execution_resource_claims r))",
    )
    .fetch_all(pool)
    .await
    .expect("durable claim snapshot")
}

#[tokio::test]
async fn every_batch_write_failure_rolls_back_leases_resources_workflows_and_audit() {
    let (pool, database) = setup_ephemeral_pool("claim_batch_rollback", 1).await;
    seed_batch(&pool).await;
    let before = durable_snapshot(&pool).await;
    for (table, action) in [
        ("workflow_steps", "UPDATE"),
        ("job_attempts", "INSERT"),
        ("job_events", "INSERT"),
    ] {
        // An AFTER STATEMENT failure happens after all rows have been written.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION reject_claim_batch() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION 'injected claim batch failure'; END $$;
             CREATE TRIGGER reject_claim_batch AFTER {action} ON {table}
             FOR EACH STATEMENT EXECUTE FUNCTION reject_claim_batch();"
        )))
        .execute(&pool)
        .await
        .expect("install batch fault");
        let error = claim_jobs(&pool, WORKER, 30, 16)
            .await
            .expect_err("claim fails");
        assert!(matches!(error, Error::QueryError(_)));
        assert_eq!(durable_snapshot(&pool).await, before, "failed {table}");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER reject_claim_batch ON {table}; DROP FUNCTION reject_claim_batch();"
        )))
        .execute(&pool)
        .await
        .expect("remove batch fault");
    }
    assert_eq!(
        claim_jobs(&pool, WORKER, 30, 16)
            .await
            .expect("claim after rollback")
            .len(),
        16
    );
    teardown_ephemeral_pool(pool, database).await;
}
