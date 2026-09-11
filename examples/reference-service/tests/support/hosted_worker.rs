use super::{ProbeResult, fixture_diagnostics::ProbeError};
use batter::{
    BoxError,
    cleanup::{CleanupBudget, CleanupOutcome, CleanupStack, SkipReason},
    lifecycle::{RunningSupervisor, ShutdownBudget, Supervisor},
    operation::OperationContext,
    settings::SettingsSource,
    startup::{Startup, StartupCause, StartupError},
};
use batter_example_reference_service::{
    config::WorkerSettings,
    delivery::{DeliveryService, DeliveryState, OwnerId, SubmitDelivery},
    schema::initialize_schema,
    worker::{
        DependencyCleanup, DependencyCleanupError, PreparedProbeWorker, STARTUP_JOB_TYPE,
        TerminationGate, TerminationState, WorkerDriverError, WorkerHost, WorkerStartupError,
        prepare_probe_worker,
    },
};
use runledger_core::{
    jobs::{JobCompletion, JobContext, JobFailure, JobType},
    prelude::async_trait,
};
use runledger_postgres::jobs::{JobEnqueue, enqueue_job_with_outcome};
use runledger_runtime::{RuntimeError, catalog::JobCatalog, registry::JobHandler};
use serde_json::{Value, json};
use sqlx::{PgPool, types::Uuid};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

const HELD_JOB_TYPE: &str = "jobs.hosted.held";
const RETRY_JOB_TYPE: &str = "jobs.hosted.retry";

pub(super) fn settings(id: &str, concurrency: usize) -> Result<WorkerSettings, BoxError> {
    Ok(WorkerSettings::from_source(&SettingsSource::from_pairs(
        [
            ("JOBS_WORKER_ID".into(), id.into()),
            ("JOBS_POLL_INTERVAL_MS".into(), "20".into()),
            ("JOBS_CLAIM_BATCH_SIZE".into(), "4".into()),
            ("JOBS_LEASE_TTL_SECONDS".into(), "60".into()),
            (
                "JOBS_MAX_GLOBAL_CONCURRENCY".into(),
                concurrency.to_string().into(),
            ),
            ("JOBS_REAPER_INTERVAL_SECONDS".into(), "1".into()),
            ("JOBS_SCHEDULE_POLL_INTERVAL_SECONDS".into(), "1".into()),
            ("JOBS_REAPER_RETRY_DELAY_MS".into(), "100".into()),
        ],
    )?)?)
}

fn cleanup_budget() -> CleanupBudget {
    CleanupBudget::new(
        Duration::from_secs(3),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .expect("static cleanup budget")
}

pub(super) fn process_supervisor() -> Supervisor {
    Supervisor::new(
        ShutdownBudget::new(
            batter_example_reference_service::worker::WORKER_SHUTDOWN_ALLOWANCE,
            Duration::from_secs(2),
            Duration::from_secs(1),
            cleanup_budget(),
        )
        .expect("static shutdown budget"),
    )
}

fn retain_body_and_cleanup<T>(
    body: Result<T, BoxError>,
    cleanup: Result<(), BoxError>,
) -> Result<T, BoxError> {
    batter_test_support::finish(
        body.map_err(ProbeError::new),
        cleanup.map_err(ProbeError::new),
    )
    .map_err(|error| Box::new(error) as BoxError)
}

async fn stop_unregistered(host: WorkerHost, body: ProbeResult) -> ProbeResult {
    let shutdown = host
        .shutdown()
        .await
        .map_err(|error| Box::new(error) as BoxError);
    retain_body_and_cleanup(body, shutdown)
}

async fn stop_unregistered_probe(host: PreparedProbeWorker, body: ProbeResult) -> ProbeResult {
    let shutdown = host
        .shutdown()
        .await
        .map_err(|error| Box::new(error) as BoxError);
    retain_body_and_cleanup(body, shutdown)
}

fn host_with_cleanup(
    host: WorkerHost,
    gate: TerminationGate,
) -> Result<(RunningSupervisor, Arc<DependencyCleanup>, Arc<AtomicUsize>), BoxError> {
    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let mut nested = CleanupStack::new();
    nested.push("hosted.dependency", move || async move {
        invoked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })?;
    let cleanup = Arc::new(DependencyCleanup::new(nested, cleanup_budget(), gate));
    let mut supervisor = process_supervisor();
    cleanup.register(&mut supervisor)?;
    host.register(&mut supervisor)?;
    let handle = supervisor.handle();
    let running = supervisor.start();
    assert!(handle.mark_ready());
    Ok((running, cleanup, calls))
}

struct HeldHandler {
    started: mpsc::UnboundedSender<Uuid>,
    release: Arc<Semaphore>,
}

struct RetryHandler {
    invocations: Arc<AtomicUsize>,
    observed: mpsc::UnboundedSender<(Uuid, i32)>,
}

#[async_trait]
impl JobHandler for RetryHandler {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(RETRY_JOB_TYPE)
    }

    async fn execute(
        &self,
        context: JobContext,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        self.observed
            .send((context.job_id, context.attempt))
            .map_err(|_| JobFailure::terminal("retry.closed", "Retry witness closed."))?;
        if context.attempt == 1 {
            Err(
                JobFailure::retryable("retry.controlled", "Controlled retry requested.")
                    .retry_not_before_delay(Duration::from_millis(20)),
            )
        } else {
            Ok(JobCompletion::success())
        }
    }
}

