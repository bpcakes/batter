use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{Notify, watch};
use tokio::time::{Instant, sleep, timeout};

use super::super::{WorkerLoop, WorkerLoopControl, run_worker_loop};
use super::{CountingHandler, setup_ephemeral_pool, teardown_ephemeral_pool};
use crate::config::JobsConfig;
use crate::observer::JobLifecycleObservers;
use crate::registry::JobRegistry;
use crate::settlement::TaskRegistry;
use crate::shutdown::{self, ShutdownSignal};
use crate::{RuntimeLoopExit, RuntimeShutdownCause};
use runledger_core::jobs::{JobStatus, JobType};
use runledger_postgres::jobs::{
    JobDefinitionUpsert, JobEnqueue, enqueue_job, get_job_by_id, upsert_job_definition_tx,
};

const UNUSED_LAZY_POOL_URL: &str =
    "postgres://postgres:postgres@127.0.0.1:65535/runledger_worker_capacity_test";
const REFILL_JOB_TYPE: JobType<'static> = JobType::new("jobs.test.pre_run_lease_loss");

fn capacity_config(max_global_concurrency: usize, poll_interval: Duration) -> JobsConfig {
    JobsConfig {
        worker_id: "worker-capacity-test".to_owned(),
        poll_interval,
        claim_batch_size: 4,
        lease_ttl_seconds: 30,
        max_global_concurrency,
        reaper_interval: Duration::from_secs(30),
        schedule_poll_interval: Duration::from_secs(30),
        reaper_retry_delay_ms: 1_000,
    }
}

fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .connect_lazy(UNUSED_LAZY_POOL_URL)
        .expect("construct lazy capacity-test pool")
}

fn worker_with_capacity(max_global_concurrency: usize) -> WorkerLoop {
    let (_, shutdown) = watch::channel(false);
    WorkerLoop::new(
        lazy_pool(),
        JobRegistry::new(),
        capacity_config(max_global_concurrency, Duration::from_millis(10)),
        shutdown,
        JobLifecycleObservers::empty(),
    )
}

// A single slot and a poll interval no test waits out: only a job exit or a
// stop request can end the capacity wait.
fn saturated_worker(shutdown: watch::Receiver<bool>, tasks: TaskRegistry) -> WorkerLoop {
    WorkerLoop::with_tasks(
        lazy_pool(),
        JobRegistry::new(),
        capacity_config(1, Duration::from_secs(60)),
        shutdown,
        JobLifecycleObservers::empty(),
        tasks,
    )
}

async fn enqueue_refill_jobs(pool: &PgPool, jobs: usize) -> Vec<uuid::Uuid> {
    let mut tx = pool.begin().await.expect("begin tx");
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type: REFILL_JOB_TYPE,
            version: 1,
            max_attempts: 1,
            default_timeout_seconds: 30,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await
    .expect("upsert job definition");
    tx.commit().await.expect("commit tx");
    let mut job_ids = Vec::with_capacity(jobs);
    for index in 0..jobs {
        job_ids.push(
            enqueue_job(
                pool,
                &JobEnqueue {
                    job_type: REFILL_JOB_TYPE,
                    organization_id: None,
                    payload: &json!({ "refill": index }),
                    priority: None,
                    max_attempts: None,
                    timeout_seconds: None,
                    next_run_at: None,
                    idempotency_key: None,
                    stage: Some(runledger_core::jobs::JobStage::Queued),
                },
            )
            .await
            .expect("enqueue refill job"),
        );
    }
    job_ids
}

async fn wait_for_claim_to_block_on_queue_lock(pool: &PgPool) {
    for _ in 0..100 {
        let waiting = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                 SELECT 1
                 FROM pg_stat_activity
                 WHERE datname = current_database()
                   AND pid <> pg_backend_pid()
                   AND wait_event_type = 'Lock'
                   AND query LIKE '%eligible_resource_jobs%'
                   AND query NOT LIKE '%pg_stat_activity%'
             )",
        )
        .fetch_one(pool)
        .await
        .expect("query waiting claim activity");
        if waiting {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("worker claim did not block on the queue table lock");
}

