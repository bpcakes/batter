//! Total deadlines and cancellation without a universal application error.

use std::{future::Future, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{
    ConfigurationError,
    telemetry::{Observation, Outcome},
    validation,
};

/// Why an execution boundary stopped polling its work.
/// Neither variant establishes whether external effects already happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Interruption {
    /// Cancellation was requested.
    #[error("operation cancelled; external outcome may be unknown")]
    Cancelled,
    /// Its total deadline expired.
    #[error("operation deadline exceeded; external outcome may be unknown")]
    DeadlineExceeded,
}

/// Application failure and execution interruption remain distinct.
#[derive(Debug, thiserror::Error)]
pub enum OperationError<E> {
    /// Original application failure, not converted to an opaque message.
    #[error("operation failed")]
    Failed(#[source] E),
    /// The future was dropped at an execution boundary.
    #[error(transparent)]
    Interrupted(#[from] Interruption),
}

/// A deadline and cancellation lineage, not a dependency container.
///
/// Clones share a token. Children have independent cancellation and can never
/// extend the deadline. Dropping this value alone does not cancel its clones.
#[derive(Clone, Debug)]
pub struct OperationContext {
    deadline: Instant,
    cancellation: CancellationToken,
}

/// Work and finalization contexts sharing one original total deadline.
///
/// Work stops before the reserved finalization interval. Finalization can use
/// all time remaining until the original deadline, including time saved by
/// work completing early. The contexts are siblings: cancelling work does not
/// cancel finalization, while cancellation of their parent cancels both.
///
/// The caller must explicitly await finalization after observing the work
/// result. This value does not run cleanup on drop, own spawned descendants,
/// shield parent cancellation, or guarantee scheduling within the reserve.
#[derive(Clone, Debug)]
pub struct OperationPhases {
    work: OperationContext,
    finalization: OperationContext,
}

impl OperationPhases {
    /// Shortened context for admission, attempts, backoff, and other work.
    pub fn work(&self) -> &OperationContext {
        &self.work
    }

    /// Sibling context bounded by the original total deadline.
    ///
    /// Use this after work completes, even when work returned an error or
    /// exhausted its deadline. Parent cancellation still interrupts this phase.
    pub fn finalization(&self) -> &OperationContext {
        &self.finalization
    }
}

impl OperationContext {
    pub(crate) fn scoped_child(&self) -> Self {
        Self::under(self.deadline, &self.cancellation)
    }

    /// Create an independent operation with a positive total time budget.
    pub fn new(budget: Duration) -> Result<Self, ConfigurationError> {
        validation::positive(budget, "operation budget")?;
        let deadline = Instant::now()
            .checked_add(budget)
            .ok_or(ConfigurationError::TooLarge("operation budget"))?;
        Ok(Self::at(deadline))
    }

    /// Create an independent context. A past deadline is valid and expired.
    pub fn at(deadline: Instant) -> Self {
        Self {
            deadline,
            cancellation: CancellationToken::new(),
        }
    }

    /// Inherit cancellation without giving this context authority over parent.
    /// This is useful for request contexts tied to process forced cancellation.
    pub fn under(deadline: Instant, parent: &CancellationToken) -> Self {
        Self {
            deadline,
            cancellation: parent.child_token(),
        }
    }

    /// Create a child whose deadline is no later than this context's deadline.
    pub fn child(&self, maximum: Duration) -> Result<Self, ConfigurationError> {
        validation::positive(maximum, "child budget")?;
        let requested = Instant::now()
            .checked_add(maximum)
            .ok_or(ConfigurationError::TooLarge("child budget"))?;
        Ok(Self::under(
            self.deadline.min(requested),
            &self.cancellation,
        ))
    }

    /// Reserve a positive interval before this context's total deadline.
    ///
    /// The reserve must leave positive work time when this method is called.
    /// The work deadline is the original deadline minus `reserve`; the
    /// finalization deadline remains the original deadline. Pass `work()` to
    /// retry/admission so their waits cannot consume the reserved interval.
    /// Both phases inherit cancellation from this context independently.
    ///
    /// Explicitly await finalization after observing the work result; using
    /// `?` to return early on a work failure would skip it. Neither this method
    /// nor the returned contexts provide cancellation shielding or async Drop.
    /// Use [`crate::command::Command::within`] for finite work whose owner must
    /// retain cleanup independently of work cancellation and borrowed waiters.
    /// When reserving inside a `run` scope, finish both phases before that
    /// enclosing scope returns and cancels its descendants.
    pub fn reserve_finalization(
        &self,
        reserve: Duration,
    ) -> Result<OperationPhases, ConfigurationError> {
        validation::positive(reserve, "finalization reserve")?;
        let work_deadline = self
            .deadline
            .checked_sub(reserve)
            .filter(|deadline| *deadline > Instant::now())
            .ok_or(ConfigurationError::InvalidReserve)?;
        Ok(OperationPhases {
            work: Self::under(work_deadline, &self.cancellation),
            finalization: Self::under(self.deadline, &self.cancellation),
        })
    }

    /// Absolute runtime deadline; not a serializable wall-clock timestamp.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Remaining total budget, including queue waits and retry sleeps.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Request cancellation for this context, its clones, and its children.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Wait for cancellation. Deadline expiration is checked separately by run.
    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    /// Check the boundary before starting additional work.
    pub fn check(&self) -> Result<(), Interruption> {
        if self.cancellation.is_cancelled() {
            Err(Interruption::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Interruption::DeadlineExceeded)
        } else {
            Ok(())
        }
    }

    /// Run a fresh future inside a child cancellation scope.
    ///
    /// The factory is not invoked when already cancelled or expired. The
    /// child is cancelled on success, error, timeout, or dropping this run
    /// future. This NOTIFIES descendants; it does not join spawned tasks.
    ///
    /// When simultaneously ready, cancellation wins over deadline, and
    /// deadline wins over completion. A future that blocks a runtime thread
    /// cannot be preempted. Panics propagate to the caller/task boundary.
    /// Completion and drop diagnostics retain the subscriber from first poll,
    /// including when a task is aborted outside that subscriber's scope.
    pub async fn run<T, E, F, Fut>(
        &self,
        operation: &'static str,
        factory: F,
    ) -> Result<T, OperationError<E>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        // Capture on first poll, as with ordinary async instrumentation. The
        // inner future owns the factory, work, and observation during drop too.
        crate::scoped_dispatch::scope(self.run_inner(operation, factory)).await
    }

    async fn run_inner<T, E, F, Fut>(
        &self,
        operation: &'static str,
        factory: F,
    ) -> Result<T, OperationError<E>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut observation = Observation::new(operation);
        let span = observation.context();
        let scope = Self::under(self.deadline, &self.cancellation);
        let cancellation = scope.cancellation.clone();
        let _cancel_on_exit = cancellation.clone().drop_guard();
        let result = async {
            self.check()?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    Err(OperationError::Interrupted(Interruption::Cancelled))
                }
                _ = tokio::time::sleep_until(self.deadline) => {
                    Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
                }
                result = async move { factory(scope).await } => {
                    result.map_err(OperationError::Failed)
                }
            }
        }
        .instrument(span)
        .await;
        observation.finish(match &result {
            Ok(_) => Outcome::Succeeded,
            Err(OperationError::Failed(_)) => Outcome::Failed,
            Err(OperationError::Interrupted(Interruption::Cancelled)) => Outcome::Cancelled,
            Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
                Outcome::DeadlineExceeded
            }
        });
        result
    }
}
