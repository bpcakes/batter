//! The validation every declaration kind shares.

use crate::{
    builder::CatalogBuilder,
    date::Expiry,
    declaration::{
        Absence, Entry, FlipDiscipline, GoverningSwitch, Invariant, InvariantRule, Kind,
        OwnedOwners, Owners, RequiredSettings, SafeState, StaleRule,
    },
    digest::DeclarationFingerprint,
    error::BuildError,
    handle::{Handle, KillSwitch},
    key::{FlagKey, SettingName},
    value::ValueType,
    vocabulary::{EnvironmentClass, ProcessKind},
};

/// What a declaration's kind fixes when it is declared.
pub(super) struct Shape {
    pub(super) kind: Kind,
    pub(super) value_type: ValueType,
    pub(super) variants: &'static [&'static str],
    pub(super) default_variant: Option<&'static str>,
    pub(super) expiry: Option<Expiry>,
    pub(super) safe_state: Option<SafeState>,
    pub(super) stale_rule: Option<StaleRule>,
}

/// A declaration in progress. The first error wins and is returned by
/// [`finish`](Self::finish); later calls change nothing.
pub(super) struct Draft<'b> {
    builder: &'b mut CatalogBuilder,
    key: Result<FlagKey, BuildError>,
    /// The value type's declared variants; empty for a switch.
    variants: &'static [&'static str],
    error: Option<BuildError>,
    owners: Option<OwnedOwners>,
    pinned_by: Option<Box<[ProcessKind]>>,
    only_in: Option<EnvironmentClass>,
    absence: Option<Absence>,
    flip_discipline: Option<FlipDiscipline>,
    governed_by: Option<GoverningSwitch>,
    requires_settings: Vec<RequiredSettings>,
    invariants: Vec<Invariant>,
}

