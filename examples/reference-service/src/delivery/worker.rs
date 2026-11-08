use crate::delivery::worker::state::ProviderAcceptance;

use super::{
    DELIVERY_JOB_TYPE, DELIVERY_PAYLOAD_VERSION, DeliveryJobPayload, ProviderEffectState,
    provider_request,
};
use crate::provider::{
    DispatchOutcome, IDEMPOTENCY_RETENTION, PreparedDispatch, ProviderClient,
    ProviderEffectRequest, ReconcileOutcome,
};
use batter::{
    admission::{Admission, AdmissionError, Bulkhead, BulkheadError},
    operation::{OperationContext, OperationError},
    runledger::{
        JobFinalization, JobPhasesRejection, JobWork, PhasedJob, PhasedJobFailure, PhasedJobHandler,
    },
};
use runledger_core::{
    jobs::{JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType},
    prelude::async_trait,
};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;

/// Kept inside the native handler deadline for the retained final-state SQL
/// that follows provider work, including after provider work expires.
const FINAL_STATE_RESERVE: Duration = Duration::from_millis(500);
const CODE_OPERATION_BUDGET_EXHAUSTED: &str = "delivery.operation_budget_exhausted";
const CODE_OPERATION_PHASES_UNAVAILABLE: &str = "delivery.operation_phases_unavailable";
const CODE_ADMISSION_OVERLOADED: &str = "delivery.admission_overloaded";
const CODE_ADMISSION_CLOSED: &str = "delivery.admission_closed";
const CODE_ADMISSION_INTERRUPTED: &str = "delivery.admission_interrupted";
const CODE_PROVIDER_UNDISPATCHED: &str = "delivery.provider_undispatched";
const CODE_RECONCILE_UNAVAILABLE: &str = "delivery.reconcile_unavailable";
const CODE_DISPATCH_INDETERMINATE: &str = "delivery.dispatch_indeterminate";
const CODE_STATE_UNAVAILABLE: &str = "delivery.state_unavailable";
const CODE_LEASE_LOST: &str = "delivery.lease_lost";
const CODE_ATTEMPTS_EXHAUSTED: &str = "delivery.attempts_exhausted";
const CODE_MANUAL_RESOLUTION: &str = "delivery.manual_resolution_required";
const CODE_INVALID_PAYLOAD: &str = "delivery.invalid_payload";

/// Application handler for the one selected provider-effect protocol.
#[derive(Clone)]
pub(crate) struct DeliveryWorker {
    pool: PgPool,
    provider: ProviderClient,
    admission: Bulkhead,
}

impl DeliveryWorker {
    pub(crate) fn new(pool: PgPool, provider: ProviderClient, admission: Bulkhead) -> Self {
        Self {
            pool,
            provider,
            admission,
        }
    }
}

impl DeliveryWorker {
    async fn admission_failure(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        state: ProviderEffectState,
        error: &AdmissionError,
    ) -> Result<JobCompletion, JobFailure> {
        let (code, message, interrupted) = admission_failure_details(error);
        let state = match state {
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched => {
                self.record_admission_undispatched(execution, payload, code, state)
                    .await
            }
            ProviderEffectState::ReconcileNeeded => {
                self.record_uncertain_failure(execution, payload, code)
                    .await
            }
            ProviderEffectState::Confirmed
            | ProviderEffectState::BusinessDenied
            | ProviderEffectState::ManualResolution
            | ProviderEffectState::Exhausted => Err(StateError::Invariant),
        }
        .map_err(state_failure)?;
        if state == ProviderEffectState::Exhausted {
            return Err(retry_or_exhausted(state, code, message, None));
        }
        if interrupted {
            return Err(JobFailure::timeout(code, message));
        }
        Err(retry_or_exhausted(state, code, message, None))
    }

    async fn prepare_retained_dispatch(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        snapshot: &EffectSnapshot,
    ) -> Result<PreparedDispatch, JobFailure> {
        let preparation = self
            .provider
            .prepare_dispatch(&snapshot.provider_key, &snapshot.provider_payload);
        if let Ok(dispatch) = preparation {
            return Ok(dispatch);
        }
        self.record_manual(
            execution,
            payload,
            "provider_request_invalid",
            None,
            ProviderAcceptance::RuledOut,
            snapshot.state,
        )
        .await
        .map_err(state_failure)?;
        Err(JobFailure::terminal(
            "delivery.provider_request_invalid",
            "The retained provider request could not be constructed.",
        ))
    }

