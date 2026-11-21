//! Cross-flag invariants over what a snapshot's views would serve.
//!
//! Declared mode refuses to start when two flags this process kind owns
//! resolve to a pair an invariant forbids. The control plane cannot refuse a
//! row that is already stored, so it holds one instead: a published entry
//! goes back to the declared base until every invariant between owned flags
//! is met. An invariant whose other flag this kind does not own is not
//! judged, as in declared mode: this process cannot know what the other kind
//! serves.
//!
//! A flag whose governing kill switch is engaged serves its code default, so
//! it is judged at that default. Whether a switch is engaged can depend on
//! the view's freshness through its stale rule, so the judgement is made for
//! a fresh and for a stale view.

use super::{Carried, EntryState, HoldCause};
use crate::{
    base::{BaseInner, BaseState},
    decision::{Freshness, SwitchState},
    declaration::{Entry, Invariant, StaleRule},
    invariants::{FlagStanding, Relation, judge},
};

/// Why a kill switch of a control-plane view is engaged whatever its
/// snapshot entry says, checked in this order before the entry is read. The
/// view's decision and the invariant judgement both read a switch through
/// [`switch_override`] and then [`entry_switch`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SwitchOverride {
    /// The settings literal engaged it.
    Floor,
    /// The view is stale and the switch engages when stale.
    StaleRule,
}

/// What overrides the switch at `index` in a control-plane view of
/// `freshness`, if anything does.
pub(crate) fn switch_override(
    base: &BaseInner,
    index: usize,
    freshness: Freshness,
) -> Option<SwitchOverride> {
    if base.states[index] == BaseState::Switch(Some(SwitchState::Engaged)) {
        return Some(SwitchOverride::Floor);
    }
    let rule = base.catalog.entries[index].stale_rule;
    (freshness == Freshness::Stale && rule == Some(StaleRule::EngageWhenStale))
        .then_some(SwitchOverride::StaleRule)
}

/// What a kill switch serves from its snapshot entry when nothing overrides
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EntrySwitch {
    /// The snapshot published this state.
    Published(SwitchState),
    /// The row is held and the switch keeps a state it published earlier.
    Carried(Carried),
    /// Nothing published it: the safe state.
    Safe(SwitchState),
}

/// What `declared`, a kill switch, serves from `entry`.
pub(crate) fn entry_switch(declared: &Entry, entry: &EntryState) -> EntrySwitch {
    match entry {
        EntryState::Switch(state) => EntrySwitch::Published(*state),
        EntryState::Held {
            carried: Some(kept),
            ..
        } => EntrySwitch::Carried(*kept),
        EntryState::Absent | EntryState::Flag(_) | EntryState::Held { .. } => {
            EntrySwitch::Safe(SwitchState::safe_for(declared))
        }
    }
}

/// Whether the switch at `index` is engaged in a view of `freshness` over
/// `entries`.
fn engaged(base: &BaseInner, entries: &[EntryState], index: usize, freshness: Freshness) -> bool {
    switch_override(base, index, freshness).is_some()
        || match entry_switch(&base.catalog.entries[index], &entries[index]) {
            EntrySwitch::Published(state) | EntrySwitch::Safe(state) => state.is_engaged(),
            EntrySwitch::Carried(kept) => kept.state.is_engaged(),
        }
}

/// Holds published entries, judged for a view of `freshness`, until every
/// invariant between flags this process kind owns is met.
pub(super) fn hold_unmet(base: &BaseInner, entries: &mut [EntryState], freshness: Freshness) {
    let catalog = &base.catalog;
    // The judged flags: release and mode flags this kind owns, by position.
    let mut nodes = Vec::new();
    let mut node_of = vec![None; catalog.entries.len()];
    for (index, entry) in catalog.entries.iter().enumerate() {
        if entry.owners.contains(&base.kind)
            && let Some(standing) = standing(base, entries, index, freshness)
        {
            node_of[index] = Some(nodes.len());
            nodes.push((index, standing));
        }
    }
    let mut relations = Vec::new();
    for &(index, _) in &nodes {
        let entry = &catalog.entries[index];
        for invariant in &entry.invariants {
            let other = catalog
                .by_key
                .get(&invariant.other)
                .and_then(|&other| node_of[other]);
            if let (Some(flag), Some(other)) = (node_of[index], other) {
                relations.push(Relation {
                    rule: invariant.rule,
                    flag,
                    variant: invariant.variant,
                    other,
                    other_variant: invariant.other_variant,
                });
            }
        }
    }
    if relations.is_empty() {
        return;
    }
    let keys: Vec<_> = nodes
        .iter()
        .map(|&(index, _)| &catalog.entries[index].key)
        .collect();
    let standings: Vec<_> = nodes.iter().map(|&(_, standing)| standing).collect();
    // A relation left unresolved has both flags at a value no hold changes.
    // Declared resolution checks the base, and control-plane registration
    // refuses one an engaged governor breaks, so only a base built with the
    // low-level `BaseBuilder` and never checked can leave one; nothing here
    // can serve otherwise, so it is left as the base has it.
    for hold in judge(&keys, &standings, &relations).holds {
        entries[nodes[hold.flag].0] = EntryState::Held {
            cause: HoldCause::InvariantUnmet,
            carried: None,
        };
    }
}

