//! The native invocation owns borrowed work, retained intent and final-state SQL.
use super::*;
use batter::{
    command::CommandCause,
    operation::{Interruption, PhasedFailure},
};
use std::sync::Mutex;

pub(super) enum Action {
    Complete,
    Confirm(String),
    Deny {
        code: &'static str,
        expected: ProviderEffectState,
    },
    Manual {
        code: &'static str,
        message: &'static str,
        expected: ProviderEffectState,
    },
    Undispatched(Duration),
    Uncertain {
        code: &'static str,
        message: &'static str,
    },
    Admission {
        state: ProviderEffectState,
        error: AdmissionError,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Progress {
    Preparing,
    Admission(ProviderEffectState),
    StateChange,
    Reconcile,
    Dispatch,
}

pub(super) struct Invocation<'a> {
    pub worker: &'a DeliveryWorker,
    pub payload: &'a DeliveryJobPayload,
    progress: Mutex<Progress>,
}
impl Invocation<'_> {
    pub fn progress(&self, progress: Progress) {
        *self.progress.lock().expect("invocation state") = progress;
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
            Action::Confirm(id) => worker.confirm(execution, payload, id.clone()).await,
            Action::Deny { code, expected } => {
                worker
                    .record_business_denial(execution, payload, code, *expected)
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
                        *expected,
                    )
                    .await
                    .map_err(state_failure)?;
                Err(JobFailure::terminal(CODE_MANUAL_RESOLUTION, *message))
            }
            Action::Undispatched(delay) => Err(worker
                .record_provider_undispatched(execution, payload, *delay)
                .await),
            Action::Uncertain { code, message } => {
                let state = worker
                    .record_uncertain_failure(execution, payload, code)
                    .await
                    .map_err(state_failure)?;
                Err(retry_or_exhausted(state, code, message, None))
            }
            Action::Admission { state, error } => {
                worker
                    .admission_failure(execution, payload, *state, error)
                    .await
            }
        }
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
        let invocation = Invocation {
            worker: self,
            payload: &payload,
            progress: Mutex::new(Progress::Preparing),
        };
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
        let progress = *self.progress.lock().expect("invocation state");
        let action = match progress {
            Progress::Admission(state) => Action::Admission {
                state,
                error: AdmissionError::Interrupted(interruption.unwrap_or(Interruption::Cancelled)),
            },
            Progress::Reconcile | Progress::Dispatch => Action::Uncertain {
                code: interruption.map_or(CODE_DISPATCH_INDETERMINATE, |reason| {
                    crate::provider::ProviderCallError::Interrupted(reason).code()
                }),
                message: "Provider work ended without a definitive retained outcome.",
            },
            // Pending state SQL may have committed. Do not invent non-dispatch
            // or overwrite a concurrent transition; the next attempt reloads it.
            Progress::Preparing | Progress::StateChange => return Ok(JobCompletion::success()),
        };
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
