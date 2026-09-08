//! Axum admission and request-operation boundary for Batter.
//! The workspace targets Unix backends. Windows is unsupported and not planned.
//!
//! This bounds obtaining a response, NOT streaming its body or a WebSocket
//! session. It does not detect disconnects that the transport does not surface
//! by dropping the handler future. Bodies, proxy trust, auth, and request IDs
//! are deliberately not interpreted. Keep liveness/readiness outside this layer.

#![forbid(unsafe_code)]

use axum::{
    Json,
    extract::{MatchedPath, Request, State},
    http::{Method, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use batter::{
    ConfigurationError,
    lifecycle::{Readiness, ShutdownHandle},
    operation::{Interruption, OperationContext, OperationError},
    telemetry::with_current_dispatch,
};
use serde::Serialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::time::Instant;
use tracing::Instrument;

type FailureRenderer = dyn Fn(HttpFailure, &Parts) -> Response + Send + Sync;

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

/// Use with `axum::middleware::from_fn_with_state(policy, request_scope)`.
///
/// Inserts a native `Extension<OperationContext>` for admitted handlers. A Ready
/// state read is the admission point; a request racing shutdown may enter if
/// that read occurred first. Existing admitted work is not cancelled by drain.
/// The child context is cancelled on completion/drop of the response future.
/// Infrastructure timeouts use 503, not 408 (client upload timeout) or an
/// invented guarantee that retrying a write is safe. No Retry-After is added.
/// Custom rendering applies to every infrastructure failure generated here.
/// Separate HTTP completion events record the actual response status (including
/// application errors), matched route, normalized method, and construction time.
/// Raw paths, queries, headers, and bodies are never automatically logged.
pub async fn request_scope(
    State(policy): State<RequestPolicy>,
    request: Request,
    next: Next,
) -> Response {
    with_current_dispatch(request_scope_inner(policy, request, next)).await
}

async fn request_scope_inner(policy: RequestPolicy, request: Request, next: Next) -> Response {
    let mut observation = HttpObservation::new(&request);
    let span = observation.span.clone();
    let response = async move {
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
    .instrument(span)
    .await;
    observation.status = Some(response.status());
    response
}

struct HttpObservation {
    span: tracing::Span,
    started: Instant,
    status: Option<StatusCode>,
}

impl HttpObservation {
    fn new(request: &Request) -> Self {
        let method = match *request.method() {
            Method::GET => "GET",
            Method::HEAD => "HEAD",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::DELETE => "DELETE",
            Method::CONNECT => "CONNECT",
            Method::OPTIONS => "OPTIONS",
            Method::TRACE => "TRACE",
            Method::PATCH => "PATCH",
            _ => "OTHER",
        };
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str)
            .unwrap_or("<unmatched>");
        Self {
            span: tracing::info_span!(
                target: "batter",
                "batter.http",
                method,
                route,
                status = tracing::field::Empty,
                http_outcome = tracing::field::Empty,
                latency_ms = tracing::field::Empty,
            ),
            started: Instant::now(),
            status: None,
        }
    }
}

impl Drop for HttpObservation {
    fn drop(&mut self) {
        let outcome = match self.status {
            Some(status) if status.is_server_error() => "server_error",
            Some(status) if status.is_client_error() => "client_error",
            Some(_) => "completed",
            None => "dropped",
        };
        let latency_ms = self.started.elapsed().as_secs_f64() * 1_000.0;
        if let Some(status) = self.status {
            self.span.record("status", status.as_u16());
        }
        self.span.record("http_outcome", outcome);
        self.span.record("latency_ms", latency_ms);
        if self.status.is_none_or(|status| status.is_server_error()) {
            tracing::warn!(target: "batter", parent: &self.span, "HTTP response boundary finished");
        } else {
            tracing::info!(target: "batter", parent: &self.span, "HTTP response boundary finished");
        }
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