/// What the flag at `index` serves before invariants: its code default under
/// an engaged governing switch, else its published variant, else its base.
/// `None` for a kill switch, which no invariant names.
fn standing<'a>(
    base: &'a BaseInner,
    entries: &[EntryState],
    index: usize,
    freshness: Freshness,
) -> Option<FlagStanding<'a>> {
    let entry = &base.catalog.entries[index];
    let default = entry.default_variant?;
    let based = based(base, index);
    if let Some(governing) = &entry.governed_by
        && engaged(base, entries, governing.index, freshness)
    {
        return Some(FlagStanding::Fixed(default));
    }
    Some(match entries[index] {
        EntryState::Flag(chosen) => FlagStanding::Published {
            variant: entry.variants[chosen],
            base: based,
        },
        EntryState::Absent | EntryState::Switch(_) | EntryState::Held { .. } => {
            FlagStanding::Fixed(based)
        }
    })
}

/// The invariants between flags this kind owns that the declared base itself
/// breaks for some reachable state of the governing kill switches involved,
/// a governed flag serving its code default while its switch is engaged.
///
/// In a control plane an operator can engage or release any switch at any
/// time, unless its settings floor keeps it engaged, so no hold can meet
/// such an invariant: every flag a hold could move already serves its base.
pub(crate) fn governed_breaks(base: &BaseInner) -> Vec<&Invariant> {
    let catalog = &base.catalog;
    let judged = |index: usize| {
        let entry = &catalog.entries[index];
        entry.default_variant.is_some() && entry.owners.contains(&base.kind)
    };
    let floor =
        |switch: usize| base.states[switch] == BaseState::Switch(Some(SwitchState::Engaged));
    let mut breaks = Vec::new();
    for (index, entry) in catalog.entries.iter().enumerate() {
        if !judged(index) {
            continue;
        }
        for invariant in &entry.invariants {
            let Some(&other) = catalog.by_key.get(&invariant.other) else {
                continue;
            };
            if !judged(other) {
                continue;
            }
            // The governors whose state is open: at most one per flag.
            let mut open: Vec<usize> = [index, other]
                .iter()
                .filter_map(|&flag| catalog.entries[flag].governed_by.as_ref())
                .map(|governing| governing.index)
                .filter(|&switch| !floor(switch))
                .collect();
            open.dedup();
            let breaks_under = |engaged: &[usize]| {
                let served = |flag: usize| {
                    let entry = &catalog.entries[flag];
                    match &entry.governed_by {
                        Some(governing)
                            if floor(governing.index) || engaged.contains(&governing.index) =>
                        {
                            entry.default_variant.unwrap_or_default()
                        }
                        Some(_) | None => based(base, flag),
                    }
                };
                !invariant.holds(served(index), served(other))
            };
            // Every subset of the open governors engaged, the empty one first.
            let assignments: Vec<&[usize]> = match open.len() {
                0 => vec![&[]],
                1 => vec![&[], &open[..1]],
                _ => vec![&[], &open[..1], &open[1..2], &open[..2]],
            };
            if assignments.iter().any(|engaged| breaks_under(engaged)) {
                breaks.push(invariant);
            }
        }
    }
    breaks
}

/// A release or mode flag's declared base: its literal, else its code
/// default.
fn based(base: &BaseInner, index: usize) -> &'static str {
    let entry = &base.catalog.entries[index];
    match base.states[index] {
        BaseState::Flag(Some(chosen)) => entry.variants[chosen],
        BaseState::Flag(None) | BaseState::Switch(_) => entry.default_variant.unwrap_or_default(),
    }
}
