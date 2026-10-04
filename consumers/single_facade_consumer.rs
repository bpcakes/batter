//! External consumer that reaches Runledger and Runlimit through one `batter`
//! dependency and its features, with no direct native-package declaration and
//! no `[patch]` section.
//!
//! Everything below executes against the disposable PostgreSQL 18 database named
//! by `DATABASE_URL`: both migration histories are applied, a durable enqueue
//! intent and an atomic queue row commit in one transaction, the registered
//! native worker executes both jobs under Batter's protected lifecycle, the
//! native PostgreSQL limiter admits and then denies a quota check, and the
//! native transport package encodes that denial's response metadata.
//!
//! The native Axum admission layer is reached through the same facade feature
//! set; `crates/batter/tests/native_transport_consumer.rs` drives a request
//! through it. This program does not, and claims no coverage for it.
//!
//! `scripts/check_single_facade_consumer.py` builds this file from a Git-free
//! source copy and asserts the resolved manifest declares only `batter` from
//! this workspace. `consumers/single_facade_harness.rs` owns the database.

mod single_facade_completion;
mod single_facade_quota;

use batter::cleanup::CleanupBudget;
use batter::lifecycle::{ShutdownBudget, Supervisor};
use batter::operation::{OperationContext, OperationOwner};
use batter::runledger::native::core::jobs::{
    JobCompletion, JobContext, JobFailure, JobHandler, JobType,
};
use batter::runledger::native::core::prelude::async_trait;
use batter::runledger::native::postgres::{
    jobs::{JobEnqueue, JobEnqueueIntent},
    migrate_after_idempotency_cutover,
};
use batter::runledger::native::runtime::{
    PreparedSupervisor, Supervisor as NativeSupervisor,
    catalog::JobCatalog,
    config::{IntentPromoterConfig, JobsConfig},
};
use batter::runledger::{PgSessionProfile, RunledgerDatabase, run_atomic, verify_schema};
use batter::runlimit::native::{Check, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
use batter::runlimit::native_transport::http::draft_11;
use batter::runlimit::postgres::PostgresLimiter;
use batter::runlimit::{Checks, Quota};
use batter::startup::Startup;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One sized application failure for startup and atomic work, retaining its cause as
/// a source rather than formatting it. `Box<dyn Error>` is unsized and therefore
/// cannot itself be the runner's error type.
#[derive(Debug)]
struct ApplicationFailure(Box<dyn std::error::Error + Send + Sync>);

impl ApplicationFailure {
    fn new(cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(cause))
    }
}

impl std::fmt::Display for ApplicationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the consumer operation failed")
    }
}

impl std::error::Error for ApplicationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

const JOB: JobType<'static> = JobType::new("facade.consumer.greeting");
const PROMOTED_KEY: &str = "facade-consumer-promoted";
const ENQUEUED_KEY: &str = "facade-consumer-enqueued";
/// Fixture key material. A deployment loads its own secret of at least 32 bytes.
const QUOTA_SECRET: [u8; 32] = [23; 32];

/// Records each executed job so the root can witness durable delivery.
struct Greet {
    executed: UnboundedSender<String>,
}

#[async_trait]
impl JobHandler for Greet {
    fn job_type(&self) -> JobType<'static> {
        JOB
    }

    async fn execute(
        &self,
        _context: JobContext,
        payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| JobFailure::terminal("facade.consumer.payload", "Expected a name."))?;
        self.executed.send(name.to_owned()).map_err(|_| {
            JobFailure::terminal("facade.consumer.observer", "Root stopped observing.")
        })?;
        JobCompletion::success()
            .progress(1, 1)
            .map_err(|_| JobFailure::terminal("facade.consumer.progress", "Invalid counts."))
    }
}

fn cleanup_budget() -> Outcome<CleanupBudget> {
    Ok(CleanupBudget::new(
        Duration::from_secs(5),
        Duration::from_secs(3),
        Duration::from_secs(1),
    )?)
}

fn budget() -> Outcome<ShutdownBudget> {
    Ok(ShutdownBudget::new(
        Duration::from_secs(10),
        Duration::from_secs(2),
        Duration::from_secs(1),
        cleanup_budget()?,
    )?)
}

fn context(seconds: u64) -> Outcome<OperationContext> {
    Ok(OperationOwner::new(Duration::from_secs(seconds))?.into_context())
}

