//! Stable execution outcomes on ordinary `tracing` spans.
//!
//! Batter never installs a global subscriber and never automatically records
//! application errors, request bodies, identities, URLs, or panic payloads.
//! Operation names must be developer-controlled constants, not user input.

use tokio::time::Instant;
use tracing::Span;

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
    started: Instant,
    outcome: Outcome,
}

impl Observation {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self {
            span: tracing::info_span!(
                target: "batter",
                "batter.operation",
                operation,
                outcome = tracing::field::Empty,
                elapsed_ms = tracing::field::Empty,
            ),
            started: Instant::now(),
            outcome: Outcome::Dropped,
        }
    }

    pub(crate) fn span(&self) -> Span {
        self.span.clone()
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
                parent: &self.span,
                outcome = self.outcome.as_str(),
                elapsed_ms,
                "operation boundary finished"
            );
        } else {
            tracing::warn!(
                target: "batter",
                parent: &self.span,
                outcome = self.outcome.as_str(),
                elapsed_ms,
                "operation boundary finished"
            );
        }
    }
}
