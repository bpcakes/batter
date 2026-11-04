//! Declaration kinds, per-declaration policy and the read-only view of a
//! built declaration.

use std::{fmt, sync::Arc};

use crate::{
    date::Expiry,
    digest::{DeclarationFingerprint, EntryView},
    key::{FlagKey, SettingName, SettingsPrefix},
    value::ValueType,
    vocabulary::{EnvironmentClass, ProcessKind},
};

/// The kind of a declaration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Kind {
    /// A temporary release flag with an expiry.
    Release,
    /// A permanent mode flag.
    Mode,
    /// A permanent kill switch.
    Switch,
}

impl Kind {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Mode => "mode",
            Self::Switch => "switch",
        }
    }
}

/// What an absent setting means for a declaration in declared mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Absence {
    /// Absence resolves to the code default, or a switch's safe state.
    Default,
    /// Absence fails startup: the setting must be present.
    Required,
}

impl Absence {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Required => "required",
        }
    }
}

/// Whether a declaration's processes may change value independently.
///
/// `Coordinated` is manifest metadata for deployment tooling: it marks a flag
/// whose processes must change together. No fleet-atomic flip exists, so a
/// control-plane write to a `Coordinated` key still reaches processes poll by
/// poll; the operator command refuses such a write unless the operator accepts
/// that explicitly.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum FlipDiscipline {
    /// Processes may pick the new value up one at a time.
    Rolling,
    /// Processes must change together; tooling schedules the flip.
    Coordinated,
}

impl FlipDiscipline {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rolling => "rolling",
            Self::Coordinated => "coordinated",
        }
    }
}

/// The state a kill switch serves where no entry exists for it.
///
/// In declared mode that is an absent setting. In control-plane mode it is
/// the state before the first publication, for an absent or archived row and
/// for a held entry with no last published state, beneath the settings floor
/// and the stale rule.
///
/// Hazard: `Engaged`, whatever the stale rule, engages the switch on every
/// process that starts during a control-plane outage without a usable
/// last-good file, so a rolling restart or scale-out engages it progressively.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SafeState {
    /// The switch is engaged where no entry exists.
    Engaged,
    /// The switch is released where no entry exists.
    Released,
}

impl SafeState {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Engaged => "engaged",
            Self::Released => "released",
        }
    }
}

/// What a kill switch does while its view is stale.
///
/// No rule releases an engaged switch on staleness. Hazard: `EngageWhenStale`,
/// whatever the safe state, engages the switch fleet-wide after an outage or
/// pool exhaustion longer than the loader's maximum age, including one present
/// at boot, because age runs from registration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum StaleRule {
    /// Keep serving the last known state.
    HoldLastKnown,
    /// Engage the switch while the view is stale.
    EngageWhenStale,
}

impl StaleRule {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HoldLastKnown => "hold_last_known",
            Self::EngageWhenStale => "engage_when_stale",
        }
    }
}

/// Which process kinds own a declaration.
///
/// Ownership governs where a value may be presented, which control-plane
/// entries apply and where the flag evaluates live. A kind that does not own
/// a flag and is not listed in its `pinned_by` neither evaluates nor resolves
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Owners<'a> {
    /// Every process kind owns the declaration.
    All,
    /// Only the listed kinds own it; the list must be non-empty and
    /// duplicate-free.
    Only(&'a [ProcessKind]),
}

/// The owners a declaration stores.
#[derive(Clone, Debug)]
pub(crate) enum OwnedOwners {
    All,
    Only(Box<[ProcessKind]>),
}

impl OwnedOwners {
    pub(crate) fn borrow(&self) -> Owners<'_> {
        match self {
            Self::All => Owners::All,
            Self::Only(kinds) => Owners::Only(kinds),
        }
    }

    pub(crate) fn contains(&self, kind: &ProcessKind) -> bool {
        match self {
            Self::All => true,
            Self::Only(kinds) => kinds.contains(kind),
        }
    }
}

/// The settings an enabling variant needs, outside the flag prefix, sorted.
#[derive(Clone, Debug)]
pub(crate) struct RequiredSettings {
    pub(crate) variant: &'static str,
    pub(crate) names: Box<[SettingName]>,
}

