//! Durable acceptance is isolated here; production initialization enqueues nothing.
use super::{
    ProbeResult,
    fixture_diagnostics::ProbeError,
    transactions::{read_committed, request},
};
use batter::{
    BoxError,
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor, check_shutdown},
    operation::OperationContext,
};
use runledger_core::{
    jobs::{JobCompletion, JobContext, JobFailure, JobType},
    prelude::async_trait,
};
use runledger_runtime::{
    catalog::JobCatalog,
    config::JobsConfig,
    registry::{JobHandler, JobRegistry},
};
use serde_json::Value;
use sqlx::{PgPool, types::Uuid};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

pub fn config() -> JobsConfig {
    JobsConfig {
        worker_id: "native-acceptance".into(),
        poll_interval: Duration::from_millis(10),
        claim_batch_size: 1,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(30),
        schedule_poll_interval: Duration::from_secs(30),
        reaper_retry_delay_ms: 100,
    }
}

pub fn process(drain: Duration, cancel: Duration) -> Supervisor {
    let second = Duration::from_secs(1);
    Supervisor::new(
        ShutdownBudget::new(
            drain,
            cancel,
            second,
            CleanupBudget::new(second, second, second).unwrap(),
        )
        .unwrap(),
    )
}

pub async fn enqueue(pool: &PgPool) -> Result<Uuid, BoxError> {
    let mut tx = read_committed(pool).await?;
    let payload = serde_json::json!({"test": true});
    let job = runledger_postgres::jobs::enqueue_job_with_outcome_tx(
        &mut tx,
        &request(&payload, Uuid::now_v7()),
    )
    .await?;
    tx.commit().await?;
    Ok(job.job_id)
}

pub async fn status(pool: &PgPool, id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub fn finish(body: ProbeResult, shutdown: ProbeResult) -> ProbeResult {
    batter_test_support::finish(
        body.map_err(ProbeError::new),
        shutdown.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}

pub async fn queue_independent_initialization(pool: PgPool) -> ProbeResult {
    batter_example_reference_service::schema::initialize_schema(&pool).await?;
    crate::support::worker::catalog(None)
        .sync_definitions(&pool)
        .await?;
    let mut blocked = pool.begin().await?;
    sqlx::query("LOCK TABLE job_queue IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocked)
        .await?;
    let mut process = process(Duration::from_secs(2), Duration::from_secs(1));
    let native_pool = pool.clone();
    batter_runledger::register(
        &mut process,
        "native",
        OperationContext::new(Duration::from_secs(2))?,
        {
            let mut config = config();
            config.poll_interval = Duration::from_secs(31_536_000);
            runledger_runtime::Supervisor::builder(&native_pool, config)?
                .with_registry(JobRegistry::new())
                .prepare()?
        },
    )?;
    let running = process.start();
    running.handle().mark_ready();
    let initialized =
        tokio::time::timeout(Duration::from_secs(1), running.handle().wait_ready()).await;
    let rollback = blocked.rollback().await;
    let shutdown =
        check_shutdown(running.shutdown().await).map_err(|error| Box::new(error) as BoxError);
    let body: ProbeResult = async {
        rollback?;
        initialized?.map_err(|_| "native initialization failed")?;
        let controls: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_queue WHERE job_type = 'jobs.startup.control'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            controls, 0,
            "initialization must not enqueue durable controls"
        );
        Ok(())
    }
    .await;
    finish(body, shutdown)
}

struct Held {
    entered: mpsc::UnboundedSender<Uuid>,
    release: Arc<Semaphore>,
    completed: Arc<AtomicBool>,
}
#[async_trait]
impl JobHandler for Held {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("jobs.compatibility.witness")
    }
    async fn execute(&self, context: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        let _ = self.entered.send(context.job_id);
        self.release
            .acquire()
            .await
            .expect("test gate stays open")
            .forget();
        self.completed.store(true, Ordering::SeqCst);
        Ok(JobCompletion::success())
    }
}

pub async fn in_flight_drain(pool: PgPool) -> ProbeResult {
    held(pool, false).await
}
pub async fn owner_drop(pool: PgPool) -> ProbeResult {
    held(pool, true).await
}

async fn held(pool: PgPool, drop_owner: bool) -> ProbeResult {
    batter_example_reference_service::schema::initialize_schema(&pool).await?;
    let (entered, mut entering) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let completed = Arc::new(AtomicBool::new(false));
    let catalog = JobCatalog::new().handler(Held {
        entered,
        release: release.clone(),
        completed: completed.clone(),
    });
    catalog.sync_definitions(&pool).await?;
    let job = enqueue(&pool).await?;
    let mut process = process(Duration::from_secs(2), Duration::from_secs(1));
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleanup = cleaned.clone();
    process.on_cleanup("dependency", move || async move {
        assert!(
            completed.load(Ordering::SeqCst),
            "cleanup preceded actual handler completion"
        );
        cleanup.store(true, Ordering::SeqCst);
        Ok(())
    })?;
    let native_pool = pool.clone();
    batter_runledger::register(
        &mut process,
        "native",
        OperationContext::new(Duration::from_secs(2))?,
        {
            runledger_runtime::Supervisor::builder(&native_pool, config())?
                .with_catalog(catalog)
                .prepare()?
        },
    )?;
    let running = process.start();
    running.handle().mark_ready();
    let entry = tokio::time::timeout(Duration::from_secs(5), entering.recv()).await;
    let observer = running.observer();
    if drop_owner {
        drop(running);
    } else {
        running.handle().request();
    }
    assert!(!cleaned.load(Ordering::SeqCst));
    release.add_permits(1);
    let report = observer.wait().await;
    let body: ProbeResult = async {
        assert_eq!(entry?, Some(job));
        assert_eq!(status(&pool, job).await?, "SUCCEEDED");
        assert!(cleaned.load(Ordering::SeqCst));
        Ok(())
    }
    .await;
    finish(
        body,
        check_shutdown(report).map_err(|error| Box::new(error) as BoxError),
    )
}

struct Reject;
#[async_trait]
impl JobHandler for Reject {
    fn job_type(&self) -> JobType<'static> {
        JobType::new("jobs.compatibility.witness")
    }
    async fn execute(&self, _: JobContext, _: Value) -> Result<JobCompletion, JobFailure> {
        Err(JobFailure::terminal(
            "test.rejected",
            "controlled business rejection",
        ))
    }
}

pub async fn business_failure(pool: PgPool) -> ProbeResult {
    batter_example_reference_service::schema::initialize_schema(&pool).await?;
    let catalog = JobCatalog::new().handler(Reject);
    catalog.sync_definitions(&pool).await?;
    let job = enqueue(&pool).await?;
    let mut process = process(Duration::from_secs(2), Duration::from_secs(1));
    let native_pool = pool.clone();
    batter_runledger::register(
        &mut process,
        "native",
        OperationContext::new(Duration::from_secs(2))?,
        {
            runledger_runtime::Supervisor::builder(&native_pool, config())?
                .with_catalog(catalog)
                .prepare()?
        },
    )?;
    let running = process.start();
    running.handle().mark_ready();
    let body: ProbeResult = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while status(&pool, job).await? != "DEAD_LETTERED" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, sqlx::Error>(())
        })
        .await??;
        assert!(
            !running.handle().is_draining(),
            "business rejection must not stop the runtime"
        );
        Ok(())
    }
    .await;
    let shutdown =
        check_shutdown(running.shutdown().await).map_err(|error| Box::new(error) as BoxError);
    finish(body, shutdown)
}