/// Connect through the facade's profiled native database owner.
fn database(url: &str) -> Outcome<RunledgerDatabase> {
    let options: sqlx::postgres::PgConnectOptions = url.parse()?;
    let login = options.get_username();
    let profile = PgSessionProfile::new(
        login,
        login,
        vec!["public".into()],
        Duration::ZERO,
        Duration::ZERO,
    )?;
    Ok(RunledgerDatabase::connect_lazy(
        options,
        profile,
        sqlx::postgres::PgPoolOptions::new().max_connections(6),
    )?)
}

/// Apply the Runledger history first, then Runlimit's history: Runlimit's
/// migrator ignores the versions Runledger already recorded, while a strict host
/// migrator would reject them. See `docs/reference-compatibility.md`.
async fn migrate(database: &RunledgerDatabase, limiter: &PostgresLimiter) -> Outcome {
    migrate_after_idempotency_cutover(database).await?;
    // The snapshot is one observed compatible state, not a future promise.
    let _snapshot = verify_schema(database).await?;
    limiter.migrate().await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS facade_consumer_audit (idempotency_key text PRIMARY KEY)",
    )
    .execute(database.pool())
    .await?;
    println!("facade consumer: runledger and runlimit migrations applied");
    Ok(())
}

/// One transaction writes application state, records the durable intent the
/// native promoter later turns into a queue row, and enqueues the second job.
async fn submit(database: &RunledgerDatabase, payload: &Value) -> Outcome {
    let intent = JobEnqueueIntent::new(JOB, payload, PROMOTED_KEY).with_max_attempts(1);
    let request = JobEnqueue {
        job_type: JOB,
        organization_id: None,
        payload,
        priority: None,
        max_attempts: Some(1),
        timeout_seconds: Some(30),
        next_run_at: None,
        idempotency_key: Some(ENQUEUED_KEY),
        stage: None,
    };
    run_atomic(
        database,
        async |mut scope| -> Result<(), ApplicationFailure> {
            scope
                .application(async |sql| {
                    sqlx::query("INSERT INTO facade_consumer_audit (idempotency_key) VALUES ($1)")
                        .bind(PROMOTED_KEY)
                        .execute(sql.executor())
                        .await
                })
                .await
                .map_err(ApplicationFailure::new)?;
            scope
                .record_required_job_enqueue_intent(&intent)
                .await
                .map_err(ApplicationFailure::new)?;
            scope
                .queue()
                .enqueue_job(&request)
                .await
                .map_err(ApplicationFailure::new)?;
            Ok(())
        },
    )
    .await?;
    let intents: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_enqueue_intents WHERE idempotency_key = $1")
            .bind(PROMOTED_KEY)
            .fetch_one(database.pool())
            .await?;
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_queue WHERE idempotency_key = $1")
            .bind(ENQUEUED_KEY)
            .fetch_one(database.pool())
            .await?;
    if (intents, queued) != (1, 1) {
        return Err("expected one committed intent and queue row".into());
    }
    println!("facade consumer: durable intent and atomic enqueue committed");
    Ok(())
}

/// Construct only inert native work; the startup owner registers it.
async fn prepare_worker(
    database: &RunledgerDatabase,
    executed: UnboundedSender<String>,
) -> Outcome<PreparedSupervisor> {
    let pool = database.pool().clone();
    let catalog = JobCatalog::new().handler(Greet { executed });
    catalog.sync_definitions(&pool).await?;
    // Validated application settings, never native `from_env` discovery.
    let config = JobsConfig {
        worker_id: "single-facade-consumer".to_owned(),
        poll_interval: Duration::from_millis(50),
        claim_batch_size: 10,
        lease_ttl_seconds: 30,
        max_global_concurrency: 4,
        reaper_interval: Duration::from_secs(1),
        schedule_poll_interval: Duration::from_secs(1),
        reaper_retry_delay_ms: 1_000,
    };
    Ok(NativeSupervisor::builder(&pool, config)?
        .with_intent_promoter_config(IntentPromoterConfig::new(Duration::from_millis(50), 10))
        .with_catalog(&catalog)
        .prepare()?)
}

