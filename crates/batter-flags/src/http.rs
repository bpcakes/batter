//! Protected route-group gates and one frozen flag view per HTTP request.
//!
//! Install [`Flags::http_view`], [`Flags::http_gate`] or [`Flags::http_gate_any`]
//! with [`batter_axum::GroupPolicy::gated`] (or `browser_gated`). Handlers use
//! [`AdmittedFlags`]; they cannot substitute a native or independently captured
//! view. A later publication affects the next request, never an admitted one.
//!
//! ```
//! use axum::routing::get;
//! use batter_axum::{GroupPolicy, GuardedRouter, HttpBoundary, RequestPolicy,
//!     ResponseConstructionBudget, RouteGroup};
//! use batter_core::lifecycle::ShutdownHandle;
//! use batter_flags::{core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule, Toggle},
//!     http::AdmittedFlags, test::TestFlags};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
//! let fast = catalog.mode_flag("fast_path", Toggle::Off).declare()?;
//! let halt = catalog.kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
//!     .declare()?;
//! let fixture = TestFlags::builder(catalog.build(), serving)?.set(&fast, Toggle::On).build()?;
//! let (control, approval) = ShutdownHandle::new_with_readiness_approval();
//! approval.approve();
//! let policy = RequestPolicy::new(control.operation_admission(),
//!     ResponseConstructionBudget::new(Duration::from_secs(1))?);
//! let routes = GuardedRouter::new().route("/work", get(move |flags: AdmittedFlags| {
//!     let fast = fast.clone();
//!     async move { flags.view().evaluate(&fast).variant().to_owned() }
//! }));
//! let client = HttpBoundary::new(policy.clone())
//!     .with_group(RouteGroup::new("work",
//!         GroupPolicy::gated(policy, fixture.flags().http_gate(&halt)?), routes))?
//!     .assemble(GuardedRouter::new()).await?.in_process();
//! # let _ = client;
//! # Ok(()) }
//! ```
//!
//! A disabled group renders [`HttpFailure::Disabled`] through its own request
//! policy (503 by default). Every method and fallback in that group is gated;
//! prefer named groups when unmatched paths should remain reachable. Probes
//! and a metadata-rendered fallback remain outside the gate. See [`GroupGate`]
//! for renderer ownership and completion-event severity configuration.
//!
//! Before publication, the reader's startup policy already determines each
//! decision. Under `start_on_defaults` (`PrePublication::ServeDefaults`), HTTP
//! serves a released safe state and disables an engaged safe state, settings
//! floor or stale rule. Under `unready_until_loaded` (`PrePublication::Unready`),
//! a provider-not-ready decision disables the group even if its safe state is
//! released; lifecycle admission is also intended to remain closed until load.
//! The loader and its acknowledgement are separate from this gate. Serving
//! defaults is not evidence of a publication: those decisions retain
//! `Source::NotLoaded`. Job/periodic policies may instead wait unconditionally.
//!
//! Capture alone emits no evaluation event. A switch gate emits exactly one
//! `feature_flag.evaluation` per listed switch per request, through core's gate
//! reader, at DEBUG while released and INFO while engaged. Errors are WARN on
//! their first occurrence per handle/cause and DEBUG thereafter, as in core.
//! Handler evaluations are additional events. The HTTP completion is separate.

use axum::{extract::FromRequestParts, http::request::Parts};
use batter_axum::{GateView, Gated, GroupGate, HttpFailure};
use batter_flags_core::{KillSwitch, SnapshotView, low_level::GateReading};

use crate::{Flags, GateError, flags::gate::SwitchGate};

/// The private-origin value retained by the route-group gate.
///
/// Only the three HTTP builders construct it. It is not `Clone`, has no public
/// constructor and cannot be recovered by value from an extraction. Application
/// gates returning a native `SnapshotView` never satisfy [`AdmittedFlags`].
/// See the [module example](self) for the supported construction path.
///
/// ```compile_fail,E0423
/// use batter_flags::{core::SnapshotView, http::RequestFlags};
/// fn forge(view: SnapshotView) { let _ = RequestFlags(view); }
/// ```
/// ```compile_fail,E0277
/// use batter_flags::http::RequestFlags;
/// fn cloneable<T: Clone>() {}
/// cloneable::<RequestFlags>();
/// ```
pub struct RequestFlags(SnapshotView);

