//! The validation every declaration kind shares.

use crate::{
    builder::CatalogBuilder,
    date::Expiry,
    declaration::{
        Absence, Entry, FlipDiscipline, GoverningSwitch, Kind, OwnedOwners, RequiredSettings,
        SafeState, StaleRule,
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

/// A declaration is either editable or permanently rejected. Invalid input
/// retains the first error and cannot mutate the remaining declaration.
pub(super) struct Draft<'b> {
    builder: &'b mut CatalogBuilder,
    state: State,
}

enum State {
    Valid(Fields),
    Invalid(BuildError),
}

struct Fields {
    key: FlagKey,
    variants: &'static [&'static str],
    owners: OwnedOwners,
    pinned_by: Vec<ProcessKind>,
    only_in: EnvironmentPolicy,
    absence: Absence,
    flip_discipline: FlipDiscipline,
    governed_by: Governance,
    requires_settings: Vec<RequiredSettings>,
}

enum EnvironmentPolicy {
    Any,
    Only(EnvironmentClass),
}

enum Governance {
    Independent,
    By(GoverningSwitch),
}

impl<'b> Draft<'b> {
    pub(super) fn new(
        builder: &'b mut CatalogBuilder,
        key: &str,
        variants: &'static [&'static str],
    ) -> Self {
        let state = match FlagKey::new(key) {
            Ok(key) => State::Valid(Fields {
                key,
                variants,
                owners: OwnedOwners::All,
                pinned_by: Vec::new(),
                only_in: EnvironmentPolicy::Any,
                absence: Absence::Default,
                flip_discipline: FlipDiscipline::Rolling,
                governed_by: Governance::Independent,
                requires_settings: Vec::new(),
            }),
            Err(cause) => State::Invalid(BuildError::InvalidKey {
                key: Box::from(key),
                cause,
            }),
        };
        Self { builder, state }
    }

    pub(super) fn key(&self) -> Option<&FlagKey> {
        match &self.state {
            State::Valid(fields) => Some(&fields.key),
            State::Invalid(_) => None,
        }
    }

    fn update(
        &mut self,
        change: impl FnOnce(&mut Fields, &CatalogBuilder) -> Result<(), BuildError>,
    ) {
        match &mut self.state {
            State::Valid(fields) => {
                if let Err(error) = change(fields, self.builder) {
                    self.state = State::Invalid(error);
                }
            }
            State::Invalid(_) => {}
        }
    }

    pub(super) fn fail(&mut self, make: impl FnOnce(FlagKey) -> BuildError) {
        self.update(|fields, _| Err(make(fields.key.clone())));
    }

    pub(super) fn owned_only_by(&mut self, kind: &ProcessKind) {
        self.update(|fields, builder| {
            fields.check_kind(builder, kind)?;
            fields.owners = OwnedOwners::Only(Box::new([kind.clone()]));
            Ok(())
        });
    }

    pub(super) fn add_owner(&mut self, kind: &ProcessKind) {
        self.update(|fields, builder| {
            fields.check_kind(builder, kind)?;
            match &mut fields.owners {
                OwnedOwners::Only(owners) => {
                    if !owners.contains(kind) {
                        let mut listed = owners.to_vec();
                        listed.push(kind.clone());
                        listed.sort_unstable_by(|left, right| left.name().cmp(right.name()));
                        *owners = listed.into_boxed_slice();
                    }
                }
                OwnedOwners::All => unreachable!("only restricted declarations add owners"),
            }
            Ok(())
        });
    }

    pub(super) fn add_pin_reader(&mut self, kind: &ProcessKind) {
        self.update(|fields, builder| {
            fields.check_kind(builder, kind)?;
            if !fields.pinned_by.contains(kind) {
                fields.pinned_by.push(kind.clone());
                fields
                    .pinned_by
                    .sort_unstable_by(|left, right| left.name().cmp(right.name()));
            }
            Ok(())
        });
    }

    pub(super) fn only_in(&mut self, class: &EnvironmentClass) {
        self.update(|fields, builder| {
            if class.catalog() != builder.id {
                return Err(BuildError::ForeignEnvironmentClass {
                    key: fields.key.clone(),
                    class: Box::from(class.name()),
                });
            }
            match fields.only_in {
                EnvironmentPolicy::Any => fields.only_in = EnvironmentPolicy::Only(class.clone()),
                EnvironmentPolicy::Only(_) => {
                    return Err(BuildError::RepeatedCall {
                        key: fields.key.clone(),
                        method: "only_in",
                    });
                }
            }
            Ok(())
        });
    }

    pub(super) fn require_absence(&mut self) {
        self.update(|fields, _| {
            fields.absence = Absence::Required;
            Ok(())
        });
    }

    pub(super) fn coordinate(&mut self) {
        self.update(|fields, _| {
            fields.flip_discipline = FlipDiscipline::Coordinated;
            Ok(())
        });
    }

    pub(super) fn governed_by(&mut self, switch: &KillSwitch) {
        self.update(|fields, builder| {
            let handle = switch.handle();
            if handle.catalog() != builder.id {
                return Err(BuildError::ForeignHandle {
                    key: fields.key.clone(),
                    handle: handle.key().clone(),
                });
            }
            match fields.governed_by {
                Governance::Independent => {
                    fields.governed_by = Governance::By(GoverningSwitch {
                        index: handle.index(),
                        key: handle.key().clone(),
                    })
                }
                Governance::By(_) => {
                    return Err(BuildError::RepeatedCall {
                        key: fields.key.clone(),
                        method: "governed_by",
                    });
                }
            }
            Ok(())
        });
    }

    pub(super) fn requires_settings(
        &mut self,
        variant: &'static str,
        default: &'static str,
        names: &[&'static str],
    ) {
        self.update(|fields, builder| fields.requires_settings(builder, variant, default, names));
    }

    pub(super) fn finish(self, shape: &Shape) -> Result<Handle, BuildError> {
        match self.state {
            State::Valid(fields) => fields.finish(self.builder, shape),
            State::Invalid(error) => Err(error),
        }
    }
}

