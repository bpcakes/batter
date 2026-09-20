use crate::{
    HttpFailure,
    observation::{ObservationState, observe_response},
};
use axum::{
    Json,
    extract::Request,
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use batter_core::telemetry::with_current_dispatch;
use serde::Serialize;
use tower_http::request_id::{MakeRequestId, MakeRequestUuid, RequestId};
use tracing::Instrument;

/// Marks the one wrapper that owns request correlation and response rewriting.
/// Observation ownership is separate because either wrapper can be outermost.
#[derive(Clone, Copy)]
struct OperationalScope;

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
/// routes, probes and fallback; [`crate::HttpBoundary`] does this. A plain
/// [`crate::observe_http`] may be inside or outside this wrapper because it
/// never short-circuits. Admission, deadlines, authentication and other
/// rejecting middleware must be inside this wrapper if their responses and
/// dropped futures require generated correlation. In particular,
/// [`crate::request_scope`] is supported only inside `operational_http`.
/// Observation ownership is distinct from operational ownership, so a manually
/// placed outer plain observer cannot suppress correlation setup. Later-added
/// routes bypass this wrapper, as with native `Router::layer`.
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
/// The request span has INFO level and target `batter::request`. A filter such as
/// `info,batter=warn,batter::request=info` retains nested request correlation while
/// suppressing Batter INFO completion events. HTTP events retain their typed ID
/// even when all INFO spans are disabled.
pub async fn operational_http(request: Request, next: Next) -> Response {
    operational_http_inner(request, next, false).await
}

/// Opt into one retained quota record alongside the existing HTTP observation.
///
/// Quota adapters take [`crate::quota_observation::QuotaRecorder`] before
/// application code, start immediately before the native check, and consume
/// the started writer with a terminal native fact. The outermost Batter HTTP
/// observer owns the only completion event, while a quota wrapper in either
/// supported nesting order publishes into its shared retained state. Completion
/// therefore retains quota facts even when admission or handler futures are
/// dropped. Fresh quota wrappers replace previously supplied records. No quota
/// backend, authentication, or admission is installed by this middleware alone.
///
/// ```
/// use axum::{Router, middleware, routing::get};
/// use batter_axum::operational_http_with_quota;
/// let app: Router = Router::new().route("/live", get(|| async { "live" }))
///     .layer(middleware::from_fn(operational_http_with_quota));
/// ```
pub async fn operational_http_with_quota(request: Request, next: Next) -> Response {
    operational_http_inner(request, next, true).await
}

async fn operational_http_inner(mut request: Request, next: Next, quota: bool) -> Response {
    with_current_dispatch(async move {
        let observation = request.extensions().get::<ObservationState>().cloned();
        if quota {
            let quota = crate::quota_observation::QuotaObservation::default();
            if let Some(observation) = &observation {
                observation.replace_quota(quota.clone());
            }
            request.extensions_mut().insert(quota);
        }
        if request.extensions().get::<OperationalScope>().is_some() {
            // The outermost operational wrapper owns correlation, the request
            // span and response rewriting. A nested supported wrapper can add
            // retained quota facts, but must not split that identity.
            return next.run(request).await;
        }
        if !quota {
            // At the first operational boundary, discard an extension supplied
            // outside Batter. Nested wrappers returned above and preserve the
            // shared record installed by an outer quota wrapper.
            request
                .extensions_mut()
                .remove::<crate::quota_observation::QuotaObservation>();
        }
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
        request.extensions_mut().insert(OperationalScope);
        if let Some(observation) = &observation {
            observation.replace_correlation(id.clone());
        }
        let span =
            tracing::info_span!(target: "batter::request", "batter.request", request_id = %id);
        let mut response = if observation.is_some() {
            next.run(request).instrument(span).await
        } else {
            observe_response(request, |request| next.run(request))
                .instrument(span)
                .await
        };
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