/// Extract the exact view captured by this request's protected flags gate.
///
/// Repeated extractions borrow the same view. An absent record or an
/// application-built gate over any other type rejects with the fixed sanitized
/// 500 [`HttpFailure::Internal`], regardless of the group's custom renderer,
/// and the completion event carries `gate_outcome="unmatched_view"`. The
/// pairing with a flags-gated group is not a compile-time property: see
/// `batter_axum::GateView` for why. Extract it only in routes of a group
/// installed with `http_view`, `http_gate` or `http_gate_any`.
/// There is no public constructor, conversion or `Clone` implementation.
/// See the [module example](self) for use in a handler.
///
/// ```compile_fail,E0423
/// use batter_axum::GateView;
/// use batter_flags::http::{AdmittedFlags, RequestFlags};
/// fn forge(view: GateView<RequestFlags>) { let _ = AdmittedFlags(view); }
/// ```
pub struct AdmittedFlags(GateView<RequestFlags>);

impl AdmittedFlags {
    /// Borrow this request's frozen native evaluation and pinning view.
    ///
    /// For example, `flags.view().evaluate(&flag)` uses the same publication
    /// and freshness as the switch that admitted the request. This accessor
    /// never captures again or emits an evaluation event itself.
    pub fn view(&self) -> &SnapshotView {
        &self.0.value().0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AdmittedFlags {
    type Rejection = HttpFailure;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        GateView::<RequestFlags>::from_request_parts(parts, state)
            .await
            .map(Self)
    }
}

impl Flags {
    /// Capture one view per admitted request without checking a kill switch.
    ///
    /// Install with `GroupPolicy::gated(policy, flags.http_view())`; handlers
    /// extract [`AdmittedFlags`] as in the [module example](self). Capture
    /// emits no evaluation event and does not itself withhold work before a
    /// publication. Startup acknowledgement belongs to the loader.
    pub fn http_view(&self) -> GroupGate {
        let flags = self.clone();
        GroupGate::new(move || Gated::Serve(RequestFlags(flags.capture())))
    }

    /// Capture once and disable the group while `switch` is engaged.
    ///
    /// See the [module example](self) for assembly and the module's startup
    /// policy for readings before publication. The handler and switch always
    /// use the same view; an engagement cannot interrupt admitted work.
    ///
    /// # Errors
    ///
    /// Returns [`GateError::ForeignHandle`] for another catalog's switch or
    /// [`GateError::NotOwned`] for a switch this process kind does not own.
    pub fn http_gate(&self, switch: &KillSwitch) -> Result<GroupGate, GateError> {
        self.gate(switch).map(http_gate)
    }

    /// Capture once and disable the group while any listed switch is engaged.
    ///
    /// The mandatory `first` makes an empty gate unrepresentable. For example,
    /// `flags.http_gate_any(&halt, &[&other_halt])?` replaces `http_gate` in the
    /// [module example](self). Every switch is read once, even if an earlier
    /// switch disables the group, so each contributes its own evaluation event.
    ///
    /// # Errors
    ///
    /// Returns [`GateError::ForeignHandle`], [`GateError::NotOwned`] or
    /// [`GateError::DuplicateSwitch`] before any request can execute.
    ///
    /// ```compile_fail,E0061
    /// fn empty(flags: &batter_flags::Flags) { flags.http_gate_any(&[]); }
    /// ```
    pub fn http_gate_any(
        &self,
        first: &KillSwitch,
        rest: &[&KillSwitch],
    ) -> Result<GroupGate, GateError> {
        self.gate_any(first, rest).map(http_gate)
    }
}

fn http_gate(gate: SwitchGate) -> GroupGate {
    GroupGate::new(move || {
        let captured = gate.enter();
        match captured.reading {
            GateReading::Open(_) => Gated::Serve(RequestFlags(captured.view)),
            GateReading::Tripped(_) => Gated::Disabled,
            GateReading::AwaitingPublication(decision) => {
                if decision.error().is_some() || decision.value().is_engaged() {
                    Gated::Disabled
                } else {
                    Gated::Serve(RequestFlags(captured.view))
                }
            }
        }
    })
}

#[cfg(test)]
mod tests;
