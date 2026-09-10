//! Axum HTTP observation and request admission/deadline boundaries for Batter.
//! The workspace targets Unix backends. Windows is unsupported and not planned.
//!
//! This bounds obtaining a response, NOT streaming its body or a WebSocket
//! session. It does not detect disconnects that the transport does not surface
//! by dropping the handler future. Bodies, proxy trust and auth remain
//! application-owned. Trusted request correlation is explicitly opt-in. Keep liveness/readiness outside admission;
//! apply observation to the assembled router, including probes and fallback.

#![forbid(unsafe_code)]

mod correlation;
mod observation;
mod readiness;
mod serving;

pub use correlation::{CorrelationId, operational_http, render_infrastructure_failure};
pub use readiness::{ReadinessPolicy, ReadinessReason, dependency_readiness};
pub use serving::register_http;

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use batter::{
    ConfigurationError,
    lifecycle::{Readiness, ShutdownHandle},
    operation::{Interruption, OperationContext, OperationError},
    telemetry::with_current_dispatch,
};
use observation::observe_response;
use serde::Serialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::time::Instant;
use tracing::Level;

type FailureRenderer = dyn Fn(HttpFailure, &Parts) -> Response + Send + Sync;

/// Application-selected HTTP completion-event level, carried in response extensions.
///
/// [`observe_http`] and [`request_scope`] read this from the response returned by
/// their inner middleware. Without it, 5xx responses emit WARN and other responses
/// emit INFO. A future dropped before a response still emits WARN. This changes
/// only event severity: actual status, HTTP outcome, sanitized fields and the
/// number of events stay unchanged. The `batter.http` span remains at INFO.
/// Completion events carry their own sanitized HTTP fields even if that span is
/// disabled. Correlation supplied by application spans still depends on those spans.
/// Subscriber filtering still controls delivery, including DEBUG/TRACE visibility.
///
/// Set this explicitly in a handler, failure renderer, or middleware **inside**
/// observation. The extension is retained on the response and is not an HTTP
/// header. Request extensions, client headers and route names do not select levels.
/// Middleware replacing a response or its status must retain, replace or remove
/// this extension to match its policy. The status-only probes do not set an override.
/// Nested observers each read the retained override; an outer response rewrite
/// does not retroactively change an inner observer's status or event level.
///
/// For an application-identified expected readiness failure, retain 503 and
/// `http_outcome="server_error"` while selecting INFO. Alerts based on status/outcome
/// still need application-owned probe filtering.
///
/// ```
/// use axum::{Extension, Router, http::StatusCode, middleware, routing::get};
/// use batter_axum::{HttpObservationLevel, observe_http};
/// use tracing::Level;
///
/// let app: Router = Router::new()
///     .route("/ready", get(|| async {
///         // Application policy has identified this as expected unavailability.
///         (Extension(HttpObservationLevel(Level::INFO)), StatusCode::SERVICE_UNAVAILABLE)
///     }))
///     .layer(middleware::from_fn(observe_http));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpObservationLevel(pub Level);

/// Application-selected request deadline and lifecycle gate.
#[derive(Clone)]
pub struct RequestPolicy {
    shutdown: ShutdownHandle,
    budget: Duration,
    failure_renderer: Option<Arc<FailureRenderer>>,
}

impl RequestPolicy {
    /// A fixed server-side budget. No client-supplied deadline is trusted.
    pub fn new(shutdown: ShutdownHandle, budget: Duration) -> Result<Self, ConfigurationError> {
        validate_budget(budget)?;
        Ok(Self {
            shutdown,
            budget,
            failure_renderer: None,
        })
    }

    /// Render infrastructure failures in the application's existing wire format.
    ///
    /// The callback receives a snapshot of the original request parts at entry
    /// to this middleware, including application extensions. Install trusted
    /// correlation/identity extensions in an outer layer before this boundary;
    /// Batter does not authenticate header values or log these parts. Handler
    /// changes to extensions are not included. The callback owns its response
    /// status, safe body, and headers, and must not block the runtime thread.
    /// Without a callback, failures use the built-in Problem JSON response.
    pub fn with_failure_renderer<F>(mut self, renderer: F) -> Self
    where
        F: Fn(HttpFailure, &Parts) -> Response + Send + Sync + 'static,
    {
        self.failure_renderer = Some(Arc::new(renderer));
        self
    }

