//! Stable execution outcomes on ordinary `tracing` spans.
//!
//! Batter never installs a global subscriber and never automatically records
//! application errors, request bodies, identities, URLs, or panic payloads.
//! Operation names must be developer-controlled constants, not user input.

use std::future::Future;
use tokio::time::Instant;
use tracing::Span;

/// Retain the current tracing subscriber while polling and destroying a future.
///
/// The subscriber is captured when this function is called, even if the returned
/// future is never polled. All of the owned future is destroyed under that
/// subscriber, including captured values and nested instrumented spans. Wrap
/// outside [`tracing::Instrument::instrument`] to protect span destruction too.
///
/// This does not capture or enter the current span; instrument the future when
/// it needs that parent context. It does not spawn work, allocate on the heap,
/// install a global subscriber, or extend the lifetime of a Tokio runtime.
/// Borrowed and non-`Send` futures are accepted; a `Send` future remains `Send`.
/// To capture at the first poll of an async entry point, call this inside its
/// async body rather than when constructing that entry point's future.
///
/// ```
/// use batter::telemetry::with_current_dispatch;
/// use tracing::Instrument;
///
/// # async fn example() {
/// let mut value = String::from("borrowed");
/// let result = with_current_dispatch(
///     async {
///         value.push_str(" value");
///         tracing::info!("adapter work finished");
///         value.as_str()
///     }
///     .instrument(tracing::info_span!("adapter.work")),
/// )
/// .await;
/// assert_eq!(result, "borrowed value");
/// # }
/// ```
pub fn with_current_dispatch<F: Future>(future: F) -> impl Future<Output = F::Output> {
    crate::scoped_dispatch::scope(future)
}

/// Boundary outcome, independent of application error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The operation returned success.
    Succeeded,
    /// The operation returned an application error.
    Failed,
    /// Cooperative cancellation won the boundary race.
    Cancelled,
    /// The deadline won the boundary race.
    DeadlineExceeded,
    /// The enclosing future was dropped before producing a result.
    Dropped,
}

impl Outcome {
    /// Stable, low-cardinality spelling for telemetry adapters.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Dropped => "dropped",
        }
    }
}

pub(crate) struct Observation {
    span: Span,
    context: Span,
    started: Instant,
    outcome: Outcome,
}

impl Observation {
    pub(crate) fn new(operation: &'static str) -> Self {
        let span = tracing::info_span!(
            target: "batter",
            "batter.operation",
            operation,
            outcome = tracing::field::Empty,
            elapsed_ms = tracing::field::Empty,
        );
        // The diagnostic span may be filtered while its application parent is
        // enabled. Capture once; a later poll/drop must not adopt another parent.
        let context = span.clone().or_current();
        Self {
            span,
            context,
            started: Instant::now(),
            outcome: Outcome::Dropped,
        }
    }

    pub(crate) fn context(&self) -> Span {
        self.context.clone()
    }

    pub(crate) fn finish(&mut self, outcome: Outcome) {
        self.outcome = outcome;
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        let elapsed_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        self.span.record("outcome", self.outcome.as_str());
        self.span.record("elapsed_ms", elapsed_ms);
        if matches!(self.outcome, Outcome::Succeeded | Outcome::Cancelled) {
            tracing::info!(
                target: "batter",
                parent: &self.context,
                outcome = self.outcome.as_str(),
                elapsed_ms,
                "operation boundary finished"
            );
        } else {
            tracing::warn!(
                target: "batter",
                parent: &self.context,
                outcome = self.outcome.as_str(),
                elapsed_ms,
                "operation boundary finished"
            );
        }
    }
}
