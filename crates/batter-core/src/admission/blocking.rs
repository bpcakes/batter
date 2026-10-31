//! Bounded blocking work whose permit lives inside the blocking closure.

use super::{Admission, AdmissionError, Bulkhead};
use crate::{
    PanicPayload,
    operation::{Interruption, OperationContext, OperationError},
};
use std::fmt;
use tokio::{sync::OwnedSemaphorePermit, task::JoinError};
use tracing::{Dispatch, Span};

/// Why [`Bulkhead::run_blocking`] did not return the closure's value.
///
/// Each variant states whether the closure ran and whether it still holds
/// bulkhead capacity. The enum is exhaustive on purpose: match every variant
/// in its own arm, because merging [`Self::NotAdmitted`] with
/// [`Self::Abandoned`] erases the fact that abandoned work is still running.
/// An application error returned by the closure is not a `BlockingError`; it
/// stays concrete inside `Ok(Err(error))`.
///
/// There are no `From` conversions, so `?` cannot turn an unrelated admission
/// or interruption failure into one of these outcomes:
///
/// ```compile_fail,E0277
/// use batter_core::{admission::BlockingError, operation::OperationContext};
///
/// fn check(context: &OperationContext) -> Result<(), BlockingError> {
///     context.check()?;
///     Ok(())
/// }
/// ```
///
/// ```compile_fail,E0277
/// use batter_core::{
///     admission::{Admission, BlockingError, Bulkhead},
///     operation::OperationContext,
/// };
///
/// async fn admit(bulkhead: &Bulkhead, context: &OperationContext) -> Result<(), BlockingError> {
///     let _permit = bulkhead.enter(context, Admission::Reject).await?;
///     Ok(())
/// }
/// ```
///
/// `Debug` and `Display` never inspect a retained panic payload; read it
/// explicitly through [`PanicPayload::try_inspect`].
#[derive(thiserror::Error)]
pub enum BlockingError {
    /// Admission was refused or interrupted, including by the context check
    /// after a permit was granted. The closure was destroyed uncalled, was
    /// never handed to Tokio and used no capacity.
    #[error("blocking work was not admitted and never started")]
    NotAdmitted(#[source] AdmissionError),
    /// The closure was admitted and handed to Tokio, which destroyed it
    /// uncalled because its runtime was shutting down; its permit was
    /// released. Batter never aborts the task, so runtime shutdown is the only
    /// cause, and Tokio does not guarantee it: a busy blocking thread still
    /// runs queued closures during shutdown.
    #[error("blocking work was dropped unstarted because its runtime is shutting down")]
    NotStarted,
    /// The closure was admitted and handed to Tokio, then cancellation or the
    /// deadline ended this wait. The closure may be queued, running or
    /// finished; it keeps its permit until it returns or unwinds, and its value
    /// or panic is destroyed unobserved. Its external effects are unknown, so
    /// this is not permission to replay it.
    #[error(
        "stopped waiting for blocking work; it may still be running and holds its capacity until it returns"
    )]
    Abandoned(#[source] Interruption),
    /// The closure panicked, or its captures panicked while being destroyed;
    /// its permit was released while unwinding. The original payload is
    /// retained, and formatting never inspects it.
    #[error("blocking work panicked")]
    Panicked(PanicPayload),
}

impl fmt::Debug for BlockingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Hand-written: a derived Debug would print the payload's legacy text.
        match self {
            Self::NotAdmitted(error) => f.debug_tuple("NotAdmitted").field(error).finish(),
            Self::NotStarted => f.write_str("NotStarted"),
            Self::Abandoned(reason) => f.debug_tuple("Abandoned").field(reason).finish(),
            Self::Panicked(_) => f.write_str("Panicked(..)"),
        }
    }
}

