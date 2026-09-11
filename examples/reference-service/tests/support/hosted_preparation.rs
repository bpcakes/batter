use super::{ProbeResult, hosted_worker, hosted_worker_ownership::assert_no_advisory_locks};
use batter::{
    cleanup::{CleanupBudget, CleanupStack},
    operation::OperationContext,
};
use batter_example_reference_service::{
    schema::initialize_schema,
    worker::{
        DependencyCleanup, STARTUP_JOB_TYPE, TerminationGate, TerminationState, UnlockOutcome,
        WorkerStartupError, prepare_probe_worker,
    },
};
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobEnqueue, claim_jobs, enqueue_job_with_outcome};
use sqlx::{PgPool, types::Uuid};
use std::time::Duration;

pub async fn wait_blocked(pool: &PgPool, query: &str) -> ProbeResult {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() AND wait_event_type = 'Lock' AND query LIKE $1)")
                .bind(format!("%{query}%")).fetch_one(pool).await?;
            if blocked { return Ok::<_, sqlx::Error>(()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}

pub async fn cancellation_releases_preparation(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    cancel_preparation(&pool, false).await?;
    cancel_preparation(&pool, true).await
}

async fn cancel_preparation(pool: &PgPool, cancel_parent: bool) -> ProbeResult {
    let mut blocker = pool.begin().await?;
    sqlx::query("LOCK TABLE job_queue IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let gate = TerminationGate::new();
    let parent = OperationContext::new(Duration::from_secs(35))?;
    let task_pool = pool.clone();
    let task_gate = gate.clone();
    let task_parent = parent.clone();
    let waiter = tokio::spawn(async move {
        prepare_probe_worker(
            &task_pool,
            &hosted_worker::settings("preparing", 1).unwrap(),
            task_gate,
            &task_parent,
            Duration::from_secs(20),
        )
        .await
    });
    let blocked = wait_blocked(pool, "FROM job_queue").await;
    if blocked.is_err() {
        waiter.abort();
    }
    blocked?;
    if cancel_parent {
        parent.cancel();
    } else {
        waiter.abort();
    }
    let result = waiter.await;
    if cancel_parent {
        assert!(
            matches!(result, Ok(Err(error)) if matches!(error.as_ref(), WorkerStartupError::Cancelled))
        );
    } else {
        assert!(result.err().unwrap().is_cancelled());
    }
    assert_preparation_settled(pool, &gate).await?;
    blocker.rollback().await?;
    let successor = prepare_probe_worker(
        pool,
        &hosted_worker::settings("successor", 1)?,
        TerminationGate::new(),
        &parent_for_successor()?,
        Duration::from_secs(5),
    )
    .await?;
    successor.shutdown().await?;
    Ok(())
}

async fn assert_preparation_settled(pool: &PgPool, gate: &TerminationGate) -> ProbeResult {
    let second = Duration::from_secs(1);
    let cleanup = DependencyCleanup::new(
        CleanupStack::new(),
        CleanupBudget::new(second, second, second)?,
        gate.clone(),
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(4), cleanup.finish())
            .await?
            .is_success()
    );
    assert_eq!(gate.state(), TerminationState::NotStarted);
    let settled = gate.settlement();
    assert!(matches!(
        settled.preparation.as_deref(),
        Some(WorkerStartupError::Cancelled)
    ));
    assert!(matches!(
        settled.release.as_deref().unwrap().unlock,
        UnlockOutcome::Confirmed
    ));
    assert_no_advisory_locks(pool).await?;
    Ok(())
}

fn parent_for_successor() -> Result<OperationContext, batter::ConfigurationError> {
    OperationContext::new(Duration::from_secs(35))
}

pub async fn leased_and_terminal_reconciliation(pool: PgPool) -> ProbeResult {
    initialize_schema(&pool).await?;
    let settings = hosted_worker::settings("reconcile", 1)?;
    prepare_probe_worker(
        &pool,
        &settings,
        TerminationGate::new(),
        &parent_for_successor()?,
        Duration::from_secs(5),
    )
    .await?
    .shutdown()
    .await?;
    for terminal_race in [false, true] {
        let payload = serde_json::json!({"witness_id": Uuid::now_v7()});
        let queued = enqueue_job_with_outcome(
            &pool,
            &JobEnqueue {
                job_type: JobType::new(STARTUP_JOB_TYPE),
                organization_id: None,
                payload: &payload,
                priority: None,
                max_attempts: None,
                timeout_seconds: None,
                next_run_at: None,
                idempotency_key: None,
                stage: None,
            },
        )
        .await?;
        let claims = claim_jobs(&pool, "previous-owner", 60, 1).await?;
        assert_eq!(claims[0].id, queued.job_id);
        let mut transition = pool.begin().await?;
        if terminal_race {
            // Hold an uncommitted terminal transition. Preparation sees LEASED,
            // then its cancellation UPDATE must wait and recheck after commit.
            sqlx::query(
                "UPDATE job_queue SET status = 'SUCCEEDED', finished_at = now() WHERE id = $1",
            )
            .bind(queued.job_id)
            .execute(&mut *transition)
            .await?;
        }
        let owned = pool.clone();
        let settings = settings.clone();
        let waiter = tokio::spawn(async move {
            prepare_probe_worker(
                &owned,
                &settings,
                TerminationGate::new(),
                &parent_for_successor().unwrap(),
                Duration::from_secs(5),
            )
            .await
        });
        if terminal_race {
            let blocked = wait_blocked(&pool, "SET status = 'CANCELED'").await;
            if blocked.is_err() {
                waiter.abort();
            }
            blocked?;
        }
        transition.commit().await?;
        waiter.await??.shutdown().await?;
        let status: String = sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
            .bind(queued.job_id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            status,
            if terminal_race {
                "SUCCEEDED"
            } else {
                "CANCELED"
            }
        );
    }
    Ok(())
}
