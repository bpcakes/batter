//! Total deadlines and cancellation without a universal application error.

mod owner;

pub use owner::{OperationOwner, RootDeadline};

use std::{future::Future, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{
    ConfigurationError,
    telemetry::{Boundary, Observation, Outcome},
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
/// Clones share observation/execution capability, without cancellation authority.
/// Owners create independent roots; children derive both deadline and cancellation
/// from this context. Dropping a context alone does not cancel its clones.
///
/// ```compile_fail,E0624
/// use batter_core::operation::OperationContext;
/// fn cannot_cancel(context: OperationContext) { context.clone().cancel(); }
/// ```
///
/// ```compile_fail,E0599
/// use batter_core::operation::OperationContext;
/// let unrelated = OperationContext::new(std::time::Duration::from_secs(1));
/// ```
///
/// ```compile_fail,E0624
/// use batter_core::operation::OperationContext;
/// let unrelated = OperationContext::at(tokio::time::Instant::now());
/// ```
///
/// ```compile_fail,E0624
/// use batter_core::operation::OperationContext;
/// let token = tokio_util::sync::CancellationToken::new();
/// let unrelated = OperationContext::under(tokio::time::Instant::now(), &token);
/// ```
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
#[derive(Debug)]
pub struct OperationPhases {
    work: OperationContext,
    finalization: OperationContext,
}

impl OperationPhases {
    /// Explicitly cancel the work phase, leaving its finalization sibling active.
    pub fn cancel_work(&self) {
        self.work.cancel();
    }

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

    pub(crate) fn scoped_child_with_maximum(&self, maximum: Duration) -> Self {
        let requested = Instant::now()
            .checked_add(maximum)
            // A requested deadline beyond Instant's range is later than the
            // already representable parent deadline, so the parent wins.
            .unwrap_or(self.deadline);
        Self::under(self.deadline.min(requested), &self.cancellation)
    }

    /// Create an independent context. A past deadline is valid and expired.
    pub(crate) fn at(deadline: Instant) -> Self {
        Self {
            deadline,
            cancellation: CancellationToken::new(),
        }
    }

    /// Inherit cancellation without giving this context authority over parent.
    /// This is useful for request contexts tied to process forced cancellation.
    pub(crate) fn under(deadline: Instant, parent: &CancellationToken) -> Self {
        Self {
            deadline,
            cancellation: parent.child_token(),
        }
    }

    /// Create a child whose deadline is no later than this context's deadline.
    pub fn child(&self, maximum: Duration) -> Result<OperationOwner, ConfigurationError> {
        validation::positive(maximum, "child budget")?;
        Ok(OperationOwner {
            context: self.scoped_child_with_maximum(maximum),
        })
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
    pub(crate) fn cancel(&self) {
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
        self.observed(
            operation,
            factory,
            |result| result,
            operation_outcome,
            Boundary::Operation,
        )
        .await
    }

    /// Run a foundation-owned wait whose caller records its own decision.
    pub(crate) fn run_internal<T, E, F, Fut>(
        &self,
        operation: &'static str,
        factory: F,
    ) -> impl Future<Output = Result<T, OperationError<E>>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.observed(
            operation,
            factory,
            |result| result,
            operation_outcome,
            Boundary::Internal,
        )
    }

    /// Resolve retained application evidence before recording the operation outcome.
    ///
    /// This is for protocols whose externally visible result can become known
    /// before cancellable local cleanup finishes. `resolve` runs exactly once
    /// after this boundary selects completion, cancellation, or deadline, and
    /// before telemetry is finalized. It is synchronous: it cannot extend the
    /// deadline or perform more work. Return [`OperationError::Interrupted`] only
    /// when the final application result is still interruption; translate a
    /// retained success or failure into the corresponding `Ok` or
    /// [`OperationError::Failed`] value.
    ///
    /// The resolver is application policy. Batter does not verify remote effects
    /// or whether retained state is authoritative. Dropping this method's outer
    /// future still records `dropped` and does not invoke `resolve`.
    ///
    /// ```
    /// use batter_core::operation::{OperationContext, OperationError};
    /// use std::sync::{Arc, Mutex};
    /// use std::time::Duration;
    ///
    /// # async fn example() {
    /// let context = batter_core::operation::OperationOwner::new(Duration::from_secs(1)).unwrap().into_context();
    /// let retained = Arc::new(Mutex::new(None));
    /// let inside = retained.clone();
    /// let result = context
    ///     .run_resolved(
    ///         "example.commit",
    ///         move |scope| async move {
    ///             *inside.lock().unwrap() = Some(Ok::<_, &'static str>(42));
    ///             // Retained evidence can resolve a later interruption.
    ///             std::future::pending::<Result<(), &'static str>>().await
    ///         },
    ///         move |boundary| match retained.lock().unwrap().take() {
    ///             Some(result) => result.map_err(OperationError::Failed),
    ///             None => boundary.map(|()| unreachable!()),
    ///         },
    ///     )
    ///     .await;
    /// assert_eq!(result.unwrap(), 42);
    /// # }
    /// ```
    pub async fn run_resolved<T, E, U, R, F, Fut, Resolve>(
        &self,
        operation: &'static str,
        factory: F,
        resolve: Resolve,
    ) -> Result<U, OperationError<R>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
        Resolve: FnOnce(Result<T, OperationError<E>>) -> Result<U, OperationError<R>>,
    {
        self.observed(
            operation,
            factory,
            resolve,
            operation_outcome,
            Boundary::Operation,
        )
        .await
    }

    /// Run one retry attempt with an outcome mapper owned by the retry boundary.
    pub(crate) fn run_retry_attempt<T, E, F, Fut>(
        &self,
        operation: &'static str,
        factory: F,
        outcome: fn(&Result<T, OperationError<E>>) -> Outcome,
    ) -> impl Future<Output = Result<T, OperationError<E>>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.observed(
            operation,
            factory,
            |result| result,
            outcome,
            Boundary::RetryAttempt,
        )
    }

    /// The one observed boundary behind every `run` variant. Dispatch is
    /// captured on first poll, as with ordinary async instrumentation, and the
    /// returned future owns the factory, work and observation during drop too.
    /// It is a plain function so wrappers add no extra async state machine.
    fn observed<T, E, U, R, F, Fut, Resolve>(
        &self,
        operation: &'static str,
        factory: F,
        resolve: Resolve,
        outcome: fn(&Result<U, OperationError<R>>) -> Outcome,
        boundary: Boundary,
    ) -> impl Future<Output = Result<U, OperationError<R>>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
        Resolve: FnOnce(Result<T, OperationError<E>>) -> Result<U, OperationError<R>>,
    {
        crate::scoped_dispatch::scope(
            self.run_inner(operation, factory, resolve, outcome, boundary),
        )
    }

    async fn run_inner<T, E, U, R, F, Fut, Resolve>(
        &self,
        operation: &'static str,
        factory: F,
        resolve: Resolve,
        outcome: fn(&Result<U, OperationError<R>>) -> Outcome,
        boundary: Boundary,
    ) -> Result<U, OperationError<R>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
        Resolve: FnOnce(Result<T, OperationError<E>>) -> Result<U, OperationError<R>>,
    {
        let mut observation = Observation::new(operation, boundary);
        let observed = &observation;
        let span = observation.context();
        let scope = Self::under(self.deadline, &self.cancellation);
        let cancellation = scope.cancellation.clone();
        let _cancel_on_exit = cancellation.clone().drop_guard();
        let result = async move {
            let boundary = async {
                self.check()?;
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        Err(OperationError::Interrupted(Interruption::Cancelled))
                    }
                    _ = tokio::time::sleep_until(self.deadline) => {
                        Err(OperationError::Interrupted(Interruption::DeadlineExceeded))
                    }
                    // Runs only when this branch is first polled, so an
                    // interruption that wins the race leaves the factory unused.
                    result = async move {
                        observed.mark_factory_invoked();
                        factory(scope).await
                    } => {
                        result.map_err(OperationError::Failed)
                    }
                }
            }
            .await;
            resolve(boundary)
        }
        .instrument(span)
        .await;
        observation.finish(outcome(&result));
        result
    }
}

fn operation_outcome<T, E>(result: &Result<T, OperationError<E>>) -> Outcome {
    match result {
        Ok(_) => Outcome::Succeeded,
        Err(OperationError::Failed(_)) => Outcome::Failed,
        Err(OperationError::Interrupted(Interruption::Cancelled)) => Outcome::Cancelled,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
            Outcome::DeadlineExceeded
        }
    }
}
