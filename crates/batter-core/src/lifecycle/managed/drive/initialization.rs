use super::{Guarded, record_settlement};
use crate::{
    BoxError,
    lifecycle::managed::{BoxFuture, ManagedInitialization, ManagedOutcome, SettlementEvidence},
    lifecycle::{ShutdownSignal, state::PendingComponentStart},
    operation::{Interruption, OperationContext},
};
use tokio::sync::watch;

// The only readiness acknowledgement travels with its initializer. Completing
// initialization consumes this owner; an interrupted initializer remains owned
// until after the native stop callback.
pub(super) struct Initializing {
    future: Guarded<Result<(), BoxError>>,
    pending_start: PendingComponentStart,
}

pub(super) enum InitializationExit {
    Ready,
    Stopped(Initializing),
    Failed,
}

impl InitializationExit {
    pub(super) fn destroy_pending(self, outcome: &mut ManagedOutcome) {
        if let Self::Stopped(initializing) = self {
            initializing.future.destroy(outcome);
        }
    }
}

impl Initializing {
    pub(super) fn new(
        future: BoxFuture<Result<(), BoxError>>,
        pending_start: PendingComponentStart,
    ) -> Self {
        Self {
            future: Guarded::new(future),
            pending_start,
        }
    }

    pub(super) async fn run(
        mut self,
        context: &OperationContext,
        signal: &ShutdownSignal,
        stopping: &mut Guarded<()>,
        settlement: &mut Guarded<SettlementEvidence>,
        outcome: &mut ManagedOutcome,
        publication: &watch::Sender<ManagedOutcome>,
    ) -> InitializationExit {
        tokio::select! {
            biased;
            _ = signal.draining() => {},
            _ = context.cancelled() => {
                outcome.initialization = ManagedInitialization::Interrupted(Interruption::Cancelled);
            }
            _ = tokio::time::sleep_until(context.deadline()) => {
                outcome.initialization = ManagedInitialization::Interrupted(Interruption::DeadlineExceeded);
            }
            result = stopping => {
                outcome.unexpected_stop = !signal.is_draining();
                if let Err(error) = result { outcome.failures.push(error); }
            }
            result = settlement => {
                outcome.unexpected_stop = !signal.is_draining();
                record_settlement(outcome, result);
            }
            result = &mut self.future => {
                outcome.initialization = match result {
                    Ok(Ok(())) => ManagedInitialization::Initialized,
                    Ok(Err(error)) => ManagedInitialization::Failed(error.into()),
                    Err(error) => ManagedInitialization::Failed(error),
                };
                self.future.destroy(outcome);
                // Both the final poll and destruction can cross the startup
                // boundary. Retain a returned failure; approve only live success.
                if matches!(outcome.initialization, ManagedInitialization::Initialized)
                    && let Err(reason) = context.check()
                {
                    outcome.initialization = ManagedInitialization::Interrupted(reason);
                }
                drop(publication.send_replace(outcome.clone()));
                if !matches!(outcome.initialization, ManagedInitialization::Initialized)
                    || !outcome.failures.is_empty()
                {
                    return InitializationExit::Failed;
                }
                self.pending_start.acknowledge();
                return InitializationExit::Ready;
            }
        }
        InitializationExit::Stopped(self)
    }
}
