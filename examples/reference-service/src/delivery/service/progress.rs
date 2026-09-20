use super::SubmitAttemptError;
use crate::delivery::{
    CommandFailure, StorageError, SubmitError, SubmitResult, UncertainSubmission,
};
use batter::operation::{OperationError, OperationError::Interrupted};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub(super) struct SubmissionProgress {
    phase: Arc<Mutex<SubmissionPhase>>,
}

enum SubmissionPhase {
    BeforeTransaction,
    TransactionActive,
    Settled(SubmissionOutcome),
}

enum SubmissionOutcome {
    Committed(SubmitResult),
    RolledBack(CommandFailure),
}

pub(super) struct ActiveSubmission {
    progress: SubmissionProgress,
}

pub(super) struct SettledSubmission(());

impl SubmissionProgress {
    pub(super) fn new() -> Self {
        Self {
            phase: Arc::new(Mutex::new(SubmissionPhase::BeforeTransaction)),
        }
    }

    pub(super) fn transaction_began(&self) -> ActiveSubmission {
        let mut phase = self.lock();
        debug_assert!(matches!(*phase, SubmissionPhase::BeforeTransaction));
        *phase = SubmissionPhase::TransactionActive;
        drop(phase);
        ActiveSubmission {
            progress: self.clone(),
        }
    }

    pub(super) fn resolve(
        self,
        operation: Result<SettledSubmission, OperationError<SubmitAttemptError>>,
    ) -> Result<SubmitResult, OperationError<SubmitError>> {
        let operation = match operation {
            Err(OperationError::Failed(error)) => {
                return Err(OperationError::Failed(error.into_submit_error()));
            }
            operation => operation,
        };

        let phase = Arc::try_unwrap(self.phase)
            .map_err(|_| OperationError::Failed(submission_state_error()))?
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match (operation, phase) {
            (Ok(SettledSubmission(())), SubmissionPhase::Settled(outcome))
            | (Err(Interrupted(_)), SubmissionPhase::Settled(outcome)) => {
                outcome.into_result().map_err(OperationError::Failed)
            }
            (Err(Interrupted(interruption)), SubmissionPhase::BeforeTransaction) => {
                Err(OperationError::Interrupted(interruption))
            }
            (Err(Interrupted(interruption)), SubmissionPhase::TransactionActive) => {
                Err(OperationError::Failed(SubmitError::Uncertain(
                    UncertainSubmission::Interrupted(interruption),
                )))
            }
            (Ok(SettledSubmission(())), _) => Err(OperationError::Failed(submission_state_error())),
            (Err(OperationError::Failed(_)), _) => unreachable!("handled before phase recovery"),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SubmissionPhase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ActiveSubmission {
    pub(super) fn committed(self, result: SubmitResult) -> SettledSubmission {
        self.settle(SubmissionOutcome::Committed(result))
    }

    pub(super) fn rolled_back(self, failure: CommandFailure) -> SettledSubmission {
        self.settle(SubmissionOutcome::RolledBack(failure))
    }

    fn settle(self, outcome: SubmissionOutcome) -> SettledSubmission {
        *self.progress.lock() = SubmissionPhase::Settled(outcome);
        SettledSubmission(())
    }
}

impl SubmissionOutcome {
    fn into_result(self) -> Result<SubmitResult, SubmitError> {
        match self {
            Self::Committed(result) => Ok(result),
            Self::RolledBack(CommandFailure::Rejected(error)) => Err(SubmitError::Rejected(error)),
            Self::RolledBack(CommandFailure::Storage(error)) => Err(SubmitError::Storage(error)),
        }
    }
}

fn submission_state_error() -> SubmitError {
    SubmitError::Storage(StorageError::Invariant(
        "submission disposition state was not retained",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        Delivery, DeliveryState, ProviderEffectState, ProviderOutcome, SubmitDisposition,
        SubmitRejection,
    };
    use batter::operation::Interruption;
    use serde_json::json;
    use uuid::Uuid;

    fn committed_result() -> SubmitResult {
        SubmitResult {
            disposition: SubmitDisposition::Accepted,
            delivery: Delivery {
                delivery_id: Uuid::now_v7(),
                record_id: Uuid::now_v7(),
                generation: 1,
                payload: json!({"message": "retained"}),
                state: DeliveryState::Pending,
                provider: ProviderOutcome {
                    state: ProviderEffectState::AwaitingAttempt,
                    provider_effect_id: None,
                    acceptance_possible: false,
                },
            },
        }
    }

    #[test]
    fn acknowledged_commit_survives_cleanup_interruption() {
        let progress = SubmissionProgress::new();
        let active = progress.transaction_began();
        let expected = committed_result();
        let _settled = active.committed(expected.clone());

        let result = progress.resolve(Err(Interrupted(Interruption::DeadlineExceeded)));

        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn acknowledged_rollback_survives_cleanup_interruption() {
        let progress = SubmissionProgress::new();
        let active = progress.transaction_began();
        let _settled =
            active.rolled_back(CommandFailure::Rejected(SubmitRejection::RecordNotObserved));

        let result = progress.resolve(Err(Interrupted(Interruption::Cancelled)));

        assert!(matches!(
            result,
            Err(OperationError::Failed(SubmitError::Rejected(
                SubmitRejection::RecordNotObserved
            )))
        ));
    }

    #[test]
    fn active_transaction_interruption_remains_uncertain() {
        let progress = SubmissionProgress::new();
        let active = progress.transaction_began();
        drop(active);

        let result = progress.resolve(Err(Interrupted(Interruption::Cancelled)));

        assert!(matches!(
            result,
            Err(OperationError::Failed(SubmitError::Uncertain(
                UncertainSubmission::Interrupted(Interruption::Cancelled)
            )))
        ));
    }

    #[test]
    fn pre_transaction_interruption_remains_ordinary() {
        let progress = SubmissionProgress::new();

        let result = progress.resolve(Err(Interrupted(Interruption::DeadlineExceeded)));

        assert!(matches!(
            result,
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
        ));
    }
}