#[async_trait]
impl JobHandler for HeldHandler {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(HELD_JOB_TYPE)
    }

    async fn execute(
        &self,
        context: JobContext,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        self.started
            .send(context.job_id)
            .map_err(|_| JobFailure::terminal("held.closed", "Held witness closed."))?;
        self.release
            .acquire()
            .await
            .map_err(|_| JobFailure::terminal("held.closed", "Held release closed."))?
            .forget();
        Ok(JobCompletion::success())
    }
}

async fn held_host(
    pool: &PgPool,
    id: &str,
) -> Result<
    (
        WorkerHost,
        TerminationGate,
        mpsc::UnboundedReceiver<Uuid>,
        Arc<Semaphore>,
        Uuid,
    ),
    BoxError,
> {
    initialize_schema(pool).await?;
    let (started, receiver) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let catalog = JobCatalog::new().handler(HeldHandler {
        started,
        release: release.clone(),
    });
    assert_eq!(
        catalog
            .to_registry()
            .registered_types()
            .into_iter()
            .map(|job_type| job_type.as_str())
            .collect::<Vec<_>>(),
        [HELD_JOB_TYPE]
    );
    catalog.sync_definitions(pool).await?;
    let payload = json!({"scenario": "held"});
    let request = JobEnqueue {
        job_type: JobType::new(HELD_JOB_TYPE),
        organization_id: None,
        payload: &payload,
        priority: None,
        max_attempts: None,
        timeout_seconds: None,
        next_run_at: None,
        idempotency_key: Some(id),
        stage: None,
    };
    let job = enqueue_job_with_outcome(pool, &request).await?;
    let gate = TerminationGate::new();
    let host = WorkerHost::start(&settings(id, 1)?, pool, &catalog, gate.clone())?;
    Ok((host, gate, receiver, release, job.job_id))
}

