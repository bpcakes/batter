//! Lock-order regressions for workflow completion.
//!
//! Completion and reaper batches take the workflow-run row `FOR UPDATE`
//! before any workflow-step row. Concurrency tests park transactions at a gate held by
//! a third session, so the interleaving that used to deadlock is deterministic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use runledger_core::jobs::{
    JobType, StepKey, WorkflowRunEnqueueBuilder, WorkflowRunStatus, WorkflowStepEnqueueBuilder,
    WorkflowStepStatus, WorkflowType,
};
use runledger_postgres::jobs::{
    CompleteExternalWorkflowStepInput, ExternalWorkflowStepTerminalOutcome, JobDefinitionUpsert,
    JobQueueRecord, WorkflowRunReadScope, claim_jobs_for_types, complete_external_workflow_step,
    complete_job_success, enqueue_workflow_run, get_workflow_run_by_id_with_scope,
    list_workflow_steps, upsert_job_definition_tx,
};
use runledger_postgres::{DbPool, Error};
use runledger_test_support::{setup_ephemeral_pool, teardown_ephemeral_pool};
use serde_json::json;
use sqlx::types::Uuid;
use tokio::sync::Barrier;
use tokio::task::JoinHandle;

const DEADLOCK_DETECTED: &str = "40P01";

#[path = "workflow_lock_order/preparation.rs"]
mod preparation;
#[path = "workflow_lock_order/reaper.rs"]
mod reaper;

async fn register_job_definition(pool: &DbPool, job_type: JobType<'static>) {
    let mut tx = pool.begin().await.expect("begin job definition setup");
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type,
            version: 1,
            max_attempts: 3,
            default_timeout_seconds: 60,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await
    .expect("upsert workflow job definition");
    tx.commit().await.expect("commit job definition setup");
}

// Mirrors the crate-private workflow-run release advisory lock key.
fn workflow_run_release_lock_key(workflow_run_id: Uuid) -> i64 {
    const NAMESPACE: u64 = 0x7275_6e6c_9e37_79b9;
    let value = workflow_run_id.as_u128();
    ((value >> 64) as u64 ^ value as u64 ^ NAMESPACE) as i64
}

async fn lock_waiters(pool: &DbPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
         FROM pg_stat_activity
         WHERE datname = current_database()
           AND wait_event_type = 'Lock'
           AND pid <> pg_backend_pid()",
    )
    .fetch_one(pool)
    .await
    .expect("count lock waiters")
}