/// The kill switch that governs a release flag.
#[derive(Clone, Debug)]
pub(crate) struct GoverningSwitch {
    pub(crate) index: usize,
    pub(crate) key: FlagKey,
}

/// One built declaration.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) key: FlagKey,
    pub(crate) kind: Kind,
    pub(crate) value_type: ValueType,
    /// Wire variant names in the value type's declaration order; empty for a switch.
    pub(crate) variants: &'static [&'static str],
    /// The code default's wire variant name; `None` for a switch.
    pub(crate) default_variant: Option<&'static str>,
    pub(crate) absence: Absence,
    pub(crate) owners: OwnedOwners,
    pub(crate) pinned_by: Box<[ProcessKind]>,
    pub(crate) only_in: Option<EnvironmentClass>,
    pub(crate) expiry: Option<Expiry>,
    pub(crate) flip_discipline: FlipDiscipline,
    pub(crate) safe_state: Option<SafeState>,
    pub(crate) stale_rule: Option<StaleRule>,
    pub(crate) governed_by: Option<GoverningSwitch>,
    /// Sorted by variant name.
    pub(crate) requires_settings: Box<[RequiredSettings]>,
    /// Declared on this flag, sorted.
    pub(crate) invariants: Box<[Invariant]>,
    pub(crate) fingerprint: DeclarationFingerprint,
}

impl Entry {
    /// This declaration's catalog-digest inputs.
    pub(crate) fn digest_view(&self) -> EntryView<'_> {
        EntryView {
            key: self.key.as_str(),
            kind: self.kind,
            value_type: self.value_type,
            variants: self.variants.to_vec(),
            default_variant: self.default_variant,
            absence: self.absence,
            owners: match &self.owners {
                OwnedOwners::All => None,
                OwnedOwners::Only(kinds) => Some(kinds.iter().map(ProcessKind::name).collect()),
            },
            pinned_by: self.pinned_by.iter().map(ProcessKind::name).collect(),
            only_in: self.only_in.as_ref().map(EnvironmentClass::name),
            safe_state: self.safe_state,
            stale_rule: self.stale_rule,
            governed_by: self.governed_by.as_ref().map(|switch| switch.key.as_str()),
            requires_settings: self
                .requires_settings
                .iter()
                .map(|required| {
                    let names = required.names.iter().map(SettingName::as_str).collect();
                    (required.variant, names)
                })
                .collect(),
            invariants: self
                .invariants
                .iter()
                .map(|invariant| {
                    let other = invariant.other.as_str();
                    (
                        invariant.rule,
                        invariant.variant,
                        other,
                        invariant.other_variant,
                    )
                })
                .collect(),
        }
    }
}

/// A read-only view of one declaration in a built catalog.
#[derive(Clone, Copy)]
pub struct Declared<'a> {
    prefix: &'a SettingsPrefix,
    entry: &'a Entry,
}

impl<'a> Declared<'a> {
    pub(crate) fn new(prefix: &'a SettingsPrefix, entry: &'a Entry) -> Self {
        Self { prefix, entry }
    }

