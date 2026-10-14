use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::watch;
use tokio::time::{sleep, timeout};

use super::super::super::{overtaken_claim_release_no_longer_applies, run_worker_loop};
use super::super::{CountingHandler, setup_ephemeral_pool, teardown_ephemeral_pool};
use super::{capacity_config, enqueue_refill_jobs};
use crate::RuntimeLoopExit;
use crate::registry::JobRegistry;
use runledger_core::jobs::JobStatus;
use runledger_postgres::jobs::{
    JobLeaseIdentity, JobQueueRecord, get_job_by_id, release_unstarted_job_claim,
};

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

// The loop checks for a stop before claiming; holding the queue table makes
// the stop arrive while the claim itself is in flight.
async fn stop_worker_during_claim(pool: &PgPool, runs: Arc<AtomicUsize>) -> RuntimeLoopExit {
    let mut queue_lock = pool.begin().await.expect("begin queue lock");
    sqlx::query("LOCK TABLE job_queue IN EXCLUSIVE MODE")
        .execute(&mut *queue_lock)
        .await
        .expect("lock job_queue");
    let mut registry = JobRegistry::new();
    registry.register(CountingHandler { runs });
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(run_worker_loop(
        pool.clone(),
        registry,
        capacity_config(4, Duration::from_secs(30)),
        shutdown_rx,
    ));
    wait_for_claim_to_block_on_queue_lock(pool).await;
    shutdown_tx
        .send(true)
        .expect("worker receives the stop request");
    queue_lock.commit().await.expect("release queue lock");

    timeout(Duration::from_secs(5), worker)
        .await
        .expect("worker exits promptly after shutdown")
        .expect("worker join succeeds")
}

async fn count_stopped_before_start_requeues(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
         FROM job_events
         WHERE event_type = 'REQUEUED'
           AND payload->>'reason' = 'WORKER_STOPPED_BEFORE_START'",
    )
    .fetch_one(pool)
    .await
    .expect("count release events")
}

#[tokio::test]
async fn stop_overtaking_an_in_flight_claim_releases_unstarted_jobs() {
    const JOBS: usize = 3;
    let (pool, database) = setup_ephemeral_pool("jobs_worker_stop_overtakes_claim", 8).await;
    let job_ids = enqueue_refill_jobs(&pool, JOBS).await;
    let runs = Arc::new(AtomicUsize::new(0));

    assert_eq!(
        stop_worker_during_claim(&pool, runs.clone()).await,
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
    assert_eq!(
        count_stopped_before_start_requeues(&pool).await,
        JOBS as i64,
        "the overtaken claim leased every job before releasing it"
    );

    teardown_ephemeral_pool(pool, database).await;
}

#[tokio::test]
async fn failed_overtaken_claim_release_does_not_stop_later_releases_or_drain() {
    const JOBS: usize = 3;
    let (pool, database) = setup_ephemeral_pool("jobs_worker_stop_partial_release", 8).await;
    let job_ids = enqueue_refill_jobs(&pool, JOBS).await;
    // Only the first release fails, whichever job it is: the failed release
    // transaction does not roll back nextval.
    sqlx::raw_sql(
        "CREATE SEQUENCE overtaken_release_attempts;
         CREATE FUNCTION fail_first_overtaken_release() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF nextval('overtaken_release_attempts') = 1 THEN
                 RAISE EXCEPTION 'injected unstarted release failure';
             END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER fail_first_overtaken_release BEFORE UPDATE ON job_queue FOR EACH ROW
         WHEN (OLD.status = 'LEASED' AND NEW.status = 'PENDING')
         EXECUTE FUNCTION fail_first_overtaken_release();",
    )
    .execute(&pool)
    .await
    .expect("inject first release failure");
    let runs = Arc::new(AtomicUsize::new(0));

    assert_eq!(
        stop_worker_during_claim(&pool, runs.clone()).await,
        RuntimeLoopExit::Shutdown
    );
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let mut left_leased = Vec::new();
    let mut released = Vec::new();
    for job_id in job_ids {
        let job = get_job_by_id(&pool, None, job_id)
            .await
            .expect("load overtaken job")
            .expect("overtaken job exists");
        match job.status {
            JobStatus::Leased => left_leased.push(job),
            JobStatus::Pending => released.push(job),
            other => panic!("overtaken job reached {other:?} without starting"),
        }
    }
    assert_eq!(
        (left_leased.len(), released.len()),
        (1, JOBS - 1),
        "the failed release must not stop the releases after it"
    );
    let failed = &left_leased[0];
    assert_eq!(failed.attempt, 1);
    assert_eq!(failed.worker_id.as_deref(), Some("worker-capacity-test"));
    assert!(released.iter().all(|job| job.attempt == 0));
    assert_eq!(
        count_stopped_before_start_requeues(&pool).await,
        JOBS as i64 - 1
    );

    assert_release_misses_are_classified_apart_from_failures(&pool, failed, &released[0]).await;

    teardown_ephemeral_pool(pool, database).await;
}

// Real release errors against the state above: a moved lease is not left for
// the reaper, while the injected failure still is.
async fn assert_release_misses_are_classified_apart_from_failures(
    pool: &PgPool,
    failed: &JobQueueRecord,
    released: &JobQueueRecord,
) {
    let release = |job: &JobQueueRecord, attempt, worker_id: &'static str| {
        let identity = JobLeaseIdentity::new(job.id, job.run_number, attempt, worker_id);
        release_unstarted_job_claim(pool, identity, "TEST_CLASSIFICATION", 0)
    };
    let requeued = release(released, 1, "worker-capacity-test")
        .await
        .expect_err("a requeued attempt no longer holds its lease");
    let reassigned = release(failed, failed.attempt, "worker-other")
        .await
        .expect_err("another worker's lease is not releasable");
    sqlx::query("SELECT setval('overtaken_release_attempts', 1, false)")
        .execute(pool)
        .await
        .expect("rearm release failure");
    let injected = release(failed, failed.attempt, "worker-capacity-test")
        .await
        .expect_err("the rearmed release fails");

    assert!(overtaken_claim_release_no_longer_applies(&requeued));
    assert!(overtaken_claim_release_no_longer_applies(&reassigned));
    assert!(!overtaken_claim_release_no_longer_applies(&injected));
}
