//! One synchronous capture/decision inside each group's admission.

use crate::{HttpFailure, HttpObservationLevel, RequestPolicy, observation::ObservationState};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::Response,
};
use std::{any::Any, sync::Arc};
use tracing::Level;

/// A gate's decision for this request, independent of any flags package.
///
/// Returned by the callback passed to [`GroupGate::new`].
pub enum Gated<V> {
    /// Continue with this one captured value, shared by all [`GateView`] extractions.
    Serve(V),
    /// Drop the request body and render [`HttpFailure::Disabled`].
    Disabled,
}

/// One flags-agnostic capture/decision for a guarded route group.
///
/// Experimental: the seam typed flags gate route groups through, recorded in
/// the repository's ADR-016.
///
/// The callback runs once after lifecycle/deadline admission, before browser
/// mutation checks and application layers. It must be fast, nonblocking and
/// non-panicking. A panic propagates like a handler panic: there is no automatic
/// recovery, and observation records one WARN `dropped` completion without status.
/// The callback receives no method or request data. Every method, including
/// OPTIONS and unsupported methods, is gated. A gated default group also gates
/// unmatched paths and guarded root/nested fallbacks; prefer named groups when
/// those should remain reachable. Probes and metadata-rendered fallbacks bypass it.
///
/// A request that already captured a value keeps it even if the source changes.
/// The library retains that exact value, not a live reader. Immutability of data
/// behind application-supplied interior mutability remains application policy.
///
/// ```
/// use axum::routing::get;
/// use batter_axum::{GateView, Gated, GroupGate, GroupPolicy, GuardedRouter,
///     HttpBoundary, RequestPolicy, ResponseConstructionBudget, RouteGroup};
/// use batter_core::lifecycle::ShutdownHandle;
/// use std::time::Duration;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (control, approval) = ShutdownHandle::new_with_readiness_approval();
/// approval.approve();
/// let request = RequestPolicy::new(control.operation_admission(),
///     ResponseConstructionBudget::new(Duration::from_secs(1))?);
/// let gate = GroupGate::new(|| Gated::Serve(42_u64));
/// let routes = GuardedRouter::new().route("/work", get(|view: GateView<u64>| async move {
///     view.value().to_string()
/// }));
/// let client = HttpBoundary::new(request.clone())
///     .with_group(RouteGroup::new("work", GroupPolicy::gated(request, gate), routes))?
///     .assemble(GuardedRouter::new()).await?.in_process();
/// # let _ = client;
/// # Ok(()) }
/// ```
///
/// Gates belong only to group constructors; request policy and low-level
/// admission have no gate or flags configuration:
/// ```compile_fail,E0599
/// use batter_axum::{RequestPolicy, GroupGate};
/// fn invalid(request: RequestPolicy, gate: GroupGate) { request.with_flags(gate); }
/// ```
/// ```compile_fail,E0599
/// use batter_axum::{RequestPolicy, GroupGate};
/// fn invalid(request: RequestPolicy, gate: GroupGate) { request.with_gate(gate); }
/// ```
/// ```compile_fail,E0599
/// use batter_axum::{RequestPolicy, GroupGate, GroupPolicy};
/// fn invalid(request: RequestPolicy, gate: GroupGate) {
///     GroupPolicy::new(request).with_gate(gate);
/// }
/// ```
#[derive(Clone)]
#[must_use = "pass the gate to GroupPolicy::gated or GroupPolicy::browser_gated"]
pub struct GroupGate {
    capture: Arc<dyn Fn() -> Gated<GateRecord> + Send + Sync>,
    disabled_level: Level,
}

impl GroupGate {
    /// Capture a value or disable the request synchronously, without request metadata.
    ///
    /// `V` need not be `Clone`; all extractions borrow the same retained allocation.
    pub fn new<V: Send + Sync + 'static>(
        capture: impl Fn() -> Gated<V> + Send + Sync + 'static,
    ) -> Self {
        Self {
            capture: Arc::new(move || match capture() {
                Gated::Serve(value) => Gated::Serve(GateRecord(Arc::new(value))),
                Gated::Disabled => Gated::Disabled,
            }),
            disabled_level: Level::INFO,
        }
    }

    /// Select the disabled completion's level, replacing even a renderer-set override.
    ///
    /// The default INFO event has target `batter`, so `batter=warn` hides it.
    /// Use `GroupGate::new(|| Gated::<()>::Disabled).with_disabled_level(tracing::Level::WARN)`
    /// to keep disablement visible under that filter. Actual response status is
    /// still renderer-owned, and status/outcome alerts are unaffected by severity.
    ///
    /// ```compile_fail
    /// #![deny(unused_must_use)]
    /// use batter_axum::GroupGate;
    /// fn discarded(gate: GroupGate) { gate.with_disabled_level(tracing::Level::WARN); }
    /// ```
    #[must_use = "retain the configured gate; discarding it loses the selected level"]
    pub fn with_disabled_level(mut self, level: Level) -> Self {
        self.disabled_level = level;
        self
    }
}

/// Non-generic so every renderer can strip it without knowing the value's type.
#[derive(Clone)]
pub(crate) struct GateRecord(Arc<dyn Any + Send + Sync>);

/// The gate's captured value, extracted without capturing again.
///
/// See [`GroupGate`] for a complete usage example. This value has no constructor
/// and is not `Clone`. Missing or wrong-typed records reject with the fixed,
/// sanitized 500 [`HttpFailure::Internal`] response, never a Rust type name.
/// Repeated extractions of the same type share the exact same value.
///
/// ```compile_fail,E0277
/// use axum::extract::Request;
/// use batter_axum::GateView;
/// fn transplant(view: GateView<u64>, mut request: Request) {
///     request.extensions_mut().insert(view);
/// }
/// ```
/// ```compile_fail,E0423
/// use batter_axum::GateView;
/// fn forge() { let view = GateView(std::sync::Arc::new(42_u64)); }
/// ```
pub struct GateView<V>(Arc<V>);

impl<V> GateView<V> {
    /// Borrow the value captured once for this request.
    pub fn value(&self) -> &V {
        &self.0
    }
}

impl<S: Send + Sync, V: Send + Sync + 'static> FromRequestParts<S> for GateView<V> {
    type Rejection = HttpFailure;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<GateRecord>()
            .and_then(|record| record.0.clone().downcast::<V>().ok())
            .map(Self)
            .ok_or(HttpFailure::Internal)
    }
}

// The boundary owns dispatch across polling and complete destruction.
pub(super) async fn run(
    State((gate, policy)): State<(GroupGate, RequestPolicy)>,
    mut request: Request,
    next: Next,
) -> Response {
    match (gate.capture)() {
        Gated::Serve(record) => {
            request.extensions_mut().insert(record);
            next.run(request).await
        }
        Gated::Disabled => {
            let (parts, body) = request.into_parts();
            if let Some(observation) = parts.extensions.get::<ObservationState>() {
                observation.record_gate_disabled();
            }
            drop(body);
            let mut response = policy.render_failure(HttpFailure::Disabled, &parts);
            response
                .extensions_mut()
                .insert(HttpObservationLevel(gate.disabled_level));
            response
        }
    }
}

#[cfg(test)]
mod tests;
