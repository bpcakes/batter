use super::{Admission, AdmissionError, Bulkhead};
use crate::operation::{OperationContext, OperationError};
use std::{fmt, future::Future};

/// Admission refusal or the complete result of admitted async execution.
/// Formatting never visits the retained application error.
pub enum BulkheadError<E> {
    /// The factory was not invoked.
    NotAdmitted(AdmissionError),
    /// The admitted operation failed or was interrupted. External effects may
    /// already have happened; interruption never authorizes replay.
    Operation(OperationError<E>),
}

impl<E> fmt::Display for BulkheadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAdmitted(error) => write!(f, "async work not admitted: {error}"),
            Self::Operation(OperationError::Failed(_)) => f.write_str("async work failed"),
            Self::Operation(OperationError::Interrupted(reason)) => reason.fmt(f),
        }
    }
}

impl<E> fmt::Debug for BulkheadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<E: std::error::Error + 'static> std::error::Error for BulkheadError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAdmitted(error) => Some(error),
            Self::Operation(error) => Some(error),
        }
    }
}

impl Bulkhead {
    /// Admit and run one async factory, retaining capacity through full future
    /// destruction on success, error, panic, interruption or caller cancellation.
    ///
    /// Migration: `Bulkhead::enter` was removed; move the complete async work
    /// into this callback. Use `run_blocking` for blocking work. No raw permit
    /// or deprecated shim is exposed because it restores premature release.
    ///
    /// ```compile_fail,E0599
    /// async fn removed(b: &batter_core::admission::Bulkhead,
    ///     c: &batter_core::operation::OperationContext) {
    ///     b.enter(c, batter_core::admission::Admission::Reject).await;
    /// }
    /// ```
    ///
    /// Admission and execution share the original absolute deadline. The context
    /// is rechecked after acquisition and before the factory is invoked. No
    /// permit or task handle escapes; no task is spawned. A never-polled call
    /// does no work. The factory and future may borrow application state.
    ///
    /// Uses [`OperationContext::run`]'s cancellation/deadline precedence and
    /// child cancellation. It owns only the returned future: detached tasks,
    /// response streams returned as values, and remote effects are outside its
    /// lifetime. Non-yielding code cannot be preempted. Waiting callers remain
    /// unbounded; select [`Admission::Reject`] or bound the upstream queue.
    /// Like any outer operation boundary, this can interrupt nested protocols;
    /// use their retained-outcome machinery when acknowledging durable effects.
    ///
    /// ```
    /// use batter_core::{admission::{Admission, Bulkhead, BulkheadCapacity},
    ///     operation::OperationOwner};
    /// use std::time::Duration;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let bulkhead = Bulkhead::new(BulkheadCapacity::new(2)?);
    /// let owner = OperationOwner::new(Duration::from_secs(1))?;
    /// let input = String::from("record");
    /// let size = bulkhead.run(owner.context(), Admission::Reject, "record.read",
    ///     |_scope| async { Ok::<_, std::io::Error>(input.len()) }).await?;
    /// assert_eq!(size, 6);
    /// # Ok(()) }
    /// ```
    pub async fn run<T, E, F, Fut>(
        &self,
        context: &OperationContext,
        admission: Admission,
        operation: &'static str,
        factory: F,
    ) -> Result<T, BulkheadError<E>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.run_resolved(context, admission, operation, factory, |result| result)
            .await
    }

    /// Admit async work and resolve application-retained evidence before telemetry.
    ///
    /// Has [`Self::run`]'s permit and deadline ownership. The resolver has
    /// [`OperationContext::run_resolved`]'s obligations: it must use authoritative
    /// application evidence, and cannot infer remote outcomes from interruption.
    /// Admission refusal never invokes the factory or resolver. Dropping the outer
    /// future still destroys work and releases capacity without invoking a resolver.
    ///
    /// ```
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// use batter_core::{admission::{Admission, Bulkhead, BulkheadCapacity}, operation::OperationOwner};
    /// let bulkhead = Bulkhead::new(BulkheadCapacity::new(1)?);
    /// let owner = OperationOwner::new(std::time::Duration::from_secs(1))?;
    /// bulkhead.run_resolved(owner.context(), Admission::Reject, "example.work",
    ///     |_| async { Ok::<_, std::io::Error>(()) }, |result| result).await?;
    /// # Ok(()) }
    /// ```
    pub async fn run_resolved<T, E, U, R, F, Fut, Resolve>(
        &self,
        context: &OperationContext,
        admission: Admission,
        operation: &'static str,
        factory: F,
        resolve: Resolve,
    ) -> Result<U, BulkheadError<R>>
    where
        F: FnOnce(OperationContext) -> Fut,
        Fut: Future<Output = Result<T, E>>,
        Resolve: FnOnce(Result<T, OperationError<E>>) -> Result<U, OperationError<R>>,
    {
        crate::scoped_dispatch::scope(async move {
            let mut decision = crate::telemetry::record::BulkheadTerminal::new();
            let admitted = self.admit(context, admission).await.and_then(|permit| {
                context
                    .check()
                    .map(|()| permit)
                    .map_err(AdmissionError::Interrupted)
            });
            decision.finish(&admitted);
            // This local outlives the entire operation future, including its
            // destruction. Never move it into the callback or its result.
            let _permit = admitted.map_err(BulkheadError::NotAdmitted)?;
            context
                .run_resolved(operation, factory, resolve)
                .await
                .map_err(BulkheadError::Operation)
        })
        .await
    }
}
