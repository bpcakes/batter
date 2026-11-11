//! Work that continues an operation lifecycle admission already let in.

use super::{ShutdownSignal, state::Shared};
use crate::operation::{OperationOwner, RootDeadline};
use std::sync::Arc;

/// Proof that lifecycle admission let an operation in, with authority to start
/// work that continues it under its own deadline.
///
/// Lower-level adapter seam. Only a successful
/// [`OperationAdmission::admit_root_with_continuation`](super::OperationAdmission::admit_root_with_continuation)
/// creates one, so holding it means the readiness decision was already made.
/// [`Self::start`] therefore does not read readiness again: a response body
/// that begins while the process drains belongs to a request admitted before
/// drain, and drain stops admission without cancelling admitted work.
///
/// The continuation is not a child of the admitted operation. It has its own
/// deadline, which may be later, and it outlives the operation's own
/// cancellation. It inherits only process forced cancellation, exactly as the
/// admitted root did. One proof starts at most one root: it is not `Clone`,
/// and [`Self::start`] consumes it, so a holder cannot keep starting work after
/// drain. The caller owns everything else: when the continued work starts and
/// that it ends. A supported adapter keeps it private to one admitted request
/// and starts it only while that request is still being answered.
///
/// ```
/// use batter_core::{lifecycle::ShutdownHandle, operation::RootDeadline};
/// use std::time::Duration;
///
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let (request, continuation) = control
///     .operation_admission()
///     .admit_root_with_continuation(RootDeadline::after(Duration::from_secs(1))?)
///     .expect("a ready process admits the operation");
/// // Read-only observation stays available after the proof is spent.
/// let signal = continuation.signal();
/// // Drain closes admission, but the admitted operation may still continue.
/// control.request();
/// let body = continuation.start(RootDeadline::after(Duration::from_secs(60))?);
/// assert!(body.context().deadline() > request.context().deadline());
/// request.cancel();
/// assert!(body.context().check().is_ok());
/// assert!(signal.is_draining());
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
///
/// A spent proof cannot start a second root, and it cannot be copied first.
/// Each snippet below fails only for that reason:
///
/// ```compile_fail,E0382
/// use batter_core::{lifecycle::OperationContinuation, operation::RootDeadline};
///
/// fn twice(continuation: OperationContinuation, deadline: RootDeadline) {
///     let _first = continuation.start(deadline);
///     let _second = continuation.start(deadline);
/// }
/// ```
///
/// ```compile_fail,E0308
/// use batter_core::lifecycle::OperationContinuation;
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
/// use batter_core::lifecycle::OperationContinuation;
///
/// fn forge() -> OperationContinuation {
///     OperationContinuation::admitted(todo!())
/// }
/// ```
///
/// ```compile_fail,E0451
/// use batter_core::lifecycle::OperationContinuation;
///
/// fn forge() -> OperationContinuation {
///     OperationContinuation { shared: todo!() }
/// }
/// ```
pub struct OperationContinuation {
    shared: Arc<Shared>,
}

impl OperationContinuation {
    pub(super) fn admitted(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// Start a root for the continued work, cancelled by process forced
    /// cancellation and by its returned owner, never by the admitted
    /// operation's own completion.
    ///
    /// It consumes the proof, so each admission continues into one root. A
    /// past deadline is accepted and fails at the operation boundary. After
    /// forced cancellation the returned context is already cancelled.
    pub fn start(self, deadline: RootDeadline) -> OperationOwner {
        OperationOwner::under(deadline, &self.shared.operation_token())
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
