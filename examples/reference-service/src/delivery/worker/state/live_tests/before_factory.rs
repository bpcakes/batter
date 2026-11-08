use super::super::super::phased::{Action, Invocation};
use super::*;
use batter::runledger::{LateJobFinalization, LateJobSettlement};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::time::Instant;
use tracing::{
    Subscriber,
    field::{Field, Visit},
    instrument::WithSubscriber,
};
use tracing_subscriber::{Layer, layer::Context, prelude::*};

// Operation span creation is after the bulkhead's admission recheck and before
// OperationContext's preflight. Delay there to exercise refusal without ever
// entering the admitted callback; no production test hook or global subscriber.
struct ExpireBeforeFactory {
    operation: &'static str,
    deadline: Arc<Mutex<Option<Instant>>>,
    fired: Arc<AtomicBool>,
}
struct OperationName<'a> {
    expected: &'a str,
    matched: bool,
}
impl Visit for OperationName<'_> {
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "operation" && value == self.expected {
            self.matched = true;
        }
    }
}
impl<S: Subscriber> Layer<S> for ExpireBeforeFactory {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _: &tracing::Id,
        _: Context<'_, S>,
    ) {
        let mut name = OperationName {
            expected: self.operation,
            matched: false,
        };
        attrs.record(&mut name);
        if name.matched && !self.fired.swap(true, Ordering::SeqCst) {
            let deadline = self
                .deadline
                .lock()
                .unwrap()
                .expect("work supplied its deadline");
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(10),
            );
        }
    }
}
struct Probe<'a> {
    invocation: Invocation<'a>,
    deadline: Arc<Mutex<Option<Instant>>>,
    late: Arc<AtomicBool>,
}
#[async_trait]
impl PhasedJobHandler for Probe<'_> {
    type Output = Action;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    async fn work(&self, scope: JobWork<'_>, payload: Value) -> Result<Action, JobFailure> {
        *self.deadline.lock().unwrap() = Some(scope.context().deadline());
        self.invocation.work(scope, payload).await
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, Action, JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        assert!(
            !self.late.load(Ordering::SeqCst),
            "settlement must not decline"
        );
        self.invocation.finalize(scope).await
    }
    fn settle_late<'a>(
        &'a self,
        scope: LateJobFinalization<'a, Action>,
    ) -> LateJobSettlement<'a, JobFailure> {
        assert!(
            self.invocation.exchange.is_admitting(),
            "neither fence nor provider callback started"
        );
        assert!(matches!(scope.value(), Action::Admission { .. }));
        self.late.store(true, Ordering::SeqCst);
        self.invocation.settle_late(scope)
    }
    fn failed(&self, failure: PhasedJobFailure<Action, JobFailure>) -> JobFailure {
        self.invocation.failed(failure)
    }
}

pub(super) async fn interruption(worker: &DeliveryWorker) -> Result<(), BoxError> {
    for source in [
        ProviderEffectState::AwaitingAttempt,
        ProviderEffectState::RetryableUndispatched,
        ProviderEffectState::ReconcileNeeded,
    ] {
        let (context, payload) = claimed_in_state(worker, source).await?;
        let deadline = Arc::new(Mutex::new(None));
        let fired = Arc::new(AtomicBool::new(false));
        let late = Arc::new(AtomicBool::new(false));
        let subscriber = tracing_subscriber::registry().with(ExpireBeforeFactory {
            operation: if source == ProviderEffectState::ReconcileNeeded {
                "provider.reconcile.admitted"
            } else {
                "provider.dispatch.admitted"
            },
            deadline: deadline.clone(),
            fired: fired.clone(),
        });
        let handler = PhasedJob::new(
            Probe {
                invocation: Invocation::new(worker, &payload),
                deadline,
                late: late.clone(),
            },
            PROBE_BUDGET - Duration::from_millis(500),
        )?;
        let run = HandlerInvocation::for_context(PROBE_BUDGET, context)?
            .execute(&handler, Value::Null)
            .with_subscriber(subscriber)
            .await;
        assert!(
            fired.load(Ordering::SeqCst),
            "deadline crossed after admission"
        );
        assert!(
            late.load(Ordering::SeqCst),
            "production late settlement ran"
        );
        let failure = returned_failure(run);
        assert_eq!(failure.kind, runledger_core::jobs::JobFailureKind::Timeout);
        assert_eq!(failure.code, CODE_ADMISSION_INTERRUPTED);
        assert!(
            failure.kind.is_retryable(),
            "unstarted work remains eligible for retry"
        );
        let (state, possible, has_fence, code): (String, bool, bool, Option<String>) = sqlx::query_as(
            "SELECT state, acceptance_possible, dispatch_possible_at IS NOT NULL, outcome_code FROM reference_delivery_effects WHERE delivery_id = $1")
            .bind(payload.delivery_id).fetch_one(&worker.pool).await?;
        let prior_fence = source == ProviderEffectState::ReconcileNeeded;
        assert_eq!(
            state,
            if prior_fence {
                "RECONCILE_NEEDED"
            } else {
                "RETRYABLE_UNDISPATCHED"
            }
        );
        assert_eq!(possible, prior_fence);
        assert_eq!(has_fence, prior_fence, "no new dispatch fence was written");
        assert_eq!(code.as_deref(), Some(CODE_ADMISSION_INTERRUPTED));
    }
    Ok(())
}

async fn claimed_in_state(
    worker: &DeliveryWorker,
    source: ProviderEffectState,
) -> Result<(JobContext, DeliveryJobPayload), BoxError> {
    let (context, payload) = claimed(&worker.pool).await?;
    if source != ProviderEffectState::AwaitingAttempt {
        in_invocation(PROBE_BUDGET, context.clone(), {
            let (worker, payload) = (worker.clone(), payload.clone());
            move |execution| {
                Box::pin(async move {
                    assert!(
                        worker
                            .mark_dispatch_possible(
                                execution,
                                &payload,
                                ProviderEffectState::AwaitingAttempt
                            )
                            .await
                            .unwrap()
                    );
                    if source == ProviderEffectState::RetryableUndispatched {
                        assert!(
                            worker
                                .record_provider_undispatched(execution, &payload, Duration::ZERO)
                                .await
                                .kind
                                .is_retryable()
                        );
                    }
                })
            }
        })
        .await;
    }
    Ok((context, payload))
}
