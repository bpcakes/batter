//! Application result retention around the library-owned shutdown and cleanup.

use super::Outcome;
use batter::lifecycle::{RunningSupervisor, ShutdownFailure};
use std::{fmt, future::Future};

/// Retain both independent failures without printing either error's contents.
pub(super) struct WorkAndShutdownFailure {
    pub(super) work: Box<dyn std::error::Error + Send + Sync>,
    pub(super) shutdown: ShutdownFailure,
}

impl fmt::Display for WorkAndShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("consumer work and shutdown both failed")
    }
}

impl fmt::Debug for WorkAndShutdownFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ShutdownFailure's own Debug is redacted; never format the work error.
        f.debug_struct("WorkAndShutdownFailure")
            .field("shutdown", &self.shutdown)
            .finish_non_exhaustive()
    }
}

impl std::error::Error for WorkAndShutdownFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.work.as_ref())
    }
}

pub(super) async fn complete<T>(
    running: RunningSupervisor,
    work: impl Future<Output = Outcome<T>>,
) -> Outcome<T> {
    let work = work.await;
    let shutdown = running.shutdown_checked().await;
    match (work, shutdown) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(Box::new(error)),
        (Err(work), Err(shutdown)) => Err(Box::new(WorkAndShutdownFailure { work, shutdown })),
    }
}

#[cfg(test)]
#[path = "single_facade_completion_tests.rs"]
mod tests;
