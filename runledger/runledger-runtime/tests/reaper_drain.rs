//! Reaper cadence: whole batches with progress drain at once, others wait.
use std::future::Future;
use std::time::Duration;

use runledger_core::jobs::{JobStage, JobType};
use runledger_postgres::jobs::{JobEnqueue, claim_jobs_for_types, enqueue_job};
use runledger_runtime::RuntimeLoopExit;
use runledger_runtime::config::JobsConfig;
use runledger_runtime::reaper::run_reaper_loop;
use runledger_runtime::registry::JobRegistry;
use runledger_test_support::{
    setup_ephemeral_pool_with_untracked_migrations as setup_ephemeral_pool, teardown_ephemeral_pool,
};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

use job_definition::register_job_definition;

#[path = "support/job_definition.rs"]
mod job_definition;

#[path = "support/pass_gate.rs"]
mod pass_gate;

const JOB_TYPE: &str = "jobs.test.reaper.cadence";
const QUIET_WINDOW: Duration = Duration::from_millis(1_500);

fn config(claim_batch_size: i64, reaper_retry_delay_ms: i32) -> JobsConfig {
    JobsConfig {
        worker_id: "reaper-cadence-test-worker".to_string(),
        poll_interval: Duration::from_secs(30),
        claim_batch_size,
        lease_ttl_seconds: 30,
        max_global_concurrency: 1,
        reaper_interval: Duration::from_secs(3_600),
        schedule_poll_interval: Duration::from_secs(30),
        reaper_retry_delay_ms,
    }
}

/// Enqueues, claims and expires `count` unstarted leases.
async fn expired_leases(pool: &PgPool, count: i64) {
    let job_type = JobType::new(JOB_TYPE);
    register_job_definition(pool, job_type).await;
    for index in 0..count {
        enqueue_job(
            pool,
            &JobEnqueue {
                job_type,
                organization_id: None,
                payload: &json!({ "index": index }),
                priority: None,
                max_attempts: None,
                timeout_seconds: None,
                next_run_at: None,
                idempotency_key: None,
                stage: Some(JobStage::Queued),
            },
        )
        .await
        .expect("enqueue job");
    }
    let mut claimed = 0;
    while claimed < count {
        let batch = claim_jobs_for_types(pool, "expired-lease-owner", 60, 100, &[job_type])
            .await
            .expect("claim jobs");
        assert!(!batch.is_empty(), "every enqueued job is claimable");
        claimed += i64::try_from(batch.len()).expect("batch length fits");
    }
    sqlx::query(
        "UPDATE job_queue SET lease_expires_at = now() - interval '10 seconds'
         WHERE job_type = $1 AND status = 'LEASED'",
    )
    .bind(JOB_TYPE)
    .execute(pool)
    .await
    .expect("expire leases");
}

async fn leased_jobs(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM job_queue WHERE job_type = $1 AND status = 'LEASED'",
    )
    .bind(JOB_TYPE)
    .fetch_one(pool)
    .await
    .expect("count leased jobs")
}

/// Makes every lease transition fail, counting attempts with a sequence that
/// survives the reaper's row savepoint rollback. Deferral keeps the lease and
/// therefore still succeeds.
async fn poison_lease_transitions(pool: &PgPool) {
    sqlx::query("CREATE SEQUENCE test_lease_transitions")
        .execute(pool)
        .await
        .expect("create attempt counter");
    sqlx::query(
        "CREATE FUNCTION test_poison_lease_transition() RETURNS trigger
         LANGUAGE plpgsql AS $body$
         BEGIN
             PERFORM nextval('test_lease_transitions');
             RAISE EXCEPTION 'poisoned lease transition';
         END
         $body$",
    )
    .execute(pool)
    .await
    .expect("create poison function");
    sqlx::query(
        "CREATE TRIGGER test_poison_lease_transition BEFORE UPDATE ON job_queue
         FOR EACH ROW
         WHEN (OLD.status = 'LEASED' AND NEW.status <> 'LEASED')
         EXECUTE FUNCTION test_poison_lease_transition()",
    )
    .execute(pool)
    .await
    .expect("create poison trigger");
}

