use super::{
    ProbeResult,
    transactions::{read_committed, request},
};
use batter::settings::SettingsSource;
use batter_example_reference_service::config::WorkerSettings;
use runledger_core::{
    jobs::{JobCompletion, JobContext, JobFailure, JobType},
    prelude::async_trait,
};
use runledger_runtime::{catalog::JobCatalog, registry::JobHandler};
use serde_json::{Value, json};
use sqlx::{PgPool, types::Uuid};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};

struct HeldWitness {
    started: mpsc::UnboundedSender<Uuid>,
    release: Arc<Semaphore>,
}
#[async_trait]
impl JobHandler for HeldWitness {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("jobs.compatibility.witness")
    }
    async fn execute(
        &self,
        context: JobContext,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        self.started
            .send(context.job_id)
            .map_err(|_| JobFailure::terminal("witness.closed", "receiver closed"))?;
        self.release
            .acquire()
            .await
            .map_err(|_| JobFailure::terminal("witness.closed", "release closed"))?
            .forget();
        Ok(JobCompletion::success())
    }
}

pub async fn probe(pool: PgPool) -> ProbeResult {
    runledger_postgres::migrate_after_idempotency_cutover(&pool).await?;
    runledger_postgres::ensure_schema_compatible_after_idempotency_cutover(&pool).await?;
    for limit in [1, 2] {
        one_capacity(&pool, limit).await?;
    }
    Ok(())
}

async fn one_capacity(pool: &PgPool, limit: usize) -> ProbeResult {
    let (started, mut receiver) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let catalog = JobCatalog::new().handler(HeldWitness {
        started,
        release: release.clone(),
    });
    catalog.sync_definitions(pool).await?;
    let total = limit + 1;
    let mut ids = HashSet::new();
    for index in 0..total {
        let payload = json!({"capacity": limit, "index": index});
        let mut tx = read_committed(pool).await?;
        let job = runledger_postgres::jobs::enqueue_job_with_outcome_tx(
            &mut tx,
            &request(&payload, Uuid::from_u128((limit * 10 + index) as u128)),
        )
        .await?;
        tx.commit().await?;
        ids.insert(job.job_id);
    }
    let settings = WorkerSettings::from_source(&SettingsSource::from_pairs([
        (
            "JOBS_WORKER_ID".into(),
            format!("capacity-worker-{limit}").into(),
        ),
        (
            "JOBS_MAX_GLOBAL_CONCURRENCY".into(),
            limit.to_string().into(),
        ),
        ("JOBS_CLAIM_BATCH_SIZE".into(), "4".into()),
        ("JOBS_POLL_INTERVAL_MS".into(), "20".into()),
    ])?)?;
    let supervisor = settings.builder(pool)?.with_catalog(&catalog).build()?;
    let stop = supervisor.shutdown_handle();
    let mut driver = tokio::spawn(
        supervisor.run_until_shutdown(std::future::pending(), Duration::from_secs(10)),
    );
    let exercise = async {
        let mut witnessed = HashSet::new();
        for _ in 0..limit {
            let id = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
                .await?
                .ok_or("worker witness closed")?;
            if !ids.contains(&id) || !witnessed.insert(id) {
                return Err("unexpected or repeated worker witness".into());
            }
        }
        // Pinned native claim commits the whole batch before spawning handlers.
        // All jobs are eligible and the batch exceeds this set, so excess capacity
        // leases the extra row even when that handler has not been scheduled yet.
        held_leases(pool, &ids, limit).await?;
        release.add_permits(total);
        let id = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await?
            .ok_or("worker witness closed")?;
        if !ids.contains(&id) || !witnessed.insert(id) {
            return Err("unexpected final worker witness".into());
        }
        if witnessed != ids {
            return Err("worker did not execute the complete configured set".into());
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    };
    let observed = tokio::select! {
        result = exercise => result,
        exit = &mut driver => {
            release.add_permits(total);
            exit??;
            return Err("worker exited before configured-capacity evidence".into());
        }
    };
    // Release on every observation failure, then request and join before returning.
    release.add_permits(total);
    stop.request_shutdown();
    let joined = driver
        .await
        .map_err(|error| Box::new(error) as batter::BoxError)
        .and_then(|result| result.map_err(|error| Box::new(error) as batter::BoxError));
    batter_test_support::finish(
        observed.map_err(super::fixture_diagnostics::ProbeError::new),
        joined.map_err(super::fixture_diagnostics::ProbeError::new),
    )?;
    let statuses: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, status::text FROM job_queue WHERE id = ANY($1)")
            .bind(ids.iter().copied().collect::<Vec<_>>())
            .fetch_all(pool)
            .await?;
    if statuses.len() != total || statuses.iter().any(|(_, status)| status != "SUCCEEDED") {
        return Err("independent job outcomes were not all SUCCEEDED".into());
    }
    Ok(())
}

async fn held_leases(pool: &PgPool, ids: &HashSet<Uuid>, limit: usize) -> ProbeResult {
    let leased: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_queue WHERE id = ANY($1) AND status = 'LEASED'",
    )
    .bind(ids.iter().copied().collect::<Vec<_>>())
    .fetch_one(pool)
    .await?;
    if leased != i64::try_from(limit)? {
        return Err("configured worker lease capacity did not match held handlers".into());
    }
    Ok(())
}
