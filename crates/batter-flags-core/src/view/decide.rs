//! How a view decides: the layers of a flag, the one kill-switch function, the
//! governing override and who may evaluate a handle.
//!
//! Everything here reads only the state a view captured. The public surface,
//! the events and pin replay are in the parent module.

use super::{Control, Loaded, Mode, SnapshotView};
use crate::{
    base::BaseState,
    decision::{
        Decision, ErrorCause, Freshness, KillSwitchKey, Layer, PrePublication, Reason, SwitchState,
    },
    declaration::{Entry, StaleRule},
    handle::{Handle, KillSwitch},
    key::FlagKey,
    snapshot::EntryState,
    value::FlagValue,
};

/// Who may evaluate or replay a handle.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Reach {
    /// Owners only: live evaluation.
    Owner,
    /// Owners and the kinds a flag lists in `pinned_by`: pin replay.
    OwnerOrPinner,
}

/// What a kill switch's layers add up to, before a decision is assembled.
pub(super) struct SwitchOutcome {
    pub(super) state: SwitchState,
    pub(super) layer: Layer,
    pub(super) reason: Reason,
    pub(super) error: Option<ErrorCause>,
}

impl SwitchOutcome {
    fn served(state: SwitchState, layer: Layer, reason: Reason) -> Self {
        Self {
            state,
            layer,
            reason,
            error: None,
        }
    }
}

impl SnapshotView {
    pub(super) fn catalog_entry(&self, handle: &Handle) -> &Entry {
        &self.base.catalog.entries[handle.index()]
    }

    /// Why this view cannot answer for `handle`, if it cannot.
    pub(super) fn refusal(&self, handle: &Handle, reach: Reach) -> Option<ErrorCause> {
        if handle.catalog() != self.base.catalog.id {
            return Some(ErrorCause::ForeignCatalog);
        }
        let entry = self.catalog_entry(handle);
        let reaches = entry.owners.contains(&self.base.kind)
            || (reach == Reach::OwnerOrPinner && entry.pinned_by.contains(&self.base.kind));
        if reaches {
            None
        } else {
            Some(ErrorCause::NotOwned)
        }
    }

    pub(super) fn flag_decision<T: FlagValue>(&self, handle: &Handle, default: T) -> Decision<T> {
        if let Some(cause) = self.refusal(handle, Reach::Owner) {
            return Decision::failed(
                (default, default.variant()),
                cause,
                self.source(),
                Layer::Base,
            );
        }
        let entry = self.catalog_entry(handle);
        if let Some(governing) = &entry.governed_by
            && self.switch_outcome(governing.index).state.is_engaged()
        {
            return self.overridden(default, &governing.key);
        }
        self.layered(handle.index(), entry, default)
    }

    /// The code default, because a governing switch is engaged.
    pub(super) fn overridden<T: FlagValue>(&self, default: T, by: &FlagKey) -> Decision<T> {
        Decision::served(
            (default, default.variant()),
            Reason::Disabled,
            self.source(),
            Layer::Override {
                by: KillSwitchKey::new(by.clone()),
            },
        )
    }

    /// A release or mode flag's value from the base and the published entry.
    fn layered<T: FlagValue>(&self, index: usize, entry: &Entry, default: T) -> Decision<T> {
        let source = self.source();
        let literal = match self.base.states[index] {
            BaseState::Flag(Some(chosen)) => T::from_variant(entry.variants[chosen]),
            BaseState::Flag(None) | BaseState::Switch(_) => None,
        };
        let from_base = |reason| {
            let value = literal.unwrap_or(default);
            Decision::served((value, value.variant()), reason, source, Layer::Base)
        };
        match &self.mode {
            Mode::Declared => match literal {
                Some(value) => Decision::served(
                    (value, value.variant()),
                    Reason::Static,
                    source,
                    Layer::Entry,
                ),
                None => from_base(Reason::Default),
            },
            Mode::Control(Control::NotLoaded(PrePublication::ServeDefaults)) => {
                from_base(Reason::Default)
            }
            Mode::Control(Control::NotLoaded(PrePublication::Unready)) => {
                let value = literal.unwrap_or(default);
                Decision::failed(
                    (value, value.variant()),
                    ErrorCause::ProviderNotReady,
                    source,
                    Layer::Base,
                )
            }
            Mode::Control(Control::Loaded { snapshot, kind }) => match snapshot.entries[index] {
                EntryState::Flag(chosen) => {
                    let value = T::from_variant(entry.variants[chosen]).unwrap_or(default);
                    Decision::served(
                        (value, value.variant()),
                        self.entry_reason(*kind),
                        source,
                        Layer::Entry,
                    )
                }
                EntryState::Absent | EntryState::Switch(_) | EntryState::Held { .. } => {
                    from_base(Reason::Default)
                }
            },
        }
    }

