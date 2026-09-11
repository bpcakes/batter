use super::{
    PreparedProbeWorker, ProbeLeaseError, StartupInvocation, TerminationGate,
    UPSTREAM_SHUTDOWN_RESERVE, WorkerStartupError,
};
use crate::config::WorkerSettings;
use batter::operation::{Interruption, OperationContext, OperationError};
use runledger_core::{
    jobs::{
        JobCompletion, JobContext, JobContract, JobDefinitionSettings, JobExecution, JobFailure,
        JobSpec, JobStatus,
    },
    prelude::async_trait,
};
use runledger_postgres::jobs::{JobEnqueue, cancel_job, enqueue_job_with_outcome};
use runledger_runtime::catalog::{CatalogError, JobCatalog};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, types::Uuid};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::Instant};

/// Dedicated job type claimed by the staged startup-control registry.
pub const STARTUP_JOB_TYPE: &str = "jobs.startup.control";
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROBE_LEASE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PROBE_LEASE_QUERY_BUDGET: Duration = Duration::from_secs(2);
/// First key of the two-key session advisory lock proving startup-control ownership.
pub const STARTUP_LEASE_LOCK_CLASS: i32 = 0x6261_7474;
/// Second key of the two-key session advisory lock proving startup-control ownership.
pub const STARTUP_LEASE_LOCK_KEY: i32 = 0x6572_7072;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct StartupPayload {
    pub(super) witness_id: Uuid,
}

pub(super) struct StartupContract;

impl JobContract for StartupContract {
    type Payload = StartupPayload;

    fn spec() -> JobSpec {
        // A witness is bounded by its deadline, never by an attempt budget. Every
        // generation mismatch consumes one durable attempt, so a finite budget
        // would let enough overlapping predecessors dead-letter a successor's
        // witness before its deadline. Stale controls are canceled by the next
        // owner, so an unbounded budget never leaves a live retry loop behind.
        JobSpec::new(runledger_core::jobs::JobType::new(STARTUP_JOB_TYPE))
            .and_then(|spec| {
                spec.with_settings(
                    JobDefinitionSettings::new()
                        .max_attempts(i32::MAX)
                        .timeout_seconds(30),
                )
            })
            .expect("static startup control job specification is valid")
    }
}

struct StartupHandler {
    invocations: mpsc::UnboundedSender<StartupInvocation>,
    expected_witness: Uuid,
}

impl StartupHandler {
    fn authorize(&self, payload: StartupPayload) -> Result<(), JobFailure> {
        authorize_witness(self.expected_witness, payload.witness_id)
    }
}

pub(super) fn authorize_witness(expected: Uuid, actual: Uuid) -> Result<(), JobFailure> {
    if actual == expected {
        Ok(())
    } else {
        Err(JobFailure::retryable(
            "startup.owner_changed",
            "A newer startup owner must execute this control.",
        )
        .retry_not_before_delay(UPSTREAM_SHUTDOWN_RESERVE))
    }
}

#[async_trait]
impl runledger_core::jobs::TypedJobHandler for StartupHandler {
    type Contract = StartupContract;

    async fn execute(
        &self,
        _context: JobContext,
        payload: StartupPayload,
    ) -> Result<JobCompletion, JobFailure> {
        self.authorize(payload)?;
        Err(JobFailure::terminal(
            "startup.services_missing",
            "Startup control requires runtime execution services.",
        ))
    }

    async fn execute_with_services(
        &self,
        execution: JobExecution<'_>,
        payload: StartupPayload,
    ) -> Result<JobCompletion, JobFailure> {
        self.authorize(payload)?;
        let durable = execution.context().clone();
        let operation = OperationContext::at(execution.deadline().into());
        match operation
            .run("worker.startup.control", |attempt| async move {
                self.invocations
                    .send(StartupInvocation {
                        job_id: durable.job_id,
                        run_number: durable.run_number,
                        attempt: durable.attempt,
                        operation_deadline: attempt.deadline(),
                        witness_id: payload.witness_id,
                    })
                    .map_err(|_| {
                        JobFailure::terminal(
                            "startup.witness_closed",
                            "Startup witness receiver is unavailable.",
                        )
                    })?;
                Ok(JobCompletion::success())
            })
            .await
        {
            Ok(completion) => Ok(completion),
            Err(OperationError::Failed(failure)) => Err(failure),
            Err(OperationError::Interrupted(Interruption::Cancelled)) => Err(JobFailure::timeout(
                "startup.cancelled",
                "Startup control was cancelled.",
            )),
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => Err(
                JobFailure::timeout("startup.deadline", "Startup control exceeded its deadline."),
            ),
        }
    }
}

