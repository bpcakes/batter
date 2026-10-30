use super::*;
use batter::runledger::test_support::{HandlerExit, HandlerInvocation, HandlerRun};
use runledger_core::jobs::JobContext;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[test]
fn terminal_projection_covers_every_retained_state_and_native_budget_pair() {
    use crate::delivery::{DeliveryState as Job, ProviderEffectState as Effect};

    // Explicit oracle: native termination cannot overwrite terminal provider
    // truth, and exhaustion requires both dead-lettering and a spent budget.
    for (job, attempts_spent, unresolved) in [
        (Job::Pending, false, None),
        (Job::Pending, true, None),
        (Job::InFlight, false, None),
        (Job::InFlight, true, None),
        (Job::Succeeded, false, Some(Effect::ManualResolution)),
        (Job::Succeeded, true, Some(Effect::ManualResolution)),
        (Job::Cancelled, false, Some(Effect::ManualResolution)),
        (Job::Cancelled, true, Some(Effect::ManualResolution)),
        (Job::DeadLettered, false, Some(Effect::ManualResolution)),
        (Job::DeadLettered, true, Some(Effect::Exhausted)),
    ] {
        for retained in [
            Effect::AwaitingAttempt,
            Effect::RetryableUndispatched,
            Effect::ReconcileNeeded,
        ] {
            assert_eq!(
                retained.project(job, attempts_spent),
                unresolved.unwrap_or(retained),
                "retained={retained:?}, job={job:?}, attempts_spent={attempts_spent}"
            );
        }
        for retained in [
            Effect::Confirmed,
            Effect::BusinessDenied,
            Effect::ManualResolution,
            Effect::Exhausted,
        ] {
            assert_eq!(
                retained.project(job, attempts_spent),
                retained,
                "retained={retained:?}, job={job:?}, attempts_spent={attempts_spent}"
            );
        }
    }
}

fn snapshot(
    state: ProviderEffectState,
    retention_active: bool,
    current_generation: Option<i64>,
) -> EffectSnapshot {
    let provider_payload = ProviderEffectRequest {
        version: crate::provider::EFFECT_PROTOCOL_VERSION,
        effect_id: Uuid::from_u128(1),
        owner_id: Uuid::from_u128(2),
        record_id: Uuid::from_u128(3),
        record_generation: 4,
        payload: json!({"channel": "planner"}),
    };
    EffectSnapshot {
        provider_key: provider_payload.idempotency_key(),
        provider_payload,
        state,
        retention_active,
        retry_delay: Duration::ZERO,
        current_generation,
    }
}

#[test]
fn durable_retry_delay_precedes_dispatch_but_not_terminal_or_generation_decisions() {
    for state in [
        ProviderEffectState::AwaitingAttempt,
        ProviderEffectState::RetryableUndispatched,
    ] {
        let mut retained = snapshot(state, false, Some(4));
        retained.retry_delay = Duration::from_secs(60);
        assert_eq!(retained.plan(4), EffectPlan::Defer(Duration::from_secs(60)));
        retained.current_generation = None;
        assert_eq!(
            retained.plan(4),
            EffectPlan::DenyGeneration { expected: state }
        );
        retained.state = ProviderEffectState::Confirmed;
        assert_eq!(retained.plan(4), EffectPlan::Complete);
    }
    assert_eq!(remaining_retry_delay(-1), Duration::ZERO);
    assert_eq!(remaining_retry_delay(0), Duration::ZERO);
    assert_eq!(remaining_retry_delay(1), Duration::from_millis(1));
}

