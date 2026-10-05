//! The admitted request's context, read through one typed extractor.

use crate::{CorrelationId, HttpFailure, RequestInterruptionResponder, correlation};
use axum::{
    extract::FromRequestParts,
    http::{Extensions, request::Parts},
    response::{IntoResponse, Response},
};
use batter_core::operation::OperationContext;
use std::{error::Error, fmt};

/// What admission records for one admitted request.
///
/// Only this crate can name the type, so no application layer can insert,
/// replace or remove the record by type. Admission replaces any record a
/// request arrives with, and the renderer filter removes it from renderer
/// metadata so that redispatched metadata is not admitted.
#[derive(Clone)]
pub(crate) struct Admission {
    context: OperationContext,
    correlation_id: CorrelationId,
    interruption: RequestInterruptionResponder,
}

/// Record the admitted request for [`AdmittedRequest`].
///
/// Only a request inside the operational wrapper that generated its server
/// correlation is recorded. Without one, any record copied from another
/// request is removed and nothing replaces it, so the extractor rejects.
pub(crate) fn record(
    extensions: &mut Extensions,
    context: OperationContext,
    interruption: RequestInterruptionResponder,
) {
    extensions.remove::<Admission>();
    if let Some(correlation_id) = correlation::server_correlation(extensions) {
        extensions.insert(Admission {
            context,
            correlation_id,
            interruption,
        });
    }
}

/// The request that lifecycle admission let in: its operation context, its
/// server correlation and its interruption responder, extracted as one value.
///
/// Extract it in handlers and route layers of an
/// [`HttpBoundary`](crate::HttpBoundary), which admits every guarded route and
/// fallback inside server correlation, instead of reading request extensions.
/// Outside admission the extractor answers [`AdmittedRequestRejection`], a
/// sanitized 500, rather than Axum's missing-extension text, which names the
/// missing type.
///
/// ```
/// use axum::{
///     response::{IntoResponse, Response},
///     routing::get,
/// };
/// use batter_axum::{AdmittedRequest, GuardedRouter};
/// use batter_core::operation::OperationError;
/// use std::{convert::Infallible, time::Duration};
///
/// async fn work(admitted: AdmittedRequest) -> Response {
///     let request_id = admitted.correlation_id().to_string();
///     match admitted
///         .context()
///         .run("demo.read", |_scope| async {
///             tokio::time::sleep(Duration::from_millis(5)).await;
///             Ok::<_, Infallible>("ok")
///         })
///         .await
///     {
///         Ok(body) => ([("x-demo-request", request_id)], body).into_response(),
///         // The admission policy's own envelope, as for its other failures.
///         Err(OperationError::Interrupted(reason)) => {
///             admitted.interruption_responder().render(reason)
///         }
///         Err(OperationError::Failed(never)) => match never {},
///     }
/// }
///
/// let guarded: GuardedRouter = GuardedRouter::new().route("/work", get(work));
/// # let _ = guarded;
/// ```
///
/// Only admission creates the value. Its field is private and it has no
/// constructor:
///
/// ```compile_fail,E0423
/// use batter_axum::AdmittedRequest;
/// use batter_core::operation::OperationContext;
///
/// fn forge(context: OperationContext) -> AdmittedRequest {
///     AdmittedRequest(context)
/// }
/// ```
///
/// Request extensions accept only `Clone` values, and this one is not
/// `Clone`, so an application cannot insert an extracted value into another
/// request:
///
/// ```compile_fail,E0277
/// use axum::extract::Request;
/// use batter_axum::AdmittedRequest;
///
/// fn transplant(admitted: AdmittedRequest, mut request: Request) {
///     request.extensions_mut().insert(admitted);
/// }
/// ```
///
/// The extractor reads admission's private record, never the native
/// `OperationContext`, [`CorrelationId`] or [`RequestInterruptionResponder`]
/// extensions. Admission and [`crate::low_level::operational_http`] still insert those for
/// Batter's adapters and existing handlers, but they are ordinary extensions:
/// any layer can insert or replace them, and reading one where it is absent
/// fails with Axum's missing-extension text. Inserting them cannot make this
/// extractor succeed, and replacing them does not change what it returns.
///
/// Admission records the value only inside the [`operational_http`] wrapper
/// that generated the request's correlation, as
/// [`HttpBoundary`](crate::HttpBoundary) and Runlimit's protected assembly
/// install it. A lower-level [`request_admission`] or [`request_scope`]
/// without that wrapper outside it records nothing, so its handlers receive
/// the rejection. Probe renderers, failure and interruption renderers and
/// browser rejection renderers receive request metadata without the record.
/// Copying a whole extension map from an admitted request into another
/// request carries the record too, like every other value in the map.
///
/// [`operational_http`]: crate::low_level::operational_http
/// [`request_admission`]: crate::low_level::request_admission
/// [`request_scope`]: crate::low_level::request_scope
pub struct AdmittedRequest(Admission);

impl AdmittedRequest {
    /// The admitted request's operation context, which carries its
    /// response-construction deadline and the cancellation that admission
    /// signals when the response future finishes or is dropped, or when the
    /// process forces cancellation.
    ///
    /// Pass it to nested operations. Clones observe and execute without
    /// authority to cancel the request.
    pub fn context(&self) -> &OperationContext {
        &self.0.context
    }

    /// The server-generated correlation of this request: the identifier on its
    /// `x-request-id` response header and its completion event.
    ///
    /// It identifies related work and grants no authority.
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.0.correlation_id
    }

    /// Render cancellation or deadline expiry with the admission policy's
    /// failure renderer, as admission does when the request itself is
    /// interrupted.
    pub fn interruption_responder(&self) -> &RequestInterruptionResponder {
        &self.0.interruption
    }
}

impl fmt::Debug for AdmittedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedRequest")
            .field("context", &self.0.context)
            .field("correlation_id", &self.0.correlation_id)
            .finish_non_exhaustive()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AdmittedRequest {
    type Rejection = AdmittedRequestRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Admission>()
            .cloned()
            .map(Self)
            .ok_or(AdmittedRequestRejection { _private: () })
    }
}

/// Sanitized rejection for extracting [`AdmittedRequest`] from a request that
/// admission did not record.
///
/// It answers exactly as [`HttpFailure::Internal`] does: status 500 with the
/// fixed `application/problem+json` body and `Cache-Control: no-store`, naming
/// no type, route or request field. Only a composition error reaches it, so the
/// completion event keeps the default WARN severity for 5xx responses. To use
/// the application's own envelope instead, extract
/// `Result<AdmittedRequest, AdmittedRequestRejection>` and render the error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedRequestRejection {
    _private: (),
}

impl fmt::Display for AdmittedRequestRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("request was not admitted inside server correlation")
    }
}

impl Error for AdmittedRequestRejection {}

impl IntoResponse for AdmittedRequestRejection {
    fn into_response(self) -> Response {
        HttpFailure::Internal.into_response()
    }
}
