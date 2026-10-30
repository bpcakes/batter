use super::{DeliveryState, ProviderEffectState};

impl ProviderEffectState {
    pub(super) fn project(self, job: DeliveryState, attempts_spent: AttemptBudget) -> Self {
        let attempts_spent = attempts_spent == AttemptBudget::Spent;
        match self {
            Self::Confirmed | Self::BusinessDenied | Self::ManualResolution | Self::Exhausted => {
                self
            }
            Self::AwaitingAttempt | Self::RetryableUndispatched | Self::ReconcileNeeded => {
                match job {
                    DeliveryState::Pending | DeliveryState::InFlight => self,
                    DeliveryState::DeadLettered if attempts_spent => Self::Exhausted,
                    DeliveryState::DeadLettered
                    | DeliveryState::Cancelled
                    | DeliveryState::Succeeded => Self::ManualResolution,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AttemptBudget {
    Spent,
    Available,
}
