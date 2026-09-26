//! Feature-independent recording hooks for foundation boundaries.
//!
//! With the `metrics` feature these forward to `telemetry::metrics`. Without
//! it every hook is an inlined no-op and every guard is zero-sized, so call
//! sites carry no configuration attributes and cannot drift between builds.

use crate::cleanup::CleanupOutcome;

/// How a registered cleanup hook ended.
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) enum CleanupHook {
    Observed(CleanupOutcome),
    Skipped,
    Dropped,
}

#[cfg(feature = "metrics")]
pub(crate) use super::metrics::{
    AdmittedDecision, BulkheadTerminal, RetryTerminal, ShutdownTerminal, cleanup, process_rejected,
    root, task,
};

#[cfg(not(feature = "metrics"))]
pub(crate) use disabled::*;

#[cfg(not(feature = "metrics"))]
mod disabled {
    use super::CleanupHook;
    use crate::{
        admission::AdmissionError,
        lifecycle::{ProcessAdmissionError, Readiness, ShutdownCause, TaskOutcome},
        retry::RetryExecutionError,
    };
    use tokio::time::Instant;

    #[inline]
    pub(crate) fn cleanup(_: CleanupHook, _: usize) {}

    #[inline]
    pub(crate) fn process_rejected(_: &ProcessAdmissionError) {}

    #[inline]
    pub(crate) fn root(_: Readiness) {}

    #[inline]
    pub(crate) fn task(_: bool, _: &'static str, _: TaskOutcome) {}

    pub(crate) struct AdmittedDecision;

    impl AdmittedDecision {
        #[inline]
        pub(crate) fn new() -> Self {
            Self
        }

        #[inline]
        pub(crate) fn record(&mut self) {}

        #[inline]
        pub(crate) fn discard(&mut self) {}
    }

    pub(crate) struct BulkheadTerminal;

    impl BulkheadTerminal {
        #[inline]
        pub(crate) fn new() -> Self {
            Self
        }

        #[inline]
        pub(crate) fn finish<T>(&mut self, _: &Result<T, AdmissionError>) {}
    }

    pub(crate) struct RetryTerminal;

    impl RetryTerminal {
        #[inline]
        pub(crate) fn new(_: &'static str) -> Self {
            Self
        }

        #[inline]
        pub(crate) fn finish<T, E>(&mut self, _: &Result<T, RetryExecutionError<E>>) {}
    }

    pub(crate) struct ShutdownTerminal;

    impl ShutdownTerminal {
        #[inline]
        pub(crate) fn new() -> Self {
            Self
        }

        #[inline]
        pub(crate) fn draining(&mut self, _: ShutdownCause, _: Instant) {}

        #[inline]
        pub(crate) fn finish(self, _: bool) {}
    }
}
