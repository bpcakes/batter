use batter_core::{
    cleanup::CleanupBudget,
    lifecycle::{ShutdownBudget, Supervisor},
    operation::{OperationContext, OperationOwner},
    startup::Startup,
};
use batter_flags::{
    Flags,
    core::{CatalogBuilder, KillSwitch, SafeState, SettingsPrefix, StaleRule},
    jobs::{DirectWhenTripped, FlagAwareHandler, GatePolicy, JobFlags, Recheck},
    test::TestFlags,
};
use batter_runledger::native::core::prelude::async_trait;
pub use batter_runledger::native::{
    core::jobs::*,
    postgres::{self, jobs::*},
    runtime,
};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::{sleep, timeout};
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T = ()> = std::result::Result<T, BoxError>;
pub const SECOND: Duration = Duration::from_secs(1);

pub async fn connect() -> Result<batter_runledger::RunledgerDatabase> {
    let url = std::env::var("DATABASE_URL")?;
    let options: sqlx::postgres::PgConnectOptions = url.parse()?;
    let login = options.get_username();
    let profile = batter_runledger::PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    let database = batter_runledger::RunledgerDatabase::connect_lazy(
        options,
        profile,
        PgPoolOptions::new().max_connections(8),
    )?;
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::integer")
        .fetch_one(database.pool())
        .await?;
    assert!(
        (180000..190000).contains(&version),
        "PostgreSQL 18 required"
    );
    postgres::migrate_after_idempotency_cutover(&database).await?;
    Ok(database)
}

pub struct Probe {
    pub name: &'static str,
    pub calls: Arc<AtomicUsize>,
    pub dead: Arc<AtomicUsize>,
}
impl Probe {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: Arc::default(),
            dead: Arc::default(),
        }
    }
}
#[async_trait]
impl FlagAwareHandler for Probe {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(self.name)
    }
    async fn execute(
        &self,
        _: JobExecution<'_>,
        _: &JobFlags,
        _: Value,
    ) -> std::result::Result<JobCompletion, JobFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(JobCompletion::success())
    }
    async fn on_dead_letter(&self, _: JobContext, _: Value, _: JobDeadLetterInfo) {
        self.dead.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn flags(state: batter_flags::core::SwitchState) -> Result<(Flags, KillSwitch)> {
    let mut b = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    let worker = b.process_kind("worker")?;
    b.environment_class("hosted")?;
    let halt = b
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()?;
    let builder = TestFlags::builder(b.build()?, worker)?;
    let test = match state {
        batter_flags::core::SwitchState::Engaged => builder.engage(&halt),
        batter_flags::core::SwitchState::Released => builder.release(&halt),
    }
    .build()?;
    Ok((test.flags().clone(), halt))
}
pub fn recheck() -> Recheck {
    Recheck::after(Duration::from_secs(30)).unwrap()
}
pub fn policy(when_tripped: DirectWhenTripped) -> GatePolicy<DirectWhenTripped> {
    GatePolicy {
        when_tripped,
        awaiting: recheck(),
    }
}
fn context() -> OperationContext {
    OperationOwner::new(Duration::from_secs(20))
        .unwrap()
        .into_context()
}

pub async fn drive<H, F>(pool: &PgPool, handler: H, body: F) -> Result
where
    H: JobHandler + 'static,
    F: Future<Output = Result> + Send + 'static,
{
    let mut registry = runtime::registry::JobRegistry::new();
    registry.try_register(handler)?;
    let config = runtime::config::JobsConfig {
        worker_id: "flags-live".into(),
        poll_interval: Duration::from_millis(20),
        claim_batch_size: 1,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: SECOND,
        schedule_poll_interval: SECOND,
        reaper_retry_delay_ms: 100,
    };
    let prepared = runtime::Supervisor::builder(pool, config)?
        .with_registry(registry)
        .disable_scheduler()
        .disable_reaper()
        .prepare()?;
    let cleanup = CleanupBudget::new(SECOND, SECOND, SECOND)?;
    let process = Supervisor::new(ShutdownBudget::new(SECOND, SECOND, SECOND, cleanup)?);
    let mut starting = Startup::scoped(process, context(), cleanup, move |scope| {
        Box::pin(async move {
            batter_runledger::register_in(scope, "flags.worker", context(), prepared)?;
            Ok::<_, batter_core::RegistrationError>(())
        })
    })
    .start();
    let running = starting.wait().await?;
    let status = running.status();
    let ready = timeout(Duration::from_secs(10), status.wait_ready()).await;
    let body = tokio::spawn(async move {
        ready?.map_err(|_| "worker did not become ready")?;
        timeout(Duration::from_secs(90), body).await?
    })
    .await;
    let shutdown = running.shutdown().await?;
    assert!(shutdown.is_success(), "{shutdown}");
    body??;
    Ok(())
}

pub async fn define(pool: &PgPool, name: &'static str, max_attempts: i32) -> Result {
    let mut tx = pool.begin().await?;
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type: JobType::new(name),
            version: 1,
            max_attempts,
            default_timeout_seconds: 10,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub fn request<'a>(name: &'static str, payload: &'a Value) -> JobEnqueue<'a> {
    JobEnqueue {
        job_type: JobType::new(name),
        organization_id: None,
        payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: None,
        stage: Some(JobStage::Queued),
    }
}
pub async fn wait_row(pool: &PgPool, id: sqlx::types::Uuid, status: &str, run: i32) -> Result {
    timeout(Duration::from_secs(80), async {
        loop {
            let matched: bool = sqlx::query_scalar(
                "SELECT status::text=$2 AND run_number=$3 FROM job_queue WHERE id=$1",
            )
            .bind(id)
            .bind(status)
            .bind(run)
            .fetch_one(pool)
            .await?;
            if matched {
                return Ok::<_, BoxError>(());
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    Ok(())
}
