//! Explicit settlement authority for a retained late value, never a failed work phase.
use batter_core::operation::{Interruption, OperationContext, PhaseOutcome, PhasedFailure};
use runledger_core::jobs::{JobCompletion, JobExecution};
use std::{future::Future, pin::Pin};

pub(super) type FinalFuture<'a, E> =
    Pin<Box<dyn Future<Output = Result<JobCompletion, E>> + Send + 'a>>;

/// A successful work value returned after its shortened deadline, with no
/// cancellation, polling panic or destruction panic. Only the adapter issues it.
/// The original invocation deadline still bounds settlement.
///
/// This is authority to attempt application settlement, not evidence that any
/// remote effect or database commit succeeded. Consume it with [`Self::settle`]
/// to propose a callback, or [`Self::decline`] to retain strict phase behavior.
///
/// ```compile_fail,E0451
/// fn forge(execution: runledger_core::jobs::JobExecution<'_>,
///     context: &batter_core::operation::OperationContext, value: &u64) {
///     let _ = batter_runledger::LateJobFinalization { execution, context, value };
/// }
/// ```
pub struct LateJobFinalization<'a, T> {
    pub(super) execution: JobExecution<'a>,
    pub(super) context: &'a OperationContext,
    pub(super) value: &'a T,
}
impl<'a, T> LateJobFinalization<'a, T> {
    /// Native services for this invocation, retaining lease fencing.
    pub fn execution(&self) -> JobExecution<'_> {
        self.execution
    }
    /// Remaining finalization budget; never extends the native deadline.
    pub fn context(&self) -> &OperationContext {
        self.context
    }
    /// The concrete successful return, without reclassifying work as successful.
    pub fn value(&self) -> &T {
        self.value
    }
    /// Propose settlement using this value and invocation. The adapter drives
    /// the returned future directly and checks both return and destruction.
    /// A successful return alone cannot publish a recovered completion.
    pub fn settle<E, F, Fut>(self, callback: F) -> LateJobSettlement<'a, E>
    where
        F: FnOnce(Self) -> Fut,
        Fut: Future<Output = Result<JobCompletion, E>> + Send + 'a,
    {
        LateJobSettlement(Some(Box::pin(callback(self))))
    }
    /// Decline recovery and run the ordinary finalizer with complete evidence.
    pub fn decline<E>(self) -> LateJobSettlement<'a, E> {
        LateJobSettlement(None)
    }
}

/// An opaque settlement proposal consumed only by [`super::PhasedJob`].
/// Constructed through [`LateJobFinalization`]; it is neither a completion
/// proof nor an independently runnable future. No invocation may execute it
/// outside the adapter's finalization phase.
///
/// ```compile_fail,E0423
/// let forged = batter_runledger::LateJobSettlement::<()>(None);
/// ```
/// ```compile_fail,E0277
/// async fn bypass(proposal: batter_runledger::LateJobSettlement<'_, ()>) {
///     proposal.await;
/// }
/// ```
#[must_use = "return this proposal to the adapter"]
pub struct LateJobSettlement<'a, E>(pub(super) Option<FinalFuture<'a, E>>);

pub(super) fn late_value<T, E>(work: &PhaseOutcome<T, E>) -> Option<&T> {
    let failure = work.classify().err()?;
    if failure.interruption_after_return() != Some(Interruption::DeadlineExceeded)
        || failure.destruction_panic().is_some()
    {
        return None;
    }
    failure.returned_result().ok()
}

// Only checked finalization may issue this proof; applications cannot name it.
pub(super) struct RecoveredCompletion(JobCompletion);
impl RecoveredCompletion {
    pub(super) fn checked<T, E>(
        failure: &PhasedFailure<T, E, JobCompletion, E>,
        execution: JobExecution<'_>,
    ) -> Option<Self> {
        let PhasedFailure::Report(report) = failure else {
            return None;
        };
        late_value(report.work())?;
        let completion = report.finalization().classify().ok()?;
        // The kernel snapshots return before destroying the callback. Recheck
        // native authority after destruction too, before accepting recovery.
        if execution.invocation()?.has_ended()
            || tokio::time::Instant::now() >= execution.deadline().into()
        {
            return None;
        }
        Some(Self(completion.clone()))
    }
    pub(super) fn into_completion(self) -> JobCompletion {
        self.0
    }
}
