//! Run-row lock order between workflow cancellation and external completion.
//!
//! Every multi-step workflow mutation takes the workflow-run row before any
//! workflow-step row, so cancellation waits on the run row while an external
//! completion holds it. The waits are observed through SQL markers in
//! `pg_stat_activity`; update the marker here if the production comment moves.

use std::time::Duration;

use runledger_core::jobs::{
    JobType, StepKey, WorkflowRunEnqueueBuilder, WorkflowStepEnqueueBuilder, WorkflowStepStatus,
    WorkflowType,
};
use runledger_postgres::jobs::{
    CompleteExternalWorkflowStepInput, ExternalWorkflowStepTerminalOutcome, cancel_workflow_run_tx,
    complete_external_workflow_step_tx, enqueue_workflow_run, list_workflow_steps,
};
use runledger_test_support::{
    setup_ephemeral_pool_with_untracked_migrations as setup_ephemeral_pool, teardown_ephemeral_pool,
};
use serde_json::json;
use tokio::time::{sleep, timeout};

use support::{query_error_code, register_job_definition};

mod support;

async fn wait_for_cancel_to_block_on_workflow_run_row(pool: &sqlx::PgPool) {
    // `current_database()` keeps this probe from observing another test's
    // intentionally blocked query on the shared PostgreSQL server.
    for _ in 0..100 {
        let waiting = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                 SELECT 1
                 FROM pg_stat_activity
                 WHERE datname = current_database()
                   AND wait_event_type = 'Lock'
                   AND query LIKE '%runledger:lock_workflow_run_row%'
             )",
        )
        .fetch_one(pool)
        .await
        .expect("query waiting cancel workflow run activity");
        if waiting {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("cancel workflow run did not block on the workflow run row");
}

#[tokio::test]
async fn external_completion_conflicts_when_cancel_waits_on_the_run_row() {
    let (pool, database) = setup_ephemeral_pool("workflow_external_cancel_run_row", 8).await;

    let job_type = JobType::new("jobs.test.workflow_external_cancel_run_row");
    register_job_definition(&pool, job_type).await;

    let payload = json!({"test": "workflow_external_cancel_run_row"});
    let metadata = json!({});
    let gate = WorkflowStepEnqueueBuilder::new_external(StepKey::new("gate"), &payload)
        .try_build()
        .expect("build external gate step");
    let dependent = WorkflowStepEnqueueBuilder::new(StepKey::new("dependent"), job_type, &payload)
        .depends_on_success(&[StepKey::new("gate")])
        .try_build()
        .expect("build dependent job step");
    let workflow = WorkflowRunEnqueueBuilder::new(
        WorkflowType::new("workflow.test.external_cancel_run_row"),
        &metadata,
    )
    .step(gate)
    .step(dependent)
    .try_build()
    .expect("build workflow");
    let run_id = enqueue_workflow_run(&pool, &workflow)
        .await
        .expect("enqueue workflow run")
        .id;

    // Hold what external completion holds before its release try-lock: the run
    // row, then every step row of the run.
    let mut external_tx = pool.begin().await.expect("begin external tx");
    sqlx::query("SELECT 1 FROM workflow_runs WHERE id = $1 FOR UPDATE")
        .bind(run_id)
        .execute(&mut *external_tx)
        .await
        .expect("hold workflow run row for external completion");
    sqlx::query("SELECT id FROM workflow_steps WHERE workflow_run_id = $1 ORDER BY id FOR UPDATE")
        .bind(run_id)
        .execute(&mut *external_tx)
        .await
        .expect("hold workflow step rows for external completion");

    let cancel_pool = pool.clone();
    let mut cancel_task = tokio::spawn(async move {
        let mut tx = cancel_pool.begin().await.expect("begin cancel tx");
        let result =
            cancel_workflow_run_tx(&mut tx, run_id, None, Some("test.cancel"), None, None).await;
        if result.is_ok() {
            tx.commit().await.expect("commit cancel tx");
        } else {
            tx.rollback().await.expect("rollback cancel tx");
        }
        result.map(|_| ())
    });

    // Cancellation takes the exclusive release lock before the run row, so it
    // now owns that lock while it waits.
    wait_for_cancel_to_block_on_workflow_run_row(&pool).await;

    let error = complete_external_workflow_step_tx(
        &mut external_tx,
        &CompleteExternalWorkflowStepInput {
            workflow_run_id: run_id,
            organization_id: None,
            step_key: StepKey::new("gate"),
            outcome: ExternalWorkflowStepTerminalOutcome::Succeeded { output: None },
            status_reason: None,
            last_error_code: None,
            last_error_message: None,
        },
    )
    .await
    .expect_err("cancel-owned release lock should make external completion conflict");
    assert_eq!(query_error_code(&error), Some("workflow.release_conflict"));
    external_tx
        .rollback()
        .await
        .expect("rollback external completion");

    match timeout(Duration::from_secs(5), &mut cancel_task).await {
        Ok(joined) => joined
            .expect("cancel task should not panic")
            .expect("cancel workflow run should succeed"),
        Err(_) => {
            cancel_task.abort();
            panic!("cancel should finish after external rollback");
        }
    }

    let steps = list_workflow_steps(&pool, None, run_id)
        .await
        .expect("list workflow steps");
    let step = |key: &str| {
        steps
            .iter()
            .find(|step| step.step_key.as_str() == key)
            .expect("workflow step exists")
    };
    assert_eq!(step("gate").status, WorkflowStepStatus::Canceled);
    assert_eq!(step("dependent").status, WorkflowStepStatus::Canceled);
    assert!(step("dependent").job_id.is_none());

    teardown_ephemeral_pool(pool, database).await;
}
