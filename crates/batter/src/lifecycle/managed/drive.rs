use super::{
    BoxFuture, ManagedComponent, ManagedFailure, ManagedInitialization, ManagedOutcome,
    ManagedShutdownBudget, Registration, SettlementEvidence,
};
use crate::{
    lifecycle::{ShutdownBudget, ShutdownSignal},
    operation::Interruption,
    scoped_dispatch,
    startup::PanicPayload,
};
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::watch;
use tracing::Instrument;

#[cfg(test)]
mod tests;

pub(super) fn start(
    registration: Registration,
    budget: ShutdownBudget,
    signal: ShutdownSignal,
    publication: watch::Sender<ManagedOutcome>,
) {
    let updates = publication.clone();
    let coordinator = tokio::spawn(scoped_dispatch::scope(
        drive(registration, budget, signal, updates).in_current_span(),
    ));
    // Neither the wrapper nor a report observer owns this join. Wrapper abortion
    // cannot destroy the native report's only driver or completion publication.
    drop(tokio::spawn(scoped_dispatch::scope(
        async move {
            let result = coordinator.await;
            publication.send_modify(|outcome| {
                if let Err(error) = result {
                    outcome
                        .failures
                        .push(ManagedFailure::Coordinator(Arc::new(error)));
                }
                outcome.finished = true;
            });
        }
        .in_current_span(),
    )));
}

async fn drive(
    registration: Registration,
    budget: ShutdownBudget,
    signal: ShutdownSignal,
    publication: watch::Sender<ManagedOutcome>,
) {
    let Some((component, context)) = construct(registration, budget, &signal, &publication) else {
        return;
    };
    let ManagedComponent {
        initialized,
        stopping,
        mut stop,
        settlement,
    } = component;
    let mut outcome = publication.borrow().clone();
    let mut initialized = Some(Guarded::new(initialized));
    let mut stopping = Guarded::new(stopping);
    let mut settlement = Guarded::new(settlement);
    let mut settled = false;
    loop {
        tokio::select! {
            biased;
            _ = signal.draining() => break,
            _ = context.cancelled(), if initialized.is_some() => {
                outcome.initialization = ManagedInitialization::Interrupted(Interruption::Cancelled);
                break;
            }
            _ = tokio::time::sleep_until(context.deadline()), if initialized.is_some() => {
                outcome.initialization = ManagedInitialization::Interrupted(Interruption::DeadlineExceeded);
                break;
            }
            result = &mut stopping => {
                outcome.unexpected_stop = !signal.is_draining();
                if let Err(error) = result { outcome.failures.push(error); }
                break;
            }
            result = &mut settlement => {
                outcome.unexpected_stop = !signal.is_draining();
                record_settlement(&mut outcome, result);
                settled = true;
                break;
            }
            result = async { initialized.as_mut().expect("guarded initialization branch").await }, if initialized.is_some() => {
                outcome.initialization = match result {
                    Ok(Ok(())) => ManagedInitialization::Initialized,
                    Ok(Err(error)) => ManagedInitialization::Failed(error.into()),
                    Err(error) => ManagedInitialization::Failed(error),
                };
                initialized.take().expect("initialization owned").destroy(&mut outcome);
                // The final poll and destruction can cross the startup boundary.
                // Preserve a returned failure; acknowledge success only while live.
                if matches!(outcome.initialization, ManagedInitialization::Initialized)
                    && let Err(reason) = context.check()
                {
                    outcome.initialization = ManagedInitialization::Interrupted(reason);
                }
                drop(publication.send_replace(outcome.clone()));
                if !matches!(outcome.initialization, ManagedInitialization::Initialized) || !outcome.failures.is_empty() {
                    break;
                }
                signal.mark_started();
            }
        }
    }
    signal.handle.request();
    let stop_started = signal
        .handle
        .shared
        .stop_started()
        .expect("drain records its clock");
    let acknowledged = propagate_stop(
        &mut *stop,
        stop_started,
        &signal,
        &mut outcome,
        &publication,
    );
    record_initialization_stop(&mut outcome);
    if let Some(initialized) = initialized.take() {
        initialized.destroy(&mut outcome);
    }
    stopping.destroy(&mut outcome);
    drop(publication.send_replace(outcome.clone()));
    if !settled {
        settle_with_clock(
            &mut settlement,
            &mut *stop,
            acknowledged,
            &signal,
            &mut outcome,
            &publication,
        )
        .await;
    }
    settlement.destroy(&mut outcome);
    drop(publication.send_replace(outcome));
}

