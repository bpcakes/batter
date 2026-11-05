//! Private ownership of an optional HTTP observation through one native check.

use runlimit_core::{BatchDecision, ConsumptionStatus};

#[cfg(feature = "axum")]
use batter_axum::quota_observation::{
    QuotaConsumption, QuotaRecorder, QuotaTerminalFacts, StartedQuotaRecorder,
};
#[cfg(feature = "axum")]
use runlimit_core::{BatchDecisionView, Denial};

// Variants select whether to record, not phases. Only start can transfer the
// writer into StartedObservation; finish consumes that writer exactly once.
pub(crate) enum CheckObservation {
    Unrecorded,
    #[cfg(feature = "axum")]
    Http(QuotaRecorder),
}

pub(crate) enum StartedObservation {
    Unrecorded,
    #[cfg(feature = "axum")]
    Http(StartedQuotaRecorder),
}

impl CheckObservation {
    pub(crate) fn start(self) -> StartedObservation {
        match self {
            Self::Unrecorded => StartedObservation::Unrecorded,
            #[cfg(feature = "axum")]
            Self::Http(record) => StartedObservation::Http(record.start()),
        }
    }
}

impl StartedObservation {
    // Publish only a native terminal result. Dropping the started owner before
    // this point leaves the shared HTTP facts unresolved, never not-consumed.
    pub(crate) fn finish(self, result: Result<&BatchDecision, ConsumptionStatus>) {
        match (self, result) {
            (Self::Unrecorded, _) => {}
            #[cfg(feature = "axum")]
            (Self::Http(record), result) => record.finish(project_terminal(result)),
        }
    }
}

#[cfg(feature = "axum")]
fn project_terminal(result: Result<&BatchDecision, ConsumptionStatus>) -> QuotaTerminalFacts {
    match result {
        Err(consumption) => QuotaTerminalFacts::BackendFailed(match consumption {
            ConsumptionStatus::Consumed => QuotaConsumption::Consumed,
            ConsumptionStatus::NotConsumed => QuotaConsumption::NotConsumed,
            ConsumptionStatus::PossiblyConsumed => QuotaConsumption::Unknown,
        }),
        Ok(decision) => match decision.view() {
            BatchDecisionView::Allowed { .. } => QuotaTerminalFacts::Allowed,
            BatchDecisionView::ShadowDenied { .. } => QuotaTerminalFacts::ShadowDenied,
            BatchDecisionView::Denied { denial, .. } => match denial {
                Denial::QuotaExceeded(_) => QuotaTerminalFacts::QuotaDenied,
                Denial::StorageCapacity { .. } => QuotaTerminalFacts::StorageCapacity,
            },
        },
    }
}
