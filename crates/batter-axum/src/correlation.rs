use crate::{HttpFailure, observation::observe_response};
use axum::{
    Json,
    extract::Request,
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use batter::telemetry::with_current_dispatch;
use serde::Serialize;
use tower_http::request_id::{MakeRequestId, MakeRequestUuid, RequestId};
use tracing::Instrument;

/// Server-generated UUID correlation, never authentication or authorization.
///
/// [`operational_http`] replaces incoming headers and identity extensions before
/// inserting this opaque value. Read it with `Extension<CorrelationId>` and pass
/// it explicitly to nested application operations or metadata. A cloned ID can
/// identify related work; it grants no authority and does not extend its lifetime.
/// It is neither parsed from inbound trace context nor accepted from a proxy.
#[derive(Clone, Debug)]
pub struct CorrelationId(RequestId);

impl CorrelationId {
    /// Generated ASCII UUID for explicit metadata propagation or safe rendering.
    pub fn as_str(&self) -> &str {
        self.0
            .header_value()
            .to_str()
            .expect("generated ASCII UUID")
    }
}

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Opt into server correlation and one HTTP response-construction observation.
///
/// Apply with `middleware::from_fn(operational_http)` **after** assembling all
/// routes, probes and fallback. Use only `request_admission` inside guarded routes;
/// adding `observe_http` or `request_scope` would add another observation.
/// Later-added routes bypass this wrapper, as with native `Router::layer`.
///
/// Replaces all x-request-id header values, Tower's RequestId and any previous
/// CorrelationId extension. Inner response IDs are also replaced. No other
/// incoming identity or trace headers are interpreted. The generated ID belongs
/// to the HTTP completion event even when INFO spans are disabled; nested native
/// tracing uses the request span and remains subject to subscriber filtering.
/// Full inner destruction retains the entry dispatch. Streaming bodies and
/// detached tasks are outside this response-construction lifetime.
///
/// ```
/// use axum::{Extension, Router, middleware, routing::get};
/// use batter_axum::{CorrelationId, operational_http};
/// let app: Router = Router::new()
///     .route("/", get(|Extension(id): Extension<CorrelationId>| async move {
///         id.as_str().to_owned()
///     }))
///     .layer(middleware::from_fn(operational_http));
/// ```
pub async fn operational_http(mut request: Request, next: Next) -> Response {
    with_current_dispatch(async move {
        request.headers_mut().remove("x-request-id");
        request.extensions_mut().remove::<RequestId>();
        request.extensions_mut().remove::<CorrelationId>();
        let id = CorrelationId(
            MakeRequestUuid
                .make_request_id(&request)
                .expect("UUID generator always returns an ID"),
        );
        request
            .headers_mut()
            .insert("x-request-id", id.0.header_value().clone());
        request.extensions_mut().insert(id.0.clone());
        request.extensions_mut().insert(id.clone());
        let span = tracing::info_span!(target: "batter", "batter.request", request_id = %id);
        let mut response = observe_response(request, |request| next.run(request))
            .instrument(span)
            .await;
        response
            .headers_mut()
            .insert("x-request-id", id.0.into_header_value());
        response
    })
    .await
}

#[derive(Serialize)]
struct InfrastructureBody<'a> {
    code: &'static str,
    message: &'static str,
    request_id: Option<&'a str>,
}

/// Opt-in standard infrastructure JSON, also usable by application mappings.
///
/// The wire contract is `{code: string, message: string, request_id: string|null}`,
/// application/json with Cache-Control: no-store and [`HttpFailure::status`].
/// Codes use [`HttpFailure::code`]; messages below are fixed sanitized English.
/// A missing typed ID renders null, never a header-derived identity. The
/// operational wrapper sets the response ID header. Domain errors remain owned
/// by the application; this function never accepts or formats a cause.
/// Existing `HttpFailure::into_response` Problem JSON is unchanged.
/// See `http_service` for handler and admission use of the same envelope.
pub fn render_infrastructure_failure(failure: HttpFailure, id: Option<&CorrelationId>) -> Response {
    let message = match failure {
        HttpFailure::Unavailable => "The service is unavailable",
        HttpFailure::Cancelled => "The operation was cancelled",
        HttpFailure::DeadlineExceeded => "The operation exceeded its time budget",
        HttpFailure::Overloaded => "The service is at capacity",
        HttpFailure::Internal => "An internal error occurred",
    };
    (
        failure.status(),
        [(header::CACHE_CONTROL, "no-store")],
        Json(InfrastructureBody {
            code: failure.code(),
            message,
            request_id: id.map(CorrelationId::as_str),
        }),
    )
        .into_response()
}
