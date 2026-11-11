//! Decoding a manifest: normalization, structure, references and digests.

use std::{collections::BTreeSet, fmt};

use serde::Deserialize;

use super::{
    Manifest, ManifestEntry, ManifestInvariant, ManifestOwners, Version,
    wire::{WireEntry, WireRetired},
};
use crate::{
    declaration::{InvariantRule, Kind},
    digest::{CatalogDigest, DeclarationFingerprint},
    key::{FlagKey, SettingsPrefix},
    value::ValueType,
};

/// The manifest as decoded, before validation.
#[derive(Deserialize)]
pub struct WireManifest {
    v: Version,
    prefix: SettingsPrefix,
    catalog_digest: CatalogDigest,
    process_kinds: Vec<String>,
    environment_classes: Vec<String>,
    flags: Vec<WireEntry>,
    retired: Vec<WireRetired>,
}

/// Why a decoded manifest was refused; surfaced through the decoder's error.
#[derive(Debug)]
pub struct Invalid(String);

impl fmt::Display for Invalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid manifest: {}", self.0)
    }
}

macro_rules! refuse {
    ($($argument:tt)*) => {
        return Err(Invalid(format!($($argument)*)))
    };
}

impl TryFrom<WireManifest> for Manifest {
    type Error = Invalid;

    fn try_from(wire: WireManifest) -> Result<Self, Invalid> {
        let mut manifest = Self {
            v: wire.v,
            prefix: wire.prefix,
            catalog_digest: wire.catalog_digest,
            process_kinds: wire.process_kinds,
            environment_classes: wire.environment_classes,
            flags: wire.flags.into_iter().map(ManifestEntry::from).collect(),
            retired: wire.retired.into_iter().map(Into::into).collect(),
        };
        normalize(&mut manifest);
        if manifest.process_kinds.is_empty() {
            refuse!("a catalog needs at least one process kind");
        }
        if manifest.environment_classes.is_empty() {
            refuse!("a catalog needs at least one environment class");
        }
        check_names("process kind", &manifest.process_kinds)?;
        check_names("environment class", &manifest.environment_classes)?;
        check_retired(&manifest)?;
        if let Some(pair) = manifest
            .flags
            .windows(2)
            .find(|pair| pair[0].key == pair[1].key)
        {
            refuse!("flag {} is listed twice", pair[0].key);
        }
        for entry in &manifest.flags {
            check_entry(&manifest, entry)?;
        }
        check_retired_unused(&manifest)?;
        if CatalogDigest::compute(manifest.digest_input()) != manifest.catalog_digest {
            refuse!("the catalog digest does not match the declarations");
        }
        Ok(manifest)
    }
}

fn normalize(manifest: &mut Manifest) {
    manifest.process_kinds.sort_unstable();
    manifest.environment_classes.sort_unstable();
    manifest
        .retired
        .sort_unstable_by(|left, right| left.name.cmp(&right.name));
    manifest
        .flags
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    for entry in &mut manifest.flags {
        entry.variants.sort_unstable();
        if let ManifestOwners::Only(kinds) = &mut entry.owners {
            kinds.sort_unstable();
        }
        entry.pinned_by.sort_unstable();
        entry
            .requires_settings
            .sort_unstable_by(|left, right| left.variant.cmp(&right.variant));
        for required in &mut entry.requires_settings {
            required.settings.sort_unstable();
        }
        entry.invariants.sort_unstable();
    }
}

fn check_names(what: &str, names: &[String]) -> Result<(), Invalid> {
    for (index, name) in names.iter().enumerate() {
        if let Err(cause) = FlagKey::new(name) {
            refuse!("{what} {name:?}: {cause}");
        }
        if index > 0 && names[index - 1] == *name {
            refuse!("{what} {name:?} is listed twice");
        }
    }
    Ok(())
}

fn check_retired(manifest: &Manifest) -> Result<(), Invalid> {
    for (index, retired) in manifest.retired.iter().enumerate() {
        if index > 0 && manifest.retired[index - 1].name == retired.name {
            refuse!("retired name {} is listed twice", retired.name);
        }
        if retired.migration_note.trim().is_empty() {
            refuse!("retired name {} has no migration note", retired.name);
        }
    }
    Ok(())
}

