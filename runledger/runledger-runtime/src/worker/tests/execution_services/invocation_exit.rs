//! Every way a handler invocation exits ends its worker-owned exit signal, and
//! it does so before the worker persists the outcome. Graceful drain keeps an
//! active invocation running; forced abandonment ends it.

use runledger_core::jobs::{JobInvocation, JobStage};
use tokio::sync::mpsc;

use super::*;

const EXIT_TYPE: JobType<'static> = JobType::new("jobs.test.invocation_exit");
const OBSERVERS: usize = 3;
const HOOKS: usize = 2;
const GATE_KEY: &str = "runledger-invocation-exit-gate";
const WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
enum Exit {
    Succeed,
    Continue,
    Fail,
    Panic,
    Pending,
    Release,
    SwallowLeaseLoss,
}

/// What one invocation exposed before choosing its exit.
struct Started {
    invocation: JobInvocation,
    ended_at_start: bool,
    observed: mpsc::UnboundedReceiver<usize>,
    hooks: Arc<AtomicUsize>,
    ended_at_handler_drop: Arc<Mutex<Option<bool>>>,
}

/// Records whether the invocation had ended when the handler future was destroyed.
struct HandlerDropProbe {
    invocation: JobInvocation,
    ended: Arc<Mutex<Option<bool>>>,
}

impl Drop for HandlerDropProbe {
    fn drop(&mut self) {
        *self.ended.lock().expect("drop probe") = Some(self.invocation.has_ended());
    }
}

struct ExitHandler {
    exit: Exit,
    started: mpsc::UnboundedSender<Started>,
    proceed: Arc<Notify>,
}

#[async_trait::async_trait]
impl JobExecutionHandler for ExitHandler {
    fn job_type(&self) -> JobType<'static> {
        EXIT_TYPE
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        _payload: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let invocation = execution
            .invocation()
            .expect("the worker owns every invocation's exit");
        let ended_at_start = invocation.has_ended();
        let ended_at_handler_drop = Arc::new(Mutex::new(None));
        let _probe = HandlerDropProbe {
            invocation: invocation.clone(),
            ended: Arc::clone(&ended_at_handler_drop),
        };
        let (acknowledged, mut acknowledgements) = mpsc::unbounded_channel();
        let (done, observed) = mpsc::unbounded_channel();
        for index in 0..OBSERVERS {
            let (invocation, acknowledged, done) =
                (invocation.clone(), acknowledged.clone(), done.clone());
            tokio::spawn(async move {
                let mut waiting = invocation.ended();
                assert!(futures_util::poll!(&mut waiting).is_pending());
                acknowledged
                    .send(())
                    .expect("handler awaits acknowledgement");
                waiting.await;
                let _ = done.send(index);
            });
        }
        for _ in 0..OBSERVERS {
            acknowledgements.recv().await.expect("observer is waiting");
        }
        let hooks = Arc::new(AtomicUsize::new(0));
        // A copied handle reaches the same invocation.
        let copied = execution;
        for registered in [invocation.clone(), copied.invocation().expect("copy")] {
            let hooks = Arc::clone(&hooks);
            registered.on_end(move || {
                hooks.fetch_add(1, Ordering::SeqCst);
            });
        }
        self.started
            .send(Started {
                invocation,
                ended_at_start,
                observed,
                hooks,
                ended_at_handler_drop,
            })
            .expect("test receives each invocation");
        match self.exit {
            Exit::Succeed => Ok(JobCompletion::success()),
            Exit::Continue => Ok(JobCompletion::continue_now()),
            Exit::Fail => Err(JobFailure::retryable(
                "jobs.test.invocation_exit_failed",
                "Injected handler failure.",
            )),
            Exit::Panic => panic!("injected handler panic"),
            Exit::Pending => pending().await,
            Exit::Release => {
                self.proceed.notified().await;
                Ok(JobCompletion::success())
            }
            Exit::SwallowLeaseLoss => {
                self.proceed.notified().await;
                let _ignored = execution.save_checkpoint(&1_u64).await;
                pending().await
            }
        }
    }
}