impl Bulkhead {
    /// Run synchronous CPU work or blocking I/O on Tokio's blocking pool while
    /// holding one permit of this bulkhead until `work` returns.
    ///
    /// Admission follows [`Self::enter`]: the same [`Admission`] policy, the
    /// same context preflight and the same single bulkhead decision. After a
    /// permit is granted the context is checked again, so a cancellation, or a
    /// deadline that passed before its timer fired, still refuses the call. A
    /// refusal returns [`BlockingError::NotAdmitted`] and destroys `work`
    /// uncalled; the closure is never handed to Tokio.
    ///
    /// Otherwise the permit moves, in the same poll, into the closure handed to
    /// [`tokio::task::spawn_blocking`]. It is released only after `work`
    /// returns or unwinds and its captures are destroyed, and before the result
    /// is delivered, so capacity is already free when `Ok` or
    /// [`BlockingError::Panicked`] arrives. `work` must finish its work before
    /// it returns: anything returned as `T` that still has work to do, such as
    /// a future from `|| async move { .. }`, a `JoinHandle` or a thread handle,
    /// runs outside the bulkhead, and awaiting a returned future runs it on the
    /// caller's async thread. Use [`Self::enter`] for async work.
    ///
    /// The context bounds only the wait, with [`OperationContext::run`]'s
    /// precedence; a result that becomes ready in the same poll as an
    /// interruption is discarded. Cancellation, the deadline or dropping this
    /// future abandons the result, never the work: the closure may be queued,
    /// running or finished, and it keeps its permit until it returns
    /// ([`BlockingError::Abandoned`]). Batter never aborts it. An abandoned
    /// value or panic payload is destroyed unobserved; when the closure
    /// finishes after the wait ended, Tokio destroys it outside the dispatcher.
    ///
    /// `work` runs under the tracing dispatcher captured at first poll, in a
    /// `batter.blocking_work` span whose parent is the caller's span at
    /// handoff. A closure that Tokio destroys unstarted is destroyed under the
    /// same dispatcher and span, and the span itself is closed under that
    /// dispatcher, so the caller's span is released there too. The closure
    /// receives no context; to stop long
    /// work early, move a `context.clone()` in and call
    /// [`OperationContext::check`] between steps.
    ///
    /// This is not process-owned work. Drain, dependent cleanup and shutdown
    /// reports neither wait for nor report an abandoned closure, and Tokio
    /// cannot terminate one. Runtime destruction waits for started closures
    /// unless the application root uses `Runtime::shutdown_timeout`, which
    /// leaves them running.
    ///
    /// Capacity counts admitted closures, queued or running, not threads.
    /// Tokio's blocking pool defaults to 512 threads plus an unbounded queue
    /// and is shared with `tokio::fs`, DNS resolution and other
    /// `spawn_blocking` callers.
    ///
    /// # Panics
    ///
    /// When first polled outside a Tokio runtime, or on a runtime built without
    /// its time driver (`enable_time`), before admission; later polls keep the
    /// result wait in that runtime, even from another context, so the deadline
    /// fires only while that runtime's time driver runs. When polled after that
    /// runtime has shut down, in its deadline timer (Tokio's own panic), whether
    /// waiting for admission or for the result; a handed-off closure keeps its
    /// permit until it returns. After queueing the closure, when the operating
    /// system refuses to create a blocking thread (Tokio's own panic); the
    /// queued closure keeps its permit until it runs or the runtime shuts down.
    ///
    /// # Examples
    ///
    /// This replaces a hand-written helper that enters the bulkhead, moves the
    /// permit into `spawn_blocking` and awaits the join:
    ///
    /// ```
    /// use batter_core::{
    ///     BoxError,
    ///     admission::{Admission, BlockingError, Bulkhead, BulkheadCapacity},
    ///     operation::OperationOwner,
    /// };
    /// use std::time::Duration;
    ///
    /// /// Stand-in for CPU-heavy synchronous work such as password hashing.
    /// fn digest(input: &[u8]) -> u64 {
    ///     input.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
    ///         (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    ///     })
    /// }
    ///
    /// async fn checksum(bulkhead: &Bulkhead, input: Vec<u8>) -> Result<u64, BoxError> {
    ///     let context = OperationOwner::new(Duration::from_secs(5))?.into_context();
    ///     // Replaces: let permit = bulkhead.enter(..).await?;
    ///     //           spawn_blocking(move || { let _permit = permit; digest(&input) }).await?
    ///     match bulkhead
    ///         .run_blocking(&context, Admission::Wait, move || digest(&input))
    ///         .await
    ///     {
    ///         Ok(value) => Ok(value),
    ///         // Never ran and used no capacity.
    ///         Err(error @ BlockingError::NotAdmitted(_)) => Err(error.into()),
    ///         // Never ran: the runtime is shutting down.
    ///         Err(error @ BlockingError::NotStarted) => Err(error.into()),
    ///         // Still running and holding its permit; the outcome is unknown,
    ///         // so never replay it automatically.
    ///         Err(error @ BlockingError::Abandoned(_)) => Err(error.into()),
    ///         // The closure, or a capture's destructor, unwound; the permit is
    ///         // free and the payload retained.
    ///         Err(error @ BlockingError::Panicked(_)) => Err(error.into()),
    ///     }
    /// }
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// let bulkhead = Bulkhead::new(BulkheadCapacity::new(1)?);
    /// let value = runtime.block_on(checksum(&bulkhead, vec![7; 4096]))?;
    /// assert_eq!(value, digest(&[7; 4096]));
    /// // When the value arrives, the closure's permit is already free.
    /// let context = OperationOwner::new(Duration::from_secs(1))?.into_context();
    /// assert!(runtime.block_on(bulkhead.enter(&context, Admission::Reject)).is_ok());
    /// # Ok::<(), BoxError>(())
    /// ```
    pub async fn run_blocking<T, F>(
        &self,
        context: &OperationContext,
        admission: Admission,
        work: F,
    ) -> Result<T, BlockingError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        crate::scoped_dispatch::scope(self.run_blocking_scoped(context, admission, work)).await
    }

    async fn run_blocking_scoped<T, F>(
        &self,
        context: &OperationContext,
        admission: Admission,
        work: F,
    ) -> Result<T, BlockingError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // Outside a Tokio runtime, or on one without its time driver, these
        // panic before any permit, decision or handoff. Building the unpolled
        // `Sleep` only looks the driver up; the wait would otherwise panic
        // after the closure already owned its permit.
        let runtime = tokio::runtime::Handle::current();
        drop(tokio::time::sleep_until(context.deadline()));
        let mut decision = crate::telemetry::record::BulkheadTerminal::new();
        let admitted = self
            .admit(context, admission)
            .await
            .and_then(|permit| recheck(context, permit));
        decision.finish(&admitted);
        let permit = admitted.map_err(BlockingError::NotAdmitted)?;
        // No await until the closure is handed to Tokio: dropping this future
        // can never leave the permit outside the closure.
        let call = BlockingCall::new(work, permit);
        let handle = runtime.spawn_blocking(move || call.run());
        // Dropping the handle on interruption only detaches the task.
        let wait = context.run_internal("batter.blocking_wait", move |_| handle);
        let mut wait = std::pin::pin!(wait);
        // A `Wait` call can reach this point in a later poll made from another
        // context. Build and poll the wait's deadline timer in the runtime
        // checked at first poll, so it cannot panic for lack of a timer after
        // handoff; its deadline then fires only while that runtime is driven.
        // The guard never lives across an await, so the future stays `Send`.
        let waited = std::future::poll_fn(|cx| {
            let _runtime = runtime.enter();
            wait.as_mut().poll(cx)
        })
        .await;
        match waited {
            Ok(value) => Ok(value),
            Err(OperationError::Interrupted(reason)) => Err(BlockingError::Abandoned(reason)),
            Err(OperationError::Failed(error)) => Err(join_failure(error)),
        }
    }
}

