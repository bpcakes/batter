//! Own exchange execution and its interruption evidence at the same boundary.
//!
//! Consumers cannot mark a fence/provider call as started independently of
//! executing it. The retained state survives destruction of interrupted work;
//! it is observation for recovery, never proof of a remote effect.
use super::phased::Action;
use super::state::UnresolvedEffectState;
use super::*;
use crate::provider::ProviderCallError;
use batter::operation::Interruption;
use std::sync::Mutex;

pub(super) struct Exchange {
    recovery: Mutex<Recovery>,
    #[cfg(test)]
    hold_fence_ack: bool,
}

#[derive(Clone, Copy)]
enum Recovery {
    Idle,
    Admission(UnresolvedEffectState),
    Fence,
    Provider,
}

pub(super) enum Sent {
    GenerationChanged,
    Outcome(Result<DispatchOutcome, ProviderCallError>),
}

impl Exchange {
    pub(super) fn new() -> Self {
        Self {
            recovery: Mutex::new(Recovery::Idle),
            #[cfg(test)]
            hold_fence_ack: false,
        }
    }

    pub(super) async fn reconcile(
        &self,
        worker: &DeliveryWorker,
        operation: &OperationContext,
        snapshot: &EffectSnapshot,
    ) -> Result<Result<ReconcileOutcome, ProviderCallError>, BulkheadError<JobFailure>> {
        // This operation is only meaningful for retained uncertain dispatch.
        if snapshot.state != ProviderEffectState::ReconcileNeeded || !snapshot.retention_active {
            return Err(BulkheadError::Operation(OperationError::Failed(
                state_failure(StateError::Invariant),
            )));
        }
        *self.recovery.lock().expect("exchange recovery") =
            Recovery::Admission(UnresolvedEffectState::ReconcileNeeded);
        let result = worker
            .admission
            .run(
                operation,
                Admission::Wait,
                "provider.reconcile.admitted",
                |scope| async move {
                    *self.recovery.lock().expect("exchange recovery") = Recovery::Provider;
                    Ok(worker
                        .provider
                        .reconcile(&scope, &snapshot.provider_key, &snapshot.provider_payload)
                        .await)
                },
            )
            .await;
        if matches!(result, Ok(Ok(ReconcileOutcome::Absent))) {
            // Lookup completed without an effect. Subsequent local planning
            // must not inherit an interrupted-provider recovery obligation.
            *self.recovery.lock().expect("exchange recovery") = Recovery::Idle;
        }
        result
    }

    pub(super) async fn dispatch(
        &self,
        worker: &DeliveryWorker,
        execution: JobExecution<'_>,
        payload: &DeliveryJobPayload,
        operation: &OperationContext,
        snapshot: &EffectSnapshot,
        origin: DispatchOrigin,
    ) -> Result<Sent, BulkheadError<JobFailure>> {
        let dispatch = worker
            .prepare_retained_dispatch(execution, payload, snapshot)
            .await
            .map_err(|error| BulkheadError::Operation(OperationError::Failed(error)))?;
        let source = snapshot.state.try_into().map_err(|error| {
            BulkheadError::Operation(OperationError::Failed(state_failure(error)))
        })?;
        *self.recovery.lock().expect("exchange recovery") = Recovery::Admission(source);
        let result = worker
            .admission
            .run(
                operation,
                Admission::Wait,
                "provider.dispatch.admitted",
                |scope| async move {
                    *self.recovery.lock().expect("exchange recovery") = Recovery::Fence;
                    let marked = match origin {
                        DispatchOrigin::Initial(expected) => {
                            worker
                                .mark_dispatch_possible(execution, payload, expected)
                                .await
                        }
                        DispatchOrigin::ProvenAbsent => {
                            worker
                                .renew_dispatch_possible_after_absence(execution, payload)
                                .await
                        }
                    };
                    let marked = marked.map_err(state_failure)?;
                    if !marked {
                        return Ok(Sent::GenerationChanged);
                    }
                    #[cfg(test)]
                    if self.hold_fence_ack {
                        // Exercise committed SQL whose acknowledgement never reaches
                        // work. This is not a simulated wire-level COMMIT failure.
                        std::future::pending::<()>().await;
                    }
                    *self.recovery.lock().expect("exchange recovery") = Recovery::Provider;
                    Ok(Sent::Outcome(dispatch.execute(&scope).await))
                },
            )
            .await;
        // Only an outcome retained by the operation boundary ends fence recovery.
        // A concrete callback error can still be superseded by that boundary's
        // interruption, so clearing inside the callback would lose ambiguity.
        if matches!(
            result,
            Ok(Sent::GenerationChanged) | Err(BulkheadError::Operation(OperationError::Failed(_)))
        ) {
            *self.recovery.lock().expect("exchange recovery") = Recovery::Idle;
        }
        result
    }

    pub(super) fn interrupted(&self, interruption: Option<Interruption>) -> Action {
        match *self.recovery.lock().expect("exchange recovery") {
            Recovery::Idle => Action::Complete,
            Recovery::Admission(state) => Action::Admission {
                state,
                error: AdmissionError::Interrupted(interruption.unwrap_or(Interruption::Cancelled)),
            },
            Recovery::Fence => Action::FenceInterrupted,
            Recovery::Provider => Action::Uncertain {
                code: interruption.map_or(CODE_DISPATCH_INDETERMINATE, |reason| {
                    ProviderCallError::Interrupted(reason).code()
                }),
                message: "Provider work ended without a definitive retained outcome.",
            },
        }
    }

    #[cfg(test)]
    pub(super) fn is_admitting(&self) -> bool {
        matches!(*self.recovery.lock().unwrap(), Recovery::Admission(_))
    }

    #[cfg(test)]
    pub(super) fn is_fencing(&self) -> bool {
        matches!(*self.recovery.lock().unwrap(), Recovery::Fence)
    }

    #[cfg(test)]
    pub(super) fn with_held_fence_ack() -> Self {
        Self {
            hold_fence_ack: true,
            ..Self::new()
        }
    }
}
