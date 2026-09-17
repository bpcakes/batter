use super::{
    DELIVERY_JOB_TYPE, DELIVERY_PAYLOAD_VERSION, DeliveryJobPayload, ProviderEffectState,
    provider_request,
};
use crate::provider::{
    DispatchOutcome, IDEMPOTENCY_RETENTION, PreparedDispatch, ProviderClient,
    ProviderEffectRequest, ReconcileOutcome,
};
use batter::{
    admission::{Admission, AdmissionError, Bulkhead},
    operation::OperationContext,
};
use runledger_core::{
    jobs::{JobCompletion, JobExecution, JobExecutionHandler, JobFailure, JobType},
    prelude::async_trait,
};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;

const FINAL_STATE_RESERVE: Duration = Duration::from_millis(500);
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
        let payload = decode_payload(execution, value)?;
        let budget = execution.remaining_work_budget(FINAL_STATE_RESERVE);
        let operation = OperationContext::new(budget).map_err(|_| {
            JobFailure::timeout(
                "delivery.operation_budget_exhausted",
                "No provider work budget remained.",
            )
        })?;

        self.execute_retained(execution, &operation, &payload).await
    }
}

impl DeliveryWorker {
    async fn admission_failure(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        state: ProviderEffectState,
        error: AdmissionError,
    ) -> Result<JobCompletion, JobFailure> {
        let (code, message, interrupted) = admission_failure_details(&error);
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

    async fn execute_retained(
        &self,
        execution: JobExecution<'_>,
        operation: &OperationContext,
        payload: &DeliveryJobPayload,
    ) -> Result<JobCompletion, JobFailure> {
        let mut snapshot = self.load_retained(execution, payload).await?;
        let provider_work = match snapshot.plan(payload.record_generation) {
            EffectPlan::Complete => return Ok(JobCompletion::success()),
            EffectPlan::Manual => {
                return Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "The retained provider outcome requires manual resolution.",
                ));
            }
            EffectPlan::Exhausted => {
                return Err(JobFailure::terminal(
                    CODE_ATTEMPTS_EXHAUSTED,
                    "Delivery attempts are exhausted.",
                ));
            }
            EffectPlan::Defer(delay) => {
                return Err(state_failure(StateError::RetryDeferred(delay)));
            }
            EffectPlan::DenyGeneration { expected } => {
                self.record_business_denial(
                    execution,
                    payload,
                    "target_generation_changed",
                    expected,
                )
                .await
                .map_err(state_failure)?;
                return Ok(JobCompletion::success());
            }
            EffectPlan::ExpireReconciliation => {
                self.record_manual(
                    execution,
                    payload,
                    "provider_retention_expired",
                    None,
                    true,
                    ProviderEffectState::ReconcileNeeded,
                )
                .await
                .map_err(state_failure)?;
                return Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "Provider retention expired before reconciliation.",
                ));
            }
            EffectPlan::Reconcile => ProviderWork::Reconcile,
            EffectPlan::Dispatch { expected } => ProviderWork::Dispatch {
                origin: DispatchOrigin::Initial(expected),
            },
        };

        let dispatch_origin = match provider_work {
            ProviderWork::Reconcile => match self
                .reconcile_existing(execution, operation, payload, &snapshot)
                .await?
            {
                ReconcileAction::Complete(completion) => return Ok(completion),
                ReconcileAction::DispatchAfterAbsence => {
                    snapshot = self.load_retained(execution, payload).await?;
                    if snapshot.state != ProviderEffectState::ReconcileNeeded {
                        return Err(state_failure(StateError::Invariant));
                    }
                    if snapshot.current_generation != Some(payload.record_generation) {
                        self.record_business_denial(
                            execution,
                            payload,
                            "target_generation_changed",
                            ProviderEffectState::ReconcileNeeded,
                        )
                        .await
                        .map_err(state_failure)?;
                        return Ok(JobCompletion::success());
                    }
                    DispatchOrigin::ProvenAbsent
                }
            },
            ProviderWork::Dispatch { origin } => origin,
        };

        self.dispatch(
            execution,
            operation,
            payload,
            &mut snapshot,
            dispatch_origin,
        )
        .await
    }

    async fn reconcile_existing(
        &self,
        execution: JobExecution<'_>,
        operation: &OperationContext,
        payload: &DeliveryJobPayload,
        snapshot: &EffectSnapshot,
    ) -> Result<ReconcileAction, JobFailure> {
        debug_assert!(snapshot.retention_active);
        let permit = match self.admission.enter(operation, Admission::Wait).await {
            Ok(permit) => permit,
            Err(error) => {
                return self
                    .admission_failure(execution, payload, snapshot.state, error)
                    .await
                    .map(ReconcileAction::Complete);
            }
        };
        match self
            .provider
            .reconcile(
                operation,
                permit,
                &snapshot.provider_key,
                &snapshot.provider_payload,
            )
            .await
        {
            Ok(ReconcileOutcome::Accepted { provider_effect_id }) => self
                .confirm(execution, payload, provider_effect_id)
                .await
                .map(ReconcileAction::Complete),
            Ok(ReconcileOutcome::Absent) => Ok(ReconcileAction::DispatchAfterAbsence),
            Ok(ReconcileOutcome::Expired) => {
                self.record_manual(
                    execution,
                    payload,
                    "provider_retention_expired",
                    None,
                    true,
                    ProviderEffectState::ReconcileNeeded,
                )
                .await
                .map_err(state_failure)?;
                Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "Provider lookup no longer retains a definitive outcome.",
                ))
            }
            Ok(ReconcileOutcome::Conflict) => {
                self.record_manual(
                    execution,
                    payload,
                    "provider_payload_conflict",
                    None,
                    true,
                    ProviderEffectState::ReconcileNeeded,
                )
                .await
                .map_err(state_failure)?;
                Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "Provider lookup conflicted with the retained payload.",
                ))
            }
            outcome @ (Ok(ReconcileOutcome::Unavailable) | Err(_)) => {
                let code = outcome
                    .as_ref()
                    .err()
                    .map_or(CODE_RECONCILE_UNAVAILABLE, |error| error.code());
                let state = self
                    .record_uncertain_failure(execution, payload, code)
                    .await
                    .map_err(state_failure)?;
                Err(retry_or_exhausted(
                    state,
                    code,
                    "Provider reconciliation did not return a definitive outcome.",
                    None,
                ))
            }
        }
    }

    async fn dispatch(
        &self,
        execution: JobExecution<'_>,
        operation: &OperationContext,
        payload: &DeliveryJobPayload,
        snapshot: &mut EffectSnapshot,
        origin: DispatchOrigin,
    ) -> Result<JobCompletion, JobFailure> {
        // Serialization and request construction finish before the durable
        // dispatch marker. Polling execute is the remote-effect boundary.
        let dispatch = self
            .prepare_retained_dispatch(execution, payload, snapshot)
            .await?;
        let permit = match self.admission.enter(operation, Admission::Wait).await {
            Ok(permit) => permit,
            Err(error) => {
                return self
                    .admission_failure(execution, payload, snapshot.state, error)
                    .await;
            }
        };
        let marked = match origin {
            DispatchOrigin::Initial(expected) => {
                self.mark_dispatch_possible(execution, payload, expected)
                    .await
            }
            DispatchOrigin::ProvenAbsent => {
                self.renew_dispatch_possible_after_absence(execution, payload)
                    .await
            }
        }
        .map_err(state_failure)?;
        if !marked {
            drop(permit);
            *snapshot = self.load_retained(execution, payload).await?;
            if snapshot.current_generation != Some(payload.record_generation) {
                self.record_business_denial(
                    execution,
                    payload,
                    "target_generation_changed",
                    snapshot.state,
                )
                .await
                .map_err(state_failure)?;
                return Ok(JobCompletion::success());
            }
            return Err(state_failure(StateError::Invariant));
        }

        let outcome = dispatch.execute(operation, permit).await;
        self.persist_dispatch_outcome(execution, payload, outcome)
            .await
    }

    async fn persist_dispatch_outcome(
        &self,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        outcome: Result<DispatchOutcome, crate::provider::ProviderCallError>,
    ) -> Result<JobCompletion, JobFailure> {
        match outcome {
            Ok(DispatchOutcome::Accepted { provider_effect_id }) => {
                self.confirm(execution, payload, provider_effect_id).await
            }
            Ok(DispatchOutcome::KnownNotDispatched { retry_not_before }) => Err(self
                .record_provider_undispatched(execution, payload, retry_not_before)
                .await),
            Ok(DispatchOutcome::BusinessDenied) => {
                self.record_business_denial(
                    execution,
                    payload,
                    "provider_business_denied",
                    ProviderEffectState::ReconcileNeeded,
                )
                .await
                .map_err(state_failure)?;
                Ok(JobCompletion::success())
            }
            Ok(DispatchOutcome::Conflict) => {
                self.record_manual(
                    execution,
                    payload,
                    "provider_payload_conflict",
                    None,
                    true,
                    ProviderEffectState::ReconcileNeeded,
                )
                .await
                .map_err(state_failure)?;
                Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "Provider identity conflicted with the retained payload.",
                ))
            }
            outcome @ (Ok(DispatchOutcome::Indeterminate) | Err(_)) => {
                let code = outcome
                    .as_ref()
                    .err()
                    .map_or(CODE_DISPATCH_INDETERMINATE, |error| error.code());
                let state = self
                    .record_uncertain_failure(execution, payload, code)
                    .await
                    .map_err(state_failure)?;
                Err(retry_or_exhausted(
                    state,
                    code,
                    "Provider dispatch may have been accepted and requires reconciliation.",
                    None,
                ))
            }
        }
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
            false,
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
    ) -> Result<EffectSnapshot, JobFailure> {
        match self.load(execution, payload).await {
            Ok(snapshot) => Ok(snapshot),
            Err(StateError::Storage) => Err(state_failure(StateError::Storage)),
            Err(StateError::LeaseLost) => Err(state_failure(StateError::LeaseLost)),
            Err(StateError::RetryDeferred(delay)) => {
                Err(state_failure(StateError::RetryDeferred(delay)))
            }
            Err(StateError::Invariant) => {
                self.record_state_invariant(execution, payload)
                    .await
                    .map_err(state_failure)?;
                Err(JobFailure::terminal(
                    CODE_MANUAL_RESOLUTION,
                    "The retained provider state requires manual resolution.",
                ))
            }
        }
    }
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
enum ProviderWork {
    Reconcile,
    Dispatch { origin: DispatchOrigin },
}

#[derive(Clone, Copy)]
enum DispatchOrigin {
    Initial(ProviderEffectState),
    ProvenAbsent,
}

enum ReconcileAction {
    Complete(JobCompletion),
    DispatchAfterAbsence,
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

mod state;
#[cfg(test)]
mod tests;