    /// A published entry is `static`, unless the view is stale or the source is
    /// the last-good file, whose age no clock confirms.
    fn entry_reason(&self, kind: Loaded) -> Reason {
        if self.freshness == Freshness::Stale || kind == Loaded::PersistedFile {
            Reason::Stale
        } else {
            Reason::Static
        }
    }

    /// The one total function every surface reads a kill switch through.
    pub(super) fn switch_outcome(&self, index: usize) -> SwitchOutcome {
        let entry = &self.base.catalog.entries[index];
        let safe = SwitchState::safe_for(entry);
        let literal = match self.base.states[index] {
            BaseState::Switch(literal) => literal,
            BaseState::Flag(_) => None,
        };
        let control = match &self.mode {
            Mode::Declared => {
                return match literal {
                    Some(state) => SwitchOutcome::served(state, Layer::Entry, Reason::Static),
                    None => SwitchOutcome::served(safe, Layer::Base, Reason::Default),
                };
            }
            Mode::Control(control) => control,
        };
        if literal == Some(SwitchState::Engaged) {
            return SwitchOutcome::served(SwitchState::Engaged, Layer::Floor, Reason::Static);
        }
        if self.freshness == Freshness::Stale
            && entry.stale_rule == Some(StaleRule::EngageWhenStale)
        {
            return SwitchOutcome::served(SwitchState::Engaged, Layer::StaleRule, Reason::Stale);
        }
        match control {
            Control::NotLoaded(PrePublication::ServeDefaults) => {
                SwitchOutcome::served(safe, Layer::Base, Reason::Default)
            }
            Control::NotLoaded(PrePublication::Unready) => SwitchOutcome {
                state: safe,
                layer: Layer::Base,
                reason: Reason::Error,
                error: Some(ErrorCause::ProviderNotReady),
            },
            Control::Loaded { snapshot, kind } => match snapshot.entries[index] {
                EntryState::Switch(state) => {
                    SwitchOutcome::served(state, Layer::Entry, self.entry_reason(*kind))
                }
                EntryState::Held {
                    carried: Some(kept),
                    ..
                } => SwitchOutcome::served(
                    kept.state,
                    Layer::Held { since: kept.since },
                    Reason::Stale,
                ),
                EntryState::Absent | EntryState::Flag(_) | EntryState::Held { .. } => {
                    SwitchOutcome::served(safe, Layer::Base, Reason::Default)
                }
            },
        }
    }

    pub(super) fn switch_decision(&self, switch: &KillSwitch) -> Decision<SwitchState> {
        let handle = switch.handle();
        let source = self.source();
        if let Some(cause) = self.refusal(handle, Reach::Owner) {
            let safe = SwitchState::from(switch.safe_state());
            return Decision::failed((safe, safe.as_str()), cause, source, Layer::Base);
        }
        let outcome = self.switch_outcome(handle.index());
        let value = (outcome.state, outcome.state.as_str());
        match outcome.error {
            None => Decision::served(value, outcome.reason, source, outcome.layer),
            Some(cause) => Decision::failed(value, cause, source, outcome.layer),
        }
    }
}
