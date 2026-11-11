use super::{OperationContext, OperationError, TrackedOperationError};
use crate::telemetry::{Boundary, Outcome};
use std::{fmt, future::Future};

/// A terminal classification carrying the value returned by an adapter.
///
/// Use with [`run_resolved_value`] when even an interrupted operation produces
/// a value, such as a rendered response. The variant and value travel together;
/// no separate retained-value slot or later completion call is needed.
/// A dropped operation has no resolved value and cannot be constructed here.
/// Formatting never visits the payload.
///
/// ```compile_fail,E0599
/// use batter_core::operation::low_level::ResolvedOutcome;
/// let invalid = ResolvedOutcome::Dropped("a completed value");
/// ```
#[must_use = "return the complete classified value to run_resolved_value"]
pub enum ResolvedOutcome<T> {
    /// Authoritative evidence resolves the operation as successful.
    Succeeded(T),
    /// Authoritative evidence resolves the operation as an application failure.
    Failed(T),
    /// The final result remains interruption; retain its adapter value too.
    Interrupted {
        /// The resolved reason, not proof about remote effects.
        reason: super::Interruption,
        /// The value to return after observing the interruption.
        value: T,
    },
}

impl<T> ResolvedOutcome<T> {
    fn outcome(&self) -> Outcome {
        match self {
            Self::Succeeded(_) => Outcome::Succeeded,
            Self::Failed(_) => Outcome::Failed,
            Self::Interrupted {
                reason: super::Interruption::Cancelled,
                ..
            } => Outcome::Cancelled,
            Self::Interrupted {
                reason: super::Interruption::DeadlineExceeded,
                ..
            } => Outcome::DeadlineExceeded,
        }
    }

    fn into_value(self) -> T {
        match self {
            Self::Succeeded(value) | Self::Failed(value) | Self::Interrupted { value, .. } => value,
        }
    }
}

impl<T> fmt::Debug for ResolvedOutcome<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.outcome().as_str())
    }
}

/// Drive an operation and observe a classified adapter value before returning it.
///
/// This lower-level seam has [`OperationContext::run_resolved`]'s execution,
/// cancellation/deadline precedence, child cancellation and resolver obligations.
/// Prefer that method for ordinary `Result` outputs, or a supported adapter's
/// protected path. This seam supports adapters whose terminal value must survive
/// an interruption classification without a separate mutable storage slot.
///
/// The synchronous resolver runs once after work-future destruction, within the
/// first-poll dispatch and operation span. It must classify authoritative local
/// or application-retained evidence; this API cannot establish remote outcomes.
/// It neither extends the deadline nor awaits cleanup. An unpolled call invokes
/// neither callback. Dropping a pending call destroys captures under the saved
/// dispatch, records `dropped`, and does not invoke the resolver. Panics propagate.
/// The execution boundary still owns only the factory's returned future, not
/// detached tasks or streams returned as values. Non-yielding code is not preempted.
///
/// ```
/// use batter_core::operation::{OperationError, OperationOwner,
///     low_level::{ResolvedOutcome, run_resolved_value}};
/// use std::{convert::Infallible, time::Duration};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let owner = OperationOwner::new(Duration::from_secs(1))?;
/// owner.cancel();
/// let reply = run_resolved_value(owner.context(), "adapter.reply",
///     |_| async { Ok::<_, Infallible>("ready") },
///     |result| match result {
///         Ok(value) => ResolvedOutcome::Succeeded(value),
///         Err(OperationError::Interrupted(reason)) => ResolvedOutcome::Interrupted {
///             reason, value: "interrupted",
///         },
///         Err(OperationError::Failed(never)) => match never {},
///     }).await;
/// assert_eq!(reply, "interrupted"); // telemetry records cancellation
/// # Ok(()) }
/// ```
pub async fn run_resolved_value<T, E, U, F, Fut, Resolve>(
    context: &OperationContext,
    operation: &'static str,
    factory: F,
    resolve: Resolve,
) -> U
where
    F: FnOnce(OperationContext) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    Resolve: FnOnce(Result<T, OperationError<E>>) -> ResolvedOutcome<U>,
{
    crate::scoped_dispatch::scope(async move {
        context
            .run_inner(
                operation,
                factory,
                |result| resolve(result.map_err(TrackedOperationError::into_untracked)),
                ResolvedOutcome::outcome,
                Boundary::Operation,
            )
            .await
            .into_value()
    })
    .await
}
