//! Explicit worker/persistence acceptance against an externally provisioned,
//! dedicated disposable PostgreSQL 18 database. No production database.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "jobs_live/contracts.rs"]
mod contracts;
#[path = "jobs_live/support.rs"]
mod support;
use support::*;

#[tokio::test]
#[ignore = "requires a dedicated disposable PostgreSQL 18 database"]
async fn native_worker_persists_every_gate_outcome_and_pin_identity() -> Result {
    let database = connect().await?;
    let pool = database.pool().clone();
    let body_pool = pool.clone();
    // Always close the pool even when a contract assertion panics. Each drive
    // also settles the owned supervisor before propagating its test failure.
    let body = tokio::spawn(async move {
        contracts::continuation_and_release(&body_pool).await?;
        contracts::retry_exhaustion(&body_pool).await?;
        contracts::drop_and_workflow_refusal(&body_pool).await?;
        contracts::keyed_pin_identity(&body_pool).await?;
        Ok::<_, BoxError>(())
    })
    .await;
    pool.close().await;
    body??;
    Ok(())
}
