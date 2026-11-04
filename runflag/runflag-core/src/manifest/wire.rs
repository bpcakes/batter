//! The decoded form of manifest records, private to manifest decoding.
//!
//! The public records only serialize. Decoding fills these mirrors, which
//! have the same fields and JSON shape, and converts them into public records
//! that [`validate`](super::validate) then checks against the whole manifest,
//! so no public record is ever built from unvalidated input.

use serde::Deserialize;

use super::{
    ManifestEntry, ManifestInvariant, ManifestOwners, ManifestRequiredSettings, ManifestRetired,
};
use crate::{
    date::CivilDate,
    declaration::{Absence, FlipDiscipline, InvariantRule, Kind, SafeState, StaleRule},
    digest::DeclarationFingerprint,
    key::{FlagKey, SettingName},
    value::ValueType,
};

#[derive(Deserialize)]
pub(super) struct WireEntry {
    key: FlagKey,
    setting_name: SettingName,
    kind: Kind,
    value_type: ValueType,
    fingerprint: DeclarationFingerprint,
    variants: Vec<String>,
    default: Option<String>,
    absence: Absence,
    owners: WireOwners,
    pinned_by: Vec<String>,
    only_in: Option<String>,
    expiry: Option<CivilDate>,
    flip_discipline: FlipDiscipline,
    safe_state: Option<SafeState>,
    stale_rule: Option<StaleRule>,
    governed_by: Option<FlagKey>,
    requires_settings: Vec<WireRequiredSettings>,
    invariants: Vec<WireInvariant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireOwners {
    All,
    Only(Vec<String>),
}

#[derive(Deserialize)]
struct WireRequiredSettings {
    variant: String,
    settings: Vec<SettingName>,
}

#[derive(Deserialize)]
struct WireInvariant {
    rule: InvariantRule,
    variant: String,
    other: FlagKey,
    other_variant: String,
}

#[derive(Deserialize)]
pub(super) struct WireRetired {
    name: SettingName,
    migration_note: String,
}

impl From<WireEntry> for ManifestEntry {
    fn from(wire: WireEntry) -> Self {
        Self {
            key: wire.key,
            setting_name: wire.setting_name,
            kind: wire.kind,
            value_type: wire.value_type,
            fingerprint: wire.fingerprint,
            variants: wire.variants,
            default: wire.default,
            absence: wire.absence,
            owners: match wire.owners {
                WireOwners::All => ManifestOwners::All,
                WireOwners::Only(kinds) => ManifestOwners::Only(kinds),
            },
            pinned_by: wire.pinned_by,
            only_in: wire.only_in,
            expiry: wire.expiry,
            flip_discipline: wire.flip_discipline,
            safe_state: wire.safe_state,
            stale_rule: wire.stale_rule,
            governed_by: wire.governed_by,
            requires_settings: wire
                .requires_settings
                .into_iter()
                .map(|required| ManifestRequiredSettings {
                    variant: required.variant,
                    settings: required.settings,
                })
                .collect(),
            invariants: wire
                .invariants
                .into_iter()
                .map(|invariant| ManifestInvariant {
                    rule: invariant.rule,
                    variant: invariant.variant,
                    other: invariant.other,
                    other_variant: invariant.other_variant,
                })
                .collect(),
        }
    }
}

impl From<WireRetired> for ManifestRetired {
    fn from(wire: WireRetired) -> Self {
        Self {
            name: wire.name,
            migration_note: wire.migration_note,
        }
    }
}
