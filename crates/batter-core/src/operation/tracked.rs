use std::{fmt, future::Future};

use super::{Interruption, OperationContext, OperationError};
use crate::telemetry::{Boundary, Outcome};

/// Evidence that this operation boundary never invoked its factory.
///
/// This is local evidence for one call, not proof that earlier calls, captured
/// values or independently spawned work caused no effects. Only Batter can
/// construct it. It grants no retry permission.
///
/// ```compile_fail,E0451
/// use batter_core::operation::{Interruption, NotStarted};
/// let proof = NotStarted { reason: Interruption::Cancelled };
/// ```
///
/// ```compile_fail,E0277
/// use batter_core::operation::{Interruption, NotStarted};
/// let proof: NotStarted = Interruption::Cancelled.into();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("operation factory was not invoked")]
pub struct NotStarted {
    pub(super) reason: Interruption,
}

impl NotStarted {
    /// Cancellation or deadline expiry that prevented factory invocation.
    pub fn reason(&self) -> Interruption {
        self.reason
    }
}

/// Exhaustive failure evidence from [`OperationContext::run_tracked`].
///
/// There are deliberately no `From` conversions: callers must preserve the
/// distinction between a factory not invoked and interruption after invocation.
/// Diagnostics omit application error contents; inspect `Failed` explicitly.
///
/// ```compile_fail,E0277
/// use batter_core::operation::{Interruption, TrackedOperationError};
/// let error: TrackedOperationError<()> = Interruption::Cancelled.into();
/// ```
///
/// ```compile_fail,E0277
/// use batter_core::operation::{OperationError, TrackedOperationError};
/// let error: TrackedOperationError<()> = OperationError::Failed(()).into();
/// ```
///
/// ```compile_fail,E0277
/// use batter_core::operation::{OperationError, TrackedOperationError};
/// let tracked = TrackedOperationError::Failed(());
/// let erased: OperationError<()> = tracked.into();
/// ```
#[derive(thiserror::Error)]
pub enum TrackedOperationError<E> {
    /// The factory was never invoked by this call.
    #[error(transparent)]
    NotStarted(NotStarted),
    /// The factory was invoked, but no result was selected before interruption.
    /// This does not establish whether external effects happened.
    #[error(transparent)]
    Interrupted(Interruption),
    /// Concrete application failure returned by the work future.
    #[error("operation failed")]
    Failed(#[source] E),
}

impl<E> fmt::Debug for TrackedOperationError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted(proof) => formatter.debug_tuple("NotStarted").field(proof).finish(),
            Self::Interrupted(reason) => {
                formatter.debug_tuple("Interrupted").field(reason).finish()
            }
            Self::Failed(_) => formatter.write_str("Failed([REDACTED])"),
        }
    }
}

impl<E> TrackedOperationError<E> {
    pub(super) fn into_untracked(self) -> OperationError<E> {
        match self {
            Self::NotStarted(proof) => OperationError::Interrupted(proof.reason),
            Self::Interrupted(reason) => OperationError::Interrupted(reason),
            Self::Failed(error) => OperationError::Failed(error),
        }
    }
}

impl OperationContext {
    /// Run a fresh future, retaining whether this call invoked its factory.
    ///
    /// Use this when a protocol needs to distinguish refusal before invocation
    /// from interruption after invocation. An invoked factory counts as started
    /// even if its future performs no effects. No result proves remote effects
    /// or authorizes replay. Creating/dropping an unpolled run invokes nothing
    /// and returns no evidence.
    ///
    /// Like [`Self::run`], cancellation wins over deadline, which wins over
    /// completion when simultaneously ready. The child scope is cancelled on
    /// every exit, including drop and panic; descendants are not joined. Panics
    /// propagate, non-yielding work cannot be preempted, and borrowed factories
    /// and futures need neither `Send` nor `'static`. Dispatch is captured on
    /// first poll and retained through destruction.
    ///
    /// ```
    /// use batter_core::operation::{OperationOwner, TrackedOperationError};
    /// use std::time::Duration;
    /// # async fn example() -> Result<(), batter_core::ConfigurationError> {
    /// let owner = OperationOwner::new(Duration::from_secs(1))?;
    /// owner.cancel();
    /// match owner.context().run_tracked("example.read", |_| async {
    ///     Ok::<_, &'static str>(42)
    /// }).await {
    ///     Ok(value) => assert_eq!(value, 42),
    ///     Err(TrackedOperationError::NotStarted(proof)) => {
    ///         assert_eq!(proof.reason(), batter_core::operation::Interruption::Cancelled);
    ///     }
    ///     Err(TrackedOperationError::Interrupted(_)) => panic!("unexpected invocation"),
    ///     Err(TrackedOperationError::Failed(error)) => panic!("{error}"),
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn run_tracked<T, E, F, Fut>(
        &self,
        operation: &'static str,
        factory: F,
    ) -> Result<T, TrackedOperationError<E>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        crate::scoped_dispatch::scope(self.run_inner(
            operation,
            factory,
            |result| result,
            tracked_outcome,
            Boundary::Operation,
        ))
        .await
    }
}

fn tracked_outcome<T, E>(result: &Result<T, TrackedOperationError<E>>) -> Outcome {
    let reason = match result {
        Ok(_) => return Outcome::Succeeded,
        Err(TrackedOperationError::Failed(_)) => return Outcome::Failed,
        Err(TrackedOperationError::NotStarted(proof)) => proof.reason(),
        Err(TrackedOperationError::Interrupted(reason)) => *reason,
    };
    match reason {
        Interruption::Cancelled => Outcome::Cancelled,
        Interruption::DeadlineExceeded => Outcome::DeadlineExceeded,
    }
}
