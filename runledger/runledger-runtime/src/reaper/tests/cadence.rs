use runledger_postgres::jobs::{
    JobDefinitionUpsert, JobEnqueue, ReapExpiredLeasesDetailedResult, claim_jobs_for_types,
    enqueue_job, reap_expired_leases_with_diagnostics, upsert_job_definition_tx,
};
use runledger_test_support::{
    setup_ephemeral_pool_with_untracked_migrations as setup_ephemeral_pool, teardown_ephemeral_pool,
};

use super::*;
use crate::reaper::cadence::ReaperNextPass::{AfterInterval, Immediately};
use crate::reaper::handle_successful_reap_iteration;

#[tokio::test]
async fn committed_batch_waits_only_when_observer_capacity_is_full() {
    let result = committed_reap_result().await;

    for (capacity, expected) in [(1, AfterInterval), (2, Immediately)] {
        let started = Arc::new(Notify::new());
        let started_count = Arc::new(AtomicUsize::new(0));
        let active_callbacks = Arc::new(AtomicUsize::new(0));
        let observers = JobLifecycleObservers::from_observer(CancellableReapedObserver {
            started: started.clone(),
            started_count: started_count.clone(),
            active_callbacks: active_callbacks.clone(),
        });
        let mut tasks = ReapedObserverTasks::owned_with_max_concurrency(capacity);
        let (_shutdown, mut shutdown_rx) = watch::channel(false);

        let next_pass = handle_successful_reap_iteration(
            &JobRegistry::new(),
            &observers,
            &mut tasks,
            &result,
            1,
            &mut shutdown_rx,
        )
        .await;
        let callback_started = timeout(
            Duration::from_secs(1),
            wait_for_count(started_count, started, 1),
        )
        .await;
        let active = active_callbacks.load(Ordering::SeqCst);
        tasks.abort_for_shutdown().await;

        callback_started.expect("the reaped observer entered its pending callback");
        assert_eq!(active, 1, "the observer was parked until explicit cleanup");
        assert_eq!(active_callbacks.load(Ordering::SeqCst), 0);
        assert_eq!(next_pass, expected, "observer capacity {capacity}");
    }
}

async fn committed_reap_result() -> ReapExpiredLeasesDetailedResult {
    let (pool, database) = setup_ephemeral_pool("reaper_observer_capacity", 2).await;
    let job_type = JobType::new("jobs.test.reaper.capacity");
    let mut tx = pool.begin().await.expect("begin definition transaction");
    upsert_job_definition_tx(
        &mut tx,
        &JobDefinitionUpsert {
            job_type,
            version: 1,
            max_attempts: 3,
            default_timeout_seconds: 30,
            default_priority: 100,
            is_enabled: true,
        },
    )
    .await
    .expect("register job definition");
    tx.commit().await.expect("commit definition");
    let job = enqueue_job(
        &pool,
        &JobEnqueue {
            job_type,
            organization_id: None,
            payload: &json!({}),
            priority: None,
            max_attempts: None,
            timeout_seconds: None,
            next_run_at: None,
            idempotency_key: None,
            stage: None,
        },
    )
    .await
    .expect("enqueue job");
    claim_jobs_for_types(&pool, "expired-lease-owner", 60, 1, &[job_type])
        .await
        .expect("claim job");
    sqlx::query(
        "UPDATE job_queue SET lease_expires_at = now() - interval '10 seconds' WHERE id = $1",
    )
    .bind(job)
    .execute(&pool)
    .await
    .expect("expire lease");
    let result = reap_expired_leases_with_diagnostics(&pool, 1, 1_000).await;
    teardown_ephemeral_pool(pool, database).await;
    let result = result.expect("commit a full progressed reaper batch");
    assert_eq!(result.summary.processed, 1);
    assert_eq!(result.reaped_leases.len(), 1);
    result
}
