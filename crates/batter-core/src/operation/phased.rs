//! Owned work followed by outcome-aware finalization.

mod action;
mod outcome;
mod owner;
mod report;

pub use action::PhaseAction;
pub use outcome::{PhaseFailure, PhaseOutcome};
pub use owner::{PhasedObserver, RunningPhasedOperation};
pub use report::{FailedPhasedReport, PhasedFailure, PhasedReport, PhasedSuccess};

use super::{OperationContext, OperationPhases};
use crate::{
    ConfigurationError, command,
    scoped_dispatch::{self, ScopedOutput, SharedOutput},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::Instrument;

/// Inert work-and-finalization specification with one original total deadline.
///
/// The work callback receives only its shortened context. After its future is
/// destroyed, the library retains its complete outcome and supplies it to the
/// finalizer. Early `?`, work cancellation and work panic cannot skip this
/// transition. Finalization can use time saved by early work completion.
///
/// Dropping the running owner cancels only work; its coordinator continues
/// finalization on the live runtime. Parent cancellation interrupts both phases,
/// as does their own deadline. Neither factory runs when its context is already
/// interrupted. No async-drop, remote rollback, scheduling or detached-task join
/// guarantee is made. An outer cancelled waiter cannot claim completion.
///
/// ```
/// use batter_core::operation::{OperationOwner, PhasedOperation};
/// use std::{convert::Infallible, time::Duration};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), batter_core::ConfigurationError> {
/// let parent = OperationOwner::new(Duration::from_secs(2))?;
/// let operation = PhasedOperation::new(parent.context(), Duration::from_millis(200),
///     async |work| {
///         work.check()?;
///         Ok::<_, batter_core::operation::Interruption>(42)
///     },
///     async |finalization| {
///         // Record success or failure using finalization.context().
///         let succeeded = match finalization.work().classify() {
///             Ok(_accepted) => true,
///             Err(failed) => {
///                 // Even a returned Ok can be unaccepted after interruption.
///                 let _raw_evidence = failed.returned_result();
///                 false
///             }
///         };
///         Ok::<_, Infallible>(succeeded)
///     },
/// )?;
/// let running = operation.start();
/// let report = running.wait().await.expect("both phases succeeded");
/// assert!(*report.finalization());
/// assert_eq!(*report.work(), 42);
/// # Ok(()) }
/// ```
///
/// Work cannot access the finalization context:
/// ```compile_fail,E0599
/// use batter_core::operation::{OperationContext, PhasedOperation};
/// fn invalid(parent: &OperationContext) {
///     let _ = PhasedOperation::new(parent, std::time::Duration::from_secs(1),
///         async |work| { work.finalization(); Ok::<_, ()>(()) },
///         async |_finalization| Ok::<_, ()>(()));
/// }
/// ```
/// The specification has no context accessor or separately callable finalizer:
/// ```compile_fail,E0599
/// use batter_core::operation::{OperationContext, PhasedOperation};
/// fn early(parent: &OperationContext) {
///     let spec = PhasedOperation::new(parent, std::time::Duration::from_secs(1),
///         async |_work| Ok::<_, ()>(()), async |_finalization| Ok::<_, ()>(())).unwrap();
///     spec.finalization();
/// }
/// ```
/// A callback that retains thread-local state across an await is rejected:
/// ```compile_fail,E0277
/// use batter_core::operation::{OperationContext, PhasedOperation};
/// fn local(parent: &OperationContext) {
///     let value = std::rc::Rc::new(1);
///     let _ = PhasedOperation::new(parent, std::time::Duration::from_secs(1),
///         async move |_work| { tokio::task::yield_now().await; Ok::<_, ()>(*value) },
///         async |_finalization| Ok::<_, ()>(()));
/// }
/// ```
/// Captured root handles, arbitrary spawning and application policy remain the
/// application's responsibility. [`OperationPhases`] is the weaker manual path.
#[must_use = "start this inert specification or explicitly abandon it"]
pub struct PhasedOperation<W, F> {
    phases: OperationPhases,
    work: W,
    finalization: F,
}

impl<W, F> PhasedOperation<W, F> {
    /// Reserve positive finalization time and store both inert callbacks.
    ///
    /// # Errors
    /// Returns a configuration error if the reserve is invalid or leaves no
    /// work time. The absolute deadlines start here, never at first poll.
    pub fn new<T, E, U, G>(
        parent: &OperationContext,
        reserve: Duration,
        work: W,
        finalization: F,
    ) -> Result<Self, ConfigurationError>
    where
        W: PhaseAction<OperationContext, T, E>,
        F: PhaseAction<FinalizationScope<T, E>, U, G>,
        T: Send + Sync + 'static,
        E: Send + Sync + 'static,
        U: Send + Sync + 'static,
        G: Send + Sync + 'static,
    {
        Ok(parent
            .reserve_finalization(reserve)?
            .into_operation(work, finalization))
    }

    /// Start one owned coordinator, without installing signal listeners.
    ///
    /// Reuses the command driver's phase polling, panic handling and destruction
    /// rules. The runtime must remain alive through completion. A borrowed
    /// waiter may be cancelled freely; dropping the owner cancels work only.
    /// Work completion cancels its context and descendants before finalization,
    /// whose sibling context stays live unless its parent or deadline ends it.
    /// Cancellation does not join arbitrary tasks using those contexts.
    ///
    /// # Panics
    /// Panics outside a Tokio runtime. Its time driver must be enabled.
    pub fn start<T, E, U, G>(self) -> RunningPhasedOperation<T, E, U, G>
    where
        W: PhaseAction<OperationContext, T, E>,
        F: PhaseAction<FinalizationScope<T, E>, U, G>,
        T: Send + Sync + 'static,
        E: Send + Sync + 'static,
        U: Send + Sync + 'static,
        G: Send + Sync + 'static,
    {
        let context = self.phases.work.clone();
        let (publication, completion) = watch::channel(None);
        let coordinator = tokio::spawn(scoped_dispatch::scope(
            async move { ScopedOutput::capture(self.drive().await.into_checked()) }
                .in_current_span(),
        ));
        drop(tokio::spawn(scoped_dispatch::scope(
            async move {
                let output = match coordinator.await {
                    Ok(output) => output,
                    Err(error) => {
                        ScopedOutput::capture(Err(PhasedFailure::Coordinator(Arc::new(error))))
                    }
                };
                publication.send_replace(Some(SharedOutput::new(output)));
            }
            .in_current_span(),
        )));
        RunningPhasedOperation::new(context, completion)
    }

    async fn drive<T, E, U, G>(self) -> PhasedReport<T, E, U, G>
    where
        W: PhaseAction<OperationContext, T, E>,
        F: PhaseAction<FinalizationScope<T, E>, U, G>,
    {
        let Self {
            phases,
            work,
            finalization,
        } = self;
        let mut work_scope = phases.work.clone();
        let work = execute(&mut work_scope, &phases.work, work).await;
        phases.work.cancel();
        let mut scope = FinalizationScope {
            context: phases.finalization.clone(),
            work,
        };
        let finalized = execute(&mut scope, &phases.finalization, finalization).await;
        phases.finalization.cancel();
        PhasedReport {
            work: scope.work,
            finalization: finalized,
        }
    }
}

/// Available only inside the finalization callback, after work destruction.
/// The work outcome is read-only and cannot be replaced or consumed by `?`.
pub struct FinalizationScope<T, E> {
    context: OperationContext,
    work: PhaseOutcome<T, E>,
}

impl<T, E> FinalizationScope<T, E> {
    /// The original total deadline, independently of work cancellation.
    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// The retained work outcome, including failure and destruction evidence.
    pub fn work(&self) -> &PhaseOutcome<T, E> {
        &self.work
    }
}

async fn execute<S, T, E, F>(
    scope: &mut S,
    context: &OperationContext,
    action: F,
) -> PhaseOutcome<T, E>
where
    F: PhaseAction<S, T, E>,
{
    let (result, interruption, destruction) = command::run_work(
        scope,
        context,
        move |scope| action.invoke(scope, action::private::Token),
        &mut command::NoSignals,
    )
    .await;
    PhaseOutcome::new(result, interruption, destruction)
}

impl OperationPhases {
    /// Transfer an existing phase pair into library-owned sequential execution.
    ///
    /// Use this for phases derived from a native invocation, retaining its exact
    /// deadlines and cancellation lineage, including a zero reserve. The pair
    /// is consumed. Previously extracted contexts remain a lower-level escape
    /// hatch; do not extract them when choosing this protected path.
    ///
    /// ```
    /// use batter_core::operation::OperationOwner;
    /// # fn example() -> Result<(), batter_core::ConfigurationError> {
    /// let parent = OperationOwner::new(std::time::Duration::from_secs(2))?;
    /// let spec = parent.context().split_finalization()?.into_operation(
    ///     async |_work| Ok::<_, ()>(()),
    ///     async |_finalization| Ok::<_, ()>(()));
    /// drop(spec); // An unstarted specification performs no application work.
    /// # Ok(()) }
    /// ```
    pub fn into_operation<W, F, T, E, U, G>(self, work: W, finalization: F) -> PhasedOperation<W, F>
    where
        W: PhaseAction<OperationContext, T, E>,
        F: PhaseAction<FinalizationScope<T, E>, U, G>,
        T: Send + Sync + 'static,
        E: Send + Sync + 'static,
        U: Send + Sync + 'static,
        G: Send + Sync + 'static,
    {
        PhasedOperation {
            phases: self,
            work,
            finalization,
        }
    }
}