    /// Opt into the standard code/message/request_id JSON envelope.
    ///
    /// Install [`operational_http`] outside admission to supply the server ID.
    /// If absent, request_id is null; untrusted headers are never used as fallback.
    /// This replaces any previously configured renderer. Calling
    /// [`Self::with_failure_renderer`] afterward selects the custom renderer instead.
    /// See the runnable `http_service` example for the complete composition.
    pub fn with_infrastructure_json(self) -> Self {
        self.with_failure_renderer(|failure, parts| {
            render_infrastructure_failure(failure, parts.extensions.get::<CorrelationId>())
        })
    }

    fn render_failure(&self, failure: HttpFailure, parts: &Parts) -> Response {
        match &self.failure_renderer {
            Some(renderer) => renderer(failure, parts),
            None => failure.into_response(),
        }
    }
}

fn validate_budget(budget: Duration) -> Result<(), ConfigurationError> {
    const FIELD: &str = "HTTP request budget";
    if budget.is_zero() {
        return Err(ConfigurationError::Zero(FIELD));
    }
    if budget > Duration::from_secs(365 * 24 * 60 * 60)
        || Instant::now().checked_add(budget).is_none()
    {
        return Err(ConfigurationError::TooLarge(FIELD));
    }
    Ok(())
}

/// Sanitized infrastructure failures. Domain-to-HTTP mappings remain app-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpFailure {
    /// Not ready or draining.
    Unavailable,
    /// Process forced cancellation interrupted admitted work.
    Cancelled,
    /// The server's execution budget was exhausted.
    DeadlineExceeded,
    /// A concurrency/admission boundary rejected this request.
    Overloaded,
    /// An unexpected internal problem with no safe client detail.
    Internal,
}

impl HttpFailure {
    /// Default transport status; applications may choose their own mapping.
    pub const fn status(self) -> StatusCode {
        match self {
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Stable, sanitized failure classification, independent of the response body.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "service_unavailable",
            Self::Cancelled => "operation_cancelled",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Overloaded => "overloaded",
            Self::Internal => "internal_error",
        }
    }
}

#[derive(Serialize)]
struct ProblemBody {
    #[serde(rename = "type")]
    problem_type: &'static str,
    title: &'static str,
    status: u16,
    code: &'static str,
}