#[test]
fn retained_state_exhaustively_selects_work_before_provider_admission() {
    let generation = 4;
    for (state, retention_active, current_generation, expected) in [
        (
            ProviderEffectState::Confirmed,
            true,
            Some(generation),
            EffectPlan::Complete,
        ),
        (
            ProviderEffectState::BusinessDenied,
            true,
            Some(generation),
            EffectPlan::Complete,
        ),
        (
            ProviderEffectState::ManualResolution,
            true,
            Some(generation),
            EffectPlan::Manual,
        ),
        (
            ProviderEffectState::Exhausted,
            true,
            Some(generation),
            EffectPlan::Exhausted,
        ),
        (
            ProviderEffectState::ReconcileNeeded,
            false,
            Some(generation),
            EffectPlan::ExpireReconciliation,
        ),
        (
            ProviderEffectState::ReconcileNeeded,
            true,
            None,
            EffectPlan::Reconcile,
        ),
        (
            ProviderEffectState::AwaitingAttempt,
            true,
            Some(generation + 1),
            EffectPlan::DenyGeneration {
                expected: ProviderEffectState::AwaitingAttempt,
            },
        ),
        (
            ProviderEffectState::RetryableUndispatched,
            true,
            None,
            EffectPlan::DenyGeneration {
                expected: ProviderEffectState::RetryableUndispatched,
            },
        ),
        (
            ProviderEffectState::AwaitingAttempt,
            false,
            Some(generation),
            EffectPlan::Dispatch {
                expected: ProviderEffectState::AwaitingAttempt,
            },
        ),
        (
            ProviderEffectState::RetryableUndispatched,
            false,
            Some(generation),
            EffectPlan::Dispatch {
                expected: ProviderEffectState::RetryableUndispatched,
            },
        ),
    ] {
        assert_eq!(
            snapshot(state, retention_active, current_generation).plan(generation),
            expected,
            "unexpected plan for {state:?}"
        );
    }
}

#[test]
fn future_dispatch_eligibility_never_defers_required_reconciliation() {
    for generation in [Some(4), Some(5), None] {
        let mut retained = snapshot(ProviderEffectState::ReconcileNeeded, true, generation);
        retained.retry_delay = Duration::from_secs(24 * 60 * 60);
        assert_eq!(retained.plan(4), EffectPlan::Reconcile);
        retained.retention_active = false;
        assert_eq!(retained.plan(4), EffectPlan::ExpireReconciliation);
    }
}

#[test]
fn retryable_outcomes_become_terminal_only_after_persisted_exhaustion() {
    assert_eq!(
        retry_or_exhausted(
            ProviderEffectState::RetryableUndispatched,
            CODE_PROVIDER_UNDISPATCHED,
            "retry",
            None,
        )
        .kind,
        runledger_core::jobs::JobFailureKind::Retryable
    );
    let exhausted = retry_or_exhausted(
        ProviderEffectState::Exhausted,
        CODE_PROVIDER_UNDISPATCHED,
        "retry",
        None,
    );
    assert_eq!(
        exhausted.kind,
        runledger_core::jobs::JobFailureKind::Terminal
    );
    assert_eq!(exhausted.code, CODE_ATTEMPTS_EXHAUSTED);
}

#[test]
fn admission_failures_preserve_distinct_native_causes() {
    for (error, code, interrupted) in [
        (AdmissionError::Overloaded, CODE_ADMISSION_OVERLOADED, false),
        (AdmissionError::Closed, CODE_ADMISSION_CLOSED, false),
        (
            AdmissionError::Interrupted(batter::operation::Interruption::DeadlineExceeded),
            CODE_ADMISSION_INTERRUPTED,
            true,
        ),
    ] {
        let (actual, _, actual_interrupted) = admission_failure_details(&error);
        assert_eq!(actual, code);
        assert_eq!(actual_interrupted, interrupted);
    }
}

/// Custom services that predate the exit signal and keep the trait default.
/// Neither the worker nor the handler test facility builds them; the control
/// below pins the handler's refusal of them.
struct Legacy(std::time::Instant);