#[tokio::test]
async fn join_set_occupancy_is_worker_capacity() {
    let mut worker = worker_with_capacity(2);
    assert_eq!(worker.available_capacity(), 2);

    worker.join_set.spawn(pending());
    assert_eq!(worker.available_capacity(), 1);
    worker.join_set.spawn(pending());
    assert_eq!(worker.available_capacity(), 0);

    worker.join_set.abort_all();
    while worker.join_set.join_next().await.is_some() {}
    assert_eq!(worker.available_capacity(), 2);
}

#[tokio::test]
async fn crashed_job_task_is_drained_and_restores_capacity() {
    let mut worker = worker_with_capacity(1);
    worker
        .join_set
        .spawn(async { panic!("capacity test panic") });
    assert_eq!(worker.available_capacity(), 0);

    for _ in 0..100 {
        worker.drain_finished_tasks().await;
        if worker.join_set.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert!(worker.join_set.is_empty());
    assert_eq!(worker.available_capacity(), 1);
}

#[tokio::test]
async fn shutdown_drain_waits_for_join_set_tasks() {
    let mut worker = worker_with_capacity(1);
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let task_started = Arc::clone(&started);
    let task_release = Arc::clone(&release);
    worker.join_set.spawn(async move {
        task_started.notify_one();
        task_release.notified().await;
    });
    started.notified().await;

    let mut drain = tokio::spawn(worker.drain(RuntimeLoopExit::Shutdown));
    assert!(
        timeout(Duration::from_millis(25), &mut drain)
            .await
            .is_err(),
        "worker drain returned before its JoinSet task completed"
    );

    release.notify_waiters();
    assert_eq!(
        timeout(Duration::from_secs(1), drain)
            .await
            .expect("worker drain should finish after release")
            .expect("worker drain task should not panic"),
        RuntimeLoopExit::Shutdown
    );
}

#[tokio::test(start_paused = true)]
async fn job_exit_at_capacity_wakes_worker_before_poll_interval() {
    let (_shutdown_tx, shutdown) = watch::channel(false);
    let mut worker = saturated_worker(shutdown, TaskRegistry::default());
    worker.join_set.spawn(sleep(Duration::from_millis(20)));
    assert_eq!(worker.available_capacity(), 0);

    let waiting = Instant::now();
    assert!(
        !worker.wait_for_shutdown_job_exit_or_poll_interval().await,
        "a job exit is not a stop request"
    );

    assert!(
        waiting.elapsed() < worker.config.poll_interval,
        "worker at capacity waited out poll_interval after a job exit"
    );
    assert!(worker.join_set.is_empty());
    assert_eq!(worker.available_capacity(), 1);
}

#[tokio::test]
async fn crashed_job_exit_at_capacity_stops_admission_through_supervised_failure() {
    let (signal, shutdown) = ShutdownSignal::channel();
    let mut worker = saturated_worker(shutdown, TaskRegistry::supervised(signal.clone()));
    worker
        .join_set
        .spawn(async { panic!("capacity wake panic") });

    timeout(
        Duration::from_secs(5),
        worker.wait_for_shutdown_job_exit_or_poll_interval(),
    )
    .await
    .expect("a crashed job exit wakes a worker at capacity");

    assert!(
        shutdown::is_requested_or_closed(&worker.shutdown),
        "consuming the crashed exit must still record the fatal descendant failure"
    );
    assert!(matches!(
        signal.cause(),
        RuntimeShutdownCause::DescendantFailure {
            task: "worker_job",
            ..
        }
    ));
    assert!(matches!(
        worker.iteration().await,
        WorkerLoopControl::Drain(RuntimeLoopExit::Shutdown)
    ));
    assert!(worker.join_set.is_empty());
}

#[tokio::test]
async fn stop_request_wins_over_simultaneous_job_exit_at_capacity() {
    let (shutdown_tx, shutdown) = watch::channel(false);
    let mut worker = saturated_worker(shutdown, TaskRegistry::default());
    let exited = worker.join_set.spawn(async {});
    while !exited.is_finished() {
        tokio::task::yield_now().await;
    }
    shutdown_tx
        .send(true)
        .expect("worker receives the stop request");

    assert!(worker.wait_for_shutdown_job_exit_or_poll_interval().await);
    assert_eq!(
        worker.join_set.len(),
        1,
        "drain, not the capacity wait, observes an exit that races a stop"
    );
    assert_eq!(
        worker.drain(RuntimeLoopExit::Shutdown).await,
        RuntimeLoopExit::Shutdown
    );
}

#[tokio::test]
async fn worker_refills_exhausted_capacity_on_job_exit_without_poll_delay() {
    const JOBS: usize = 4;
    let (pool, database) = setup_ephemeral_pool("jobs_worker_exit_refill", 8).await;
    let job_ids = enqueue_refill_jobs(&pool, JOBS).await;

    let runs = Arc::new(AtomicUsize::new(0));
    let mut registry = JobRegistry::new();
    registry.register(CountingHandler { runs: runs.clone() });
    let mut config = capacity_config(1, Duration::from_secs(30));
    config.claim_batch_size = 1;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_worker_loop(pool.clone(), registry, config, shutdown_rx));

    // Every claim fills the only slot; with the old capacity wait each refill
    // would have waited out the 30 s poll interval.
    let deadline = Instant::now() + Duration::from_secs(10);
    while runs.load(Ordering::SeqCst) < JOBS {
        assert!(
            Instant::now() < deadline,
            "worker at capacity waited for poll_interval after a job exit freed its slot"
        );
        sleep(Duration::from_millis(10)).await;
    }

    shutdown_tx
        .send(true)
        .expect("worker receives the stop request");
    assert_eq!(
        timeout(Duration::from_secs(5), worker)
            .await
            .expect("worker exits promptly after shutdown")
            .expect("worker join succeeds"),
        RuntimeLoopExit::Shutdown
    );
    assert_eq!(runs.load(Ordering::SeqCst), JOBS);
    for job_id in job_ids {
        let persisted = get_job_by_id(&pool, None, job_id)
            .await
            .expect("load refill job")
            .expect("refill job exists");
        assert_eq!(persisted.status, JobStatus::Succeeded);
    }

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn stop_overtaking_an_in_flight_claim_releases_unstarted_jobs() {
    const JOBS: usize = 3;
    let (pool, database) = setup_ephemeral_pool("jobs_worker_stop_overtakes_claim", 8).await;
    let job_ids = enqueue_refill_jobs(&pool, JOBS).await;

    // The loop checks for a stop before claiming; holding the queue table makes
    // the stop arrive while the claim itself is in flight.
    let mut queue_lock = pool.begin().await.expect("begin queue lock");
    sqlx::query("LOCK TABLE job_queue IN EXCLUSIVE MODE")
        .execute(&mut *queue_lock)
        .await
        .expect("lock job_queue");
    let runs = Arc::new(AtomicUsize::new(0));
    let mut registry = JobRegistry::new();
    registry.register(CountingHandler { runs: runs.clone() });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_worker_loop(
        pool.clone(),
        registry,
        capacity_config(4, Duration::from_secs(30)),
        shutdown_rx,
    ));
    wait_for_claim_to_block_on_queue_lock(&pool).await;
    shutdown_tx
        .send(true)
        .expect("worker receives the stop request");
    queue_lock.commit().await.expect("release queue lock");

    assert_eq!(
        timeout(Duration::from_secs(5), worker)
            .await
            .expect("worker exits promptly after shutdown")
            .expect("worker join succeeds"),
        RuntimeLoopExit::Shutdown
    );
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "a claim overtaken by a stop request must not start its jobs"
    );
    for job_id in job_ids {
        let job = get_job_by_id(&pool, None, job_id)
            .await
            .expect("load released job")
            .expect("released job exists");
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.attempt, 0, "release restores the attempt budget");
        assert!(job.worker_id.is_none());
    }
    let attempts = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM job_attempts")
        .fetch_one(&pool)
        .await
        .expect("count attempts");
    assert_eq!(attempts, 0, "release deletes the unstarted attempt rows");
    let released = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
         FROM job_events
         WHERE event_type = 'REQUEUED'
           AND payload->>'reason' = 'WORKER_STOPPED_BEFORE_START'",
    )
    .fetch_one(&pool)
    .await
    .expect("count release events");
    assert_eq!(
        released, JOBS as i64,
        "the overtaken claim leased every job before releasing it"
    );

    teardown_ephemeral_pool(pool, database).await;
}
