//! Axum HTTP observation, request admission, and browser transport boundaries for Batter.
//! The workspace targets Unix backends. Windows is unsupported and not planned.
//!
//! This bounds obtaining a response, NOT streaming its body or a WebSocket
//! session. It does not detect disconnects that the transport does not surface
//! by dropping the handler future. Bodies, proxy trust and auth remain
//! application-owned. Trusted request correlation is explicitly opt-in. The
//! [`browser`] module supplies validated cookie/header mechanics without an
//! account, session, authorization, CORS, or CSRF-token model.
//!
//! [`HttpBoundary`] is the canonical composition: it keeps probes outside
//! admission and installs correlation with the single HTTP observer outermost,
//! so the layer order is library-owned. Only the outermost observer emits a
//! completion event; nested Batter middleware contributes adapter facts to
//! that observer's shared retained state. The individual middlewares remain
//! available for compositions the boundary cannot express and document the
//! ordering they leave with the caller.

#![forbid(unsafe_code)]

mod boundary;
mod correlation;
mod observation;
mod readiness;
mod serving;

pub mod quota_observation;

/// Browser-carried opaque credential transport primitives.
pub mod browser;

pub use boundary::{
    AssembledHttp, BoundaryAssemblyError, GuardedRouter, HttpBoundary, ProbePath, ProbePathError,
    ProbeRegistrationError,
};
pub use correlation::{
    CorrelationId, operational_http, operational_http_with_quota, render_infrastructure_failure,
};
pub use readiness::{
    ReadinessDecision, ReadinessPolicy, default_readiness_level, dependency_readiness,
    readiness_status,
};
pub use serving::{register_http, register_http_in, register_http_with_connect_info_in};

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use batter_core::{
    ConfigurationError,
    lifecycle::{LifecycleStatus, OperationAdmission, Readiness},
    operation::{Interruption, OperationError},
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
/// Only the outermost observer emits a completion event, so it reads the
/// override retained on the response it actually returns.
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
///
/// Configuration methods take and return the policy by value, so a discarded
/// result would silently keep the previous configuration:
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use batter_axum::RequestPolicy;
/// fn discarded(policy: RequestPolicy) {
///     policy.with_infrastructure_json();
/// }
/// ```
#[derive(Clone)]
#[must_use = "retain the configured policy; a discarded result keeps the previous configuration"]
pub struct RequestPolicy {
    admission: OperationAdmission,
    budget: Duration,
    failure_renderer: Option<Arc<FailureRenderer>>,
}

/// Renders an interruption using the request's admission policy and original metadata.
///
/// [`request_admission`] installs this opaque capability on admitted requests.
/// A nested adapter can use it when its own operation observes cancellation or
/// deadline expiry before the outer admission operation is polled again. The
/// application cannot construct one with a different request or policy. The
/// captured metadata excludes Batter's private quota observation writer.
///
/// ```
/// use axum::{extract::Request, response::Response};
/// use batter_core::operation::Interruption;
/// use batter_axum::RequestInterruptionResponder;
///
/// fn nested_interruption(request: &Request, reason: Interruption) -> Option<Response> {
///     request.extensions().get::<RequestInterruptionResponder>()
///         .map(|responder| responder.render(reason))
/// }
/// ```
#[derive(Clone)]
pub struct RequestInterruptionResponder(InterruptionRendering);

#[derive(Clone)]
enum InterruptionRendering {
    Default,
    Custom {
        renderer: Arc<FailureRenderer>,
        original_parts: Arc<Parts>,
    },
}

fn renderer_parts(parts: &Parts) -> Parts {
    let mut snapshot = parts.clone();
    // The renderer needs request metadata, never the observer's private writer.
    snapshot
        .extensions
        .remove::<quota_observation::QuotaObservation>();
    snapshot
}

impl RequestInterruptionResponder {
    fn from_policy(policy: &RequestPolicy, original_parts: &Parts) -> Self {
        match &policy.failure_renderer {
            Some(renderer) => Self(InterruptionRendering::Custom {
                renderer: renderer.clone(),
                original_parts: Arc::new(renderer_parts(original_parts)),
            }),
            None => Self(InterruptionRendering::Default),
        }
    }

    /// Render cancellation or deadline expiry with the admission policy's envelope.
    ///
    /// The custom renderer receives the original request metadata captured
    /// before inner adapter extensions were inserted, without the private
    /// quota observation writer.
    pub fn render(&self, reason: Interruption) -> Response {
        let failure = match reason {
            Interruption::Cancelled => HttpFailure::Cancelled,
            Interruption::DeadlineExceeded => HttpFailure::DeadlineExceeded,
        };
        match &self.0 {
            InterruptionRendering::Default => failure.into_response(),
            InterruptionRendering::Custom {
                renderer,
                original_parts,
            } => renderer(failure, original_parts.as_ref()),
        }
    }
}

/// A validated budget for constructing an HTTP response.
///
/// This bounds the handler future through response construction, not response
/// body streaming. The checked value can be retained in application settings and
/// later handed to [`RequestPolicy::new`] without another fallible step.
///
/// ```
/// use batter_core::lifecycle::ShutdownHandle;
/// use batter_axum::{RequestPolicy, ResponseConstructionBudget};
/// use std::time::Duration;
///
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(2))?;
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let policy = RequestPolicy::new(control.operation_admission(), budget);
/// # let _ = policy;
/// # Ok::<(), batter_core::ConfigurationError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseConstructionBudget(Duration);

