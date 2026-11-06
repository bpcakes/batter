//! Work returns a final-state action; the adapter runs its persistence afterward.
use super::phased::{Action, Invocation, Progress};
use super::*;
use crate::provider::ProviderCallError;

enum Sent {
    GenerationChanged,
    Outcome(Result<DispatchOutcome, ProviderCallError>),
}

impl Invocation<'_> {
    pub(super) async fn prepare(
        &self,
        execution: JobExecution<'_>,
        operation: &OperationContext,
    ) -> Result<Action, JobFailure> {
        let worker = self.worker;
        let payload = self.payload;
        let mut snapshot = worker.load_retained(execution, payload).await?;
        let origin = match snapshot.plan(payload.record_generation) {
            EffectPlan::Complete => return Ok(Action::Complete),
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
                return Ok(Action::Deny {
                    code: "target_generation_changed",
                    expected,
                });
            }
            EffectPlan::ExpireReconciliation => {
                return Ok(Action::Manual {
                    code: "provider_retention_expired",
                    message: "Provider retention expired before reconciliation.",
                    expected: ProviderEffectState::ReconcileNeeded,
                });
            }
            EffectPlan::Reconcile => {
                if let Some(action) = self.reconcile(execution, operation, &snapshot).await? {
                    return Ok(action);
                }
                self.progress(Progress::Preparing);
                snapshot = worker.load_retained(execution, payload).await?;
                if snapshot.state != ProviderEffectState::ReconcileNeeded {
                    return Err(state_failure(StateError::Invariant));
                }
                if snapshot.current_generation != Some(payload.record_generation) {
                    return Ok(Action::Deny {
                        code: "target_generation_changed",
                        expected: ProviderEffectState::ReconcileNeeded,
                    });
                }
                DispatchOrigin::ProvenAbsent
            }
            EffectPlan::Dispatch { expected } => DispatchOrigin::Initial(expected),
        };
        self.dispatch(execution, operation, &snapshot, origin).await
    }

    async fn reconcile(
        &self,
        _execution: JobExecution<'_>,
        operation: &OperationContext,
        snapshot: &EffectSnapshot,
    ) -> Result<Option<Action>, JobFailure> {
        debug_assert!(
            snapshot.retention_active,
            "reconciliation is planned only during retention"
        );
        self.progress(Progress::Admission(snapshot.state));
        let result = self
            .worker
            .admission
            .run(
                operation,
                Admission::Wait,
                "provider.reconcile.admitted",
                |scope| async move {
                    self.progress(Progress::Reconcile);
                    Ok::<_, JobFailure>(
                        self.worker
                            .provider
                            .reconcile(&scope, &snapshot.provider_key, &snapshot.provider_payload)
                            .await,
                    )
                },
            )
            .await;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(BulkheadError::NotAdmitted(error)) => {
                return Ok(Some(Action::Admission {
                    state: snapshot.state,
                    error,
                }));
            }
            Err(BulkheadError::Operation(OperationError::Failed(error))) => return Err(error),
            Err(BulkheadError::Operation(OperationError::Interrupted(reason))) => {
                Err(ProviderCallError::Interrupted(reason))
            }
        };
        Ok(match outcome {
            Ok(ReconcileOutcome::Accepted { provider_effect_id }) => {
                Some(Action::Confirm(provider_effect_id))
            }
            Ok(ReconcileOutcome::Absent) => None,
            Ok(ReconcileOutcome::Expired) => Some(Action::Manual {
                code: "provider_retention_expired",
                message: "Provider lookup no longer retains a definitive outcome.",
                expected: ProviderEffectState::ReconcileNeeded,
            }),
            Ok(ReconcileOutcome::Conflict) => Some(Action::Manual {
                code: "provider_payload_conflict",
                message: "Provider lookup conflicted with the retained payload.",
                expected: ProviderEffectState::ReconcileNeeded,
            }),
            outcome @ (Ok(ReconcileOutcome::Unavailable) | Err(_)) => Some(Action::Uncertain {
                code: outcome
                    .as_ref()
                    .err()
                    .map_or(CODE_RECONCILE_UNAVAILABLE, ProviderCallError::code),
                message: "Provider reconciliation did not return a definitive outcome.",
            }),
        })
    }

    async fn dispatch(
        &self,
        execution: JobExecution<'_>,
        operation: &OperationContext,
        snapshot: &EffectSnapshot,
        origin: DispatchOrigin,
    ) -> Result<Action, JobFailure> {
        let worker = self.worker;
        let payload = self.payload;
        let dispatch = worker
            .prepare_retained_dispatch(execution, payload, snapshot)
            .await?;
        self.progress(Progress::Admission(snapshot.state));
        let result = worker
            .admission
            .run(
                operation,
                Admission::Wait,
                "provider.dispatch.admitted",
                |scope| async move {
                    self.progress(Progress::StateChange);
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
                    }
                    .map_err(state_failure)?;
                    if !marked {
                        return Ok(Sent::GenerationChanged);
                    }
                    self.progress(Progress::Dispatch);
                    Ok(Sent::Outcome(dispatch.execute(&scope).await))
                },
            )
            .await;
        let outcome = match result {
            Ok(Sent::Outcome(outcome)) => outcome,
            Ok(Sent::GenerationChanged) => {
                self.progress(Progress::Preparing);
                let snapshot = worker.load_retained(execution, payload).await?;
                if snapshot.current_generation != Some(payload.record_generation) {
                    return Ok(Action::Deny {
                        code: "target_generation_changed",
                        expected: snapshot.state,
                    });
                }
                return Err(state_failure(StateError::Invariant));
            }
            Err(BulkheadError::NotAdmitted(error)) => {
                return Ok(Action::Admission {
                    state: snapshot.state,
                    error,
                });
            }
            Err(BulkheadError::Operation(OperationError::Failed(error))) => return Err(error),
            Err(BulkheadError::Operation(OperationError::Interrupted(reason))) => {
                Err(ProviderCallError::Interrupted(reason))
            }
        };
        Ok(match outcome {
            Ok(DispatchOutcome::Accepted { provider_effect_id }) => {
                Action::Confirm(provider_effect_id)
            }
            Ok(DispatchOutcome::KnownNotDispatched { retry_not_before }) => {
                Action::Undispatched(retry_not_before)
            }
            Ok(DispatchOutcome::BusinessDenied) => Action::Deny {
                code: "provider_business_denied",
                expected: ProviderEffectState::ReconcileNeeded,
            },
            Ok(DispatchOutcome::Conflict) => Action::Manual {
                code: "provider_payload_conflict",
                message: "Provider identity conflicted with the retained payload.",
                expected: ProviderEffectState::ReconcileNeeded,
            },
            outcome @ (Ok(DispatchOutcome::Indeterminate) | Err(_)) => Action::Uncertain {
                code: outcome
                    .as_ref()
                    .err()
                    .map_or(CODE_DISPATCH_INDETERMINATE, ProviderCallError::code),
                message: "Provider dispatch may have been accepted and requires reconciliation.",
            },
        })
    }
}
