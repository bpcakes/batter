use std::time::Duration;

use runledger_core::{
    jobs::{JobCompletion, JobContext, JobFailure, JobType},
    prelude::async_trait,
};
use runledger_runtime::{catalog::JobCatalog, registry::JobHandler};
use serde_json::{Value, json};
use sqlx::{PgPool, types::Uuid};
use tokio::sync::mpsc;

use super::{
    ProbeResult,
    transactions::{read_committed, request},
};

struct Witness(Option<mpsc::UnboundedSender<Uuid>>);

#[async_trait]
impl JobHandler for Witness {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("jobs.compatibility.witness")
    }

    async fn execute(
        &self,
        context: JobContext,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        if let Some(sender) = &self.0 {
            sender
                .send(context.job_id)
                .map_err(|_| JobFailure::terminal("witness.closed", "Witness receiver closed"))?;
        }
        Ok(JobCompletion::success())
    }
}

pub fn catalog(sender: Option<mpsc::UnboundedSender<Uuid>>) -> JobCatalog {
    JobCatalog::new().handler(Witness(sender))
}

pub async fn probe(pool: PgPool) -> ProbeResult {
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool).await?;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let catalog = catalog(Some(sender));
    catalog.sync_definitions(&pool).await?;
    let mut tx = read_committed(&pool).await?;
    let job = runledger_postgres::jobs::enqueue_job_with_outcome_tx(
        &mut tx,
        &request(&json!({"value": 1}), Uuid::from_u128(1)),
    )
    .await?;
    tx.commit().await?;
    let config = batter_example_reference_service::config::WorkerSettings::from_source(
        &batter::settings::SettingsSource::from_pairs([
            ("JOBS_WORKER_ID".into(), "compatibility-worker".into()),
            ("JOBS_POLL_INTERVAL_MS".into(), "20".into()),
            ("JOBS_CLAIM_BATCH_SIZE".into(), "1".into()),
            ("JOBS_LEASE_TTL_SECONDS".into(), "60".into()),
            ("JOBS_MAX_GLOBAL_CONCURRENCY".into(), "1".into()),
            ("JOBS_REAPER_INTERVAL_SECONDS".into(), "1".into()),
            ("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS".into(), "1".into()),
            ("JOBS_REAPER_RETRY_DELAY_MS".into(), "100".into()),
        ])?,
    )?;
    let supervisor = config.builder(&pool)?.with_catalog(&catalog).build()?;
    let stop = supervisor.shutdown_handle();
    let mut driver = tokio::spawn(
        supervisor.run_until_shutdown(std::future::pending(), Duration::from_secs(10)),
    );
    let witnessed = tokio::select! {
        exit = &mut driver => {
            // Retain the actual runtime/join failure, including before startup.
            exit??;
            return Err("worker exited successfully before its startup witness".into());
        }
        result = tokio::time::timeout(Duration::from_secs(20), receiver.recv()) => result,
    };
    // Always request and observe shutdown before inspecting the witness result.
    stop.request_shutdown();
    let joined = driver.await?;
    joined?;
    assert!(
        matches!(witnessed, Ok(Some(id)) if id == job.job_id),
        "controlled handler was not witnessed"
    );
    let status: String = sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
        .bind(job.job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SUCCEEDED");
    Ok(())
}