fn exit_registry(exit: Exit) -> (JobRegistry, mpsc::UnboundedReceiver<Started>, Arc<Notify>) {
    let (started, received) = mpsc::unbounded_channel();
    let proceed = Arc::new(Notify::new());
    let mut registry = JobRegistry::new();
    registry.register(
        ExitHandler {
            exit,
            started,
            proceed: Arc::clone(&proceed),
        }
        .into_job_handler(),
    );
    (registry, received, proceed)
}

async fn next_started(received: &mut mpsc::UnboundedReceiver<Started>) -> Started {
    let started = timeout(WAIT, received.recv())
        .await
        .expect("handler starts")
        .expect("handler reports its invocation");
    assert!(
        !started.ended_at_start,
        "no earlier invocation's exit reaches a new invocation"
    );
    started
}

/// Every acknowledged observer and hook saw the end; late observation is immediate.
async fn assert_ended(started: &mut Started) {
    let mut seen = Vec::new();
    for _ in 0..OBSERVERS {
        seen.push(
            timeout(WAIT, started.observed.recv())
                .await
                .expect("observer is notified")
                .expect("observer reports"),
        );
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..OBSERVERS).collect::<Vec<_>>());
    assert!(started.invocation.has_ended());
    assert_eq!(started.hooks.load(Ordering::SeqCst), HOOKS);
    assert_eq!(
        *started.ended_at_handler_drop.lock().expect("drop probe"),
        Some(false),
        "the handler future is destroyed before the invocation ends"
    );
    timeout(
        Duration::from_millis(100),
        started.invocation.clone().ended(),
    )
    .await
    .expect("a late observer completes immediately");
    let late = Arc::new(AtomicUsize::new(0));
    let hook = Arc::clone(&late);
    started.invocation.on_end(move || {
        hook.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(late.load(Ordering::SeqCst), 1, "a late hook runs at once");
}

fn assert_running(started: &mut Started) {
    assert!(!started.invocation.has_ended());
    assert!(started.observed.try_recv().is_err());
    assert_eq!(started.hooks.load(Ordering::SeqCst), 0);
}

async fn status(pool: &PgPool, job_id: uuid::Uuid) -> JobStatus {
    get_job_by_id(pool, None, job_id)
        .await
        .expect("read job")
        .expect("job")
        .status
}

/// Blocks every transition out of LEASED while the test holds the advisory gate.
async fn install_outcome_gate(pool: &PgPool) {
    sqlx::raw_sql(
        "CREATE FUNCTION invocation_exit_gate() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN PERFORM pg_advisory_xact_lock(hashtext('runledger-invocation-exit-gate')); RETURN NEW; END $$;
         CREATE TRIGGER invocation_exit_gate BEFORE UPDATE ON job_queue FOR EACH ROW
         WHEN (OLD.status = 'LEASED' AND NEW.status <> 'LEASED')
         EXECUTE FUNCTION invocation_exit_gate();",
    )
    .execute(pool)
    .await
    .expect("install outcome gate");
}

async fn hold_gate(pool: &PgPool) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    let mut held = pool.acquire().await.expect("gate connection");
    sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
        .bind(GATE_KEY)
        .execute(&mut *held)
        .await
        .expect("hold outcome gate");
    held
}

/// While the outcome write is held, every observer has already seen the end.
async fn finish_through_gate(
    pool: &PgPool,
    job_id: uuid::Uuid,
    task: &mut JoinHandle<()>,
    started: &mut Started,
    mut held: sqlx::pool::PoolConnection<sqlx::Postgres>,
) {
    assert_ended(started).await;
    assert_eq!(
        status(pool, job_id).await,
        JobStatus::Leased,
        "the exit signal precedes outcome persistence"
    );
    sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock(hashtext($1))")
        .bind(GATE_KEY)
        .fetch_one(&mut *held)
        .await
        .expect("release outcome gate");
    drop(held);
    await_spawned_task(task, WAIT, "outcome persists", "worker joins").await;
}

async fn gated_exit(
    pool: &PgPool,
    exit: Exit,
    max_attempts: i32,
    timeout_seconds: Option<i32>,
) -> (uuid::Uuid, Started) {
    let (job_id, mut job) =
        enqueue_and_claim_job(pool, EXIT_TYPE, max_attempts, json!({}), "exit-worker").await;
    if let Some(seconds) = timeout_seconds {
        job.timeout_seconds = seconds;
    }
    let (registry, mut received, _) = exit_registry(exit);
    let held = hold_gate(pool).await;
    let mut task = tokio::spawn(process_claimed_job(
        pool.clone(),
        Arc::new(registry),
        job,
        30,
    ));
    let mut started = next_started(&mut received).await;
    finish_through_gate(pool, job_id, &mut task, &mut started, held).await;
    (job_id, started)
}

