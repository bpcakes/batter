//! The native invocation owns borrowed work, retained intent and final-state SQL.
use super::*;
use super::{exchange::Exchange, state::UnresolvedEffectState};
use batter::{
    command::CommandCause,
    operation::{PhaseOutcome, PhasedFailure},
    runledger::{LateJobFinalization, LateJobSettlement},
};

pub(super) enum Action {
    Complete,
    Invariant,
    Confirm(String),
    Deny {
        code: &'static str,
        expected: UnresolvedEffectState,
    },
    Manual {
        code: &'static str,
        message: &'static str,
        expected: UnresolvedEffectState,
    },
    Undispatched(Duration),
    FenceInterrupted,
    Uncertain {
        code: &'static str,
        message: &'static str,
    },
    Admission {
        state: UnresolvedEffectState,
        error: AdmissionError,
    },
}

pub(super) struct Invocation<'a> {
    pub worker: &'a DeliveryWorker,
    pub payload: &'a DeliveryJobPayload,
    pub(super) exchange: Exchange,
}
impl Invocation<'_> {
    pub fn new<'a>(worker: &'a DeliveryWorker, payload: &'a DeliveryJobPayload) -> Invocation<'a> {
        Invocation {
            worker,
            payload,
            exchange: Exchange::new(),
        }
    }

    async fn finish(
        &self,
        execution: JobExecution<'_>,
        action: &Action,
    ) -> Result<JobCompletion, JobFailure> {
        let worker = self.worker;
        let payload = self.payload;
        match action {
            Action::Complete => Ok(JobCompletion::success()),
            Action::Invariant => worker.finish_state_invariant(execution, payload).await,
            Action::Confirm(id) => worker.confirm(execution, payload, id.clone()).await,
            Action::Deny { code, expected } => {
                worker
                    .record_business_denial(execution, payload, code, expected.retained())
                    .await
                    .map_err(state_failure)?;
                Ok(JobCompletion::success())
            }
            Action::Manual {
                code,
                message,
                expected,
            } => {
                worker
                    .record_manual(
                        execution,
                        payload,
                        code,
                        None,
                        ProviderAcceptance::Possible,
                        expected.retained(),
                    )
                    .await
                    .map_err(state_failure)?;
                Err(JobFailure::terminal(CODE_MANUAL_RESOLUTION, *message))
            }
            Action::Undispatched(delay) => Err(worker
                .record_provider_undispatched(execution, payload, *delay)
                .await),
            Action::FenceInterrupted => self.finish_interrupted_fence(execution).await,
            Action::Uncertain { code, message } => {
                let state = worker
                    .record_uncertain_failure(execution, payload, code)
                    .await
                    .map_err(state_failure)?;
                Err(retry_or_exhausted(state, code, message, None))
            }
            Action::Admission { state, error } => {
                worker
                    .admission_failure(execution, payload, state.retained(), error)
                    .await
            }
        }
    }

    async fn finish_interrupted_fence(
        &self,
        execution: JobExecution<'_>,
    ) -> Result<JobCompletion, JobFailure> {
        // Cancellation cannot tell us whether the fence committed. Reload
        // under the live native lease before choosing a legal source state.
        let RetainedLoad::Snapshot(snapshot) =
            self.worker.load_retained(execution, self.payload).await?
        else {
            return self
                .worker
                .finish_state_invariant(execution, self.payload)
                .await;
        };
        let state = match snapshot.state {
            state @ (ProviderEffectState::AwaitingAttempt
            | ProviderEffectState::RetryableUndispatched) => {
                self.worker
                    .record_admission_undispatched(
                        execution,
                        self.payload,
                        CODE_OPERATION_BUDGET_EXHAUSTED,
                        state,
                    )
                    .await
            }
            ProviderEffectState::ReconcileNeeded => {
                self.worker
                    .record_uncertain_failure(
                        execution,
                        self.payload,
                        CODE_OPERATION_BUDGET_EXHAUSTED,
                    )
                    .await
            }
            ProviderEffectState::Confirmed
            | ProviderEffectState::BusinessDenied
            | ProviderEffectState::ManualResolution
            | ProviderEffectState::Exhausted => return Ok(JobCompletion::success()),
        }
        .map_err(state_failure)?;
        let message = "Provider dispatch fence ended before acknowledgement.";
        if state == ProviderEffectState::Exhausted {
            return Err(retry_or_exhausted(
                state,
                CODE_OPERATION_BUDGET_EXHAUSTED,
                message,
                None,
            ));
        }
        Err(JobFailure::timeout(
            CODE_OPERATION_BUDGET_EXHAUSTED,
            message,
        ))
    }
}

