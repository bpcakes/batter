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
//! so the layer order is library-owned. Named [`RouteGroup`]s give sets of
//! guarded routes their own [`RequestPolicy`] and optional [`BrowserPolicy`]
//! inside that same fixed order. A router built by another router builder joins
//! through [`GuardedRouter::from_router`] and serves only its declared routes. Only the outermost observer emits a
//! completion event; nested Batter middleware contributes adapter facts to
//! that observer's shared retained state. Guarded handlers extract the admitted
//! request's context, correlation and interruption responder as one
//! [`AdmittedRequest`]. Assembly is sealed: the result is consumed into
//! protected serving or into an opaque [`InProcessClient`], never back into a
//! [`Router`](axum::Router). The individual middlewares and registration
//! helpers remain available under [`low_level`] for compositions the boundary
//! cannot express, and document the ordering they leave with the caller.

#![forbid(unsafe_code)]

mod admitted;
mod boundary;
mod correlation;
mod observation;
mod readiness;
mod serving;

pub mod low_level;

pub mod quota_observation;

/// Browser-carried opaque credential transport primitives.
pub mod browser;

pub use admitted::{AdmittedRequest, AdmittedRequestRejection};
pub use boundary::{
    AssembledHttp, BoundaryAssemblyError, BrowserPolicy, GroupPolicy, GuardedRouter, HttpBoundary,
    InProcessClient, ProbePath, ProbePathError, ProbeRegistrationError, RouteGroup,
    RouteGroupError, RouteInventory, RouteInventoryError,
};
pub use correlation::{CorrelationId, render_infrastructure_failure};
pub use readiness::{
    ReadinessDecision, ReadinessPolicy, default_readiness_level, readiness_status,
};

use axum::{
    Json,
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use batter_core::{ConfigurationError, lifecycle::OperationAdmission, operation::Interruption};
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use tracing::Level;

type FailureRenderer = dyn Fn(HttpFailure, &Parts) -> Response + Send + Sync;

/// Application-selected HTTP completion-event level, carried in response extensions.
///
/// [`low_level::observe_http`] and [`low_level::request_scope`] read this from the response returned by
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
/// use batter_axum::{HttpObservationLevel, low_level::observe_http};
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
/// [`low_level::request_admission`] creates this opaque capability for each admitted
/// request. Handlers and application layers take it from
/// [`AdmittedRequest::interruption_responder`] when their own operation observes
/// cancellation or deadline expiry before the outer admission operation is
/// polled again; Batter's nested adapters read the same value from the request
/// extensions. The application cannot construct one with a different request
/// or policy. The captured metadata excludes Batter's private quota writer,
/// shared observation state, operational ownership marker and admission record.
/// Public correlation and application extensions remain available; redispatch
/// cannot mutate the original observer or admit the new request.
///
/// ```
/// use axum::response::Response;
/// use batter_core::operation::Interruption;
/// use batter_axum::AdmittedRequest;
///
/// fn nested_interruption(admitted: &AdmittedRequest, reason: Interruption) -> Response {
///     admitted.interruption_responder().render(reason)
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

impl RequestInterruptionResponder {
    fn from_policy(policy: &RequestPolicy, original_parts: &Parts) -> Self {
        match &policy.failure_renderer {
            Some(renderer) => Self(InterruptionRendering::Custom {
                renderer: renderer.clone(),
                original_parts: Arc::new(correlation::renderer_parts(original_parts.clone())),
            }),
            None => Self(InterruptionRendering::Default),
        }
    }

    /// Render cancellation or deadline expiry with the admission policy's envelope.
    ///
    /// The custom renderer receives the original request metadata captured
    /// before inner adapter extensions were inserted, without private quota,
    /// observation or operational ownership state.
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
    /// Batter's private quota writer, shared observation state, operational
    /// ownership marker and admission record. Redispatching cloned metadata
    /// creates independent observation and correlation under an operational
    /// wrapper and is admitted only by admission of its own. Install trusted
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
    /// Install [`low_level::operational_http`] outside admission to supply the server ID.
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
            Some(renderer) => renderer(failure, &correlation::renderer_parts(parts.clone())),
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
