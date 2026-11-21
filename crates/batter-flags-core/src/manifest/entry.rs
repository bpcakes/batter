//! One declaration in a manifest, and the records it carries.

use serde::Serialize;

use super::{kind_names, sorted};
use crate::{
    date::{CivilDate, Expiry},
    declaration::{
        Absence, Entry, FlipDiscipline, InvariantRule, Kind, OwnedOwners, SafeState, StaleRule,
    },
    digest::{DeclarationFingerprint, EntryView},
    key::{FlagKey, SettingName, SettingsPrefix},
    value::ValueType,
};

/// One declaration in a [`Manifest`](super::Manifest).
///
/// It has no decoder of its own: only decoding a whole manifest constructs
/// it, after validating it against the rest of that manifest. The same holds
/// for [`ManifestOwners`], [`ManifestRequiredSettings`], [`ManifestInvariant`]
/// and [`ManifestRetired`].
///
/// ```compile_fail,E0277
/// let _entry: batter_flags_core::ManifestEntry = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManifestEntry {
    pub(in crate::manifest) key: FlagKey,
    pub(in crate::manifest) setting_name: SettingName,
    pub(in crate::manifest) kind: Kind,
    pub(in crate::manifest) value_type: ValueType,
    pub(in crate::manifest) fingerprint: DeclarationFingerprint,
    pub(in crate::manifest) variants: Vec<String>,
    pub(in crate::manifest) default: Option<String>,
    pub(in crate::manifest) absence: Absence,
    pub(in crate::manifest) owners: ManifestOwners,
    pub(in crate::manifest) pinned_by: Vec<String>,
    pub(in crate::manifest) only_in: Option<String>,
    pub(in crate::manifest) expiry: Option<CivilDate>,
    pub(in crate::manifest) flip_discipline: FlipDiscipline,
    pub(in crate::manifest) safe_state: Option<SafeState>,
    pub(in crate::manifest) stale_rule: Option<StaleRule>,
    pub(in crate::manifest) governed_by: Option<FlagKey>,
    pub(in crate::manifest) requires_settings: Vec<ManifestRequiredSettings>,
    pub(in crate::manifest) invariants: Vec<ManifestInvariant>,
}

impl ManifestEntry {
    /// The key.
    pub fn key(&self) -> &FlagKey {
        &self.key
    }

    /// The setting name declared mode reads.
    pub fn setting_name(&self) -> &SettingName {
        &self.setting_name
    }

    /// The kind.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The value-type tag.
    pub fn value_type(&self) -> ValueType {
        self.value_type
    }

    /// The declaration fingerprint.
    pub fn fingerprint(&self) -> DeclarationFingerprint {
        self.fingerprint
    }

    /// The wire variant names, sorted; empty for a switch.
    pub fn variants(&self) -> &[String] {
        &self.variants
    }

