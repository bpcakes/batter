//! Work that continues an operation lifecycle admission already let in.

use super::{OperationAdmission, Readiness, ShutdownSignal, state::Shared};
use crate::operation::{OperationContext, OperationOwner, RootDeadline};
use std::sync::Arc;

/// Lower-level adapter seam: admit one operation exactly as
/// [`OperationAdmission::admit_root`] does, and also return the proof that
/// lets the adapter start work continuing it.
///
/// It is not the protected path. The proof leaves the caller obligations that
/// [`OperationContinuation`] lists; an application starts work with
/// `admit_root`, and a response body that outlives its handler goes through
/// its adapter, such as Axum's `AdmittedRequest::into_streaming_response`.
pub fn admit_root_with_continuation(
    admission: &OperationAdmission,
    deadline: RootDeadline,
) -> Result<(OperationOwner, OperationContinuation), Readiness> {
    let (owner, shared) = admission.admit_with_state(deadline)?;
    let continuation = OperationContinuation::admitted(shared, owner.context().clone());
    Ok((owner, continuation))
}

/// Proof that lifecycle admission let an operation in, with authority to start
/// work that continues it under its own deadline.
///
/// Lower-level adapter seam. Only a successful
/// [`admit_root_with_continuation`] creates one, so holding it means the
/// readiness decision was already made. [`Self::start`] therefore does not
/// read readiness again: a response body that begins while the process drains
/// belongs to a request admitted before drain, and drain stops admission
/// without cancelling admitted work.
///
/// The continuation is not a child of the admitted operation. It has its own
/// deadline, which may be later, and it outlives the operation's own
/// cancellation. It inherits only process forced cancellation, exactly as the
/// admitted root did. One proof starts at most one root: it is not `Clone`,
/// and [`Self::start`] consumes it, so a holder cannot keep starting work after
/// drain. A start after the admitted operation was cancelled or passed its
/// deadline returns an already-cancelled owner. The caller owns what the proof
/// cannot observe: an admitted owner abandoned without cancellation stays live
/// until its deadline, so when the continued work starts, and that it ends,
/// remain the holder's obligations. A supported adapter keeps the proof
/// private to one admitted request and starts it only while that request is
/// still being answered.
///
/// ```
/// use batter_core::{
///     lifecycle::{ShutdownHandle, low_level::admit_root_with_continuation},
///     operation::RootDeadline,
/// };
/// use std::time::Duration;
///
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let (admission, second) = (control.operation_admission(), Duration::from_secs(1));
/// let (request, continuation) =
///     admit_root_with_continuation(&admission, RootDeadline::after(second)?)
///         .expect("a ready process admits the operation");
/// let (abandoned, late) = admit_root_with_continuation(&admission, RootDeadline::after(second)?)
///     .expect("a ready process admits the operation");
/// // Read-only observation stays available after the proof is spent.
/// let signal = continuation.signal();
/// // Drain closes admission, but the admitted operation may still continue.
/// control.request();
/// let body = continuation.start(RootDeadline::after(second * 60)?);
/// assert!(body.context().deadline() > request.context().deadline());
/// request.cancel();
/// assert!(body.context().check().is_ok());
/// assert!(signal.is_draining());
/// // A proof that outlived its operation starts nothing live.
/// abandoned.cancel();
/// assert!(late.start(RootDeadline::after(second * 60)?).context().check().is_err());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
///
/// A spent proof cannot start a second root, and it cannot be copied first.
/// Each snippet below fails only for that reason:
///
/// ```compile_fail,E0382
/// use batter_core::{lifecycle::low_level::OperationContinuation, operation::RootDeadline};
///
/// fn twice(continuation: OperationContinuation, deadline: RootDeadline) {
///     let _first = continuation.start(deadline);
///     let _second = continuation.start(deadline);
/// }
/// ```
///
/// ```compile_fail,E0308
/// use batter_core::lifecycle::low_level::OperationContinuation;
///
/// fn keep(continuation: &OperationContinuation) -> OperationContinuation {
///     continuation.clone()
/// }
/// ```
///
/// Admission is the only source. Each forgery below is otherwise well typed,
/// so privacy is its only error (stable rustdoc does not check the codes):
///
/// ```compile_fail,E0624
/// use batter_core::lifecycle::low_level::OperationContinuation;
///
/// fn forge() -> OperationContinuation {
///     OperationContinuation::admitted(todo!(), todo!())
/// }
/// ```
///
/// ```compile_fail,E0451
/// use batter_core::lifecycle::low_level::OperationContinuation;
///
/// fn forge() -> OperationContinuation {
///     OperationContinuation { shared: todo!(), admitted: todo!() }
/// }
/// ```
pub struct OperationContinuation {
    shared: Arc<Shared>,
    /// The admitted operation's context, observed once, when the proof is
    /// spent.
    admitted: OperationContext,
}