fn propagate_stop(
    stop: &mut (dyn FnMut(tokio::time::Instant) -> tokio::time::Instant + Send),
    started: tokio::time::Instant,
    signal: &ShutdownSignal,
    outcome: &mut ManagedOutcome,
    publication: &watch::Sender<ManagedOutcome>,
) -> Option<tokio::time::Instant> {
    match catch_unwind(AssertUnwindSafe(|| stop(started))) {
        Ok(native_started) => {
            signal.handle.shared.request_since(native_started);
            Some(native_started.min(started))
        }
        Err(payload) => {
            outcome.failures.push(panic_failure(payload));
            // Publish at the catch boundary: native settlement may never finish.
            drop(publication.send_replace(outcome.clone()));
            None
        }
    }
}

async fn settle_with_clock(
    settlement: &mut Guarded<SettlementEvidence>,
    stop: &mut (dyn FnMut(tokio::time::Instant) -> tokio::time::Instant + Send),
    mut acknowledged: Option<tokio::time::Instant>,
    signal: &ShutdownSignal,
    outcome: &mut ManagedOutcome,
    publication: &watch::Sender<ManagedOutcome>,
) {
    loop {
        tokio::select! {
            biased;
            result = &mut *settlement => { record_settlement(outcome, result); return; }
            earlier = signal.handle.shared.stop_before(acknowledged.unwrap_or_else(tokio::time::Instant::now)), if acknowledged.is_some() => {
                acknowledged = propagate_stop(stop, earlier, signal, outcome, publication);
            }
        }
    }
}

fn construct(
    registration: Registration,
    budget: ShutdownBudget,
    signal: &ShutdownSignal,
    publication: &watch::Sender<ManagedOutcome>,
) -> Option<(
    ManagedComponent<SettlementEvidence>,
    crate::operation::OperationContext,
)> {
    let Registration {
        context, factory, ..
    } = registration;
    let mut outcome = ManagedOutcome::default();
    let interruption = context.check().err();
    if signal.is_draining() || interruption.is_some() {
        outcome.initialization = ManagedInitialization::NotStarted { interruption };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(factory))) {
            outcome.failures.push(panic_failure(payload));
        }
        drop(publication.send_replace(outcome));
        return None;
    }
    outcome.initialization = ManagedInitialization::Pending;
    drop(publication.send_replace(outcome.clone()));
    let constructed = catch_unwind(AssertUnwindSafe(|| {
        factory(ManagedShutdownBudget {
            graceful: budget.drain,
            abort: budget.cancel,
        })
    }));
    outcome.initialization = match constructed {
        Ok(Ok(component)) => return Some((component, context)),
        Ok(Err(error)) => ManagedInitialization::Rejected(Arc::from(error)),
        Err(payload) => ManagedInitialization::Failed(panic_failure(payload)),
    };
    signal.handle.request();
    drop(publication.send_replace(outcome));
    None
}

fn record_initialization_stop(outcome: &mut ManagedOutcome) {
    if matches!(outcome.initialization, ManagedInitialization::Pending) {
        outcome.initialization = if outcome.unexpected_stop {
            ManagedInitialization::Stopped
        } else {
            ManagedInitialization::Draining
        };
    }
}

fn record_settlement(
    outcome: &mut ManagedOutcome,
    result: Result<SettlementEvidence, ManagedFailure>,
) {
    match result {
        Ok(report) => {
            if let Some(failure) = &report.classification_failure {
                outcome.failures.push(failure.clone());
            }
            outcome.settlement = Some(report);
        }
        Err(error) => outcome.failures.push(error),
    }
}

fn panic_failure(payload: Box<dyn std::any::Any + Send>) -> ManagedFailure {
    ManagedFailure::Panicked(Arc::new(PanicPayload::new(payload)))
}

// Keep the future alive after Ready/Panicked so its destruction is separately
// caught and cannot replace a returned failure or drop the native driver.
struct Guarded<T> {
    future: BoxFuture<T>,
}

impl<T> Guarded<T> {
    fn new(future: BoxFuture<T>) -> Self {
        Self { future }
    }

    fn destroy(self, outcome: &mut ManagedOutcome) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(self))) {
            outcome.failures.push(panic_failure(payload));
        }
    }
}

impl<T> Future for Guarded<T> {
    type Output = Result<T, ManagedFailure>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match catch_unwind(AssertUnwindSafe(|| self.future.as_mut().poll(cx))) {
            Ok(poll) => poll.map(Ok),
            Err(payload) => Poll::Ready(Err(panic_failure(payload))),
        }
    }
}