    /// The code default's wire variant; `None` for a switch.
    pub fn default_variant(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// What an absent setting means.
    pub fn absence(&self) -> Absence {
        self.absence
    }

    /// The owning kinds.
    pub fn owners(&self) -> &ManifestOwners {
        &self.owners
    }

    /// The kinds that may resolve pins without owning the flag, sorted.
    pub fn pinned_by(&self) -> &[String] {
        &self.pinned_by
    }

    /// The environment class the flag is restricted to, if any.
    pub fn only_in(&self) -> Option<&str> {
        self.only_in.as_deref()
    }

    /// A release flag's last valid day.
    pub fn expiry(&self) -> Option<CivilDate> {
        self.expiry
    }

    /// The flip discipline.
    pub fn flip_discipline(&self) -> FlipDiscipline {
        self.flip_discipline
    }

    /// A kill switch's safe state.
    pub fn safe_state(&self) -> Option<SafeState> {
        self.safe_state
    }

    /// A kill switch's stale rule.
    pub fn stale_rule(&self) -> Option<StaleRule> {
        self.stale_rule
    }

    /// The governing switch of a release flag, if any.
    pub fn governed_by(&self) -> Option<&FlagKey> {
        self.governed_by.as_ref()
    }

    /// The settings each enabling variant requires, in variant order.
    pub fn requires_settings(&self) -> &[ManifestRequiredSettings] {
        &self.requires_settings
    }

    /// The invariants declared on this flag, sorted.
    pub fn invariants(&self) -> &[ManifestInvariant] {
        &self.invariants
    }

    pub(super) fn from_entry(prefix: &SettingsPrefix, entry: &Entry) -> Self {
        Self {
            key: entry.key.clone(),
            setting_name: entry.key.setting_name(prefix),
            kind: entry.kind,
            value_type: entry.value_type,
            fingerprint: entry.fingerprint,
            variants: sorted(entry.variants.iter().map(|variant| (*variant).to_owned())),
            default: entry.default_variant.map(str::to_owned),
            absence: entry.absence,
            owners: match &entry.owners {
                OwnedOwners::All => ManifestOwners::All,
                OwnedOwners::Only(kinds) => ManifestOwners::Only(kind_names(kinds)),
            },
            pinned_by: kind_names(&entry.pinned_by),
            only_in: entry.only_in.as_ref().map(|class| class.name().to_owned()),
            expiry: entry.expiry.map(Expiry::last_valid_day),
            flip_discipline: entry.flip_discipline,
            safe_state: entry.safe_state,
            stale_rule: entry.stale_rule,
            governed_by: entry.governed_by.as_ref().map(|switch| switch.key.clone()),
            requires_settings: entry
                .requires_settings
                .iter()
                .map(|required| ManifestRequiredSettings {
                    variant: required.variant.to_owned(),
                    settings: required.names.to_vec(),
                })
                .collect(),
            invariants: entry
                .invariants
                .iter()
                .map(|invariant| ManifestInvariant {
                    rule: invariant.rule,
                    variant: invariant.variant.to_owned(),
                    other: invariant.other.clone(),
                    other_variant: invariant.other_variant.to_owned(),
                })
                .collect(),
        }
    }

    pub(super) fn digest_view(&self) -> EntryView<'_> {
        EntryView {
            key: self.key.as_str(),
            kind: self.kind,
            value_type: self.value_type,
            variants: self.variants.iter().map(String::as_str).collect(),
            default_variant: self.default.as_deref(),
            absence: self.absence,
            owners: match &self.owners {
                ManifestOwners::All => None,
                ManifestOwners::Only(kinds) => Some(kinds.iter().map(String::as_str).collect()),
            },
            pinned_by: self.pinned_by.iter().map(String::as_str).collect(),
            only_in: self.only_in.as_deref(),
            safe_state: self.safe_state,
            stale_rule: self.stale_rule,
            governed_by: self.governed_by.as_ref().map(FlagKey::as_str),
            requires_settings: self
                .requires_settings
                .iter()
                .map(|required| {
                    let names = required.settings.iter().map(SettingName::as_str).collect();
                    (required.variant.as_str(), names)
                })
                .collect(),
            invariants: self
                .invariants
                .iter()
                .map(|invariant| {
                    let other = invariant.other.as_str();
                    (
                        invariant.rule,
                        invariant.variant.as_str(),
                        other,
                        invariant.other_variant.as_str(),
                    )
                })
                .collect(),
        }
    }
}

/// The owners of a manifest entry: `"all"`, or `{"only": [...]}` with sorted
/// kind names.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestOwners {
    /// Every process kind owns the declaration.
    All,
    /// Only the listed kinds own it.
    Only(Vec<String>),
}

/// The settings one enabling variant requires.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManifestRequiredSettings {
    pub(in crate::manifest) variant: String,
    pub(in crate::manifest) settings: Vec<SettingName>,
}

impl ManifestRequiredSettings {
    /// The enabling variant.
    pub fn variant(&self) -> &str {
        &self.variant
    }

    /// The setting names it requires, sorted.
    pub fn settings(&self) -> &[SettingName] {
        &self.settings
    }
}

/// One invariant declared on a manifest entry.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ManifestInvariant {
    pub(in crate::manifest) rule: InvariantRule,
    pub(in crate::manifest) variant: String,
    pub(in crate::manifest) other: FlagKey,
    pub(in crate::manifest) other_variant: String,
}

impl ManifestInvariant {
    /// The relation.
    pub fn rule(&self) -> InvariantRule {
        self.rule
    }

    /// The variant of the entry the invariant is declared on.
    pub fn variant(&self) -> &str {
        &self.variant
    }

    /// The other flag.
    pub fn other(&self) -> &FlagKey {
        &self.other
    }

    /// The other flag's variant.
    pub fn other_variant(&self) -> &str {
        &self.other_variant
    }
}

/// A retired name and its migration note.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ManifestRetired {
    pub(in crate::manifest) name: SettingName,
    pub(in crate::manifest) migration_note: String,
}

impl ManifestRetired {
    /// The retired setting name.
    pub fn name(&self) -> &SettingName {
        &self.name
    }

    /// The note that tells an operator how to migrate.
    pub fn migration_note(&self) -> &str {
        &self.migration_note
    }
}
