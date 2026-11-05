//! Gate reads: a kill switch classified for the surfaces that gate work.

use super::SnapshotView;
use crate::{
    decision::{Decision, Layer, Source, SwitchState},
    events::Surface,
    handle::KillSwitch,
};

/// What a gate learns from its kill switch.
///
/// Three readings, because they call for three different actions and two of
/// them look alike in a bare [`SwitchState`]: a switch nobody has published is
/// not a switch an operator tripped. A gate matches this exhaustively, so no
/// surface can drop work, or answer as disabled, on the strength of data it
/// never received.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, Freshness, PrePublication, SafeState, SettingsPrefix, StaleRule,
///     low_level::{BaseBuilder, GateReading, gate_switch},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// catalog.environment_class("hosted")?;
/// let halt = catalog
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()?;
/// let base = BaseBuilder::new(catalog.build()?, serving)?.finish();
///
/// // Nothing is published yet, so the gate is waiting, not tripped.
/// let view = base.not_loaded_view(PrePublication::Unready, Freshness::Fresh);
/// assert!(matches!(gate_switch(&view, &halt), GateReading::AwaitingPublication(_)));
///
/// // Declared mode has no such window.
/// assert!(matches!(gate_switch(&base.declared_view(), &halt), GateReading::Open(_)));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "a gate reading says whether the gated work may run"]
pub enum GateReading {
    /// No control-plane or persisted-file snapshot has been published, so no
    /// operator or stored state has been read. A settings floor is the one
    /// exception: it is the operator's explicit setting and reads as
    /// [`Tripped`](Self::Tripped). Whatever the decision's value, a gate waits
    /// and rechecks; it never treats this as an operator's decision.
    AwaitingPublication(Decision<SwitchState>),
    /// The switch is released.
    Open(Decision<SwitchState>),
    /// The switch is engaged by a published state, the settings floor or its
    /// stale rule.
    Tripped(Decision<SwitchState>),
}

impl GateReading {
    /// The decision behind the reading: its layer, reason and source.
    pub fn decision(&self) -> &Decision<SwitchState> {
        match self {
            Self::AwaitingPublication(decision)
            | Self::Open(decision)
            | Self::Tripped(decision) => decision,
        }
    }

    /// Consumes the reading into its decision.
    pub fn into_decision(self) -> Decision<SwitchState> {
        match self {
            Self::AwaitingPublication(decision)
            | Self::Open(decision)
            | Self::Tripped(decision) => decision,
        }
    }

    fn classify(decision: Decision<SwitchState>) -> Self {
        let unpublished = matches!(decision.source(), Source::NotLoaded { .. });
        if unpublished && decision.layer() != &Layer::Floor {
            Self::AwaitingPublication(decision)
        } else if decision.value().is_engaged() {
            Self::Tripped(decision)
        } else {
            Self::Open(decision)
        }
    }
}

/// Reads a kill switch for a gate: the same decision as
/// [`SnapshotView::switch`], classified as a [`GateReading`] and reported at
/// INFO while the switch is engaged and DEBUG while it is released.
///
/// Every gate, the HTTP gate, a gated job handler and the periodic gate, reads
/// its switch through this one function, so they agree on the value, on the
/// reading and on the event.
///
/// # Caller obligations
///
/// The switch must be one the view's base can answer for: this catalog's, and
/// owned by the base's process kind, which
/// [`Base::check_switch`](crate::low_level::Base::check_switch) tests. This
/// function does not refuse any other. Such a switch reads as its safe state,
/// classified like any other reading (engaged is
/// [`Tripped`](GateReading::Tripped), released is
/// [`Open`](GateReading::Open)), and only the decision's error cause and one
/// WARN event say that it was never read. A gate is built once, so it should
/// check its switches when it is built; the gates of `batter-flags` do.
pub fn gate_switch(view: &SnapshotView, switch: &KillSwitch) -> GateReading {
    let decision = view.switch_decision(switch);
    let surface = if decision.value().is_engaged() {
        Surface::GateEngaged
    } else {
        Surface::GateReleased
    };
    view.report(switch.handle(), &decision, surface);
    GateReading::classify(decision)
}