#[async_trait]
impl runledger_core::jobs::JobExecutionServices for Legacy {
    fn deadline(&self) -> std::time::Instant {
        self.0
    }
    fn remaining_budget(&self) -> Duration {
        self.0
            .saturating_duration_since(tokio::time::Instant::now().into_std())
    }
    async fn persist_progress(
        &self,
        _: runledger_core::jobs::JobExecutionUpdate<'_>,
    ) -> Result<(), runledger_core::jobs::JobExecutionError> {
        unreachable!("rejected before any write")
    }
}

type ProbeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type ProbeBody<T> = Box<dyn for<'a> FnOnce(JobExecution<'a>) -> ProbeFuture<'a, T> + Send>;

/// A handler that runs one probe body and keeps its output for the test.
struct Probe<T> {
    body: Mutex<Option<ProbeBody<T>>>,
    output: Arc<Mutex<Option<T>>>,
}

#[async_trait]
impl<T: Send + 'static> JobExecutionHandler for Probe<T> {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }

    async fn execute(
        &self,
        execution: JobExecution<'_>,
        _: Value,
    ) -> Result<JobCompletion, JobFailure> {
        let body = self
            .body
            .lock()
            .expect("probe")
            .take()
            .expect("one invocation");
        let output = body(execution).await;
        *self.output.lock().expect("probe") = Some(output);
        Ok(JobCompletion::success())
    }
}

/// Runs `body` with the execution of one invocation of `context`, driven by
/// the handler test facility as the worker drives a handler, and returns its
/// output once that invocation has ended.
pub(super) async fn in_invocation<T: Send + 'static>(
    budget: Duration,
    context: JobContext,
    body: impl for<'a> FnOnce(JobExecution<'a>) -> ProbeFuture<'a, T> + Send + 'static,
) -> T {
    let output = Arc::new(Mutex::new(None));
    let probe = Probe {
        body: Mutex::new(Some(Box::new(body))),
        output: Arc::clone(&output),
    };
    let run = HandlerInvocation::for_context(budget, context)
        .expect("claimed context")
        .execute(&probe.into_job_handler(), Value::Null)
        .await;
    assert!(
        matches!(run.exit, HandlerExit::Returned(Ok(_))),
        "probe exit: {:?}",
        run.exit
    );
    assert!(run.exit_hook_panics.is_empty());
    let output = output.lock().expect("probe").take();
    output.expect("probe output")
}

/// The handler's own failure, observed before the deadline without hook panics.
pub(super) fn returned_failure(run: HandlerRun) -> JobFailure {
    assert!(run.exit_hook_panics.is_empty());
    match run.exit {
        HandlerExit::Returned(Err(failure)) => failure,
        exit => panic!("expected the handler's own failure: {exit:?}"),
    }
}

async fn offline_worker() -> Result<DeliveryWorker, batter::BoxError> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://fixture:fixture@127.0.0.1:9/fixture")?;
    pool.close().await;
    Ok(DeliveryWorker::new(
        pool,
        ProviderClient::new(
            url::Url::parse("http://127.0.0.1:9/")?,
            batter::settings::SecretString::new("fixture-token"),
        )?,
        Bulkhead::new(batter::admission::BulkheadCapacity::new(1)?),
    ))
}

fn owned_payload() -> (JobContext, DeliveryJobPayload) {
    let payload = DeliveryJobPayload {
        version: DELIVERY_PAYLOAD_VERSION,
        delivery_id: Uuid::from_u128(2),
        owner_id: Uuid::from_u128(3),
        record_id: Uuid::from_u128(4),
        record_generation: 1,
        payload: json!({}),
    };
    let context = JobContext {
        job_id: Uuid::from_u128(1),
        run_number: 1,
        attempt: 1,
        worker_id: "fixture-worker".into(),
        organization_id: Some(payload.owner_id),
        checkpoint: None,
    };
    (context, payload)
}

