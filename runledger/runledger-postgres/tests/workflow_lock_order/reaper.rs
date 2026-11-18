use super::*;
use runledger_postgres::jobs::{
    claim_prestart_jobs_for_types, reap_expired_leases_with_diagnostics,
};
use sqlx::postgres::PgPoolOptions;

pub(super) async fn backend(pool: &DbPool) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pool)
        .await
        .expect("backend id")
}

pub(super) async fn blocked_by(pool: &DbPool, waiter: i32, blocker: i32) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
         WHERE datname = current_database() AND pid = $1
           AND wait_event_type = 'Lock' AND $2 = ANY(pg_blocking_pids(pid)))",
    )
    .bind(waiter)
    .bind(blocker)
    .fetch_one(pool)
    .await
    .expect("observe blocker")
}

pub(super) async fn wait_blocked(pool: &DbPool, waiter: i32, blocker: i32) -> bool {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !blocked_by(pool, waiter, blocker).await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

pub(super) async fn joined<T>(mut task: JoinHandle<T>) -> Result<T, &'static str> {
    match tokio::time::timeout(Duration::from_secs(10), &mut task).await {
        Ok(result) => result.map_err(|_| "task join failed"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err("task did not settle after gate release")
        }
    }
}

async fn enqueue_mixed_batch(pool: &DbPool, prestart: bool) -> (Uuid, Uuid) {
    let job_type = JobType::new("jobs.test.workflow_reaper_mixed");
    register_job_definition(pool, job_type).await;
    let payload = json!({});
    let metadata = json!({});
    let workflow =
        WorkflowRunEnqueueBuilder::new(WorkflowType::new("workflow.test.reaper_mixed"), &metadata)
            .step(
                WorkflowStepEnqueueBuilder::new(StepKey::new("retry"), job_type, &payload)
                    .max_attempts(3)
                    .try_build()
                    .expect("retry step"),
            )
            .step(
                WorkflowStepEnqueueBuilder::new(StepKey::new("terminal"), job_type, &payload)
                    .max_attempts(1)
                    .try_build()
                    .expect("terminal step"),
            )
            .step(
                WorkflowStepEnqueueBuilder::new_external(StepKey::new("external"), &payload)
                    .try_build()
                    .expect("external step"),
            )
            .try_build()
            .expect("workflow");
    let run = enqueue_workflow_run(pool, &workflow)
        .await
        .expect("enqueue");
    let steps = list_workflow_steps(pool, None, run.id)
        .await
        .expect("steps");
    let retry = steps
        .iter()
        .find(|s| s.step_key.as_str() == "retry")
        .and_then(|s| s.job_id)
        .expect("retry job");
    let terminal = steps
        .iter()
        .find(|s| s.step_key.as_str() == "terminal")
        .and_then(|s| s.job_id)
        .expect("terminal job");
    let claimed = if prestart {
        claim_prestart_jobs_for_types(pool, "reaper-order-worker", 30, 2, &[job_type]).await
    } else {
        claim_jobs_for_types(pool, "reaper-order-worker", 30, 2, &[job_type]).await
    }
    .expect("claim");
    assert_eq!(claimed.len(), 2);
    // Even in the prestart case the second row follows the terminal path.
    sqlx::query("UPDATE job_attempts SET execution_started_persisted_at = now() WHERE job_id = $1")
        .bind(terminal)
        .execute(pool)
        .await
        .expect("mark terminal attempt started");
    sqlx::query(
        "UPDATE job_queue SET lease_expires_at = now() -
        CASE WHEN id = $1 THEN interval '20 seconds' ELSE interval '10 seconds' END
        WHERE id IN ($1, $2)",
    )
    .bind(retry)
    .bind(terminal)
    .execute(pool)
    .await
    .expect("expire in order");

    (run.id, retry)
}

async fn mixed_batch(prestart: bool) {
    let (pool, database) = setup_ephemeral_pool("workflow_reaper_mixed_order", 8).await;
    let (run_id, retry) = enqueue_mixed_batch(&pool, prestart).await;
    run_mixed_race(&pool, database.url(), run_id, retry).await;
    assert_eq!(
        step_status(&pool, run_id, "retry").await,
        WorkflowStepStatus::Enqueued
    );
    assert_eq!(
        step_status(&pool, run_id, "terminal").await,
        WorkflowStepStatus::Failed
    );
    teardown_ephemeral_pool(pool, database).await;
}

async fn hold_retry_gate(pool: &DbPool, retry: Uuid) -> (runledger_postgres::DbTx<'_>, i32) {
    const GATE: i64 = 991_381;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION test_gate_reaper_step() RETURNS trigger LANGUAGE plpgsql AS $body$
         BEGIN IF NEW.job_id = '{retry}'::uuid AND NEW.status = 'ENQUEUED' THEN
           PERFORM pg_advisory_xact_lock({GATE}); END IF; RETURN NEW; END $body$;
         CREATE TRIGGER test_gate_reaper_step AFTER UPDATE ON workflow_steps
         FOR EACH ROW EXECUTE FUNCTION test_gate_reaper_step();"
    )))
    .execute(pool)
    .await
    .expect("install step gate");
    let mut gate = pool.begin().await.expect("gate transaction");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate backend");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *gate)
        .await
        .expect("hold gate");
    (gate, gate_pid)
}

async fn run_mixed_race(pool: &DbPool, url: &str, run_id: Uuid, retry: Uuid) {
    let (gate, gate_pid) = hold_retry_gate(pool, retry).await;
    let reaper_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .expect("reaper pool");
    let external_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .expect("external pool");
    let reaper_pid = backend(&reaper_pool).await;
    let external_pid = backend(&external_pool).await;
    let reaper = tokio::spawn({
        let pool = reaper_pool.clone();
        async move { reap_expired_leases_with_diagnostics(&pool, 2, 60_000).await }
    });
    let reaper_parked = wait_blocked(pool, reaper_pid, gate_pid).await;
    let external = tokio::spawn({
        let pool = external_pool.clone();
        async move {
            complete_external_workflow_step(
                &pool,
                &CompleteExternalWorkflowStepInput {
                    workflow_run_id: run_id,
                    organization_id: None,
                    step_key: StepKey::new("external"),
                    outcome: ExternalWorkflowStepTerminalOutcome::Succeeded { output: None },
                    status_reason: None,
                    last_error_code: None,
                    last_error_message: None,
                },
            )
            .await
        }
    });
    // Fixed code waits on the run; the original waits on the first step. Both
    // must actually reach their wait before opening the gate.
    let external_parked = wait_blocked(pool, external_pid, reaper_pid).await;
    gate.rollback().await.expect("open step gate");
    let reaped = joined(reaper).await;
    let completed = joined(external).await;
    reaper_pool.close().await;
    external_pool.close().await;
    assert!(
        reaper_parked && external_parked,
        "both waits must be observed"
    );
    let reaped = reaped.expect("reaper joins").expect("batch commits");
    completed
        .expect("external joins")
        .expect("external completion does not deadlock");
    assert_eq!(reaped.summary.processed, 2);
    assert_eq!(reaped.deferred_row_error_count, 0);
    assert_eq!(reaped.summary.terminal_dead_lettered.len(), 1);
}

#[tokio::test]
async fn retry_then_terminal_batch_does_not_deadlock_with_external_completion() {
    mixed_batch(false).await;
}

#[tokio::test]
async fn unstarted_release_then_terminal_batch_does_not_deadlock_with_external_completion() {
    mixed_batch(true).await;
}
