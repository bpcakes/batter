use super::{
    ProbeResult,
    fixture_diagnostics::ProbeError,
    transactions::{read_committed, request},
};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    settings::SettingsSource,
};
use batter_example_reference_service::{
    config::WorkerSettings,
    worker::{TerminationGate, TerminationState, WorkerHost},
};
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
    let ids = enqueue_jobs(pool, limit, total).await?;

    let settings = configured_settings(limit)?;
    let gate = TerminationGate::new();
    let host = WorkerHost::start(&settings, pool, &catalog, gate.clone())?;
    let before_registration: Result<HashSet<Uuid>, BoxError> = async {
        let mut witnessed = HashSet::new();
        for _ in 0..limit {
            let id = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
                .await?
                .ok_or("worker witness closed")?;
            if !ids.contains(&id) || !witnessed.insert(id) {
                return Err("unexpected or repeated worker witness".into());
            }
        }
        // Pinned native claim commits the whole batch before spawning handlers;
        // excess capacity leases the extra row before its handler is scheduled.
        held_leases(pool, &ids, limit).await?;
        Ok(witnessed)
    }
    .await;
    let mut witnessed = match before_registration {
        Ok(witnessed) => witnessed,
        Err(error) => {
            release.add_permits(total);
            return finish_unregistered(host, Err(error)).await;
        }
    };

    // The actual held-handler evidence authorizes component acknowledgement.
    let second = Duration::from_secs(1);
    let cleanup = CleanupBudget::new(second, second, second)?;
    let mut supervisor = Supervisor::new(ShutdownBudget::new(
        batter_example_reference_service::worker::WORKER_SHUTDOWN_ALLOWANCE,
        Duration::from_secs(2),
        second,
        cleanup,
    )?);
    if let Err(error) = host.register(&mut supervisor) {
        let (registration, host) = error.into_parts();
        release.add_permits(total);
        return finish_unregistered(host, Err(Box::new(registration))).await;
    }
    let handle = supervisor.handle();
    let running = supervisor.start();
    let ready: ProbeResult = if !handle.mark_ready() {
        Err("configured host rejected readiness".into())
    } else {
        handle
            .wait_ready()
            .await
            .map_err(|_| "configured host did not acknowledge startup".into())
    };
    release.add_permits(total);
    let body: ProbeResult = async {
        ready?;
        let id = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await?
            .ok_or("worker witness closed")?;
        if !ids.contains(&id) || !witnessed.insert(id) || witnessed != ids {
            return Err("worker did not execute the complete configured set".into());
        }
        Ok(())
    }
    .await;
    let shutdown = running
        .shutdown()
        .await
        .map_err(|error| Box::new(error) as BoxError)
        .and_then(|report| {
            report
                .is_success()
                .then_some(())
                .ok_or_else(|| "configured host shutdown failed".into())
        });
    finish_results(body, shutdown)?;
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
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

fn configured_settings(limit: usize) -> Result<WorkerSettings, BoxError> {
    Ok(WorkerSettings::from_source(&SettingsSource::from_pairs(
        [
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
        ],
    )?)?)
}

async fn enqueue_jobs(
    pool: &PgPool,
    limit: usize,
    total: usize,
) -> Result<HashSet<Uuid>, BoxError> {
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
    Ok(ids)
}

async fn finish_unregistered(host: WorkerHost, body: ProbeResult) -> ProbeResult {
    let shutdown = host
        .shutdown()
        .await
        .map_err(|error| Box::new(error) as BoxError);
    finish_results(body, shutdown)
}

fn finish_results(body: ProbeResult, cleanup: Result<(), BoxError>) -> ProbeResult {
    batter_test_support::finish(
        body.map_err(ProbeError::new),
        cleanup.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
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