/// A retired name may be neither a declared setting name nor a required setting.
fn check_retired_unused(manifest: &Manifest) -> Result<(), Invalid> {
    for entry in &manifest.flags {
        let required = entry.requires_settings.iter().flat_map(|r| &r.settings);
        for name in std::iter::once(&entry.setting_name).chain(required) {
            if manifest
                .retired
                .binary_search_by(|retired| retired.name.cmp(name))
                .is_ok()
            {
                refuse!("setting name {name} is retired and also in use");
            }
        }
    }
    Ok(())
}

fn check_entry(manifest: &Manifest, entry: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    if *entry.setting_name() != key.setting_name(&manifest.prefix) {
        refuse!(
            "flag {key}: setting name {} does not derive from the prefix",
            entry.setting_name
        );
    }
    match entry.kind {
        Kind::Switch => check_switch(entry)?,
        Kind::Release | Kind::Mode => check_flag(manifest, entry)?,
    }
    check_vocabulary(manifest, entry)?;
    let variants: Vec<&str> = entry.variants.iter().map(String::as_str).collect();
    let fingerprint = DeclarationFingerprint::compute(
        key.as_str(),
        entry.kind.as_str(),
        entry.value_type.as_str(),
        &variants,
    );
    if fingerprint != entry.fingerprint {
        refuse!("flag {key}: the fingerprint does not match the declaration");
    }
    Ok(())
}

fn check_switch(entry: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    if entry.value_type != ValueType::Switch {
        refuse!("flag {key}: a kill switch has the value type switch");
    }
    if entry.safe_state.is_none() || entry.stale_rule.is_none() {
        refuse!("flag {key}: a kill switch needs a safe state and a stale rule");
    }
    let flag_only = !entry.variants.is_empty()
        || entry.default.is_some()
        || entry.expiry.is_some()
        || entry.governed_by.is_some()
        || !entry.pinned_by.is_empty()
        || entry.only_in.is_some()
        || !entry.requires_settings.is_empty()
        || !entry.invariants.is_empty();
    if flag_only {
        refuse!("flag {key}: a kill switch carries a field only release and mode flags have");
    }
    Ok(())
}

fn check_flag(manifest: &Manifest, entry: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    if entry.value_type == ValueType::Switch {
        refuse!("flag {key}: only a kill switch has the value type switch");
    }
    if entry.safe_state.is_some() || entry.stale_rule.is_some() {
        refuse!("flag {key}: only a kill switch has a safe state or a stale rule");
    }
    if entry.expiry.is_some() != (entry.kind == Kind::Release) {
        refuse!("flag {key}: a release flag, and only a release flag, has an expiry");
    }
    check_variants(entry)?;
    if let Some(switch) = &entry.governed_by {
        let governing = manifest
            .flag(switch.as_str())
            .filter(|governing| governing.kind == Kind::Switch && entry.kind == Kind::Release);
        let Some(governing) = governing else {
            refuse!("flag {key}: only a release flag is governed, and only by a kill switch");
        };
        check_governing(entry, governing)?;
    }
    if entry
        .requires_settings
        .windows(2)
        .any(|pair| pair[0].variant == pair[1].variant)
    {
        refuse!("flag {key}: requires settings for one variant twice");
    }
    if entry.invariants.windows(2).any(|pair| pair[0] == pair[1]) {
        refuse!("flag {key}: an invariant is listed twice");
    }
    let variants: BTreeSet<&str> = entry.variants.iter().map(String::as_str).collect();
    for required in &entry.requires_settings {
        if !variants.contains(required.variant.as_str())
            || entry.default.as_deref() == Some(required.variant.as_str())
        {
            refuse!("flag {key}: settings are required by an undeclared or default variant");
        }
        if required.settings.is_empty()
            || required.settings.windows(2).any(|pair| pair[0] == pair[1])
            || required
                .settings
                .iter()
                .any(|name| manifest.prefix.covers(name.as_str()))
        {
            refuse!("flag {key}: required settings must be distinct names outside the prefix");
        }
    }
    for invariant in &entry.invariants {
        check_invariant(manifest, entry, &variants, invariant)?;
    }
    Ok(())
}