#[tokio::test]
async fn returned_results_panics_and_deadlines_end_the_invocation_before_the_outcome_persists() {
    let (pool, database) = setup_ephemeral_pool("invocation_exit_returns", 4).await;
    record_postgres_server_version(&pool, "invocation exit before outcome persistence").await;
    install_outcome_gate(&pool).await;
    for (exit, max_attempts, timeout_seconds, expected) in [
        (Exit::Succeed, 3, None, JobStatus::Succeeded),
        (Exit::Fail, 1, None, JobStatus::DeadLettered),
        (Exit::Panic, 3, None, JobStatus::DeadLettered),
        (Exit::Pending, 1, Some(1), JobStatus::DeadLettered),
    ] {
        let (job_id, _) = gated_exit(&pool, exit, max_attempts, timeout_seconds).await;
        let saved = get_job_by_id(&pool, None, job_id)
            .await
            .expect("read outcome")
            .expect("job");
        assert_eq!(saved.status, expected, "{exit:?}");
        if timeout_seconds.is_some() {
            assert_eq!(
                saved.last_error_code.as_deref(),
                Some("job.timeout_exceeded")
            );
        }
    }
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn continuations_and_retries_receive_fresh_invocations() {
    let (pool, database) = setup_ephemeral_pool("invocation_exit_fresh", 4).await;
    install_outcome_gate(&pool).await;
    for exit in [Exit::Continue, Exit::Fail] {
        let (job_id, first) = gated_exit(&pool, exit, 3, None).await;
        assert_eq!(status(&pool, job_id).await, JobStatus::Pending, "{exit:?}");
        sqlx::query("UPDATE job_queue SET next_run_at = now() WHERE id = $1")
            .bind(job_id)
            .execute(&pool)
            .await
            .expect("make the next invocation claimable");
        let next = claim_prestart_jobs(&pool, "exit-worker-next", 30, 1)
            .await
            .expect("claim next invocation")
            .pop()
            .expect("next invocation");
        assert_eq!(next.id, job_id);
        let (registry, mut received, proceed) = exit_registry(Exit::Release);
        let held = hold_gate(&pool).await;
        let mut task = tokio::spawn(process_claimed_job(
            pool.clone(),
            Arc::new(registry),
            next,
            30,
        ));
        let mut second = next_started(&mut received).await;
        assert!(first.invocation.has_ended());
        assert_running(&mut second);
        proceed.notify_one();
        finish_through_gate(&pool, job_id, &mut task, &mut second, held).await;
        assert_eq!(status(&pool, job_id).await, JobStatus::Succeeded);
    }
    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn lease_loss_and_lease_maintenance_failure_end_the_invocation() {
    let (pool, database) = setup_ephemeral_pool("invocation_exit_lease", 6).await;
    // A progress write discovers a replaced lease and the handler swallows it.
    let (job_id, job) = enqueue_and_claim_job(&pool, EXIT_TYPE, 3, json!({}), "exit-worker").await;
    let (registry, mut received, proceed) = exit_registry(Exit::SwallowLeaseLoss);
    let mut task = tokio::spawn(process_claimed_job(
        pool.clone(),
        Arc::new(registry),
        job,
        30,
    ));
    let mut started = next_started(&mut received).await;
    replace_lease_owner(&pool, job_id).await;
    proceed.notify_one();
    await_spawned_task(&mut task, WAIT, "lease loss abandons", "worker joins").await;
    assert_ended(&mut started).await;
    assert_eq!(status(&pool, job_id).await, JobStatus::Leased);

    // A heartbeat finds a replaced lease, or cannot finish within its budget.
    for blocked in [false, true] {
        let (job_id, job) =
            enqueue_and_claim_job_with_lease_ttl(&pool, EXIT_TYPE, 3, json!({}), "exit-worker", 3)
                .await;
        let (registry, mut received, _) = exit_registry(Exit::Pending);
        let mut task = tokio::spawn(process_claimed_job(
            pool.clone(),
            Arc::new(registry),
            job,
            3,
        ));
        let mut started = next_started(&mut received).await;
        let blocker = if blocked {
            let mut blocker = pool.begin().await.expect("begin heartbeat blocker");
            sqlx::query("SELECT id FROM job_queue WHERE id = $1 FOR UPDATE")
                .bind(job_id)
                .fetch_one(&mut *blocker)
                .await
                .expect("hold the job row");
            Some(blocker)
        } else {
            replace_lease_owner(&pool, job_id).await;
            None
        };
        await_spawned_task(
            &mut task,
            WAIT,
            "heartbeat failure abandons",
            "worker joins",
        )
        .await;
        assert_ended(&mut started).await;
        if let Some(blocker) = blocker {
            blocker.rollback().await.expect("release heartbeat blocker");
        }
    }
    teardown_ephemeral_pool(pool, database).await;
}

async fn replace_lease_owner(pool: &PgPool, job_id: uuid::Uuid) {
    sqlx::query("UPDATE job_queue SET worker_id = 'replacement-worker' WHERE id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .expect("replace lease owner");
}

#[tokio::test]
async fn graceful_drain_keeps_the_invocation_and_forced_abandonment_ends_it() {
    let (pool, database) = setup_ephemeral_pool("invocation_exit_drain", 4).await;
    // Aborting the task that drives a pending invocation destroys it.
    let (_, job) = enqueue_and_claim_job(&pool, EXIT_TYPE, 3, json!({}), "exit-worker").await;
    let (registry, mut received, _) = exit_registry(Exit::Pending);
    let task = tokio::spawn(process_claimed_job(
        pool.clone(),
        Arc::new(registry),
        job,
        30,
    ));
    let mut started = next_started(&mut received).await;
    task.abort();
    assert!(task.await.expect_err("job task aborted").is_cancelled());
    assert_ended(&mut started).await;

    for forced in [false, true] {
        let job_id = enqueue_exit_job(&pool).await;
        let (registry, mut received, proceed) = exit_registry(Exit::Release);
        let (stop, shutdown) = watch::channel(false);
        let mut worker = tokio::spawn(run_worker_loop(
            pool.clone(),
            registry,
            loop_config(),
            shutdown,
        ));
        let mut started = next_started(&mut received).await;
        stop.send(true).expect("worker observes stop");
        sleep(Duration::from_millis(200)).await;
        assert_running(&mut started);
        assert!(!worker.is_finished(), "drain awaits the active invocation");
        if forced {
            // Native escalation destroys the loop, whose task set aborts its jobs.
            worker.abort();
            assert!(worker.await.expect_err("loop aborted").is_cancelled());
            assert_ended(&mut started).await;
            assert_eq!(status(&pool, job_id).await, JobStatus::Leased);
        } else {
            proceed.notify_one();
            let exit = timeout(WAIT, &mut worker)
                .await
                .expect("drain finishes after the invocation")
                .expect("worker loop joins");
            assert_eq!(exit, RuntimeLoopExit::Shutdown);
            assert_ended(&mut started).await;
            assert_eq!(status(&pool, job_id).await, JobStatus::Succeeded);
        }
    }
    teardown_ephemeral_pool(pool, database).await;
}

async fn enqueue_exit_job(pool: &PgPool) -> uuid::Uuid {
    let mut tx = pool.begin().await.expect("begin definition");
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type: EXIT_TYPE,
            version: 1,
            max_attempts: 3,
            default_timeout_seconds: 30,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await
    .expect("upsert definition");
    tx.commit().await.expect("commit definition");
    let payload = json!({});
    enqueue_job(
        pool,
        &JobEnqueue {
            job_type: EXIT_TYPE,
            organization_id: None,
            payload: &payload,
            priority: None,
            max_attempts: None,
            timeout_seconds: None,
            next_run_at: None,
            idempotency_key: None,
            stage: Some(JobStage::Queued),
        },
    )
    .await
    .expect("enqueue job")
}

fn loop_config() -> JobsConfig {
    JobsConfig {
        worker_id: "exit-loop-worker".to_owned(),
        poll_interval: Duration::from_millis(20),
        claim_batch_size: 1,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(30),
        schedule_poll_interval: Duration::from_secs(30),
        reaper_retry_delay_ms: 1_000,
    }
}