async fn lease_transition_attempts(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM test_lease_transitions",
    )
    .fetch_one(pool)
    .await
    .expect("read attempt counter")
}

async fn wait_until<F, Fut>(what: &str, within: Duration, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + within;
    while !condition().await {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

fn start(pool: &PgPool, config: JobsConfig) -> (watch::Sender<bool>, JoinHandle<RuntimeLoopExit>) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(run_reaper_loop(
        pool.clone(),
        JobRegistry::new(),
        config,
        shutdown_rx,
    ));
    (shutdown_tx, task)
}

async fn stop(shutdown_tx: watch::Sender<bool>, task: JoinHandle<RuntimeLoopExit>) {
    shutdown_tx.send(true).expect("reaper receives shutdown");
    let exit = timeout(Duration::from_secs(5), task)
        .await
        .expect("reaper returns within one pass of shutdown")
        .expect("reaper task joins");
    assert_eq!(exit, RuntimeLoopExit::Shutdown);
}

#[tokio::test]
async fn reaper_loop_drains_full_batches_without_waiting_for_the_interval() {
    const BATCH: i64 = 2;
    const TOTAL: i64 = 3 * BATCH + 1;
    let (pool, database) = setup_ephemeral_pool("runtime_reaper_backlog_drain", 4).await;
    expired_leases(&pool, TOTAL).await;

    let (shutdown_tx, task) = start(&pool, config(BATCH, 1_000));
    wait_until(
        "every expired lease is reaped inside one hour-long reaper interval",
        Duration::from_secs(10),
        || async { leased_jobs(&pool).await == 0 },
    )
    .await;
    stop(shutdown_tx, task).await;

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn reaper_loop_waits_after_a_full_batch_of_only_deferred_rows() {
    const BATCH: i64 = 3;
    let (pool, database) = setup_ephemeral_pool("runtime_reaper_deferred_batch", 4).await;
    expired_leases(&pool, BATCH).await;
    poison_lease_transitions(&pool).await;

    // A one-millisecond retry delay makes every deferred row expire again
    // before the next pass could start.
    let (shutdown_tx, task) = start(&pool, config(BATCH, 1));
    wait_until(
        "each poisoned lease is attempted",
        Duration::from_secs(5),
        || async { lease_transition_attempts(&pool).await >= BATCH },
    )
    .await;
    sleep(QUIET_WINDOW).await;

    assert_eq!(
        lease_transition_attempts(&pool).await,
        BATCH,
        "a whole batch of deferred rows made no progress, so the reaper must wait its interval"
    );
    assert_eq!(
        leased_jobs(&pool).await,
        BATCH,
        "deferred rows keep their lease"
    );
    stop(shutdown_tx, task).await;

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn reaper_loop_observes_shutdown_between_back_to_back_full_batches() {
    const BATCH: i64 = 1;
    const TOTAL: i64 = 8;
    let (pool, database) = setup_ephemeral_pool("runtime_reaper_drain_shutdown", 4).await;
    expired_leases(&pool, TOTAL).await;
    let (gate, gate_pid) = pass_gate::install(
        &pool,
        "CREATE TRIGGER test_park_reaper_pass BEFORE UPDATE ON job_queue FOR EACH ROW
         WHEN (OLD.status = 'LEASED' AND NEW.status <> 'LEASED')
         EXECUTE FUNCTION test_gate_pass()",
    )
    .await;

    let (shutdown_tx, task) = start(&pool, config(BATCH, 1_000));
    pass_gate::wait(&pool, gate_pid).await;
    assert_eq!(TOTAL - leased_jobs(&pool).await, 3);
    shutdown_tx
        .send(true)
        .expect("request stop while pass four is held");
    gate.rollback().await.expect("release pass four");
    assert_eq!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("reaper stops")
            .expect("join"),
        RuntimeLoopExit::Shutdown
    );
    let reaped_after_shutdown = TOTAL - leased_jobs(&pool).await;
    assert_eq!(
        reaped_after_shutdown, 4,
        "only the held pass may finish after stop"
    );

    teardown_ephemeral_pool(pool, database).await;
}
