use super::{
    PreparedProbeWorker, TerminationGate, UPSTREAM_SHUTDOWN_RESERVE, WorkerHost, WorkerStartupError,
};
use super::{lease::release_probe_lease, probe::prepare_control};
use crate::config::WorkerSettings;
use batter::operation::OperationContext;
use sqlx::{PgConnection, PgPool};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{sync::oneshot, time::Instant};

struct CancelOnDrop(OperationContext);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(super) async fn prepare(
    pool: &PgPool,
    settings: &WorkerSettings,
    gate: TerminationGate,
    parent: &OperationContext,
    witness_budget: Duration,
) -> Result<PreparedProbeWorker, Arc<WorkerStartupError>> {
    // Validate before both external acquisition and spawning native work.
    Instant::now()
        .checked_add(witness_budget)
        .ok_or(WorkerStartupError::WitnessBudgetOverflow)?;
    let work_deadline = parent
        .deadline()
        .checked_sub(UPSTREAM_SHUTDOWN_RESERVE)
        .filter(|deadline| *deadline > Instant::now())
        .ok_or(WorkerStartupError::WitnessBudgetUnavailable)?;
    gate.begin_preparation()
        .map_err(WorkerStartupError::Build)?;
    let cancellation = OperationContext::at(work_deadline);
    let _cancel_on_drop = CancelOnDrop(cancellation.clone());
    let pool = pool.clone();
    let settings = settings.clone();
    let parent = parent.clone();
    let owned_gate = gate.clone();
    let (sender, receiver) = oneshot::channel();
    let preparation = tokio::spawn(batter::telemetry::with_current_dispatch(async move {
        drive(
            &pool,
            &settings,
            &owned_gate,
            &parent,
            &cancellation,
            witness_budget,
        )
        .await
    }));
    drop(tokio::spawn(batter::telemetry::with_current_dispatch(
        observe_preparation(preparation, gate, sender),
    )));
    receiver
        .await
        .expect("preparation monitor retains delivery")
}

async fn observe_preparation(
    preparation: tokio::task::JoinHandle<Result<PreparedProbeWorker, WorkerStartupError>>,
    gate: TerminationGate,
    sender: oneshot::Sender<Result<PreparedProbeWorker, Arc<WorkerStartupError>>>,
) {
    let result = match preparation.await {
        Ok(result) => result.map_err(Arc::new),
        Err(error) => {
            // A construction panic may have spawned work without a native
            // observer. Publish uncertainty, never restore NotStarted.
            gate.mark_driver_observed();
            Err(Arc::new(WorkerStartupError::PreparationJoin(Arc::new(
                error,
            ))))
        }
    };
    gate.finish_preparation(result.as_ref().err().cloned());
    // Failed delivery drops the prepared host and requests its owned stop.
    drop(sender.send(result));
}

async fn cancellable<T>(
    parent: &OperationContext,
    work: &OperationContext,
    future: impl Future<Output = Result<T, WorkerStartupError>>,
) -> Result<T, WorkerStartupError> {
    tokio::select! {
        biased;
        _ = parent.cancelled() => Err(WorkerStartupError::Cancelled),
        _ = work.cancelled() => Err(WorkerStartupError::Cancelled),
        _ = tokio::time::sleep_until(work.deadline()) => Err(WorkerStartupError::PreparationTimeout),
        result = future => result,
    }
}

async fn drive(
    pool: &PgPool,
    settings: &WorkerSettings,
    gate: &TerminationGate,
    parent: &OperationContext,
    work: &OperationContext,
    witness_budget: Duration,
) -> Result<PreparedProbeWorker, WorkerStartupError> {
    // No await between checkout completion and storing the raw connection in
    // this owner. All subsequent interruptible operations borrow that connection.
    let mut lease = cancellable(parent, work, async {
        pool.acquire()
            .await
            .map(|connection| connection.detach())
            .map_err(WorkerStartupError::ProbeLease)
    })
    .await?;
    let prepared = prepare_database(pool, &mut lease, parent, work).await;
    let control = match prepared {
        Ok(control) => control,
        Err(error) => return Err(settle_preparation(lease, gate, error).await),
    };
    // Computing the witness deadline remains before spawning; validation above
    // rejects unreasonable allowances before even acquiring the session.
    let Some(requested) = Instant::now().checked_add(witness_budget) else {
        return Err(
            settle_preparation(lease, gate, WorkerStartupError::WitnessBudgetOverflow).await,
        );
    };
    let supervisor = match WorkerHost::build_supervisor(settings, pool, &control.catalog, gate) {
        Ok(supervisor) => supervisor,
        Err(error) => return Err(settle_preparation(lease, gate, error.into()).await),
    };
    // Successful build already owns spawned loops. Transfer synchronously with
    // no fallible operation between it and the native driver.
    let mut host = WorkerHost::host_supervisor(supervisor, gate.clone(), Some(lease));
    let mut invocations = control.invocations;
    let result = tokio::select! {
        biased;
        _ = parent.cancelled() => Err(WorkerStartupError::Cancelled),
        _ = work.cancelled() => Err(WorkerStartupError::Cancelled),
        result = host.witness(pool, control.job_id, control.witness_id, &mut invocations, requested.min(work.deadline())) => result,
    };
    match result {
        Ok(invocation) => Ok(PreparedProbeWorker { host, invocation }),
        Err(witness) => {
            // witness() also stops on its own error. This repeated observation
            // neither restarts shutdown nor replaces an already combined error.
            let shutdown = host.shutdown().await;
            if matches!(witness, WorkerStartupError::Cancelled)
                && let Err(shutdown) = shutdown
            {
                return Err(WorkerStartupError::WitnessAndShutdown {
                    witness: Box::new(witness),
                    shutdown: Box::new(shutdown),
                });
            }
            Err(witness)
        }
    }
}