async fn wait_for_lock_waiters(pool: &DbPool, expected: i64, waiting_for: &str) {
    let started = Instant::now();
    while lock_waiters(pool).await < expected {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {expected} lock waiter(s): {waiting_for}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn spawn_job_success(pool: &DbPool, job: JobQueueRecord) -> JoinHandle<Result<(), Error>> {
    let pool = pool.clone();
    tokio::spawn(async move {
        let worker_id = job.worker_id.clone().expect("claimed job has a worker id");
        complete_job_success(&pool, job.id, job.run_number, job.attempt, &worker_id, None)
            .await
            .map(|_| ())
    })
}

fn assert_not_deadlocked(result: &Result<(), Error>, operation: &str) {
    if let Err(Error::QueryError(error)) = result {
        assert_ne!(
            error.sqlstate(),
            Some(DEADLOCK_DETECTED),
            "{operation} lost a deadlock: {}",
            error.internal_message()
        );
    }
}

async fn step_status(pool: &DbPool, run_id: Uuid, key: &str) -> WorkflowStepStatus {
    list_workflow_steps(pool, None, run_id)
        .await
        .expect("list workflow steps")
        .into_iter()
        .find(|step| step.step_key.as_str() == key)
        .expect("workflow step exists")
        .status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disjoint_branch_completions_do_not_deadlock_on_the_run_row() {
    let (pool, database) = setup_ephemeral_pool("workflow_lock_order_disjoint", 8).await;
    let job_type = JobType::new("jobs.test.workflow_lock_order_disjoint");
    register_job_definition(&pool, job_type).await;
    let payload = json!({});
    let metadata = json!({});
    let roots = [StepKey::new("root-a"), StepKey::new("root-b")];
    let mut builder = WorkflowRunEnqueueBuilder::new(
        WorkflowType::new("workflow.test.lock_order_disjoint"),
        &metadata,
    );
    for (root, dependent) in roots.into_iter().zip(["dependent-a", "dependent-b"]) {
        builder = builder
            .step(
                WorkflowStepEnqueueBuilder::new(root, job_type, &payload)
                    .try_build()
                    .expect("build branch root"),
            )
            .step(
                WorkflowStepEnqueueBuilder::new_external(StepKey::new(dependent), &payload)
                    .depends_on_success(&[root])
                    .try_build()
                    .expect("build branch dependent"),
            );
    }
    let run = enqueue_workflow_run(&pool, &builder.try_build().expect("build workflow"))
        .await
        .expect("enqueue disjoint workflow");
    let jobs = claim_jobs_for_types(&pool, "lock-order-worker", 30, 2, &[job_type])
        .await
        .expect("claim both roots");
    assert_eq!(jobs.len(), 2);

    // FOR KEY SHARE is compatible with FOR SHARE but not FOR UPDATE. Before the
    // fix, each completion took the run row FOR SHARE while releasing its
    // dependent and parked here before upgrading to FOR UPDATE, so opening the
    // gate made the two upgrades wait on each other.
    let mut gate = pool.begin().await.expect("begin gate");
    sqlx::query("SELECT 1 FROM workflow_runs WHERE id = $1 FOR KEY SHARE")
        .bind(run.id)
        .execute(&mut *gate)
        .await
        .expect("hold run row gate");
    let barrier = Arc::new(Barrier::new(jobs.len()));
    let completions = jobs
        .into_iter()
        .map(|job| {
            let barrier = Arc::clone(&barrier);
            let pool = pool.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                spawn_job_success(&pool, job)
                    .await
                    .expect("completion task")
            })
        })
        .collect::<Vec<_>>();
    wait_for_lock_waiters(&pool, 2, "both completions parked at the run-row gate").await;
    gate.commit().await.expect("open gate");

    for completion in completions {
        let result = completion.await.expect("completion task joins");
        assert_not_deadlocked(&result, "root completion");
        result.expect("root completion succeeds");
    }
    for key in ["root-a", "root-b"] {
        assert_eq!(
            step_status(&pool, run.id, key).await,
            WorkflowStepStatus::Succeeded
        );
    }
    for key in ["dependent-a", "dependent-b"] {
        assert_eq!(
            step_status(&pool, run.id, key).await,
            WorkflowStepStatus::WaitingForExternal
        );
    }
    let run = get_workflow_run_by_id_with_scope(&pool, WorkflowRunReadScope::Global, run.id)
        .await
        .expect("load workflow run")
        .expect("workflow run exists");
    assert_eq!(run.status, WorkflowRunStatus::WaitingForExternal);
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whole_run_step_lockers_do_not_deadlock_with_a_dependent_declared_first() {
    let (pool, database) = setup_ephemeral_pool("workflow_lock_order_inverted", 8).await;
    let job_type = JobType::new("jobs.test.workflow_lock_order_inverted");
    register_job_definition(&pool, job_type).await;
    let payload = json!({});
    let metadata = json!({});
    // Declaring the dependent first gives it the smaller step id, so step-id
    // order disagrees with dependency order.
    let workflow = WorkflowRunEnqueueBuilder::new(
        WorkflowType::new("workflow.test.lock_order_inverted"),
        &metadata,
    )
    .step(
        WorkflowStepEnqueueBuilder::new_external(StepKey::new("dependent"), &payload)
            .depends_on_success(&[StepKey::new("prerequisite")])
            .try_build()
            .expect("build dependent"),
    )
    .step(
        WorkflowStepEnqueueBuilder::new(StepKey::new("prerequisite"), job_type, &payload)
            .try_build()
            .expect("build prerequisite"),
    )
    .step(
        WorkflowStepEnqueueBuilder::new_external(StepKey::new("gate"), &payload)
            .try_build()
            .expect("build external gate step"),
    )
    .try_build()
    .expect("build workflow");
    let run = enqueue_workflow_run(&pool, &workflow)
        .await
        .expect("enqueue workflow");
    let steps = list_workflow_steps(&pool, None, run.id)
        .await
        .expect("list steps");
    let id_of = |key: &str| {
        steps
            .iter()
            .find(|step| step.step_key.as_str() == key)
            .expect("fixture step exists")
            .id
    };
    assert!(
        id_of("dependent") < id_of("prerequisite"),
        "fixture needs inverted ids"
    );
    let job = claim_jobs_for_types(&pool, "lock-order-worker", 30, 1, &[job_type])
        .await
        .expect("claim prerequisite")
        .pop()
        .expect("prerequisite job claimed");

    // Holding the exclusive release lock parks job completion inside its lock
    // sequence. Before the fix it already held the prerequisite step there, and
    // external completion then locked the dependent and waited on the
    // prerequisite; opening the gate closed the cycle.
    let mut gate = pool.begin().await.expect("begin gate");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(workflow_run_release_lock_key(run.id))
        .execute(&mut *gate)
        .await
        .expect("hold release lock gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("gate backend");
    let job_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("job completion pool");
    let external_owner = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await
        .expect("external completion pool");
    let job_pid = reaper::backend(&job_pool).await;
    let external_pid = reaper::backend(&external_owner).await;
    let job_completion = spawn_job_success(&job_pool, job);
    let job_reached_gate = reaper::wait_blocked(&pool, job_pid, gate_pid).await;
    let external_pool = external_owner.clone();
    let mut external_completion = tokio::spawn(async move {
        complete_external_workflow_step(
            &external_pool,
            &CompleteExternalWorkflowStepInput {
                workflow_run_id: run.id,
                organization_id: None,
                step_key: StepKey::new("gate"),
                outcome: ExternalWorkflowStepTerminalOutcome::Succeeded { output: None },
                status_reason: None,
                last_error_code: None,
                last_error_message: None,
            },
        )
        .await
        .map(|_| ())
    });
    // Fixed code returns a release conflict while the gate is held. The old
    // order waits on the job's step. Observe either, never merely elapsed time.
    let rendezvous = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if external_completion.is_finished() {
                return Some((&mut external_completion).await);
            }
            if reaper::blocked_by(&pool, external_pid, job_pid).await {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let synchronized = rendezvous.is_ok();
    let job_still_parked = reaper::blocked_by(&pool, job_pid, gate_pid).await;
    gate.commit().await.expect("open gate");

    let job_result = reaper::joined(job_completion).await;
    let returned_under_gate = matches!(&rendezvous, Ok(Some(_)));
    let external_result = match rendezvous {
        Ok(Some(result)) => result.map_err(|_| "external completion join failed"),
        _ => reaper::joined(external_completion).await,
    };
    job_pool.close().await;
    external_owner.close().await;
    let job_result = job_result.expect("job completion joins");
    let external_result = external_result.expect("external completion joins");
    assert!(
        job_reached_gate && synchronized && job_still_parked,
        "interleaving must be observed before opening the gate"
    );
    if returned_under_gate {
        assert!(matches!(&external_result, Err(Error::QueryError(error))
            if error.code() == "workflow.release_conflict"));
    }
    assert_not_deadlocked(&job_result, "job completion");
    assert_not_deadlocked(&external_result, "external completion");
    job_result.expect("job completion succeeds");
    match external_result {
        Ok(()) => {}
        // The gate is indistinguishable from an in-progress cancellation, which
        // external completion reports instead of waiting for.
        Err(Error::QueryError(error)) if error.code() == "workflow.release_conflict" => {}
        Err(error) => panic!("unexpected external completion failure: {error:?}"),
    }
    assert_eq!(
        step_status(&pool, run.id, "prerequisite").await,
        WorkflowStepStatus::Succeeded
    );
    assert_eq!(
        step_status(&pool, run.id, "dependent").await,
        WorkflowStepStatus::WaitingForExternal
    );
    teardown_ephemeral_pool(pool, database).await;
}