impl IntoResponse for HttpFailure {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ProblemBody {
            problem_type: "about:blank",
            title: status.canonical_reason().unwrap_or("Error"),
            status: status.as_u16(),
            code: self.code(),
        };
        (
            status,
            [
                (header::CONTENT_TYPE, "application/problem+json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            Json(body),
        )
            .into_response()
    }
}

/// Observe response construction independently of admission or deadlines.
///
/// Use with `axum::middleware::from_fn(observe_http)` on the assembled router,
/// after merging application routes, probes and fallback. Axum's `Router::layer`
/// runs after routing and only covers routes already assembled when it is called.
/// Routes appended afterward bypass it. A service wrapper outside the router
/// has no `MatchedPath` at entry and records `<unmatched>` even for matched routes.
/// Put trusted request identity outside observation, and rejection/status-changing
/// middleware inside it so their responses are observed too.
///
/// Records normalized method, matched route template (or `<unmatched>`), actual
/// response status, outcome and construction latency. Raw paths, queries, headers,
/// bodies and error contents are not logged. A polled future dropped before a
/// response emits `dropped` without a status; a never-polled future emits nothing.
/// These fields belong to the completion event as well as its INFO span, so an
/// enabled event retains them when filtering disables the HTTP span. The observer
/// retains that span or the current enabled application span at first poll for
/// execution and completion parenting; later ambient spans cannot replace it.
/// HTTP fields are never recorded into the fallback application span. Subscriber
/// and per-layer filters still determine whether a sink sees that context.
/// Polling and full future/span destruction retain the first-poll subscriber.
/// Handler panics propagate; an unwind before a response emits `dropped` without
/// a status. Batter neither catches the panic nor changes the default panic hook.
/// Body streaming, WebSockets and unreported disconnects are outside this lifetime.
/// Responses default to WARN for 5xx and INFO otherwise; [`HttpObservationLevel`]
/// in response extensions overrides only the completion-event level.
///
/// Each installed observer emits its own completion event. When adding this
/// layer, replace inner [`request_scope`] with [`request_admission`] to avoid
/// duplicate HTTP events. Operation completion events remain separate.
///
/// ```
/// use axum::{Extension, Router, http::StatusCode, middleware, routing::get};
/// use batter::{lifecycle::ShutdownHandle, operation::OperationContext};
/// use batter_axum::{RequestPolicy, liveness, observe_http, readiness, request_admission};
/// use std::time::Duration;
/// # fn main() -> Result<(), batter::ConfigurationError> {
/// let handle = ShutdownHandle::new();
/// let guarded = Router::new()
///     .route("/work", get(|Extension(context): Extension<OperationContext>| async move {
///         context.check().expect("admitted context");
///         "ok"
///     }))
///     .route_layer(middleware::from_fn_with_state(
///         RequestPolicy::new(handle.clone(), Duration::from_secs(2))?,
///         request_admission,
///     ));
/// let app: Router = Router::new()
///     .route("/live", get(liveness))
///     .route("/ready", get(readiness))
///     .with_state(handle)
///     .merge(guarded)
///     .fallback(|| async { StatusCode::NOT_FOUND })
///     .layer(middleware::from_fn(observe_http));
/// // Install application-owned server request identity outside observation.
/// # Ok(())
/// # }
/// ```
pub async fn observe_http(request: Request, next: Next) -> Response {
    with_current_dispatch(observe_response(request, |request| next.run(request))).await
}

/// Apply lifecycle admission and a fixed deadline without HTTP observations.
///
/// Use with `axum::middleware::from_fn_with_state(policy, request_admission)`.
/// Keep probes and fallback outside admission; see [`observe_http`] for a complete
/// composition example that observes those routes and admission rejections.
///
/// Inserts a native `Extension<OperationContext>` for admitted handlers. A Ready
/// state read is the admission point; a request racing shutdown may enter if
/// that read occurred first. Existing admitted work is not cancelled by drain.
/// The child context is cancelled on completion/drop of the response future.
/// The deadline ends at response construction, not body streaming or WebSockets.
/// Infrastructure timeouts use 503, not 408 (client upload timeout) or an
/// invented guarantee that retrying a write is safe. No Retry-After is added.
/// Custom rendering applies to every infrastructure failure generated here.
/// Operation events and full inner-future destruction retain the first-poll
/// subscriber. This middleware does not create a `batter.http` span or HTTP event.
pub async fn request_admission(
    State(policy): State<RequestPolicy>,
    request: Request,
    next: Next,
) -> Response {
    with_current_dispatch(request_admission_inner(policy, request, next)).await
}

/// Combined HTTP observation and admission/deadline compatibility entry point.
///
/// Use with `axum::middleware::from_fn_with_state(policy, request_scope)`.
/// Retains [`request_admission`]'s policy/context/rendering behavior and
/// [`observe_http`]'s sanitized response-construction events. Keep probes outside
/// this middleware's readiness gate. To observe the complete assembled router,
/// use outer [`observe_http`] and inner [`request_admission`] instead; adding an
/// observer around this combined entry point produces two HTTP observations.
pub async fn request_scope(
    State(policy): State<RequestPolicy>,
    request: Request,
    next: Next,
) -> Response {
    with_current_dispatch(observe_response(request, |request| {
        request_admission_inner(policy, request, next)
    }))
    .await
}

async fn request_admission_inner(policy: RequestPolicy, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    if policy.shutdown.readiness() != Readiness::Ready {
        return policy.render_failure(HttpFailure::Unavailable, &parts);
    }
    let Some(deadline) = Instant::now().checked_add(policy.budget) else {
        return policy.render_failure(HttpFailure::Internal, &parts);
    };
    // Preserve the caller's metadata only when its renderer needs it.
    let saved_parts = policy.failure_renderer.as_ref().map(|_| parts.clone());
    let mut request = Request::from_parts(parts, body);
    let parent = policy.shutdown.operation_token();
    let context = OperationContext::under(deadline, &parent);
    let failure = match context
        .run("http.response_construction", |scope| async move {
            request.extensions_mut().insert(scope);
            Ok::<_, Infallible>(next.run(request).await)
        })
        .await
    {
        Ok(response) => return response,
        Err(OperationError::Interrupted(Interruption::Cancelled)) => HttpFailure::Cancelled,
        Err(OperationError::Interrupted(Interruption::DeadlineExceeded)) => {
            HttpFailure::DeadlineExceeded
        }
        Err(OperationError::Failed(never)) => match never {},
    };
    match saved_parts {
        Some(parts) => policy.render_failure(failure, &parts),
        None => failure.into_response(),
    }
}

/// A readiness probe. Mount outside the guarded application router.
pub async fn readiness(State(handle): State<ShutdownHandle>) -> StatusCode {
    if handle.readiness() == Readiness::Ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

/// Process liveness only; not proof of dependency health or scheduler health.
pub async fn liveness() -> StatusCode {
    StatusCode::OK
}