/// `Wait` admission can grant a permit after the deadline when its timer has
/// not fired yet, and cancellation can race the grant; refuse both before
/// handoff. A refused permit is released here.
fn recheck(
    context: &OperationContext,
    permit: OwnedSemaphorePermit,
) -> Result<OwnedSemaphorePermit, AdmissionError> {
    context.check()?;
    Ok(permit)
}

/// A `JoinError` is either a panic or a cancellation. Batter holds the only
/// handle and never aborts it, so a cancellation means Tokio destroyed the
/// closure unstarted during runtime shutdown.
fn join_failure(error: JoinError) -> BlockingError {
    match error.try_into_panic() {
        Ok(payload) => BlockingError::Panicked(PanicPayload::new(payload)),
        Err(error) => {
            debug_assert!(error.is_cancelled());
            BlockingError::NotStarted
        }
    }
}

/// The closure handed to Tokio. It owns the permit, which its fields release
/// only after `work` and its captures are destroyed.
struct BlockingCall<F> {
    work: Option<F>,
    _permit: OwnedSemaphorePermit,
    dispatch: Dispatch,
    span: Span,
}

impl<F> BlockingCall<F> {
    fn new(work: F, permit: OwnedSemaphorePermit) -> Self {
        Self {
            work: Some(work),
            _permit: permit,
            dispatch: tracing::dispatcher::get_default(Dispatch::clone),
            // Keep the caller's causality even when this span is filtered out.
            span: tracing::info_span!(target: "batter", "batter.blocking_work").or_current(),
        }
    }

    fn run<T>(mut self) -> T
    where
        F: FnOnce() -> T,
    {
        let work = self.work.take().expect("blocking work runs at most once");
        tracing::dispatcher::with_default(&self.dispatch, || self.span.in_scope(work))
        // `self` drops through `Drop` on return or while unwinding, releasing
        // the permit before Tokio publishes the output.
    }
}

impl<F> Drop for BlockingCall<F> {
    fn drop(&mut self) {
        // Runs after `run` returns or unwinds, and when Tokio destroys the
        // closure unstarted. Closing `batter.blocking_work` releases its parent
        // through the thread's default dispatcher, so the span, and any unrun
        // `work` inside it, are destroyed under the captured dispatcher. The
        // permit field is released afterwards.
        let span = std::mem::replace(&mut self.span, Span::none());
        let work = self.work.take();
        tracing::dispatcher::with_default(&self.dispatch, || {
            if let Some(work) = work {
                span.in_scope(|| drop(work));
            }
            drop(span);
        });
    }
}

#[cfg(test)]
mod tests;