pub async fn probe_registry_and_normal_drain(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let (request_deadline, delivery_id) = submit_pending_delivery(&pool).await?;
    let gate = TerminationGate::new();
    let startup = OperationContext::new(Duration::from_secs(35))?;
    let prepared = prepare_probe_worker(
        &pool,
        &settings("hosted-probe", 1)?,
        gate.clone(),
        &startup,
        Duration::from_secs(20),
    )
    .await?;
    let invocation = prepared.invocation();
    let observed: ProbeResult = async {
        if invocation.attempt != 1 || invocation.run_number != 1 {
            return Err("startup witness did not observe the first durable attempt".into());
        }
        if invocation.operation_deadline <= tokio::time::Instant::now()
            || invocation.operation_deadline <= request_deadline
        {
            return Err("startup witness did not receive its fresh attempt deadline".into());
        }
        assert_probe_and_delivery_attempts(&pool, invocation, delivery_id).await
    }
    .await;
    if let Err(error) = observed {
        return stop_unregistered_probe(prepared, Err(error)).await;
    }
    let competing_gate = TerminationGate::new();
    let competing = prepare_probe_worker(
        &pool,
        &settings("hosted-probe-competing", 1)?,
        competing_gate.clone(),
        &startup,
        Duration::from_secs(1),
    )
    .await;
    match competing {
        Err(ref error)
            if matches!(error.as_ref(), WorkerStartupError::ProbeAlreadyActive)
                && competing_gate.state() == TerminationState::NotStarted => {}
        Err(error) => {
            return stop_unregistered_probe(prepared, Err(Box::new(error))).await;
        }
        Ok(competing) => {
            let competing_shutdown = competing
                .shutdown()
                .await
                .map_err(|error| Box::new(error) as BoxError);
            let body = retain_body_and_cleanup(
                Err("a second startup-control owner was admitted".into()),
                competing_shutdown,
            );
            return stop_unregistered_probe(prepared, body).await;
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let cleanup_pool = pool.clone();
    let mut nested = CleanupStack::new();
    nested.push("hosted.dependency", move || async move {
        super::hosted_worker_ownership::assert_no_advisory_locks(&cleanup_pool).await?;
        invoked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })?;
    let cleanup = Arc::new(DependencyCleanup::new(
        nested,
        cleanup_budget(),
        gate.clone(),
    ));
    let mut supervisor = process_supervisor();
    cleanup.register(&mut supervisor)?;
    prepared.register(&mut supervisor)?;
    let handle = supervisor.handle();
    let running = supervisor.start();
    let readiness: ProbeResult = if handle.mark_ready() {
        handle.wait_ready().await.map_err(|state| {
            Box::new(std::io::Error::other(format!(
                "hosted worker did not acknowledge startup: {state:?}"
            ))) as BoxError
        })
    } else {
        Err("hosted worker rejected readiness".into())
    };
    let process = running
        .shutdown()
        .await
        .map_err(|error| Box::new(std::io::Error::other(error.to_string())) as BoxError);
    let report = retain_body_and_cleanup(process, readiness)?;
    assert!(report.is_success(), "normal hosted drain failed: {report}");
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
    let nested = cleanup.finish().await;
    assert!(nested.is_success());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    super::hosted_worker_ownership::stale_control_and_registration_recovery(&pool).await?;
    Ok(())
}

pub async fn witness_failure_prevents_readiness(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    sqlx::query(
        "CREATE FUNCTION reject_control_success() RETURNS trigger
         LANGUAGE plpgsql AS $$
         BEGIN
             IF NEW.job_type::text = 'jobs.startup.control'
                AND NEW.status::text = 'SUCCEEDED' THEN
                 RAISE EXCEPTION 'controlled startup persistence failure';
             END IF;
             RETURN NEW;
         END;
         $$",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER reject_control_success
         BEFORE UPDATE ON job_queue
         FOR EACH ROW EXECUTE FUNCTION reject_control_success()",
    )
    .execute(&pool)
    .await?;

    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let mut nested = CleanupStack::new();
    nested.push("witness.dependency", move || async move {
        invoked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })?;
    let gate = TerminationGate::new();
    let cleanup = Arc::new(DependencyCleanup::new(
        nested,
        cleanup_budget(),
        gate.clone(),
    ));
    let initializer_cleanup = cleanup.clone();
    let initializer_pool = pool.clone();
    let initializer_gate = gate.clone();
    let worker_settings = settings("hosted-witness-failure", 1)?;
    let startup_context = OperationContext::new(Duration::from_secs(15))?;
    let initializer_context = startup_context.clone();
    let mut starting = Startup::new(
        process_supervisor(),
        startup_context,
        cleanup_budget(),
        move |scope| {
            Box::pin(async move {
                initializer_cleanup.register(scope.supervisor())?;
                let prepared = prepare_probe_worker(
                    &initializer_pool,
                    &worker_settings,
                    initializer_gate,
                    &initializer_context,
                    Duration::from_millis(250),
                )
                .await
                .map_err(|error| Box::new(error) as BoxError)?;
                prepared.register(scope.supervisor())?;
                Ok::<_, BoxError>(())
            })
        },
    )
    .without_readiness_approval()
    .start();

    let report = match starting.wait().await {
        Err(StartupError::Failed(report)) => report,
        Err(StartupError::Coordinator(error)) => return Err(Box::new(error)),
        Ok(_) => return Err("controlled witness failure unexpectedly reached handoff".into()),
    };
    let StartupCause::Failed(cause) = &report.cause else {
        return Err("controlled witness failure had a different startup cause".into());
    };
    assert!(matches!(
        cause
            .downcast_ref::<Arc<WorkerStartupError>>()
            .map(AsRef::as_ref),
        Some(WorkerStartupError::WitnessTimeout)
    ));
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
    assert_eq!(report.cleanup.records.len(), 1);
    assert_eq!(report.cleanup.records[0].outcome, CleanupOutcome::Succeeded);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cleanup.finish().await.is_success());

    let (status, attempts): (String, i64) = sqlx::query_as(
        "SELECT q.status::text, count(a.id)
           FROM job_queue q
           LEFT JOIN job_attempts a ON a.job_id = q.id
          WHERE q.job_type = $1
          GROUP BY q.status",
    )
    .bind(STARTUP_JOB_TYPE)
    .fetch_one(&pool)
    .await?;
    assert_ne!(status, "SUCCEEDED");
    assert_eq!(attempts, 1);
    Ok(())
}

async fn submit_pending_delivery(pool: &PgPool) -> Result<(tokio::time::Instant, Uuid), BoxError> {
    let owner_id = Uuid::from_u128(0x401);
    let record_id = Uuid::from_u128(0x402);
    sqlx::query("INSERT INTO reference_records (id, owner_id, generation) VALUES ($1, $2, 1)")
        .bind(record_id)
        .bind(owner_id)
        .execute(pool)
        .await?;
    let operation = OperationContext::new(Duration::from_secs(10))?;
    let service = DeliveryService::new(pool.clone());
    let command = SubmitDelivery {
        expected_generation: 1,
        idempotency_key: "pending-before-worker".to_owned(),
        payload: json!({"scenario": "registry-omission"}),
    };
    let first = service
        .submit(
            &operation,
            OwnerId::new(owner_id)?,
            record_id,
            command.clone(),
        )
        .await?;
    assert_eq!(first.delivery.state, DeliveryState::Pending);
    let replay = service
        .submit(&operation, OwnerId::new(owner_id)?, record_id, command)
        .await?;
    assert_eq!(first.delivery.delivery_id, replay.delivery.delivery_id);
    Ok((operation.deadline(), first.delivery.delivery_id))
}

async fn assert_probe_and_delivery_attempts(
    pool: &PgPool,
    invocation: batter_example_reference_service::worker::StartupInvocation,
    delivery_id: Uuid,
) -> Result<(), BoxError> {
    let startup_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM job_attempts a
           JOIN job_queue q ON q.id = a.job_id
          WHERE q.job_type = $1 AND q.id = $2",
    )
    .bind(STARTUP_JOB_TYPE)
    .bind(invocation.job_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(startup_attempts, 1);
    let (delivery_status, delivery_attempts): (String, i64) = sqlx::query_as(
        "SELECT q.status::text, count(a.id)
           FROM reference_deliveries d
           JOIN job_queue q ON q.id = d.job_id
           LEFT JOIN job_attempts a ON a.job_id = q.id
          WHERE d.id = $1
          GROUP BY q.status",
    )
    .bind(delivery_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(delivery_status, "PENDING");
    assert_eq!(delivery_attempts, 0);
    Ok(())
}

pub async fn retry_attempt_accounting(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let invocations = Arc::new(AtomicUsize::new(0));
    let (observed, mut receiver) = mpsc::unbounded_channel();
    let catalog = JobCatalog::new()
        .handler(RetryHandler {
            invocations: invocations.clone(),
            observed,
        })
        .retry_delay_override(RETRY_JOB_TYPE, "retry.controlled", 20);
    catalog.sync_definitions(&pool).await?;
    let payload = json!({"scenario": "controlled-retry"});
    let request = JobEnqueue {
        job_type: JobType::new(RETRY_JOB_TYPE),
        organization_id: None,
        payload: &payload,
        priority: None,
        max_attempts: Some(2),
        timeout_seconds: Some(30),
        next_run_at: None,
        idempotency_key: Some("hosted-retry"),
        stage: None,
    };
    let job = enqueue_job_with_outcome(&pool, &request).await?;
    let gate = TerminationGate::new();
    let host = WorkerHost::start(&settings("hosted-retry", 1)?, &pool, &catalog, gate.clone())?;
    let before_registration: ProbeResult = async {
        let attempts = tokio::time::timeout(Duration::from_secs(15), async {
            let mut attempts = Vec::new();
            while attempts.len() < 2 {
                attempts.push(receiver.recv().await.ok_or("retry witness closed")?);
            }
            Ok::<_, BoxError>(attempts)
        })
        .await??;
        if attempts != [(job.job_id, 1), (job.job_id, 2)] {
            return Err("retry handler attempts did not match the durable job".into());
        }

        wait_for_job_success(&pool, job.job_id).await?;
        let durable_attempts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM job_attempts WHERE job_id = $1")
                .bind(job.job_id)
                .fetch_one(&pool)
                .await?;
        if durable_attempts != 2 || invocations.load(Ordering::SeqCst) != 2 {
            return Err("retry attempt accounting diverged before replay".into());
        }

        let replay = enqueue_job_with_outcome(&pool, &request).await?;
        let replay_attempts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM job_attempts WHERE job_id = $1")
                .bind(job.job_id)
                .fetch_one(&pool)
                .await?;
        if replay.job_id != job.job_id
            || replay_attempts != 2
            || invocations.load(Ordering::SeqCst) != 2
        {
            return Err("idempotent replay changed the completed retry attempt".into());
        }
        Ok(())
    }
    .await;
    if let Err(error) = before_registration {
        return stop_unregistered(host, Err(error)).await;
    }

    let mut supervisor = process_supervisor();
    host.register(&mut supervisor)?;
    let handle = supervisor.handle();
    let running = supervisor.start();
    let readiness = if handle.mark_ready() {
        handle
            .wait_ready()
            .await
            .map_err(|_| "retry host did not acknowledge startup".into())
    } else {
        Err("retry host rejected readiness".into())
    };
    let shutdown = running
        .shutdown()
        .await
        .map_err(|error| Box::new(error) as BoxError)
        .and_then(|report| {
            report
                .is_success()
                .then_some(())
                .ok_or_else(|| "retry host shutdown failed".into())
        });
    retain_body_and_cleanup(readiness, shutdown)?;
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
    Ok(())
}

async fn wait_for_job_success(pool: &PgPool, job_id: Uuid) -> Result<(), BoxError> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status: String =
                sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
                    .bind(job_id)
                    .fetch_one(pool)
                    .await?;
            if status == "SUCCEEDED" {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

pub async fn in_flight_finishes_after_drain(pool: PgPool) -> ProbeResult {
    let (host, gate, mut started, release, job_id) = held_host(&pool, "hosted-drain").await?;
    let witnessed: Result<Uuid, BoxError> = async {
        Ok(
            tokio::time::timeout(Duration::from_secs(10), started.recv())
                .await?
                .ok_or("held handler witness closed")?,
        )
    }
    .await;
    let witnessed = match witnessed {
        Ok(witnessed) if witnessed == job_id => witnessed,
        Ok(_) => {
            release.add_permits(1);
            return stop_unregistered(host, Err("unexpected held handler witness".into())).await;
        }
        Err(error) => {
            release.add_permits(1);
            return stop_unregistered(host, Err(error)).await;
        }
    };
    debug_assert_eq!(witnessed, job_id);
    let (running, cleanup, calls) = host_with_cleanup(host, gate.clone())?;
    let handle = running.handle();
    let readiness = handle.wait_ready().await.map_err(|state| {
        std::io::Error::other(format!(
            "held worker did not acknowledge startup: {state:?}"
        ))
    });
    handle.request();
    let draining = handle.is_draining();
    release.add_permits(1);
    let process = running
        .wait()
        .await
        .map_err(|error| Box::new(std::io::Error::other(error.to_string())) as BoxError);
    let observation: ProbeResult = readiness
        .map_err(|error| Box::new(error) as BoxError)
        .and_then(|()| {
            draining
                .then_some(())
                .ok_or_else(|| "held worker did not enter drain".into())
        });
    let report = retain_body_and_cleanup(process, observation)?;
    assert!(report.is_success(), "in-flight drain failed: {report}");
    let status: String = sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SUCCEEDED");
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
    assert!(cleanup.finish().await.is_success());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}

pub async fn dropped_owner_is_observed_before_cleanup(pool: PgPool) -> ProbeResult {
    let (host, gate, mut started, release, job_id) = held_host(&pool, "hosted-owner-drop").await?;
    let witnessed: Result<Uuid, BoxError> = async {
        Ok(
            tokio::time::timeout(Duration::from_secs(10), started.recv())
                .await?
                .ok_or("held handler witness closed")?,
        )
    }
    .await;
    match witnessed {
        Ok(witnessed) if witnessed == job_id => {}
        Ok(_) => {
            release.add_permits(1);
            return stop_unregistered(host, Err("unexpected held handler witness".into())).await;
        }
        Err(error) => {
            release.add_permits(1);
            return stop_unregistered(host, Err(error)).await;
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let mut nested = CleanupStack::new();
    nested.push("owner-drop.dependency", move || async move {
        invoked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })?;
    let cleanup = DependencyCleanup::new(nested, cleanup_budget(), gate.clone());

    release.add_permits(1);
    drop(host);
    let report = cleanup.finish().await;
    assert!(report.is_success());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.state(), TerminationState::CooperativelyStopped);
    let status: String = sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "SUCCEEDED");
    Ok(())
}

pub async fn timeout_skips_dependencies(pool: PgPool) -> ProbeResult {
    let (host, gate, mut started, release, job_id) = held_host(&pool, "hosted-timeout").await?;
    let witnessed: Result<Uuid, BoxError> = async {
        Ok(
            tokio::time::timeout(Duration::from_secs(10), started.recv())
                .await?
                .ok_or("held handler witness closed")?,
        )
    }
    .await;
    match witnessed {
        Ok(witnessed) if witnessed == job_id => {}
        Ok(_) => {
            release.add_permits(1);
            return stop_unregistered(host, Err("unexpected held handler witness".into())).await;
        }
        Err(error) => {
            release.add_permits(1);
            return stop_unregistered(host, Err(error)).await;
        }
    }
    let (running, cleanup, calls) = host_with_cleanup(host, gate.clone())?;
    let readiness = running.handle().wait_ready().await.map_err(|state| {
        std::io::Error::other(format!(
            "held worker did not acknowledge startup: {state:?}"
        ))
    });
    let process = running
        .shutdown()
        .await
        .map_err(|error| Box::new(std::io::Error::other(error.to_string())) as BoxError);
    let readiness = readiness.map_err(|error| Box::new(error) as BoxError);
    let report = retain_body_and_cleanup(process, readiness)?;
    assert!(!report.is_success());
    let worker = report
        .tasks
        .iter()
        .find(|record| record.name == "worker")
        .ok_or("worker failure record missing")?;
    let driver = worker
        .error
        .as_deref()
        .and_then(|error| error.downcast_ref::<WorkerDriverError>())
        .ok_or("worker driver error was not retained")?;
    assert!(matches!(
        driver,
        WorkerDriverError::Native(error)
            if matches!(error.as_ref(), runledger_runtime::Error::Runtime(
                RuntimeError::ShutdownTimeout { .. }
            ))
    ));
    let grouped = report
        .cleanup
        .records
        .iter()
        .find(|record| record.name == "worker.dependencies")
        .ok_or("grouped dependency cleanup record missing")?;
    assert_eq!(grouped.outcome, CleanupOutcome::Failed);
    let retained = grouped
        .error
        .as_deref()
        .and_then(|error| error.downcast_ref::<DependencyCleanupError>())
        .ok_or("nested cleanup failure was not retained")?;
    assert_eq!(retained.report().skipped.len(), 1);
    assert_eq!(
        retained.report().skipped[0].reason,
        SkipReason::UnsafeTaskExit
    );
    let nested = cleanup.finish().await;
    assert_eq!(nested.skipped.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(gate.state(), TerminationState::Unproven);
    Ok(())
}
