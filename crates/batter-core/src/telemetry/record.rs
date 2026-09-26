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
    /// Taken by a close driver that was destroyed before observing it; the
    /// hook may have started.
    Abandoned,
    /// Still registered when its stack was destroyed; the hook never ran.
    Dropped,
}

#[cfg(feature = "metrics")]
pub(crate) use super::metrics::{
    AttemptTerminal, BulkheadTerminal, RetryTerminal, ShutdownTerminal, cleanup, process_admitted,
    process_rejected, root, task,
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
        telemetry::Outcome,
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

    #[inline]
    pub(crate) fn process_admitted() {}

    pub(crate) struct AttemptTerminal;

    impl AttemptTerminal {
        #[inline]
        pub(crate) fn new(_: &'static str) -> Self {
            Self
        }

        #[inline]
        pub(crate) fn finish(&mut self, _: Outcome) {}
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
        pub(crate) fn draining(&mut self, _: ShutdownCause) {}

        #[inline]
        pub(crate) fn finish(&mut self, _: bool, _: Instant) {}
    }
}
