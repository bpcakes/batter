//! One record-exactly-once guard shared by every boundary whose result is
//! known only when it finishes or is destroyed.

use super::{AdmissionKind, End, vocabulary};
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
    fn record(&mut self, label: &'static str, finished: bool);
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
            self.series.record(label, true);
        }
    }

    /// Disarm without recording, for a boundary whose outcome is recorded
    /// elsewhere.
    fn discard(&mut self) {
        self.armed = false;
    }
}

impl<S: Series> Drop for Terminal<S> {
    fn drop(&mut self) {
        if std::mem::replace(&mut self.armed, false) {
            let label = self.series.abandoned();
            self.series.record(label, false);
        }
    }
}

struct Retry(&'static str);

impl Series for Retry {
    fn abandoned(&self) -> &'static str {
        vocabulary::retry::<()>(abandoned())
    }

    fn record(&mut self, label: &'static str, _: bool) {
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

    fn record(&mut self, label: &'static str, _: bool) {
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

struct Admitted;

impl Series for Admitted {
    // The decision was already made; destruction only fixes when it is seen.
    fn abandoned(&self) -> &'static str {
        vocabulary::process(Ok(()))
    }

    fn record(&mut self, label: &'static str, _: bool) {
        super::admission(AdmissionKind::Process, label);
    }
}

/// A process admission carried by its queued entry. The coordinator records
/// it when it takes ownership of the entry, before the task can run or exit;
/// an admitted entry destroyed unspawned still records it. An entry the queue
/// rejected is discarded, because the caller records that rejection.
pub(crate) struct AdmittedDecision(Terminal<Admitted>);

impl AdmittedDecision {
    pub(crate) fn new() -> Self {
        Self(Terminal::new(Admitted))
    }

    pub(crate) fn record(&mut self) {
        self.0.finish(vocabulary::process(Ok(())));
    }

    pub(crate) fn discard(&mut self) {
        self.0.discard();
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

    fn record(&mut self, label: &'static str, finished: bool) {
        // Duration is drain start to the final report; abandoned drivers
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

    /// Select the cause; `started` is the canonical lifecycle stop instant
    /// from which every drain budget is measured.
    pub(crate) fn draining(&mut self, cause: ShutdownCause, started: Instant) {
        self.0.series.cause = Some(cause);
        self.0.series.drain_started = Some(started);
    }

    /// Record the completed report's result now; call before `Stopped` is
    /// published so an application root cannot flush before this sample.
    pub(crate) fn finish(mut self, success: bool) {
        self.0
            .finish(vocabulary::shutdown_result(End::Finished(success)));
    }
}