#[async_trait]
impl JobExecutionHandler for DeliveryWorker {
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    async fn execute(
        &self,
        execution: JobExecution<'_>,
        value: Value,
    ) -> Result<JobCompletion, JobFailure> {
        use runledger_core::jobs::JobHandler;
        let payload = decode_payload(execution, value)?;
        let invocation = Invocation::new(self, &payload);
        PhasedJob::new(invocation, FINAL_STATE_RESERVE)
            .expect("positive final-state reserve")
            .execute_with_services(execution, Value::Null)
            .await
    }
}

#[async_trait]
impl PhasedJobHandler for Invocation<'_> {
    type Output = Action;
    type Error = JobFailure;
    fn job_type(&self) -> JobType<'static> {
        JobType::new(DELIVERY_JOB_TYPE)
    }
    async fn work(&self, scope: JobWork<'_>, _: Value) -> Result<Action, JobFailure> {
        self.prepare(scope.execution(), scope.context()).await
    }
    fn settle_late<'a>(
        &'a self,
        scope: LateJobFinalization<'a, Action>,
    ) -> LateJobSettlement<'a, JobFailure> {
        scope.settle(async move |scope| self.finish(scope.execution(), scope.value()).await)
    }
    async fn finalize(
        &self,
        scope: JobFinalization<'_, Action, JobFailure>,
    ) -> Result<JobCompletion, JobFailure> {
        let failure = match scope.work().classify() {
            Ok(action) => return self.finish(scope.execution(), action).await,
            Err(failure) => failure,
        };
        // A result returned at the boundary is evidence even when it arrived late.
        if let Ok(action) = failure.returned_result() {
            return self.finish(scope.execution(), action).await;
        }
        let interruption = match failure.returned_result() {
            Err(CommandCause::Interrupted(reason)) => Some(*reason),
            _ => failure.interruption_after_return(),
        };
        let action = self.exchange.interrupted(interruption);
        self.finish(scope.execution(), &action).await
    }
    fn failed(&self, failure: PhasedJobFailure<Action, JobFailure>) -> JobFailure {
        match failure {
            PhasedJobFailure::Rejected(
                JobPhasesRejection::Exhausted | JobPhasesRejection::Ended,
            ) => JobFailure::timeout(
                CODE_OPERATION_BUDGET_EXHAUSTED,
                "No provider work budget remained.",
            ),
            PhasedJobFailure::Rejected(_) => JobFailure::terminal(
                CODE_OPERATION_PHASES_UNAVAILABLE,
                "Provider work phases could not be derived from this invocation.",
            ),
            PhasedJobFailure::Phases(PhasedFailure::Report(report)) => {
                if phase_panicked(report.work()) || phase_panicked(report.finalization()) {
                    return JobFailure::panicked("job.handler_panic", "Job handler panicked.");
                }
                if let Err(work) = report.work().classify()
                    && let Err(CommandCause::Failed(error)) = work.returned_result()
                {
                    return error.clone();
                }
                if let Err(finalization) = report.finalization().classify()
                    && let Err(CommandCause::Failed(error)) = finalization.returned_result()
                {
                    return error.clone();
                }
                JobFailure::timeout(
                    CODE_OPERATION_BUDGET_EXHAUSTED,
                    "Provider work ended before completion.",
                )
            }
            PhasedJobFailure::Phases(PhasedFailure::Coordinator(_)) => JobFailure::terminal(
                CODE_OPERATION_PHASES_UNAVAILABLE,
                "Provider phase driver failed.",
            ),
        }
    }
}

fn phase_panicked<T, E>(phase: &PhaseOutcome<T, E>) -> bool {
    phase.classify().is_err_and(|failure| {
        matches!(failure.returned_result(), Err(CommandCause::Panicked(_)))
            || failure.destruction_panic().is_some()
    })
}