/// Build the probe-only registry and return a host only after its exact control
/// job invocation and durable success are both observed.
///
/// The registry intentionally omits `records.delivery.execute`, so ordinary
/// delivery jobs are not claimable. A monitored session advisory lock permits
/// one probe host per database and stale controls are canceled while it is held.
/// Lease liveness is checked throughout preparation and native execution. A
/// predecessor cannot complete or terminally fail a successor's witness, and the
/// witness is bounded only by its deadline: predecessor claims consume durable
/// attempts without exhausting the control.
///
/// Preparation runs under an independent owner from first acquisition onward.
/// Dropping this waiter requests cancellation; retain `termination` and await
/// [`super::DependencyCleanup::finish`] to observe settlement on a live runtime.
/// Errors are shared with [`TerminationGate::settlement`] even if delivery is lost.
/// The owner attempts bounded unlock and client closure; an unsuccessful release
/// is retained and never called confirmed. Client closure does not prove backend exit.
/// Runtime destruction and arbitrary task panics do not guarantee remote cleanup.
///
/// `parent` clamps work ahead of the complete native shutdown, abort-drain and
/// lease-release allowance. Cancelling it never cancels that settlement phase.
pub async fn prepare_probe_worker(
    pool: &PgPool,
    settings: &WorkerSettings,
    termination: TerminationGate,
    parent: &OperationContext,
    witness_budget: Duration,
) -> Result<PreparedProbeWorker, Arc<WorkerStartupError>> {
    super::preparation::prepare(pool, settings, termination, parent, witness_budget).await
}

/// Durable control state prepared while the lease session is proved live.
pub(super) struct ControlPreparation {
    pub(super) catalog: JobCatalog,
    pub(super) invocations: mpsc::UnboundedReceiver<StartupInvocation>,
    pub(super) job_id: Uuid,
    pub(super) witness_id: Uuid,
}

pub(super) async fn prepare_control(
    pool: &PgPool,
    probe_lease: &mut PgConnection,
    parent_work_deadline: Instant,
) -> Result<ControlPreparation, WorkerStartupError> {
    preparation_with_lease(
        probe_lease,
        parent_work_deadline,
        reconcile_stale_controls(pool),
    )
    .await?;

    let witness_id = Uuid::now_v7();
    let (sender, invocations) = mpsc::unbounded_channel();
    let spec = StartupContract::spec();
    let catalog =
        startup_catalog(&spec, sender, witness_id).map_err(WorkerStartupError::Catalog)?;
    preparation_with_lease(probe_lease, parent_work_deadline, async {
        catalog
            .sync_definitions(pool)
            .await
            .map_err(WorkerStartupError::Definition)
    })
    .await?;

    let mut submission = StartupContract::submit(&StartupPayload { witness_id })
        .map_err(WorkerStartupError::Submission)?;
    submission.idempotency_key = Some(format!("startup-{witness_id}"));
    let enqueue = JobEnqueue::from(&submission);
    let outcome = preparation_with_lease(probe_lease, parent_work_deadline, async {
        enqueue_job_with_outcome(pool, &enqueue)
            .await
            .map_err(WorkerStartupError::Enqueue)
    })
    .await?;
    Ok(ControlPreparation {
        catalog,
        invocations,
        job_id: outcome.job_id,
        witness_id,
    })
}

async fn preparation_with_lease<T, F>(
    lease: &mut PgConnection,
    deadline: Instant,
    operation: F,
) -> Result<T, WorkerStartupError>
where
    F: Future<Output = Result<T, WorkerStartupError>>,
{
    tokio::time::timeout_at(deadline, async {
        tokio::select! {
            biased;
            failure = monitor_probe_lease(lease) => {
                Err(WorkerStartupError::ProbeLeaseLost(Arc::new(failure)))
            }
            result = operation => result,
        }
    })
    .await
    .map_err(|_| WorkerStartupError::PreparationTimeout)?
}

pub(super) fn startup_catalog(
    spec: &JobSpec,
    invocations: mpsc::UnboundedSender<StartupInvocation>,
    expected_witness: Uuid,
) -> Result<JobCatalog, CatalogError> {
    use runledger_core::jobs::TypedJobHandler;

    JobCatalog::new().try_handler_for_spec(
        spec,
        StartupHandler {
            invocations,
            expected_witness,
        }
        .into_job_handler(),
    )
}