    /// The key.
    pub fn key(&self) -> &'a FlagKey {
        &self.entry.key
    }

    /// The setting name declared mode reads: the prefix plus the key in upper case.
    pub fn setting_name(&self) -> SettingName {
        self.entry.key.setting_name(self.prefix)
    }

    /// The kind.
    pub fn kind(&self) -> Kind {
        self.entry.kind
    }

    /// The value-type tag.
    pub fn value_type(&self) -> ValueType {
        self.entry.value_type
    }

    /// The wire variant names in the value type's declaration order; empty
    /// for a switch. A toggle's are `off` and `on`, whatever the catalog's
    /// [`ToggleLiterals`](crate::ToggleLiterals).
    pub fn variants(&self) -> &'static [&'static str] {
        self.entry.variants
    }

    /// The code default's wire variant name; `None` for a switch.
    pub fn default_variant(&self) -> Option<&'static str> {
        self.entry.default_variant
    }

    /// What an absent setting means.
    pub fn absence(&self) -> Absence {
        self.entry.absence
    }

    /// The owning kinds.
    pub fn owners(&self) -> Owners<'a> {
        self.entry.owners.borrow()
    }

    /// The kinds that may resolve pins of this flag without owning it.
    pub fn pinned_by(&self) -> &'a [ProcessKind] {
        &self.entry.pinned_by
    }

    /// The environment class the flag is restricted to, if any.
    pub fn only_in(&self) -> Option<&'a EnvironmentClass> {
        self.entry.only_in.as_ref()
    }

    /// The expiry of a release flag; `None` for other kinds.
    pub fn expiry(&self) -> Option<Expiry> {
        self.entry.expiry
    }

    /// The flip discipline.
    pub fn flip_discipline(&self) -> FlipDiscipline {
        self.entry.flip_discipline
    }

    /// A kill switch's safe state; `None` for other kinds.
    pub fn safe_state(&self) -> Option<SafeState> {
        self.entry.safe_state
    }

    /// A kill switch's stale rule; `None` for other kinds.
    pub fn stale_rule(&self) -> Option<StaleRule> {
        self.entry.stale_rule
    }

    /// The key of the kill switch governing this release flag, if any.
    pub fn governed_by(&self) -> Option<&'a FlagKey> {
        self.entry.governed_by.as_ref().map(|switch| &switch.key)
    }

    /// Each enabling variant, in name order, with the sorted setting names it
    /// requires.
    pub fn requires_settings(
        &self,
    ) -> impl ExactSizeIterator<Item = (&'static str, &'a [SettingName])> + 'a {
        self.entry
            .requires_settings
            .iter()
            .map(|required| (required.variant, &*required.names))
    }

    /// The cross-flag invariants declared on this flag, sorted.
    pub fn invariants(&self) -> &'a [Invariant] {
        &self.entry.invariants
    }

    /// The declaration's fingerprint.
    pub fn fingerprint(&self) -> DeclarationFingerprint {
        self.entry.fingerprint
    }
}

impl fmt::Debug for Declared<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Declared")
            .field("key", &self.entry.key.as_str())
            .field("kind", &self.entry.kind)
            .field("value_type", &self.entry.value_type)
            .finish_non_exhaustive()
    }
}

/// The relation a cross-flag invariant states.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum InvariantRule {
    /// When the flag is at its variant, the other flag must be at its variant.
    Requires,
    /// The flag at its variant and the other flag at its variant may not both hold.
    Conflicts,
    /// Either both toggles are `on` or neither is.
    BothOrNone,
}

impl InvariantRule {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requires => "requires",
            Self::Conflicts => "conflicts",
            Self::BothOrNone => "both_or_none",
        }
    }
}

/// A cross-flag invariant, validated once at startup by declared mode.
///
/// It relates this flag at [`variant`](Self::variant) to the other flag at
/// [`other_variant`](Self::other_variant) by [`rule`](Self::rule). The build
/// refuses an invariant the two code defaults already break.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Invariant {
    pub(crate) rule: InvariantRule,
    pub(crate) flag: FlagKey,
    pub(crate) variant: &'static str,
    pub(crate) other: FlagKey,
    pub(crate) other_variant: &'static str,
}

impl Invariant {
    /// The relation.
    pub fn rule(&self) -> InvariantRule {
        self.rule
    }

    /// The flag the invariant is declared on.
    pub fn flag(&self) -> &FlagKey {
        &self.flag
    }

    /// The wire variant of that flag the invariant speaks about.
    pub fn variant(&self) -> &'static str {
        self.variant
    }

    /// The other flag.
    pub fn other(&self) -> &FlagKey {
        &self.other
    }

    /// The wire variant of the other flag.
    pub fn other_variant(&self) -> &'static str {
        self.other_variant
    }

    /// Whether the two resolved variants satisfy the invariant.
    pub fn holds(&self, variant: &str, other_variant: &str) -> bool {
        let this = variant == self.variant;
        let that = other_variant == self.other_variant;
        match self.rule {
            InvariantRule::Requires => !this || that,
            InvariantRule::Conflicts => !(this && that),
            InvariantRule::BothOrNone => this == that,
        }
    }
}

/// A retired setting name that fails startup when present, with the note
/// that tells an operator how to migrate.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Retired {
    pub(crate) name: SettingName,
    pub(crate) migration_note: Arc<str>,
}

impl Retired {
    /// The retired setting name.
    pub fn name(&self) -> &SettingName {
        &self.name
    }

    /// The migration note reported when the name is present.
    pub fn migration_note(&self) -> &str {
        &self.migration_note
    }
}
