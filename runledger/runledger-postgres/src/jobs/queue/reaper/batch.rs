//! The batch owns lock preparation and processing on the same transaction.
//! Successful preparation precedes every row savepoint; rolling a row back
//! cannot invalidate the retained run witness. No row can reattempt preparation
//! after another row has already mutated a workflow step.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::jobs::workflows::{WorkflowRunRowLock, lock_workflow_run_for_reaper_tx};
use crate::{DbTx, Error, Result, RollbackFailure};
use sqlx::types::Uuid;

use super::{
    ReapExpiredLeaseBatchOutcome, ReapExpiredLeaseRow, defer_failed_row,
    reap_expired_lease_row_with_savepoint_tx,
};

pub(super) async fn reap_rows(
    tx: &mut DbTx<'_>,
    rows: Vec<ReapExpiredLeaseRow>,
    retry_delay_ms: i32,
) -> Result<ReapExpiredLeaseBatchOutcome> {
    let job_ids: Vec<_> = rows.iter().map(|row| row.job_id).collect();
    // Do not join workflow_runs here: direct-job roles have only this metadata
    // access, and an empty result must not require workflow-run permissions.
    let associations: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT job_id, workflow_run_id FROM workflow_steps WHERE job_id = ANY($1::uuid[])",
    )
    .bind(&job_ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| {
        Error::from_query_sqlx_with_context("read reaper workflow associations", error)
    })?;
    let job_runs: HashMap<_, _> = associations.into_iter().collect();
    let run_ids: BTreeSet<_> = job_runs.values().copied().collect();
    let mut runs = BTreeMap::new();
    for run_id in run_ids {
        runs.insert(run_id, prepare_run(tx, run_id).await?);
    }

    let mut outcome = ReapExpiredLeaseBatchOutcome::default();
    for row in rows {
        let preparation = job_runs
            .get(&row.job_id)
            .map(|run| runs.get(run).expect("every associated run was prepared"));
        match preparation {
            Some(Err(error)) => {
                defer_failed_row(tx, &row, retry_delay_ms, &mut outcome, error).await?
            }
            Some(Ok(_)) | None => {
                reap_expired_lease_row_with_savepoint_tx(tx, &row, retry_delay_ms, &mut outcome)
                    .await?;
            }
        }
    }
    Ok(outcome)
}

// The outer Result covers savepoint failure (the batch cannot continue); the
// inner Result is a run-local acquisition failure, retained for every affected
// row. No error is turned into evidence that a job is not workflow-managed.
async fn prepare_run(tx: &mut DbTx<'_>, run: Uuid) -> Result<Result<WorkflowRunRowLock>> {
    sqlx::query("SAVEPOINT reaper_workflow_prepare")
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            Error::from_query_sqlx_with_context("create reaper workflow savepoint", error)
        })?;
    let prepared = lock_workflow_run_for_reaper_tx(tx, run).await;
    match prepared {
        Ok(lock) => {
            sqlx::query("RELEASE SAVEPOINT reaper_workflow_prepare")
                .execute(&mut **tx)
                .await
                .map_err(|error| {
                    Error::from_query_sqlx_with_context("release reaper workflow savepoint", error)
                })?;
            Ok(Ok(lock))
        }
        Err(operation) => {
            if let Err(rollback) = sqlx::raw_sql(
                "ROLLBACK TO SAVEPOINT reaper_workflow_prepare;
                 RELEASE SAVEPOINT reaper_workflow_prepare",
            )
            .execute(&mut **tx)
            .await
            {
                return Err(Error::RollbackFailure(Box::new(RollbackFailure {
                    operation,
                    rollback,
                })));
            }
            Ok(Err(operation))
        }
    }
}