impl<'b> Draft<'b> {
    pub(super) fn new(
        builder: &'b mut CatalogBuilder,
        key: &str,
        variants: &'static [&'static str],
    ) -> Self {
        let key = FlagKey::new(key).map_err(|cause| BuildError::InvalidKey {
            key: Box::from(key),
            cause,
        });
        Self {
            builder,
            key,
            variants,
            error: None,
            owners: None,
            pinned_by: None,
            only_in: None,
            absence: None,
            flip_discipline: None,
            governed_by: None,
            requires_settings: Vec::new(),
            invariants: Vec::new(),
        }
    }

    pub(super) fn key(&self) -> Option<&FlagKey> {
        self.key.as_ref().ok()
    }

    /// Records the first error; an invalid key outranks everything after it.
    pub(super) fn fail(&mut self, make: impl FnOnce(FlagKey) -> BuildError) {
        if self.error.is_none()
            && let Ok(key) = &self.key
        {
            self.error = Some(make(key.clone()));
        }
    }

    fn repeated(&mut self, method: &'static str) {
        self.fail(|key| BuildError::RepeatedCall { key, method });
    }

    pub(super) fn owners(&mut self, owners: Owners<'_>) {
        if self.owners.is_some() {
            return self.repeated("owners");
        }
        let owned = match owners {
            Owners::All => OwnedOwners::All,
            Owners::Only([]) => return self.fail(|key| BuildError::EmptyOwners { key }),
            Owners::Only(kinds) => match self.checked_kinds(kinds) {
                Some(kinds) => OwnedOwners::Only(kinds),
                None => return,
            },
        };
        self.owners = Some(owned);
    }

    pub(super) fn pinned_by(&mut self, kinds: &[ProcessKind]) {
        if self.pinned_by.is_some() {
            return self.repeated("pinned_by");
        }
        if kinds.is_empty() {
            return self.fail(|key| BuildError::EmptyPinnedBy { key });
        }
        if let Some(kinds) = self.checked_kinds(kinds) {
            self.pinned_by = Some(kinds);
        }
    }

    pub(super) fn only_in(&mut self, class: &EnvironmentClass) {
        if self.only_in.is_some() {
            return self.repeated("only_in");
        }
        if class.catalog() != self.builder.id {
            return self.fail(|key| BuildError::ForeignEnvironmentClass {
                key,
                class: Box::from(class.name()),
            });
        }
        self.only_in = Some(class.clone());
    }

    pub(super) fn absence(&mut self, absence: Absence) {
        if self.absence.is_some() {
            return self.repeated("absence");
        }
        self.absence = Some(absence);
    }

    pub(super) fn flip_discipline(&mut self, discipline: FlipDiscipline) {
        if self.flip_discipline.is_some() {
            return self.repeated("flip_discipline");
        }
        self.flip_discipline = Some(discipline);
    }

    pub(super) fn governed_by(&mut self, switch: &KillSwitch) {
        if self.governed_by.is_some() {
            return self.repeated("governed_by");
        }
        let handle = switch.handle();
        if handle.catalog() != self.builder.id {
            return self.fail(|key| BuildError::ForeignHandle {
                key,
                handle: handle.key().clone(),
            });
        }
        self.governed_by = Some(GoverningSwitch {
            index: handle.index(),
            key: handle.key().clone(),
        });
    }

    pub(super) fn requires_settings(
        &mut self,
        variant: &'static str,
        default: &'static str,
        names: &[&'static str],
    ) {
        if !self.variants.contains(&variant) {
            return self.fail(|key| BuildError::UndeclaredVariant {
                flag: key.clone(),
                key,
                variant,
            });
        }
        if variant == default {
            return self.fail(|key| BuildError::RequiredSettingsOnDefault { key, variant });
        }
        if self.requires_settings.iter().any(|r| r.variant == variant) {
            return self.fail(|key| BuildError::DuplicateRequiresSettings { key, variant });
        }
        if names.is_empty() {
            return self.fail(|key| BuildError::EmptyRequiredSettings { key, variant });
        }
        let mut checked: Vec<SettingName> = Vec::with_capacity(names.len());
        for name in names {
            let name = match SettingName::new(name) {
                Ok(valid) => valid,
                Err(cause) => {
                    return self.fail(|key| BuildError::InvalidRequiredSetting {
                        key,
                        name: Box::from(*name),
                        cause,
                    });
                }
            };
            if self.builder.prefix.covers(name.as_str()) {
                return self.fail(|key| BuildError::RequiredSettingUnderPrefix { key, name });
            }
            if checked.contains(&name) {
                return self.fail(|key| BuildError::DuplicateRequiredSetting { key, name });
            }
            if self.builder.retired.contains_key(&name) {
                return self.fail(|_| BuildError::RetiredNameInUse { name });
            }
            checked.push(name);
        }
        checked.sort_unstable();
        self.requires_settings.push(RequiredSettings {
            variant,
            names: checked.into_boxed_slice(),
        });
    }

    pub(super) fn invariant(
        &mut self,
        rule: InvariantRule,
        (variant, default): (&'static str, &'static str),
        other: &Handle,
        other_variant: &'static str,
    ) {
        if other.catalog() != self.builder.id {
            return self.fail(|key| BuildError::ForeignHandle {
                key,
                handle: other.key().clone(),
            });
        }
        let Some(key) = self.key().cloned() else {
            return;
        };
        if !self.variants.contains(&variant) {
            return self.fail(|key| BuildError::UndeclaredVariant {
                flag: key.clone(),
                key,
                variant,
            });
        }
        if !self.builder.entries[other.index()]
            .variants
            .contains(&other_variant)
        {
            return self.fail(|key| BuildError::UndeclaredVariant {
                key,
                flag: other.key().clone(),
                variant: other_variant,
            });
        }
        let invariant = Invariant {
            rule,
            flag: key,
            variant,
            other: other.key().clone(),
            other_variant,
        };
        if self.invariants.contains(&invariant) {
            return self.fail(|key| BuildError::DuplicateInvariant {
                key,
                rule,
                other: other.key().clone(),
            });
        }
        let other_default = self.builder.entries[other.index()]
            .default_variant
            .unwrap_or_default();
        if !invariant.holds(default, other_default) {
            return self.fail(|key| BuildError::DefaultsViolateInvariant {
                key,
                rule,
                other: other.key().clone(),
            });
        }
        self.invariants.push(invariant);
    }

    /// Validates the declaration against the catalog so far, records it and
    /// returns its handle.
    pub(super) fn finish(self, shape: &Shape) -> Result<Handle, BuildError> {
        let key = self.key?;
        if let Some(error) = self.error {
            return Err(error);
        }
        let builder = self.builder;
        if builder.keys.contains_key(&key) {
            return Err(BuildError::DuplicateKey { key });
        }
        let setting_name = key.setting_name(&builder.prefix);
        if builder.retired.contains_key(&setting_name) {
            return Err(BuildError::RetiredNameInUse { name: setting_name });
        }
        let owners = self.owners.unwrap_or(OwnedOwners::All);
        let pinned_by = self.pinned_by.unwrap_or_default();
        check_pinned_by(&key, &owners, &pinned_by)?;
        if let Some(switch) = &self.governed_by {
            let switch_owners = &builder.entries[switch.index].owners;
            check_governing(&key, switch, switch_owners, &owners, &pinned_by)?;
        }
        let mut requires_settings = self.requires_settings;
        requires_settings.sort_unstable_by_key(|required| required.variant);
        let mut invariants = self.invariants;
        invariants.sort_unstable();
        let fingerprint = DeclarationFingerprint::compute(
            key.as_str(),
            shape.kind.as_str(),
            shape.value_type.as_str(),
            shape.variants,
        );
        let index = builder.entries.len();
        builder.entries.push(Entry {
            key: key.clone(),
            kind: shape.kind,
            value_type: shape.value_type,
            variants: shape.variants,
            default_variant: shape.default_variant,
            absence: self.absence.unwrap_or(Absence::Default),
            owners,
            pinned_by,
            only_in: self.only_in,
            expiry: shape.expiry,
            flip_discipline: self.flip_discipline.unwrap_or(FlipDiscipline::Rolling),
            safe_state: shape.safe_state,
            stale_rule: shape.stale_rule,
            governed_by: self.governed_by,
            requires_settings: requires_settings.into_boxed_slice(),
            invariants: invariants.into_boxed_slice(),
            fingerprint,
        });
        builder.keys.insert(key.clone(), index);
        Ok(Handle::new(builder.id, index, key, fingerprint))
    }

    fn checked_kinds(&mut self, kinds: &[ProcessKind]) -> Option<Box<[ProcessKind]>> {
        for (index, kind) in kinds.iter().enumerate() {
            if kind.catalog() != self.builder.id {
                self.fail(|key| BuildError::ForeignProcessKind {
                    key,
                    kind: Box::from(kind.name()),
                });
                return None;
            }
            if kinds[..index].contains(kind) {
                self.fail(|key| BuildError::DuplicateKind {
                    key,
                    kind: Box::from(kind.name()),
                });
                return None;
            }
        }
        let mut sorted = kinds.to_vec();
        sorted.sort_unstable_by(|left, right| left.name().cmp(right.name()));
        Some(sorted.into_boxed_slice())
    }
}

