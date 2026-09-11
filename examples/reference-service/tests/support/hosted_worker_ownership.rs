use super::{ProbeResult, fixture_diagnostics::ProbeError, hosted_worker};
use batter::BoxError;
use batter::operation::OperationContext;
use batter_example_reference_service::worker::{
    ProbeLeaseError, STARTUP_JOB_TYPE, STARTUP_LEASE_LOCK_CLASS, STARTUP_LEASE_LOCK_KEY,
    TerminationGate, TerminationState, WorkerDriverError, prepare_probe_worker,
};
use runledger_core::jobs::JobType;
use runledger_postgres::jobs::{JobEnqueue, enqueue_job_with_outcome};
use serde_json::json;
use sqlx::{PgPool, types::Uuid};
use std::time::Duration;

pub async fn stale_control_and_registration_recovery(pool: &PgPool) -> ProbeResult {
    let stale_payload = json!({"witness_id": Uuid::now_v7()});
    let stale_request = JobEnqueue {
        job_type: JobType::new(STARTUP_JOB_TYPE),
        organization_id: None,
        payload: &stale_payload,
        priority: None,
        max_attempts: Some(1),
        timeout_seconds: Some(30),
        next_run_at: None,
        idempotency_key: Some("stale-startup-control"),
        stage: None,
    };
    let stale = enqueue_job_with_outcome(pool, &stale_request).await?;
    let rejected_gate = TerminationGate::new();
    let rejected = prepare_probe_worker(
        pool,
        &hosted_worker::settings("hosted-probe-rejected", 1)?,
        rejected_gate.clone(),
        &OperationContext::new(Duration::from_secs(35))?,
        Duration::from_secs(20),
    )
    .await?;
    let stale_status: String =
        sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
            .bind(stale.job_id)
            .fetch_one(pool)
            .await?;
    assert_eq!(stale_status, "CANCELED");

    let mut duplicate = hosted_worker::process_supervisor();
    duplicate.register("worker", |_signal| async { Ok(()) })?;
    let rejected = rejected
        .register(&mut duplicate)
        .expect_err("duplicate registration must retain the prepared host");
    assert!(matches!(
        rejected.registration_error(),
        batter::RegistrationError::Duplicate("worker")
    ));
    rejected.into_host().shutdown().await?;
    assert_eq!(
        rejected_gate.state(),
        TerminationState::CooperativelyStopped
    );

    let dropped_gate = TerminationGate::new();
    let prepared = prepare_probe_worker(
        pool,
        &hosted_worker::settings("hosted-probe-unstarted", 1)?,
        dropped_gate.clone(),
        &OperationContext::new(Duration::from_secs(35))?,
        Duration::from_secs(20),
    )
    .await?;
    let mut unstarted = hosted_worker::process_supervisor();
    prepared.register(&mut unstarted)?;
    drop(unstarted);
    tokio::time::timeout(Duration::from_secs(15), async {
        while dropped_gate.state() == TerminationState::Unproven {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "dropping an unstarted supervisor did not stop its registered worker")?;
    assert_eq!(dropped_gate.state(), TerminationState::CooperativelyStopped);
    assert_no_advisory_locks(pool).await?;
    Ok(())
}

/// Fail unless the startup-control lease lock is free in the current database.
///
/// Only the two-key advisory lock owned by the probe host is inspected, so an
/// unrelated advisory lock can neither mask nor manufacture a leaked lease.
pub async fn assert_no_advisory_locks(pool: &PgPool) -> ProbeResult {
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM pg_locks
          WHERE locktype = 'advisory'
            AND database = (SELECT oid FROM pg_database WHERE datname = current_database())
            AND classid::int = $1
            AND objid::int = $2
            AND objsubid = 2
            AND granted",
    )
    .bind(STARTUP_LEASE_LOCK_CLASS)
    .bind(STARTUP_LEASE_LOCK_KEY)
    .fetch_one(pool)
    .await?;
    if remaining != 0 {
        return Err("startup-control lease remained held".into());
    }
    Ok(())
}

/// Locate the one backend granted the startup-control lease lock.
async fn lease_session_pid(pool: &PgPool) -> Result<i32, BoxError> {
    let pids: Vec<i32> = sqlx::query_scalar(
        "SELECT pid
           FROM pg_locks
          WHERE locktype = 'advisory'
            AND database = (SELECT oid FROM pg_database WHERE datname = current_database())
            AND classid::int = $1
            AND objid::int = $2
            AND objsubid = 2
            AND granted
            AND pid <> pg_backend_pid()",
    )
    .bind(STARTUP_LEASE_LOCK_CLASS)
    .bind(STARTUP_LEASE_LOCK_KEY)
    .fetch_all(pool)
    .await?;
    match pids.as_slice() {
        [pid] => Ok(*pid),
        [] => Err("the startup-control lease session was not found".into()),
        _ => Err("more than one session holds the startup-control lease".into()),
    }
}

/// Terminate the lease backend and wait until the server no longer lists it.
///
/// `pg_terminate_backend` only signals the backend. Its lock is released when
/// the backend actually exits, so the successor must not attempt acquisition
/// until that exit is observed rather than merely requested.
async fn terminate_lease_session(pool: &PgPool) -> Result<i32, BoxError> {
    let pid = lease_session_pid(pool).await?;
    let signalled: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(pool)
        .await?;
    if !signalled {
        return Err("the startup-control lease session could not be terminated".into());
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let alive: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid = $1)")
                    .bind(pid)
                    .fetch_one(pool)
                    .await?;
            if !alive {
                return Ok::<_, BoxError>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "the terminated lease backend did not exit")??;
    assert_no_advisory_locks(pool).await?;
    Ok(pid)
}

pub async fn lease_loss_stops_host(pool: PgPool) -> ProbeResult {
    batter_example_reference_service::schema::initialize_schema(&pool).await?;
    let gate = TerminationGate::new();
    let prepared = prepare_probe_worker(
        &pool,
        &hosted_worker::settings("hosted-probe-lease-loss", 1)?,
        gate.clone(),
        &OperationContext::new(Duration::from_secs(35))?,
        Duration::from_secs(20),
    )
    .await?;
    let mut supervisor = hosted_worker::process_supervisor();
    prepared.register(&mut supervisor)?;
    let handle = supervisor.handle();
    let running = supervisor.start();

    // From here every failure is a returned value. The running supervisor is
    // always awaited below, so an early failure cannot drop it mid-shutdown.
    let disrupted: ProbeResult = async {
        if !handle.mark_ready() {
            return Err("lease-loss host rejected readiness".into());
        }
        handle
            .wait_ready()
            .await
            .map_err(|_| "lease-loss host did not acknowledge readiness")?;
        terminate_lease_session(&pool).await?;

        // Start the successor before awaiting the predecessor. If the old native
        // loop wins the new control claim, its generation check must retry rather
        // than complete or terminally fail the successor's witness.
        let replacement_gate = TerminationGate::new();
        let replacement = prepare_probe_worker(
            &pool,
            &hosted_worker::settings("hosted-probe-lease-replacement", 1)?,
            replacement_gate.clone(),
            &OperationContext::new(Duration::from_secs(35))?,
            Duration::from_secs(20),
        )
        .await?;
        replacement.shutdown().await?;
        if replacement_gate.state() != TerminationState::CooperativelyStopped {
            return Err("replacement host did not prove cooperative termination".into());
        }
        Ok(())
    }
    .await;
    if disrupted.is_err() {
        // The lease may still be healthy; without a request the predecessor
        // would run until the outer timeout.
        handle.request();
    }

    let predecessor: ProbeResult = async {
        let outcome = tokio::time::timeout(Duration::from_secs(15), running.wait())
            .await
            .map_err(|_| "lease loss did not stop the hosted worker")?;
        let report = outcome.map_err(|error| {
            Box::new(std::io::Error::other(error.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;
        if disrupted.is_err() {
            // Shutdown was requested only to retain the primary failure.
            return Ok(());
        }
        let driver = report
            .tasks
            .iter()
            .find(|task| task.name == "worker")
            .and_then(|task| task.error.as_deref())
            .and_then(|error| error.downcast_ref::<WorkerDriverError>())
            .ok_or("lease-loss driver failure was not retained")?;
        let WorkerDriverError::Release {
            release,
            shutdown: Some(driver),
        } = driver
        else {
            return Err("lease loss must retain both unavailable release and native cause".into());
        };
        assert!(!release.is_success());
        assert!(matches!(
            release.unlock,
            batter_example_reference_service::worker::UnlockOutcome::Unavailable
        ));
        if !matches!(
            driver.as_ref(),
            WorkerDriverError::ProbeLease { failure, .. }
                if matches!(failure.as_ref(), ProbeLeaseError::Query(_))
        ) {
            return Err(format!("lease loss was classified as {driver:?}").into());
        }
        if gate.state() != TerminationState::Unproven {
            return Err("lease loss did not remain an unproven termination".into());
        }
        Ok(())
    }
    .await;

    let combined = batter_test_support::finish(
        disrupted.map_err(ProbeError::new),
        predecessor.map_err(ProbeError::new),
    );
    let locks = assert_no_advisory_locks(&pool)
        .await
        .map_err(ProbeError::new);
    batter_test_support::finish(combined, locks)
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?;
    Ok(())
}