pub(super) async fn monitor_probe_lease(connection: &mut PgConnection) -> ProbeLeaseError {
    let mut interval = tokio::time::interval(PROBE_LEASE_CHECK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match tokio::time::timeout(
            PROBE_LEASE_QUERY_BUDGET,
            sqlx::query("SELECT 1").execute(&mut *connection),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return ProbeLeaseError::Query(Arc::new(error)),
            Err(_) => return ProbeLeaseError::Timeout,
        }
    }
}

async fn reconcile_stale_controls(pool: &PgPool) -> Result<(), WorkerStartupError> {
    loop {
        let stale: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id
               FROM job_queue
              WHERE job_type = $1
                AND status IN ('PENDING', 'LEASED')
              ORDER BY created_at, id
              LIMIT 100",
        )
        .bind(STARTUP_JOB_TYPE)
        .fetch_all(pool)
        .await
        .map_err(WorkerStartupError::StaleControlRead)?;
        if stale.is_empty() {
            return Ok(());
        }
        for job_id in stale {
            let result = cancel_job(
                pool,
                None,
                job_id,
                Some("replaced by a new startup ownership witness"),
            )
            .await;
            if let Err(error) = result {
                reconcile_cancel_race(pool, job_id, error).await?;
            }
        }
    }
}

async fn reconcile_cancel_race(
    pool: &PgPool,
    job_id: Uuid,
    error: runledger_postgres::Error,
) -> Result<(), WorkerStartupError> {
    if matches!(&error, runledger_postgres::Error::QueryError(cause) if cause.code() == "job.invalid_state_transition")
    {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
                .bind(job_id)
                .fetch_optional(pool)
                .await
                .map_err(WorkerStartupError::StaleControlRead)?;
        if matches!(
            status.as_deref(),
            Some("SUCCEEDED" | "CANCELED" | "DEAD_LETTERED")
        ) {
            return Ok(());
        }
    }
    Err(WorkerStartupError::StaleControlCancel(error))
}

pub(super) async fn observe_witness(
    pool: &PgPool,
    expected_job: Uuid,
    expected_witness: Uuid,
    invocations: &mut mpsc::UnboundedReceiver<StartupInvocation>,
    deadline: Instant,
) -> Result<StartupInvocation, WorkerStartupError> {
    let invocation =
        receive_expected_witness(expected_job, expected_witness, invocations, deadline).await?;

    loop {
        let context = OperationContext::at(deadline);
        let mut lease = batter_sqlx::PgLease::acquire(pool, &context)
            .await
            .map_err(|error| match error {
                OperationError::Failed(error) => WorkerStartupError::Status(error.into_native()),
                OperationError::Interrupted(_) => WorkerStartupError::WitnessTimeout,
            })?;
        let status: String = tokio::time::timeout_at(
            deadline,
            sqlx::query_scalar("SELECT status::text FROM job_queue WHERE id = $1")
                .bind(expected_job)
                .fetch_one(lease.connection()),
        )
        .await
        .map_err(|_| WorkerStartupError::WitnessTimeout)?
        .map_err(WorkerStartupError::Status)?;
        lease.return_to_pool();
        match JobStatus::from_db_value(&status) {
            Some(JobStatus::Succeeded) => return Ok(invocation),
            Some(JobStatus::DeadLettered | JobStatus::Canceled) => {
                return Err(WorkerStartupError::UnsuccessfulStatus);
            }
            Some(JobStatus::Pending | JobStatus::Leased) => {}
            None => return Err(WorkerStartupError::UnsuccessfulStatus),
        }
        tokio::time::timeout_at(deadline, tokio::time::sleep(STATUS_POLL_INTERVAL))
            .await
            .map_err(|_| WorkerStartupError::WitnessTimeout)?;
    }
}

pub(super) async fn receive_expected_witness(
    expected_job: Uuid,
    expected_witness: Uuid,
    invocations: &mut mpsc::UnboundedReceiver<StartupInvocation>,
    deadline: Instant,
) -> Result<StartupInvocation, WorkerStartupError> {
    loop {
        let invocation = tokio::time::timeout_at(deadline, invocations.recv())
            .await
            .map_err(|_| WorkerStartupError::WitnessTimeout)?
            .ok_or(WorkerStartupError::WitnessClosed)?;
        if invocation.job_id == expected_job && invocation.witness_id == expected_witness {
            return Ok(invocation);
        }
    }
}