impl ResponseConstructionBudget {
    /// Validate a positive duration that Tokio can represent as a deadline.
    pub fn new(budget: Duration) -> Result<Self, ConfigurationError> {
        const FIELD: &str = "HTTP request budget";
        if budget.is_zero() {
            return Err(ConfigurationError::Zero(FIELD));
        }
        if budget > Duration::from_secs(365 * 24 * 60 * 60)
            || Instant::now().checked_add(budget).is_none()
        {
            return Err(ConfigurationError::TooLarge(FIELD));
        }
        Ok(Self(budget))
    }

    /// Return the validated duration for diagnostics or native handoff.
    pub const fn get(self) -> Duration {
        self.0
    }
}

impl RequestPolicy {
    /// Use a validated fixed server-side budget. No client deadline is trusted.
    ///
    /// ```compile_fail,E0308
    /// use batter_core::lifecycle::ShutdownHandle;
    /// use batter_axum::RequestPolicy;
    /// use std::time::Duration;
    ///
    /// fn cannot_build_from_raw(handle: ShutdownHandle, budget: Duration) {
    ///     let policy = RequestPolicy::new(handle.operation_admission(), budget);
    /// }
    /// ```
    ///
    /// Root shutdown control cannot be retained by request policy:
    ///
    /// ```compile_fail,E0308
    /// use batter_core::lifecycle::ShutdownHandle;
    /// use batter_axum::{RequestPolicy, ResponseConstructionBudget};
    ///
    /// fn cannot_retain_control(handle: ShutdownHandle, budget: ResponseConstructionBudget) {
    ///     let policy = RequestPolicy::new(handle, budget);
    /// }
    /// ```
    pub fn new(admission: OperationAdmission, budget: ResponseConstructionBudget) -> Self {
        Self {
            admission,
            budget: budget.0,
            failure_renderer: None,
        }
    }