/// Every kind that owns or pins the flag must own its governing switch.
fn check_governing(entry: &ManifestEntry, switch: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    let ManifestOwners::Only(switch_owners) = &switch.owners else {
        return Ok(());
    };
    let ManifestOwners::Only(owners) = &entry.owners else {
        refuse!(
            "flag {key} is owned by every process kind but its governing switch {} is not",
            switch.key
        );
    };
    let unowned = owners
        .iter()
        .chain(&entry.pinned_by)
        .find(|kind| switch_owners.binary_search(kind).is_err());
    if let Some(kind) = unowned {
        refuse!(
            "flag {key}: process kind {kind:?} owns or pins the flag but not its governing switch {}",
            switch.key
        );
    }
    Ok(())
}

fn check_variants(entry: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    if entry.variants.is_empty() {
        refuse!("flag {key}: a flag needs at least one variant");
    }
    for (index, variant) in entry.variants.iter().enumerate() {
        if FlagKey::new(variant).is_err() || (index > 0 && entry.variants[index - 1] == *variant) {
            refuse!("flag {key}: variant {variant:?} is invalid or listed twice");
        }
    }
    if entry.value_type == ValueType::Toggle && entry.variants != ["off", "on"] {
        refuse!("flag {key}: a toggle has exactly the variants off and on");
    }
    match entry.default.as_deref() {
        Some(default) if entry.variants.iter().any(|variant| variant == default) => Ok(()),
        _ => refuse!("flag {key}: the default is not a declared variant"),
    }
}

fn check_invariant(
    manifest: &Manifest,
    entry: &ManifestEntry,
    variants: &BTreeSet<&str>,
    invariant: &ManifestInvariant,
) -> Result<(), Invalid> {
    let key = &entry.key;
    let other = manifest
        .flag(invariant.other.as_str())
        .filter(|other| other.kind != Kind::Switch && other.key != *key);
    let Some(other) = other else {
        refuse!(
            "flag {key}: an invariant names {} which is not another flag",
            invariant.other
        );
    };
    let declared = variants.contains(invariant.variant.as_str())
        && other.variants.contains(&invariant.other_variant);
    let toggles = entry.value_type == ValueType::Toggle && other.value_type == ValueType::Toggle;
    if !declared || (invariant.rule == InvariantRule::BothOrNone && !toggles) {
        refuse!(
            "flag {key}: the invariant on {} names an undeclared variant",
            invariant.other
        );
    }
    let both_on = invariant.variant == "on" && invariant.other_variant == "on";
    if invariant.rule == InvariantRule::BothOrNone && !both_on {
        refuse!(
            "flag {key}: the both_or_none invariant on {} must relate both toggles at on",
            invariant.other
        );
    }
    let defaults = (
        entry.default.as_deref().unwrap_or_default(),
        invariant.variant.as_str(),
    );
    let other_defaults = (
        other.default.as_deref().unwrap_or_default(),
        invariant.other_variant.as_str(),
    );
    if !invariant.rule.admits(defaults, other_defaults) {
        refuse!(
            "flag {key}: the code defaults break its invariant on {}",
            invariant.other
        );
    }
    Ok(())
}

fn check_vocabulary(manifest: &Manifest, entry: &ManifestEntry) -> Result<(), Invalid> {
    let key = &entry.key;
    let known = |name: &String| manifest.process_kinds.binary_search(name).is_ok();
    let owners: &[String] = match &entry.owners {
        ManifestOwners::All => &[],
        ManifestOwners::Only(kinds) => {
            if kinds.is_empty() || kinds.windows(2).any(|pair| pair[0] == pair[1]) {
                refuse!("flag {key}: owners must be a non-empty list of distinct kinds");
            }
            kinds
        }
    };
    if entry.pinned_by.windows(2).any(|pair| pair[0] == pair[1]) {
        refuse!("flag {key}: a pinned_by kind is listed twice");
    }
    if !owners.iter().chain(&entry.pinned_by).all(known) {
        refuse!("flag {key}: an owner or pinned_by kind is not declared");
    }
    let overlap = entry.pinned_by.iter().any(|kind| owners.contains(kind));
    let all_owners = matches!(entry.owners, ManifestOwners::All);
    if !entry.pinned_by.is_empty() && (overlap || all_owners) {
        refuse!("flag {key}: pinned_by kinds must be distinct non-owners");
    }
    if let Some(class) = &entry.only_in
        && manifest.environment_classes.binary_search(class).is_err()
    {
        refuse!("flag {key}: environment class {class:?} is not declared");
    }
    Ok(())
}