impl OperationContinuation {
    fn admitted(shared: Arc<Shared>, admitted: OperationContext) -> Self {
        Self { shared, admitted }
    }

    /// Start a root for the continued work, cancelled by process forced
    /// cancellation and by its returned owner, never by the admitted
    /// operation's own completion.
    ///
    /// It consumes the proof, so each admission continues into one root. A
    /// past deadline is accepted and fails at the operation boundary. After
    /// forced cancellation, or once the admitted operation was cancelled or
    /// passed its deadline, the returned context is already cancelled.
    pub fn start(self, deadline: RootDeadline) -> OperationOwner {
        let owner = OperationOwner::under(deadline, &self.shared.operation_token());
        if self.admitted.check().is_err() {
            owner.cancel();
        }
        owner
    }

    /// Read-only drain and forced-cancellation observation for the continued
    /// work, which decides for itself whether to end at drain.
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal::observing(self.shared.clone())
    }
}

impl std::fmt::Debug for OperationContinuation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OperationContinuation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lifecycle::ShutdownHandle, operation::Interruption};
    use std::time::Duration;

    const SECOND: Duration = Duration::from_secs(1);

    fn ready() -> (ShutdownHandle, OperationAdmission) {
        let (control, approval) = ShutdownHandle::new_with_readiness_approval();
        approval.approve();
        let admission = control.operation_admission();
        (control, admission)
    }

    fn admitted(admission: &OperationAdmission) -> (OperationOwner, OperationContinuation) {
        let deadline = RootDeadline::after(SECOND).expect("a positive budget");
        admit_root_with_continuation(admission, deadline).expect("a ready process admits")
    }

    fn started(continuation: OperationContinuation) -> Result<(), Interruption> {
        let deadline = RootDeadline::after(SECOND * 60).expect("a positive budget");
        continuation.start(deadline).context().check()
    }

    /// Start honours the admitted operation as it is when the proof is spent:
    /// live, cancelled or expired.
    #[tokio::test(start_paused = true)]
    async fn start_refuses_once_the_admitted_operation_ended() {
        let (_control, admission) = ready();
        let (live, live_proof) = admitted(&admission);
        let (cancelled, cancelled_proof) = admitted(&admission);
        let (_expired, expired_proof) = admitted(&admission);

        cancelled.cancel();
        assert_eq!(
            started(cancelled_proof),
            Err(Interruption::Cancelled),
            "cancelled"
        );
        assert_eq!(started(live_proof), Ok(()), "a live operation continues");
        // Once started, the continued root outlives its admitted operation.
        live.cancel();

        tokio::time::advance(SECOND * 2).await;
        assert_eq!(
            started(expired_proof),
            Err(Interruption::Cancelled),
            "expired"
        );
    }

    /// Process forced cancellation reaches a root started from a proof of a
    /// still live operation.
    #[tokio::test(start_paused = true)]
    async fn start_after_forced_cancellation_is_already_cancelled() {
        let (_control, admission) = ready();
        let (_live, proof) = admitted(&admission);
        proof.shared.force_cancel();
        assert_eq!(started(proof), Err(Interruption::Cancelled), "forced");
    }
}
