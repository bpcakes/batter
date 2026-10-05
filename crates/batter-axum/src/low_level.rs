//! Deliberately caller-ordered HTTP composition.
//!
//! [`HttpBoundary`](crate::HttpBoundary) is the canonical path: it owns the
//! layer order, keeps probes outside admission and consumes into protected
//! serving or an opaque [`InProcessClient`](crate::InProcessClient). The
//! helpers in this module are the low-level escape hatch for compositions that
//! boundary cannot express. They are not equivalent to the protected path: each
//! one leaves an ordering, placement or lifecycle obligation with the caller,
//! documented on the item itself.
//!
//! Choosing this module is choosing those obligations. A composition assembled
//! here can put a probe inside an admission gate, add a second observer, place
//! correlation inside admission so rejections lose their generated identity, or
//! append routes that bypass every layer. None of that is reachable through
//! [`HttpBoundary`](crate::HttpBoundary).
//!
//! Supported value and policy types are *not* here: they stay at the crate
//! root, where the canonical path also uses them.
//!
//! None of these helpers has a crate-root alias, so a canonical composition
//! cannot reach one by import alone. Each alias has its own control, so a
//! single returning alias cannot hide behind another still-absent name:
//!
//! ```compile_fail,E0432
//! use batter_axum::observe_http;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::request_admission;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::request_scope;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::operational_http;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::operational_http_with_quota;
//! ```
//!
//! The status-only probe handler's name is also the adapter's private
//! `readiness` module, so the crate root rejects it as private rather than as
//! absent; either way it is not reachable, and a returning public function
//! alias would make this import resolve.
//!
//! ```compile_fail,E0603
//! use batter_axum::readiness;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::liveness;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::dependency_readiness;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::register_http;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::register_http_in;
//! ```
//!
//! ```compile_fail,E0432
//! use batter_axum::register_http_with_connect_info_in;
//! ```

use crate::{
    HttpFailure, RequestInterruptionResponder, RequestPolicy, admitted,
    observation::observe_response,
};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use batter_core::{
    lifecycle::{LifecycleStatus, Readiness},
    operation::OperationError,
    telemetry::with_current_dispatch,
};
use std::convert::Infallible;
use tokio::time::Instant;

pub use crate::correlation::{operational_http, operational_http_with_quota};
pub use crate::readiness::dependency_readiness;
pub use crate::serving::{register_http, register_http_in, register_http_with_connect_info_in};

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
/// Responses default to WARN for 5xx and INFO otherwise; [`HttpObservationLevel`](crate::HttpObservationLevel)
/// in response extensions overrides only the completion-event level.
///
/// Only the outermost observer emits a completion event. [`observe_http`] and
/// [`operational_http`] can nest in either order because observation does not
/// short-circuit. [`request_scope`] also performs admission and can return or
/// expire without polling inner middleware, so `operational_http` must be
/// outside it when every response and completion event requires server
/// correlation. Operation completion events remain separate. Prefer
/// [`HttpBoundary`](crate::HttpBoundary), which makes this order unchangeable.
///
/// ```
/// use axum::{Router, http::StatusCode, middleware, routing::get};
/// use batter_core::lifecycle::ShutdownHandle;
/// use batter_axum::{
///     RequestPolicy, ResponseConstructionBudget,
///     low_level::{liveness, observe_http, readiness, request_admission},
/// };
/// use std::time::Duration;
/// # fn main() -> Result<(), batter_core::ConfigurationError> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let status = control.status();
/// let budget = ResponseConstructionBudget::new(Duration::from_secs(2))?;
/// let guarded = Router::new()
///     .route("/work", get(|| async { "ok" }))
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
/// lifecycle gate. [`HttpBoundary`](crate::HttpBoundary) owns that placement. See [`observe_http`]
/// for a manual composition example that observes probes, guarded fallbacks and
/// admission rejections.
///
/// Inside [`operational_http`], as correlation requires, admitted handlers
/// extract [`AdmittedRequest`](crate::AdmittedRequest); without that wrapper outside this layer no
/// admitted request is recorded and the extractor rejects. The native
/// `OperationContext` and [`RequestInterruptionResponder`] extensions are also
/// inserted for adapters and existing handlers; inner layers can replace them.
/// A Ready state read is the admission point; a request racing shutdown may
/// enter if that read occurred first. Existing admitted work is not cancelled
/// by drain. The child context is cancelled on completion/drop of the response
/// future.
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
/// Prefer [`HttpBoundary`](crate::HttpBoundary), which owns this composition.
///
/// ```
/// use axum::{Router, middleware};
/// use batter_axum::{RequestPolicy, low_level::{operational_http, request_scope}};
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
    let Ok(owner) = policy
        .admission
        .admit_root(batter_core::operation::RootDeadline::at(deadline))
    else {
        return policy.render_failure(HttpFailure::Unavailable, &parts);
    };
    let context = owner.into_context();
    // One request-scoped capability owns both the policy and the original parts.
    let responder = RequestInterruptionResponder::from_policy(&policy, &parts);
    let mut request = Request::from_parts(parts, body);
    request.extensions_mut().insert(responder.clone());
    let interruption = responder.clone();
    match context
        .run("http.response_construction", |scope| async move {
            admitted::record(request.extensions_mut(), scope.clone(), interruption);
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
/// [`HttpBoundary::with_readiness`](crate::HttpBoundary::with_readiness) mounts
/// the dependency-aware probe there.
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