    async fn load_retained(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<RetainedLoad, JobFailure> {
        match self.load(execution, payload).await {
            Ok(snapshot) => Ok(RetainedLoad::Snapshot(snapshot)),
            Err(StateError::Storage) => Err(state_failure(StateError::Storage)),
            Err(StateError::LeaseLost) => Err(state_failure(StateError::LeaseLost)),
            Err(StateError::RetryDeferred(delay)) => {
                Err(state_failure(StateError::RetryDeferred(delay)))
            }
            Err(StateError::Invariant) => Ok(RetainedLoad::Invariant),
        }
    }

    async fn finish_state_invariant(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
    ) -> Result<JobCompletion, JobFailure> {
        self.record_state_invariant(execution, payload)
            .await
            .map_err(state_failure)?;
        Err(JobFailure::terminal(
            CODE_MANUAL_RESOLUTION,
            "The retained provider state requires manual resolution.",
        ))
    }
}

// Detection belongs to work; the terminal write belongs to finalization.
enum RetainedLoad {
    Snapshot(EffectSnapshot),
    Invariant,
}

struct EffectSnapshot {
    provider_key: String,
    provider_payload: ProviderEffectRequest,
    state: ProviderEffectState,
    retention_active: bool,
    retry_delay: Duration,
    current_generation: Option<i64>,
}

impl EffectSnapshot {
    fn plan(&self, expected_generation: i64) -> EffectPlan {
        match self.state {
            ProviderEffectState::Confirmed | ProviderEffectState::BusinessDenied => {
                EffectPlan::Complete
            }
            ProviderEffectState::ManualResolution => EffectPlan::Manual,
            ProviderEffectState::Exhausted => EffectPlan::Exhausted,
            ProviderEffectState::ReconcileNeeded if !self.retention_active => {
                EffectPlan::ExpireReconciliation
            }
            ProviderEffectState::ReconcileNeeded => EffectPlan::Reconcile,
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched
                if self.current_generation != Some(expected_generation) =>
            {
                EffectPlan::DenyGeneration {
                    expected: self.state,
                }
            }
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched
                if !self.retry_delay.is_zero() =>
            {
                EffectPlan::Defer(self.retry_delay)
            }
            ProviderEffectState::AwaitingAttempt | ProviderEffectState::RetryableUndispatched => {
                EffectPlan::Dispatch {
                    expected: self.state,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EffectPlan {
    Complete,
    Manual,
    Exhausted,
    Defer(Duration),
    DenyGeneration { expected: ProviderEffectState },
    ExpireReconciliation,
    Reconcile,
    Dispatch { expected: ProviderEffectState },
}

#[derive(Clone, Copy)]
enum DispatchOrigin {
    Initial(ProviderEffectState),
    ProvenAbsent,
}

#[derive(Clone, Copy, Debug)]
enum StateError {
    Storage,
    LeaseLost,
    Invariant,
    RetryDeferred(Duration),
}

fn remaining_retry_delay(milliseconds: i64) -> Duration {
    Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0))
}

fn decode_payload(
    execution: JobExecution<'_>,
    value: Value,
) -> Result<DeliveryJobPayload, JobFailure> {
    let payload: DeliveryJobPayload = serde_json::from_value(value)
        .map_err(|_| JobFailure::terminal(CODE_INVALID_PAYLOAD, "Delivery payload is invalid."))?;
    if payload.version != DELIVERY_PAYLOAD_VERSION
        || payload.delivery_id.is_nil()
        || payload.owner_id.is_nil()
        || payload.record_id.is_nil()
        || payload.record_generation <= 0
        || execution.context().organization_id != Some(payload.owner_id)
    {
        return Err(JobFailure::terminal(
            CODE_INVALID_PAYLOAD,
            "Delivery payload identity is invalid.",
        ));
    }
    Ok(payload)
}

fn state_failure(error: StateError) -> JobFailure {
    match error {
        StateError::Storage => JobFailure::retryable(
            CODE_STATE_UNAVAILABLE,
            "Delivery effect state could not be persisted.",
        ),
        StateError::LeaseLost => JobFailure::retryable(
            CODE_LEASE_LOST,
            "Delivery effect state rejected an expired or reassigned lease.",
        ),
        StateError::Invariant => JobFailure::terminal(
            "delivery.state_invariant",
            "Delivery effect state violated a retained invariant.",
        ),
        StateError::RetryDeferred(delay) => JobFailure::retryable(
            "delivery.provider_retry_deferred",
            "Provider retry eligibility has not arrived.",
        )
        .retry_not_before_delay(delay),
    }
}

fn admission_failure_details(error: &AdmissionError) -> (&'static str, &'static str, bool) {
    match error {
        AdmissionError::Overloaded => (
            CODE_ADMISSION_OVERLOADED,
            "Provider admission refused this native attempt.",
            false,
        ),
        AdmissionError::Closed => (
            CODE_ADMISSION_CLOSED,
            "Provider admission closed during this native attempt.",
            false,
        ),
        AdmissionError::Interrupted(_) => (
            CODE_ADMISSION_INTERRUPTED,
            "Provider admission was interrupted.",
            true,
        ),
    }
}

fn retry_or_exhausted(
    state: ProviderEffectState,
    retry_code: &'static str,
    message: &'static str,
    retry_not_before: Option<Duration>,
) -> JobFailure {
    if state == ProviderEffectState::Exhausted {
        return JobFailure::terminal(CODE_ATTEMPTS_EXHAUSTED, "Delivery attempts are exhausted.");
    }
    let failure = JobFailure::retryable(retry_code, message);
    match retry_not_before {
        Some(delay) => failure.retry_not_before_delay(delay),
        None => failure,
    }
}

mod exchange;
mod execute;
mod phased;
mod state;
#[cfg(test)]
mod tests;
