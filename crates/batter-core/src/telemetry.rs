//! Stable execution outcomes on ordinary `tracing` spans.
//!
//! Batter never installs a global subscriber and never automatically records
//! application errors, request bodies, identities, URLs, or panic payloads.
//! Operation names must be developer-controlled constants, not user input.

use std::future::Future;
use tokio::time::Instant;
use tracing::Span;

/// Define a closed label vocabulary once: an exhaustive `const fn` mapping and
/// its published value list are generated from the same arms, so a new enum
/// variant cannot compile without a label that is also in the list.
macro_rules! closed_domain {
    (
        $(#[$attr:meta])*
        $vis:vis $values:ident, fn $label:ident $(<$generic:ident>)? ($value:ident: $input:ty) {
            $($pattern:pat => $text:literal,)+
        }
    ) => {
        $(#[$attr])*
        $vis const $values: &[&str] = &[$($text),+];

        pub(crate) const fn $label $(<$generic>)? ($value: $input) -> &'static str {
            match $value {
                $($pattern => $text,)+
            }
        }
    };
}

#[cfg(feature = "metrics")]
pub mod metrics;
pub(crate) mod record;

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
/// use batter_core::telemetry::with_current_dispatch;
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

closed_domain! {
    #[cfg_attr(not(feature = "metrics"), allow(dead_code))]
    pub(crate) OUTCOME_LABELS, fn outcome_label(value: Outcome) {
        Outcome::Succeeded => "succeeded",
        Outcome::Failed => "failed",
        Outcome::Cancelled => "cancelled",
        Outcome::DeadlineExceeded => "deadline_exceeded",
        Outcome::Dropped => "dropped",
    }
}

impl Outcome {
    /// Stable, low-cardinality spelling for telemetry adapters.
    pub const fn as_str(self) -> &'static str {
        outcome_label(self)
    }
}

/// Which metric family, if any, a finished operation boundary belongs to.
#[derive(Clone, Copy)]
pub(crate) enum Boundary {
    /// An application operation.
    Operation,
    /// One retry attempt, counted only once its factory has been invoked; the
    /// retry execution records its own terminal result.
    RetryAttempt,
    /// A foundation-owned wait (admission, backoff) whose owning boundary
    /// records the decision; it is traced but emits no operation metrics.
    Internal,
}

pub(crate) struct Observation {
    #[cfg(feature = "metrics")]
    operation: &'static str,
    #[cfg(feature = "metrics")]
    boundary: Boundary,
    #[cfg(feature = "metrics")]
    factory_invoked: std::sync::atomic::AtomicBool,
    span: Span,
    context: Span,
    started: Instant,
    outcome: Outcome,
}

impl Observation {
    #[cfg_attr(not(feature = "metrics"), allow(unused_variables))]
    pub(crate) fn new(operation: &'static str, boundary: Boundary) -> Self {
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
            #[cfg(feature = "metrics")]
            operation,
            #[cfg(feature = "metrics")]
            boundary,
            #[cfg(feature = "metrics")]
            factory_invoked: std::sync::atomic::AtomicBool::new(false),
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

    /// Record that the boundary is about to invoke its factory. Call this in
    /// the branch that invokes it, not from a preflight check.
    pub(crate) fn mark_factory_invoked(&self) {
        // Shared by reference into a `Send` future, so this must be `Sync`.
        #[cfg(feature = "metrics")]
        self.factory_invoked
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        #[cfg(feature = "metrics")]
        metrics::operation(
            self.boundary,
            self.factory_invoked
                .load(std::sync::atomic::Ordering::Relaxed),
            self.operation,
            self.outcome,
            elapsed,
        );
        let elapsed_ms = elapsed.as_secs_f64() * 1_000.0;
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