async fn prepare_database(
    pool: &PgPool,
    lease: &mut PgConnection,
    parent: &OperationContext,
    work: &OperationContext,
) -> Result<super::probe::ControlPreparation, WorkerStartupError> {
    // The pinned native helpers accept a pool and hide their checkouts. A small
    // temporary pool closes every returned connection before SQLx's reuse ping,
    // including transactions/queries abandoned by cancellation. Do not use the
    // doc-hidden, internal-only PoolOptions::parent API for capacity sharing.
    let preparation = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(pool.options().get_acquire_timeout())
        .after_release(|_, _| Box::pin(async { Ok(false) }))
        .connect_lazy_with((*pool.connect_options()).clone());
    let result = cancellable(parent, work, async {
        acquire_lock(lease).await?;
        prepare_control(&preparation, lease, work.deadline()).await
    })
    .await;
    if tokio::time::timeout(super::PROBE_LEASE_RELEASE_BUDGET, preparation.close())
        .await
        .is_err()
    {
        return Err(WorkerStartupError::PreparationPoolClose {
            preparation: result.err().map(Box::new),
        });
    }
    let control = result?;
    // Pool closure consumes time too; do not spawn after the work interval.
    cancellable(parent, work, async { Ok(control) }).await
}

async fn acquire_lock(connection: &mut PgConnection) -> Result<(), WorkerStartupError> {
    sqlx::query("SET idle_session_timeout = '10s'")
        .execute(&mut *connection)
        .await
        .map_err(WorkerStartupError::ProbeLease)?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1, $2)")
        .bind(super::STARTUP_LEASE_LOCK_CLASS)
        .bind(super::STARTUP_LEASE_LOCK_KEY)
        .fetch_one(connection)
        .await
        .map_err(WorkerStartupError::ProbeLease)?;
    if acquired {
        Ok(())
    } else {
        Err(WorkerStartupError::ProbeAlreadyActive)
    }
}

async fn settle_preparation(
    connection: PgConnection,
    gate: &TerminationGate,
    error: WorkerStartupError,
) -> WorkerStartupError {
    let usable = !matches!(error, WorkerStartupError::ProbeLeaseLost(_));
    let release = Arc::new(release_probe_lease(connection, usable).await);
    gate.record_release(release.clone());
    if release.is_success() {
        error
    } else {
        WorkerStartupError::PreparationAndRelease {
            preparation: Box::new(error),
            release,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batter::cleanup::{CleanupBudget, CleanupStack, SkipReason};

    #[tokio::test]
    async fn preparation_owner_failure_publishes_uncertainty_and_unblocks_cleanup() {
        let gate = TerminationGate::new();
        gate.begin_preparation().unwrap();
        gate.arm().unwrap();
        let (sender, receiver) = oneshot::channel();
        let owner = tokio::spawn(async { panic!("controlled preparation owner failure") });
        observe_preparation(owner, gate.clone(), sender).await;
        let error = receiver
            .await
            .unwrap()
            .err()
            .expect("owner failure retained");
        assert!(
            matches!(error.as_ref(), WorkerStartupError::PreparationJoin(error) if error.is_panic())
        );
        assert!(Arc::ptr_eq(
            &error,
            gate.settlement().preparation.as_ref().unwrap()
        ));
        assert_eq!(gate.state(), super::super::TerminationState::Unproven);
        let mut stack = CleanupStack::new();
        stack
            .push("dependency", || async {
                panic!("uncertain work forbids cleanup")
            })
            .unwrap();
        let second = Duration::from_secs(1);
        let cleanup = super::super::DependencyCleanup::new(
            stack,
            CleanupBudget::new(second, second, second).unwrap(),
            gate,
        );
        let report = tokio::time::timeout(second, cleanup.finish())
            .await
            .expect("owner failure cannot leave a pending publication");
        assert_eq!(report.skipped[0].reason, SkipReason::UnsafeTaskExit);
    }
}
