//! Borrowing execution kernel for adapters that own the containing invocation.
use super::{PhaseOutcome, PhasedFailure, PhasedReport};
use crate::{
    command,
    operation::{OperationContext, OperationPhases},
};
use std::future::Future;

/// Lower-level adapter protocol for a host-owned, borrowing invocation.
///
/// The host must drive both phases until completion or until its authoritative
/// invocation ends. It must not expose independent cancellation of this driver
/// as a protected operation. Dropping the driver cancels both contexts and
/// destroys the current future; it does not run asynchronous finalization.
/// Use [`super::PhasedOperation`] for independently owned application work.
///
/// ```
/// use batter_core::operation::{OperationContext, PhaseOutcome, low_level::ScopedPhases};
/// struct Adapter<'a>(&'a mut usize);
/// impl ScopedPhases for Adapter<'_> {
///     type Work = usize;
///     type WorkError = ();
///     type Final = ();
///     type FinalError = ();
///     async fn work(&mut self, _: &OperationContext) -> Result<usize, ()> { Ok(*self.0) }
///     async fn finalize(&mut self, _: &OperationContext, work: &PhaseOutcome<usize, ()>)
///         -> Result<(), ()> {
///         if let Ok(value) = work.classify() { *self.0 = value + 1; }
///         Ok(())
///     }
/// }
/// ```
pub trait ScopedPhases: Send {
    /// Work result retained for finalization.
    type Work: Send + Sync;
    /// Concrete work failure retained for finalization.
    type WorkError: Send + Sync;
    /// Finalization result.
    type Final: Send;
    /// Concrete finalization failure.
    type FinalError: Send;

    /// Execute work under the shortened context. May borrow host services.
    fn work(
        &mut self,
        context: &OperationContext,
    ) -> impl Future<Output = Result<Self::Work, Self::WorkError>> + Send;

    /// Execute only after work's future has been destroyed and its context
    /// cancelled. A returned work error or panic cannot skip this call.
    fn finalize(
        &mut self,
        context: &OperationContext,
        work: &PhaseOutcome<Self::Work, Self::WorkError>,
    ) -> impl Future<Output = Result<Self::Final, Self::FinalError>> + Send;
}

/// Checked adapter result; success requires both phases to succeed.
pub type ScopedPhaseResult<P> = Result<
    (<P as ScopedPhases>::Work, <P as ScopedPhases>::Final),
    PhasedFailure<
        <P as ScopedPhases>::Work,
        <P as ScopedPhases>::WorkError,
        <P as ScopedPhases>::Final,
        <P as ScopedPhases>::FinalError,
    >,
>;

/// Drive a borrowing adapter protocol inside its host-owned invocation.
///
/// This is not an owned operation or cancellation shield. The obligations of
/// [`ScopedPhases`] apply; a supported adapter must control this whole future.
/// Shares owned command/phase polling, panic capture and destruction semantics.
pub async fn drive_phases<P: ScopedPhases>(
    phases: OperationPhases,
    driver: &mut P,
) -> ScopedPhaseResult<P> {
    crate::scoped_dispatch::scope(async move {
        let _cancel_work = phases.work.cancellation.clone().drop_guard();
        let _cancel_finalization = phases.finalization.cancellation.clone().drop_guard();
        let mut work_scope = (&mut *driver, &phases.work);
        let (result, interruption, destruction) = command::run_work(
            &mut work_scope,
            &phases.work,
            |scope| Box::pin(async move { scope.0.work(scope.1).await }),
            &mut command::NoSignals,
        )
        .await;
        let work = PhaseOutcome::new(result, interruption, destruction);
        phases.work.cancel();
        let mut final_scope = (driver, &phases.finalization, &work);
        let (result, interruption, destruction) = command::run_work(
            &mut final_scope,
            &phases.finalization,
            |scope| Box::pin(async move { scope.0.finalize(scope.1, scope.2).await }),
            &mut command::NoSignals,
        )
        .await;
        phases.finalization.cancel();
        PhasedReport {
            work,
            finalization: PhaseOutcome::new(result, interruption, destruction),
        }
        .into_owned_checked()
    })
    .await
}