    /// Render infrastructure failures in the application's existing wire format.
    ///
    /// The callback receives a snapshot of the original request parts at entry
    /// to this middleware, including application extensions but excluding
    /// Batter's private quota observation writer. Install trusted
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
            Some(renderer) => renderer(failure, &renderer_parts(parts)),
            None => failure.into_response(),
        }
    }
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
/// Only the outermost observer emits a completion event. [`observe_http`] and
/// [`operational_http`] can nest in either order because observation does not
/// short-circuit. [`request_scope`] also performs admission and can return or
/// expire without polling inner middleware, so `operational_http` must be
/// outside it when every response and completion event requires server
/// correlation. Operation completion events remain separate. Prefer
/// [`HttpBoundary`], which makes this order unchangeable.
///
/// ```
/// use axum::{Extension, Router, http::StatusCode, middleware, routing::get};
/// use batter_core::{lifecycle::ShutdownHandle, operation::OperationContext};
/// use batter_axum::{
///     RequestPolicy, ResponseConstructionBudget, liveness, observe_http,
///     readiness, request_admission,
/// };
/// use std::time::Duration;
/// # fn main() -> Result<(), batter_core::ConfigurationError> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let status = control.status();
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(2))?;
/// let guarded = Router::new()
///     .route("/work", get(|Extension(context): Extension<OperationContext>| async move {
///         context.check().expect("admitted context");
///         "ok"
///     }))
///     .fallback(|| async { StatusCode::NOT_FOUND })
///     .layer(middleware::from_fn_with_state(
///         RequestPolicy::new(control.operation_admission(), budget),
///         request_admission,
///     ));
/// let app: Router = Router::new()
///     .route("/live", get(liveness))
///     .route("/ready", get(readiness))
///     .with_state(status)
///     .merge(guarded)
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
/// Keep probes outside admission, but assemble the guarded fallback before
/// applying this layer so unmatched application requests cannot bypass the
/// lifecycle gate. [`HttpBoundary`] owns that placement. See [`observe_http`]
/// for a manual composition example that observes probes, guarded fallbacks and
/// admission rejections.
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
/// this middleware's readiness gate. Inside an outer observer this entry point
/// performs admission only; the outer observer emits the single HTTP event.
/// When combined with [`operational_http`], install `operational_http` as the
/// outer layer. This middleware can reject or time out without polling an inner
/// layer, so the reverse order cannot promise correlation on every completion.
/// Prefer [`HttpBoundary`], which owns this composition.
///
/// ```
/// use axum::{Router, middleware};
/// use batter_axum::{operational_http, request_scope, RequestPolicy};
///
/// fn ordered(guarded: Router, policy: RequestPolicy) -> Router {
///     guarded
///         .route_layer(middleware::from_fn_with_state(policy, request_scope))
///         // Router layers added later run outside earlier layers.
///         .layer(middleware::from_fn(operational_http))
/// }
/// ```
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
    let Some(deadline) = Instant::now().checked_add(policy.budget) else {
        return policy.render_failure(HttpFailure::Internal, &parts);
    };
    let Ok(context) = policy
        .admission
        .admit_root(batter_core::operation::RootDeadline::at(deadline))
    else {
        return policy.render_failure(HttpFailure::Unavailable, &parts);
    };
    // One request-scoped capability owns both the policy and the original parts.
    let responder = RequestInterruptionResponder::from_policy(&policy, &parts);
    let mut request = Request::from_parts(parts, body);
    request.extensions_mut().insert(responder.clone());
    match context
        .run("http.response_construction", |scope| async move {
            request.extensions_mut().insert(scope);
            Ok::<_, Infallible>(next.run(request).await)
        })
        .await
    {
        Ok(response) => response,
        Err(OperationError::Interrupted(reason)) => responder.render(reason),
        Err(OperationError::Failed(never)) => match never {},
    }
}

/// A status-only readiness probe. Mount outside the guarded application router;
/// [`HttpBoundary::with_readiness`] mounts the dependency-aware probe there.
pub async fn readiness(State(status): State<LifecycleStatus>) -> StatusCode {
    if status.readiness() == Readiness::Ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

/// Process liveness only; not proof of dependency health or scheduler health.
pub async fn liveness() -> StatusCode {
    StatusCode::OK
}