impl Fields {
    fn check_kind(&self, builder: &CatalogBuilder, kind: &ProcessKind) -> Result<(), BuildError> {
        if kind.catalog() == builder.id {
            Ok(())
        } else {
            Err(BuildError::ForeignProcessKind {
                key: self.key.clone(),
                kind: Box::from(kind.name()),
            })
        }
    }

    fn requires_settings(
        &mut self,
        builder: &CatalogBuilder,
        variant: &'static str,
        default: &'static str,
        names: &[&'static str],
    ) -> Result<(), BuildError> {
        let key = &self.key;
        if !self.variants.contains(&variant) {
            return Err(BuildError::UndeclaredVariant {
                flag: key.clone(),
                key: key.clone(),
                variant,
            });
        }
        if variant == default {
            return Err(BuildError::RequiredSettingsOnDefault {
                key: key.clone(),
                variant,
            });
        }
        if self.requires_settings.iter().any(|r| r.variant == variant) {
            return Err(BuildError::DuplicateRequiresSettings {
                key: key.clone(),
                variant,
            });
        }
        if names.is_empty() {
            return Err(BuildError::EmptyRequiredSettings {
                key: key.clone(),
                variant,
            });
        }
        let mut checked = Vec::with_capacity(names.len());
        for name in names {
            let name =
                SettingName::new(name).map_err(|cause| BuildError::InvalidRequiredSetting {
                    key: key.clone(),
                    name: Box::from(*name),
                    cause,
                })?;
            if builder.prefix.covers(name.as_str()) {
                return Err(BuildError::RequiredSettingUnderPrefix {
                    key: key.clone(),
                    name,
                });
            }
            if checked.contains(&name) {
                return Err(BuildError::DuplicateRequiredSetting {
                    key: key.clone(),
                    name,
                });
            }
            if builder.retired.contains_key(&name) {
                return Err(BuildError::RetiredNameInUse { name });
            }
            checked.push(name);
        }
        checked.sort_unstable();
        self.requires_settings.push(RequiredSettings {
            variant,
            names: checked.into_boxed_slice(),
        });
        Ok(())
    }

    /// Validates the declaration against the catalog so far, records it and
    /// returns its handle.
    fn finish(self, builder: &mut CatalogBuilder, shape: &Shape) -> Result<Handle, BuildError> {
        let key = self.key;
        if builder.keys.contains_key(&key) {
            return Err(BuildError::DuplicateKey { key });
        }
        let setting_name = key.setting_name(&builder.prefix);
        if builder.retired.contains_key(&setting_name) {
            return Err(BuildError::RetiredNameInUse { name: setting_name });
        }
        let owners = self.owners;
        let pinned_by = self.pinned_by.into_boxed_slice();
        check_pinned_by(&key, &owners, &pinned_by)?;
        if let Governance::By(switch) = &self.governed_by {
            let switch_owners = &builder.entries[switch.index].owners;
            check_governing(&key, switch, switch_owners, &owners, &pinned_by)?;
        }
        let mut requires_settings = self.requires_settings;
        requires_settings.sort_unstable_by_key(|required| required.variant);
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
            absence: self.absence,
            owners,
            pinned_by,
            only_in: match self.only_in {
                EnvironmentPolicy::Any => None,
                EnvironmentPolicy::Only(class) => Some(class),
            },
            expiry: shape.expiry,
            flip_discipline: self.flip_discipline,
            safe_state: shape.safe_state,
            stale_rule: shape.stale_rule,
            governed_by: match self.governed_by {
                Governance::Independent => None,
                Governance::By(switch) => Some(switch),
            },
            requires_settings: requires_settings.into_boxed_slice(),
            invariants: Box::default(),
            fingerprint,
        });
        builder.keys.insert(key.clone(), index);
        Ok(Handle::new(
            builder.id,
            index,
            key,
            fingerprint,
            (shape.kind, shape.value_type),
        ))
    }
}

/// A pin reader that already owns the flag is a likely mistake: an owner can
/// resolve pins, and listing it again would not say which was meant.
fn check_pinned_by(
    key: &FlagKey,
    owners: &OwnedOwners,
    pinned_by: &[ProcessKind],
) -> Result<(), BuildError> {
    debug_assert!(
        pinned_by.is_empty() || matches!(owners, OwnedOwners::Only(_)),
        "pin readers exist only on a declaration restricted to some kinds"
    );
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
