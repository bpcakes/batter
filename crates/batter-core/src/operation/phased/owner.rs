use super::{PhasedFailure, PhasedSuccess};
use crate::{completion::wait_published, operation::OperationContext};
use tokio::sync::watch;

type Completion<T, E, U, F> = Result<PhasedSuccess<T, U>, PhasedFailure<T, E, U, F>>;
type Receiver<T, E, U, F> = watch::Receiver<Option<Completion<T, E, U, F>>>;

/// Sole work-cancellation owner of a phased operation. Drop cancels work only.
///
/// ```compile_fail,E0599
/// use batter_core::operation::RunningPhasedOperation;
/// fn duplicate(owner: RunningPhasedOperation<(), (), (), ()>) { owner.clone(); }
/// ```
/// The owned coordinator retains finalization independently on a live runtime.
#[must_use = "retain through completion; dropping the owner requests work cancellation"]
pub struct RunningPhasedOperation<T, E, U, F> {
    context: OperationContext,
    observer: PhasedObserver<T, E, U, F>,
}

impl<T, E, U, F> RunningPhasedOperation<T, E, U, F> {
    pub(super) fn new(context: OperationContext, completion: Receiver<T, E, U, F>) -> Self {
        Self {
            context,
            observer: PhasedObserver { completion },
        }
    }

    /// Cancel work, leaving finalization and the parent unaffected.
    pub fn cancel(&self) {
        self.context.cancel();
    }

    /// Retain completion observation without retaining work ownership.
    pub fn observer(&self) -> PhasedObserver<T, E, U, F> {
        self.observer.clone()
    }

    /// Wait for both phases. Dropping this borrowed waiter changes neither.
    ///
    /// # Errors
    /// Returns [`PhasedFailure::Report`] if either phase failed, retaining both
    /// outcomes. A coordinator failure is [`PhasedFailure::Coordinator`].
    /// An `Ok` value carries both accepted phase values.
    /// Migration: `RunningPhasedOperation::wait` now returns [`PhasedSuccess`].
    /// # Panics
    /// Panics if runtime death destroys publication before it completes.
    pub async fn wait(&self) -> Result<PhasedSuccess<T, U>, PhasedFailure<T, E, U, F>> {
        self.observer.wait().await
    }
}

impl<T, E, U, F> Drop for RunningPhasedOperation<T, E, U, F> {
    fn drop(&mut self) {
        self.context.cancel();
    }
}

/// Cloneable report observation that cannot cancel either phase.
pub struct PhasedObserver<T, E, U, F> {
    completion: Receiver<T, E, U, F>,
}

impl<T, E, U, F> Clone for PhasedObserver<T, E, U, F> {
    fn clone(&self) -> Self {
        Self {
            completion: self.completion.clone(),
        }
    }
}

impl<T, E, U, F> PhasedObserver<T, E, U, F> {
    /// Wait for the same retained outcome; cancelling a waiter has no effect.
    ///
    /// # Errors
    /// Returns the complete failed report or a coordinator join failure.
    /// `Ok` is returned only when both phases succeeded.
    /// Migration: `PhasedObserver::wait` now returns [`PhasedSuccess`].
    /// # Panics
    /// Panics if runtime death destroys publication before it completes.
    pub async fn wait(&self) -> Result<PhasedSuccess<T, U>, PhasedFailure<T, E, U, F>> {
        wait_published(self.completion.clone())
            .await
            .expect("owned phase monitor retains publication")
    }
}
