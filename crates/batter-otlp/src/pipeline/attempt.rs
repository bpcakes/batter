//! Private linear authority and observations for one SDK export invocation.

use crate::diagnostics::{ExportFailure, ExportOutcome};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::time::{Instant, timeout_at};

/// No phase/outcome cross-product, and no writable evidence outside this module.
enum State {
    Encoding,
    Preparing,
    Sending,
    Receiving,
    Completed(ExportOutcome),
    Closed,
}

enum Timing {
    Returned,
    Expired,
}

struct Attempt {
    deadline: Instant,
    state: Mutex<State>,
}

impl Attempt {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
enum Admission {
    #[default]
    Closed,
    Armed(Arc<Attempt>),
}

/// Shared only with the SDK client. Claiming removes the one dispatch permission.
#[derive(Default)]
pub(super) struct Gate(Mutex<Admission>);

impl Gate {
    fn lock(&self) -> MutexGuard<'_, Admission> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn claim(&self) -> Option<Preparing> {
        let Admission::Armed(attempt) = std::mem::take(&mut *self.lock()) else {
            return None;
        };
        {
            let mut state = attempt.lock();
            if !matches!(*state, State::Encoding) {
                return None;
            }
            *state = State::Preparing;
        }
        Some(Preparing(attempt))
    }
}

/// Only this exclusive owner can arm the gate; callers cannot pair begin/finish.
#[derive(Default)]
pub(super) struct Owner {
    gate: Arc<Gate>,
}

impl Owner {
    pub(super) fn gate(&self) -> Arc<Gate> {
        self.gate.clone()
    }

    pub(super) async fn run<F: Future>(
        &mut self,
        deadline: Instant,
        factory: impl FnOnce() -> F,
    ) -> ExportOutcome {
        // An unpolled run installs no authority and never invokes the factory.
        let active = Active {
            gate: &self.gate,
            attempt: Arc::new(Attempt {
                deadline,
                state: Mutex::new(State::Encoding),
            }),
        };
        *active.gate.lock() = Admission::Armed(active.attempt.clone());
        // Drop the complete SDK future before resolving or releasing this attempt.
        let timing = match timeout_at(deadline, factory()).await {
            Ok(_) => Timing::Returned,
            Err(_) => Timing::Expired,
        };
        active.resolve(timing)
    }
}

/// Synchronous revocation only; neither async cleanup nor remote rollback.
struct Active<'a> {
    gate: &'a Gate,
    attempt: Arc<Attempt>,
}

impl Active<'_> {
    fn resolve(self, timing: Timing) -> ExportOutcome {
        match (&*self.attempt.lock(), timing) {
            (State::Completed(outcome), _) => *outcome,
            (State::Receiving, Timing::Expired) => {
                ExportOutcome::Failed(ExportFailure::BodyTimedOut)
            }
            (State::Encoding | State::Preparing | State::Sending, Timing::Expired) => {
                ExportOutcome::Failed(ExportFailure::HeadersTimedOut)
            }
            // The SDK returned without a complete bounded-client observation.
            _ => ExportOutcome::Failed(ExportFailure::Encoding),
        }
    }
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        *self.gate.lock() = Admission::Closed;
        *self.attempt.lock() = State::Closed;
    }
}

// No Clone, public constructors, generic phase setter, or generic success writer.
// Each operation consumes its phase capability; stale handles fail closed.
pub(super) struct Preparing(Arc<Attempt>);
pub(super) struct Sending(Arc<Attempt>);
pub(super) struct Receiving(Arc<Attempt>);

impl Preparing {
    pub(super) fn send(self) -> Option<Sending> {
        {
            let mut state = self.0.lock();
            if !matches!(*state, State::Preparing) {
                return None;
            }
            if Instant::now() >= self.0.deadline {
                *state = State::Completed(ExportOutcome::Failed(ExportFailure::HeadersTimedOut));
                return None;
            }
            *state = State::Sending;
        }
        Some(Sending(self.0))
    }

    pub(super) fn fail(self, failure: ExportFailure) {
        let mut state = self.0.lock();
        if matches!(*state, State::Preparing) {
            *state = State::Completed(ExportOutcome::Failed(failure));
        }
    }
}

impl Sending {
    pub(super) fn deadline(&self) -> Instant {
        self.0.deadline
    }

    pub(super) fn receive(self) -> Option<Receiving> {
        {
            let mut state = self.0.lock();
            if !matches!(*state, State::Sending) {
                return None;
            }
            *state = State::Receiving;
        }
        Some(Receiving(self.0))
    }

    pub(super) fn fail(self, failure: ExportFailure) {
        let mut state = self.0.lock();
        if matches!(*state, State::Sending) {
            *state = State::Completed(ExportOutcome::Failed(failure));
        }
    }
}

impl Receiving {
    /// Only the bounded response validator can supply acknowledgement evidence.
    pub(super) fn complete(self, result: Result<super::Acknowledged, ExportFailure>) -> bool {
        let mut state = self.0.lock();
        if !matches!(*state, State::Receiving) {
            return false;
        }
        *state = State::Completed(match result {
            Ok(_) => ExportOutcome::Acknowledged,
            Err(failure) => ExportOutcome::Failed(failure),
        });
        result.is_ok()
    }
}

#[cfg(test)]
#[path = "tests/attempt.rs"]
mod tests;
