//! One record-exactly-once guard shared by every boundary whose result is
//! known only when it finishes or is destroyed.

use crate::telemetry::metrics::vocabulary::ThreadState;

use super::{AdmissionKind, End, vocabulary};
use crate::telemetry::Outcome;
use crate::{admission::AdmissionError, lifecycle::ShutdownCause, retry::RetryExecutionError};
use tokio::time::Instant;

/// How a guarded boundary without a result ended: destroyed while its thread
/// unwinds (`panicked`) or otherwise abandoned (`dropped`).
fn abandoned<T>() -> End<T> {
    if std::thread::panicking() {
        End::Panicked
    } else {
        End::Dropped
    }
}

/// The metric a guarded boundary records once.
trait Series {
    /// Label for a boundary destroyed before it finished.
    fn abandoned(&self) -> &'static str;
    /// Record `label`; `finished` distinguishes a completed boundary.
    fn record(&mut self, label: &'static str, finished: BoundaryCompletion);
}

/// Records its series exactly once: on `finish`, or with the shared
/// abandonment classification when destroyed first.
struct Terminal<S: Series> {
    series: S,
    armed: bool,
}

impl<S: Series> Terminal<S> {
    const fn new(series: S) -> Self {
        Self {
            series,
            armed: true,
        }
    }

    fn finish(&mut self, label: &'static str) {
        if std::mem::replace(&mut self.armed, false) {
            self.series.record(label, BoundaryCompletion::Finished);
        }
    }
}

impl<S: Series> Drop for Terminal<S> {
    fn drop(&mut self) {
        if std::mem::replace(&mut self.armed, false) {
            let label = self.series.abandoned();
            self.series.record(label, BoundaryCompletion::Abandoned);
        }
    }
}

struct Retry(&'static str);

impl Series for Retry {
    fn abandoned(&self) -> &'static str {
        vocabulary::retry::<()>(abandoned())
    }

    fn record(&mut self, label: &'static str, _: BoundaryCompletion) {
        super::retry_execution(self.0, label);
    }
}

/// One retry execution result, or `dropped`/`panicked`.
pub(crate) struct RetryTerminal(Terminal<Retry>);

impl RetryTerminal {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self(Terminal::new(Retry(operation)))
    }

    pub(crate) fn finish<T, E>(&mut self, result: &Result<T, RetryExecutionError<E>>) {
        self.0.finish(vocabulary::retry(End::Finished(
            result.as_ref().map(|_| ()),
        )));
    }
}

struct Bulkhead;

impl Series for Bulkhead {
    fn abandoned(&self) -> &'static str {
        vocabulary::bulkhead(abandoned())
    }

    fn record(&mut self, label: &'static str, _: BoundaryCompletion) {
        super::admission(AdmissionKind::Bulkhead, label);
    }
}

/// One bulkhead decision, or `dropped`/`panicked` for an abandoned wait.
pub(crate) struct BulkheadTerminal(Terminal<Bulkhead>);

impl BulkheadTerminal {
    pub(crate) fn new() -> Self {
        Self(Terminal::new(Bulkhead))
    }

    pub(crate) fn finish<T>(&mut self, result: &Result<T, AdmissionError>) {
        self.0.finish(vocabulary::bulkhead(End::Finished(
            result.as_ref().map(|_| ()),
        )));
    }
}

struct Attempt(&'static str);

impl Series for Attempt {
    fn abandoned(&self) -> &'static str {
        match abandoned::<()>() {
            End::Panicked => vocabulary::outcome(Outcome::Dropped, ThreadState::Unwinding),
            _ => vocabulary::outcome(Outcome::Dropped, ThreadState::Running),
        }
    }

    fn record(&mut self, label: &'static str, _: BoundaryCompletion) {
        super::retry_attempt(self.0, label);
    }
}

/// One retry attempt, armed by the retry boundary when it invokes the
/// attempt's factory; an attempt whose factory never ran has no guard.
pub(crate) struct AttemptTerminal(Terminal<Attempt>);

impl AttemptTerminal {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self(Terminal::new(Attempt(operation)))
    }

    pub(crate) fn finish(&mut self, outcome: Outcome) {
        self.0
            .finish(vocabulary::outcome(outcome, ThreadState::Running));
    }
}

struct Shutdown {
    cause: Option<ShutdownCause>,
    drain_started: Option<Instant>,
}

impl Series for Shutdown {
    fn abandoned(&self) -> &'static str {
        vocabulary::shutdown_result(abandoned())
    }

    fn record(&mut self, label: &'static str, finished: BoundaryCompletion) {
        let finished = finished == BoundaryCompletion::Finished;
        // Duration is canonical stop instant to final report; abandoned drivers
        // produce no report and therefore no duration sample.
        let elapsed = self
            .drain_started
            .filter(|_| finished)
            .map(|started| started.elapsed());
        super::shutdown(self.cause, label, elapsed);
    }
}

/// Armed from the start of a supervisor drive; records one shutdown.
pub(crate) struct ShutdownTerminal(Terminal<Shutdown>);

impl ShutdownTerminal {
    pub(crate) fn new() -> Self {
        Self(Terminal::new(Shutdown {
            cause: None,
            drain_started: None,
        }))
    }

    /// Select the cause independently of later stop-clock tightening.
    pub(crate) fn draining(&mut self, cause: ShutdownCause) {
        self.0.series.cause = Some(cause);
    }

    /// Record the completed report using the final canonical stop instant.
    pub(crate) fn finish(
        &mut self,
        report: &crate::lifecycle::ShutdownReport,
        started: impl FnOnce() -> Instant,
    ) {
        let success = report.is_success();
        self.0.series.drain_started = Some(started());
        self.0
            .finish(vocabulary::shutdown_result(End::Finished(success)));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoundaryCompletion {
    Finished,
    Abandoned,
}
