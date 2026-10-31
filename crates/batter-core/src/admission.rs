//! Process-local concurrency admission, distinct from Runlimit quota checks,
//! and bounded blocking work that holds its permit until the closure returns.

mod blocking;

pub use blocking::BlockingError;

use crate::{
    ConfigurationError,
    operation::{Interruption, OperationContext, OperationError},
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

/// Admission policy. Waiting consumes the caller's existing total budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Reject immediately when capacity is exhausted.
    Reject,
    /// Wait in Tokio's semaphore queue until admitted or interrupted.
    Wait,
}

/// The bounded-work admission outcome.
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    /// No permit is immediately available.
    #[error("concurrency capacity exhausted")]
    Overloaded,
    /// This bulkhead no longer accepts work.
    #[error("concurrency admission is closed")]
    Closed,
    /// Waiting was cancelled or exceeded its total deadline.
    #[error(transparent)]
    Interrupted(#[from] Interruption),
}

/// A shared process-local concurrency bound.
///
/// It does not bound the number of waiting callers or request body memory.
/// Use Reject, an upstream bounded queue, or transport limits for that need.
#[derive(Clone, Debug)]
pub struct Bulkhead {
    semaphore: Arc<Semaphore>,
}

/// A validated, nonzero process-local concurrency capacity.
///
/// This value retains the result of checking Tokio's semaphore limit, so it can
/// be handed to [`Bulkhead::new`] without another fallible configuration step.
///
/// ```
/// use batter_core::admission::{Bulkhead, BulkheadCapacity};
///
/// let capacity = BulkheadCapacity::new(32)?;
/// let bulkhead = Bulkhead::new(capacity);
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BulkheadCapacity(usize);

impl BulkheadCapacity {
    /// Validate a nonzero capacity within Tokio's supported semaphore range.
    pub fn new(capacity: usize) -> Result<Self, ConfigurationError> {
        if capacity == 0 {
            return Err(ConfigurationError::Zero("bulkhead capacity"));
        }
        if capacity > Semaphore::MAX_PERMITS {
            return Err(ConfigurationError::TooLarge("bulkhead capacity"));
        }
        Ok(Self(capacity))
    }

    /// Return the validated permit count for diagnostics or native handoff.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Bulkhead {
    /// Create a bulkhead from an already validated capacity.
    ///
    /// ```compile_fail,E0308
    /// use batter_core::admission::Bulkhead;
    ///
    /// fn cannot_build_from_raw(capacity: usize) {
    ///     let bulkhead = Bulkhead::new(capacity);
    /// }
    /// ```
    pub fn new(capacity: BulkheadCapacity) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(capacity.0)),
        }
    }

    /// Reject new admissions and wake waiting callers. Existing permits remain.
    pub fn close(&self) {
        self.semaphore.close();
    }

    /// Obtain a native Tokio permit. Hold it for the work's full lifetime.
    ///
    /// The permit is released whenever it is dropped, including when the future
    /// holding it is cancelled or dropped while work it handed to another thread
    /// keeps running. Use it for async work, or move it into that work's owner,
    /// such as a finite process task. For synchronous CPU work or blocking I/O
    /// use [`Self::run_blocking`], which moves the permit into the blocking
    /// closure, rechecks the context after admission and maps every join
    /// outcome; a permit moved into `spawn_blocking` by hand leaves those steps
    /// to the caller. `Wait` admission can return a permit after the context's
    /// deadline when its timer has not fired yet, so check the context before
    /// starting work.
    pub async fn enter(
        &self,
        context: &OperationContext,
        admission: Admission,
    ) -> Result<OwnedSemaphorePermit, AdmissionError> {
        // Record exactly one decision, including `dropped` when a waiting
        // caller abandons this future before admission completes.
        let mut decision = crate::telemetry::record::BulkheadTerminal::new();
        let result = self.admit(context, admission).await;
        decision.finish(&result);
        result
    }

    /// Admit under `admission` without recording; each caller records exactly
    /// one decision around it.
    async fn admit(
        &self,
        context: &OperationContext,
        admission: Admission,
    ) -> Result<OwnedSemaphorePermit, AdmissionError> {
        context.check()?;
        let semaphore = self.semaphore.clone();
        match admission {
            Admission::Reject => semaphore.try_acquire_owned().map_err(|error| match error {
                TryAcquireError::NoPermits => AdmissionError::Overloaded,
                TryAcquireError::Closed => AdmissionError::Closed,
            }),
            Admission::Wait => match context
                .run_internal("batter.admission", |_| async move {
                    semaphore.acquire_owned().await
                })
                .await
            {
                Ok(permit) => Ok(permit),
                Err(OperationError::Failed(_)) => Err(AdmissionError::Closed),
                Err(OperationError::Interrupted(reason)) => {
                    Err(AdmissionError::Interrupted(reason))
                }
            },
        }
    }
}