/// Startup owns the pool finalizer before the first database operation. Running
/// work always awaits checked shutdown; only the lifecycle driver may authorize
/// dependency cleanup, including after a readiness, submission or receive error.
async fn run_worker(database: RunledgerDatabase, payload: &Value, subject: &str) -> Outcome {
    let (executed, mut observed) = unbounded_channel();
    let initializing = database.clone();
    let mut starting = Startup::scoped(
        Supervisor::new(budget()?),
        context(60)?,
        cleanup_budget()?,
        move |scope| {
            Box::pin(async move {
                let result: Outcome = async {
                    let pool = initializing.pool().clone();
                    scope
                        .reserve_cleanup("postgres.pool")?
                        .register(move || async move {
                            pool.close().await;
                            Ok(())
                        });
                    let limiter = PostgresLimiter::new(initializing.pool().clone());
                    migrate(&initializing, &limiter).await?;
                    let prepared = prepare_worker(&initializing, executed).await?;
                    batter::runledger::register_in(scope, "worker", context(5)?, prepared)?;
                    Ok(())
                }
                .await;
                result.map_err(ApplicationFailure)
            })
        },
    )
    .start();
    let running = starting.wait().await?;
    let status = running.status();
    single_facade_completion::complete(running, async {
        status
            .wait_ready()
            .await
            .map_err(|readiness| format!("worker did not become ready: {readiness:?}"))?;
        submit(&database, payload).await?;
        let mut names = Vec::new();
        while names.len() < 2 {
            let name = tokio::time::timeout(Duration::from_secs(60), observed.recv())
                .await
                .map_err(|_| "durable jobs did not execute within the consumer budget")?
                .ok_or("worker stopped before executing both durable jobs")?;
            names.push(name);
        }
        if names != vec!["facade".to_owned(); 2] {
            return Err("both executions must use the committed payload".into());
        }
        run_limiter(PostgresLimiter::new(database.pool().clone()), subject).await
    })
    .await?;
    println!("facade consumer: worker executed both durable jobs");
    Ok(())
}

/// One native PostgreSQL fixed-window policy, admitted once and then denied.
async fn run_limiter(limiter: PostgresLimiter, subject: &str) -> Outcome {
    let policy = FixedWindowPolicy::new(
        PolicyId::new("facade.consumer.quota")?,
        ScopeId::new("owner")?,
        1,
        Duration::from_secs(3600),
    )?;
    let hasher = KeyHasher::new(QUOTA_SECRET)?;
    let checks = [Check::new(hasher.hash_for(&policy, subject))];
    let quota = Quota::new(limiter);
    let context = context(20)?;
    let mut invoked = 0_u32;
    let admitted = quota
        .run(&context, Checks::new(&checks)?, |_| {
            invoked += 1;
            async { Ok::<_, std::convert::Infallible>(()) }
        })
        .await;
    single_facade_quota::expect_admission(admitted)?;
    let denied = quota
        .run(&context, Checks::new(&checks)?, |_| {
            invoked += 1;
            async { Ok::<_, std::convert::Infallible>(()) }
        })
        .await;
    let exhausted = single_facade_quota::expect_exhaustion(denied)?;
    if invoked != 1 {
        return Err("a denied quota must not invoke work".into());
    }
    println!("facade consumer: postgres quota admitted then denied");
    // The application, not the native encoder, chooses the disclosed policy name
    // and the response status this metadata accompanies.
    let (policy_field, _) = draft_11::quota_policy("facade-consumer", &policy)?;
    let (limit_field, limit_value) = draft_11::service_limit("facade-consumer", exhausted)?;
    if policy_field.as_str() != "ratelimit-policy"
        || limit_field.as_str() != "ratelimit"
        || !limit_value.to_str()?.contains(";r=0;")
    {
        return Err("expected draft-11 fields with no remaining quota".into());
    }
    println!("facade consumer: native draft-11 response metadata encoded");
    Ok(())
}

#[tokio::main]
async fn main() -> Outcome {
    let url = std::env::var("DATABASE_URL")?;
    let subject = std::env::var("FACADE_CONSUMER_SUBJECT")?;
    let database = database(&url)?;
    let payload = json!({"name": "facade"});
    run_worker(database, &payload, &subject).await?;
    println!("single batter dependency reached runledger and runlimit");
    Ok(())
}
