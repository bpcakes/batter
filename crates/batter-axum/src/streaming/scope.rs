//! What an owned response body's stream receives in place of the request's
//! context.

use crate::CorrelationId;
use batter_core::{lifecycle::ShutdownSignal, operation::OperationContext};
use std::fmt;

/// What a response body receives in place of the request's context.
///
/// Only [`AdmittedRequest::into_streaming_response`](crate::AdmittedRequest::into_streaming_response) creates one. Move it into
/// the stream that produces the body. A literal naming every field is
/// otherwise well typed, so privacy is its only error:
///
/// ```compile_fail,E0451
/// use batter_axum::{BodyScope, CorrelationId};
/// use batter_core::{lifecycle::ShutdownSignal, operation::OperationContext};
///
/// fn forge(
///     context: OperationContext,
///     shutdown: ShutdownSignal,
///     correlation_id: CorrelationId,
/// ) -> BodyScope {
///     BodyScope { context, shutdown, correlation_id }
/// }
/// ```
pub struct BodyScope {
    pub(super) context: OperationContext,
    pub(super) shutdown: ShutdownSignal,
    pub(super) correlation_id: CorrelationId,
}

impl BodyScope {
    /// The body's operation context. Its deadline is the body's total bound.
    /// It is cancelled by process forced cancellation and when the body ends
    /// or is destroyed, and not when the handler returns.
    ///
    /// Run each step of the stream through it, exactly as a handler runs work
    /// through [`AdmittedRequest::context`](crate::AdmittedRequest::context), and give a single step a tighter
    /// bound with [`OperationContext::child`]. A clone kept beyond the body
    /// observes the body's end as cancellation.
    ///
    /// The library checks this context before it polls the stream. When the
    /// body's own bound or forced cancellation is the cause, the library
    /// therefore ends the body with an error itself and a step's
    /// `Interrupted` result is normally never reached; it remains reachable
    /// for a step's own child deadline. Either way, end the stream with an
    /// error rather than normally, so the body does not end as a complete
    /// one (see [`AdmittedRequest::into_streaming_response`](crate::AdmittedRequest::into_streaming_response) for what a peer
    /// then sees). The library does not poll a stream again after its first
    /// error.
    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// Read-only drain and forced-cancellation observation.
    ///
    /// Ending at drain is the stream's decision: an event stream usually ends
    /// when [`ShutdownSignal::draining`] resolves so its client reconnects
    /// elsewhere, while a download continues through drain. Either way the
    /// library ends the body at forced cancellation.
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }

    /// The server-generated correlation of the request this body answers.
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
}

impl fmt::Debug for BodyScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BodyScope")
            .field("context", &self.context)
            .field("correlation_id", &self.correlation_id)
            .finish_non_exhaustive()
    }
}
