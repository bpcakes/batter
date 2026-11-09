//! Gate reads: a kill switch classified for the surfaces that gate work.

use super::SnapshotView;
use crate::{
    decision::{Decision, ErrorCause, Layer, Source, SwitchState},
    events::Surface,
    handle::KillSwitch,
};

/// What a gate learns from its kill switch.
///
/// Three readings, because they call for three different actions and two of
/// them look alike in a bare [`SwitchState`]: a switch nobody has published is
/// not a switch an operator tripped. A gate matches this exhaustively and
/// chooses its pre-publication policy explicitly. A surface may wait, or use
/// the decision's startup-default value and error to admit or withhold work;
/// it must not describe an unpublished default as an observed operator trip.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, Freshness, PrePublication, SafeState, SettingsPrefix, StaleRule,
///     low_level::{BaseBuilder, GateReading, gate_switch},
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut catalog, serving, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let halt = catalog
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()?;
/// let base = BaseBuilder::new(catalog.build(), serving)?.finish();
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
    /// [`Tripped`](Self::Tripped). The surface decides whether to wait and
    /// recheck or serve according to the startup-default decision. Neither
    /// choice turns this into an observed operator decision. The retained
    /// decision includes safe state, stale rules and any provider-not-ready error.
    AwaitingPublication(Decision<SwitchState>),
    /// The switch is released.
    Open(Decision<SwitchState>),
    /// The switch is engaged by a published state, the settings floor or its
    /// stale rule, or the view cannot answer for it at all: another catalog's
    /// switch, or one this process kind does not own. A gate fails closed on
    /// such a switch; its decision keeps the safe state it serves and the
    /// error cause, so the two are told apart.
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
        let unanswerable = matches!(
            decision.error(),
            Some(ErrorCause::ForeignCatalog | ErrorCause::NotOwned)
        );
        let unpublished = matches!(decision.source(), Source::NotLoaded { .. });
        if unanswerable {
            Self::Tripped(decision)
        } else if unpublished && decision.layer() != &Layer::Floor {
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
/// A switch the view's base cannot answer for, another catalog's or one the
/// base's process kind does not own, reads as
/// [`Tripped`](GateReading::Tripped) whatever its safe state, so the gate
/// withholds work; the decision keeps the safe state and the error cause, and
/// one WARN event is emitted. A gate is built once, so it should still check
/// its switches with
/// [`Base::check_switch`](crate::low_level::Base::check_switch) when it is
/// built and refuse them at startup; the gates of `batter-flags` do.
///
/// Migration: such a switch whose safe state is released read as
/// [`Open`](GateReading::Open); it now reads as `Tripped`.
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
