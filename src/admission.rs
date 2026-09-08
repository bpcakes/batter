//! Process-local concurrency admission, distinct from Runlimit quota checks.

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

impl Bulkhead {
    /// Create a nonzero concurrency bound within Tokio's supported capacity.
    pub fn new(capacity: usize) -> Result<Self, ConfigurationError> {
        if capacity == 0 {
            return Err(ConfigurationError::Zero("bulkhead capacity"));
        }
        if capacity > Semaphore::MAX_PERMITS {
            return Err(ConfigurationError::TooLarge("bulkhead capacity"));
        }
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(capacity)),
        })
    }

    /// Reject new admissions and wake waiting callers. Existing permits remain.
    pub fn close(&self) {
        self.semaphore.close();
    }

    /// Obtain a native Tokio permit. Hold it for the work's full lifetime.
    pub async fn enter(
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
            Admission::Wait => {
                match context
                    .run("batter.admission", |_| async move {
                        semaphore.acquire_owned().await
                    })
                    .await
                {
                    Ok(permit) => Ok(permit),
                    Err(OperationError::Failed(_)) => Err(AdmissionError::Closed),
                    Err(OperationError::Interrupted(reason)) => {
                        Err(AdmissionError::Interrupted(reason))
                    }
                }
            }
        }
    }
}