#[tokio::test(start_paused = true)]
async fn rejected_derivation_starts_no_state_or_provider_work() -> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let handler = worker.clone().into_job_handler();
    let (context, payload) = owned_payload();
    let value = serde_json::to_value(&payload)?;
    // Exactly the reserve leaves no provider time.
    let run = HandlerInvocation::for_context(FINAL_STATE_RESERVE, context.clone())?
        .execute(&handler, value.clone())
        .await;
    let failure = returned_failure(run);
    assert_eq!(failure.kind, runledger_core::jobs::JobFailureKind::Timeout);
    assert_eq!(failure.code, CODE_OPERATION_BUDGET_EXHAUSTED);
    let legacy = Legacy((tokio::time::Instant::now() + Duration::from_secs(10)).into_std());
    let failure = worker
        .execute(JobExecution::new(&context, &legacy), value.clone())
        .await
        .expect_err("services without an exit claim are refused");
    assert_eq!(failure.kind, runledger_core::jobs::JobFailureKind::Terminal);
    assert_eq!(failure.code, CODE_OPERATION_PHASES_UNAVAILABLE);
    // A derivable invocation reaches retained state, unreachable offline.
    let run = HandlerInvocation::for_context(Duration::from_secs(10), context)?
        .execute(&handler, value)
        .await;
    assert_eq!(returned_failure(run).code, CODE_STATE_UNAVAILABLE);
    Ok(())
}

#[tokio::test]
async fn invocation_exit_reaches_provider_admission_and_calls() -> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let independent = batter::operation::OperationOwner::new(Duration::from_secs(60))?;
    let held = worker
        .admission
        .enter(independent.context(), Admission::Wait)
        .await?;
    let (context, payload) = owned_payload();
    let admission = worker.admission.clone();
    let (phases, waiting) = in_invocation(Duration::from_secs(60), context, move |execution| {
        Box::pin(async move {
            let phases = provider_phases(execution).expect("phases");
            // Provider work that outlives the handler waits in its own task.
            let work = phases.work().clone();
            let waiting =
                tokio::spawn(
                    async move { admission.enter(&work, Admission::Wait).await.map(drop) },
                );
            tokio::task::yield_now().await;
            assert!(
                !waiting.is_finished(),
                "provider admission waits for the held slot"
            );
            (phases, waiting)
        })
    })
    .await;
    // The invocation's exit, not the test, interrupts the waiting admission.
    assert!(matches!(
        waiting.await?,
        Err(AdmissionError::Interrupted(
            batter::operation::Interruption::Cancelled
        ))
    ));
    // A provider call under the ended invocation is refused before any request.
    let request = provider_request(&payload);
    let dispatch = worker
        .provider
        .prepare_dispatch(&request.idempotency_key(), &request)?;
    let refused = dispatch
        .execute(phases.work(), held)
        .await
        .expect_err("cancelled provider work");
    assert_eq!(refused.code(), "delivery.provider_cancelled");
    assert!(
        independent.context().check().is_ok(),
        "only linked work stops"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn provider_work_expiry_leaves_the_final_state_reserve() -> Result<(), batter::BoxError> {
    let worker = offline_worker().await?;
    let (context, payload) = owned_payload();
    let budget = Duration::from_secs(10);
    let failure = in_invocation(budget, context, move |execution| {
        Box::pin(async move {
            let phases = provider_phases(execution).expect("phases");
            tokio::time::advance(budget - FINAL_STATE_RESERVE).await;
            let expired = worker
                .admission
                .enter(phases.work(), Admission::Wait)
                .await
                .expect_err("provider work expired");
            assert!(matches!(
                expired,
                AdmissionError::Interrupted(batter::operation::Interruption::DeadlineExceeded)
            ));
            assert!(phases.finalization().check().is_ok());
            assert_eq!(phases.finalization().remaining(), FINAL_STATE_RESERVE);
            // The handler still records the retained state; offline storage refuses it.
            worker
                .admission_failure(
                    execution,
                    &payload,
                    ProviderEffectState::AwaitingAttempt,
                    expired,
                )
                .await
                .expect_err("offline state")
        })
    })
    .await;
    assert_eq!(failure.code, CODE_STATE_UNAVAILABLE);
    Ok(())
}