fn check_pinned_by(
    key: &FlagKey,
    owners: &OwnedOwners,
    pinned_by: &[ProcessKind],
) -> Result<(), BuildError> {
    if pinned_by.is_empty() {
        return Ok(());
    }
    if matches!(owners, OwnedOwners::All) {
        return Err(BuildError::PinnedByWithAllOwners { key: key.clone() });
    }
    match pinned_by.iter().find(|kind| owners.contains(kind)) {
        Some(kind) => Err(BuildError::PinnedByOwner {
            key: key.clone(),
            kind: Box::from(kind.name()),
        }),
        None => Ok(()),
    }
}

/// Every kind that evaluates or resolves the flag must read its switch.
fn check_governing(
    key: &FlagKey,
    switch: &GoverningSwitch,
    switch_owners: &OwnedOwners,
    owners: &OwnedOwners,
    pinned_by: &[ProcessKind],
) -> Result<(), BuildError> {
    let readers = match (owners, switch_owners) {
        (_, OwnedOwners::All) => return Ok(()),
        (OwnedOwners::All, OwnedOwners::Only(_)) => {
            return Err(BuildError::GoverningSwitchRestricted {
                key: key.clone(),
                switch: switch.key.clone(),
            });
        }
        (OwnedOwners::Only(kinds), OwnedOwners::Only(_)) => kinds,
    };
    match readers
        .iter()
        .chain(pinned_by)
        .find(|kind| !switch_owners.contains(kind))
    {
        Some(kind) => Err(BuildError::GoverningSwitchNotOwned {
            key: key.clone(),
            switch: switch.key.clone(),
            kind: Box::from(kind.name()),
        }),
        None => Ok(()),
    }
}
