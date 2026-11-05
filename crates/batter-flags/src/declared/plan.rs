//! The resolution plan: every check of declared mode, run before anything is
//! recorded.
//!
//! The plan reads the source and the catalog and mutates neither, so a
//! failure leaves nothing half-built, and every refusal is decided in one
//! place and one order. It keeps one [`Record`] per declaration, carrying what
//! each later phase needs, so no phase has to re-derive a declaration's facts or
//! line two sequences up by position.

use std::collections::BTreeMap;

use batter_core::settings::SettingsSource;
use batter_flags_core::{Catalog, FlagKey, Invariant, SettingName, low_level::BaseBuilder};

use super::{
    applicability::{Applicability, Process},
    error::DeclaredError,
    read::{Literal, read},
};

/// One declaration, with what every phase of resolution needs of it.
struct Record {
    key: FlagKey,
    applicability: Applicability,
    literal: Option<Literal>,
    /// A release or mode flag's code default; none for a kill switch.
    default_variant: Option<&'static str>,
    /// Each enabling variant with the settings it requires.
    requires: Vec<(&'static str, Vec<SettingName>)>,
    /// The invariants declared on this flag.
    invariants: Vec<Invariant>,
    /// Enabling variants whose required settings are absent or empty here.
    settings_absent: Vec<&'static str>,
}

impl Record {
    /// The variant a release or mode flag resolved to: its setting's, else its
    /// code default. A kill switch resolves to a state, not a variant.
    fn variant(&self) -> Option<&'static str> {
        match self.literal {
            Some(Literal::Variant(variant)) => Some(variant),
            Some(Literal::Switch(_)) | None => self.default_variant,
        }
    }
}

/// Every check passed; what remains is to record it.
pub(super) struct Plan {
    records: Vec<Record>,
}

impl Plan {
    /// Runs every check of declared mode against `source`.
    pub(super) fn read(
        catalog: &Catalog,
        process: Process<'_>,
        source: &SettingsSource,
    ) -> Result<Self, DeclaredError> {
        let mut plan = Self::declared(catalog, process, source)?;
        retired(catalog, source)?;
        unknown(catalog, source)?;
        plan.required_settings(source)?;
        plan.invariants()?;
        Ok(plan)
    }

    /// Every declared setting, in key order.
    fn declared(
        catalog: &Catalog,
        process: Process<'_>,
        source: &SettingsSource,
    ) -> Result<Self, DeclaredError> {
        let mut records = Vec::with_capacity(catalog.declarations().len());
        for declared in catalog.declarations() {
            let found = read(source, &declared, process)?;
            records.push(Record {
                key: declared.key().clone(),
                applicability: found.applicability,
                literal: found.literal,
                default_variant: declared.default_variant(),
                requires: declared
                    .requires_settings()
                    .map(|(variant, names)| (variant, names.to_vec()))
                    .collect(),
                invariants: declared.invariants().to_vec(),
                settings_absent: Vec::new(),
            });
        }
        Ok(Self { records })
    }

    /// The settings an enabling variant needs, for each flag this process
    /// evaluates: present and non-empty where the flag resolved to that
    /// variant, and recorded as absent for the control plane where it did not.
    /// A flag this process does not evaluate owes nothing.
    fn required_settings(&mut self, source: &SettingsSource) -> Result<(), DeclaredError> {
        for record in &mut self.records {
            if !record.applicability.evaluates() {
                continue;
            }
            for (variant, names) in &record.requires {
                let missing: Vec<SettingName> = names
                    .iter()
                    .filter(|name| {
                        source
                            .get(name.as_str())
                            .is_none_or(|value| value.is_empty())
                    })
                    .cloned()
                    .collect();
                if missing.is_empty() {
                    continue;
                }
                if record.literal == Some(Literal::Variant(variant)) {
                    return Err(DeclaredError::RequiredSettings {
                        key: record.key.clone(),
                        variant,
                        names: missing,
                    });
                }
                record.settings_absent.push(variant);
            }
        }
        Ok(())
    }

    /// The invariants between flags this process evaluates, over what each
    /// resolved to.
    ///
    /// An invariant whose other flag belongs to another kind is not checked:
    /// this process cannot know what that kind resolves, and the code default
    /// would be a guess that could fail a correct deployment. A flag restricted
    /// to another class is checked at its code default, because that is what
    /// this process evaluates.
    fn invariants(&self) -> Result<(), DeclaredError> {
        let resolved: BTreeMap<&FlagKey, &'static str> = self
            .records
            .iter()
            .filter(|record| record.applicability.evaluates())
            .filter_map(|record| record.variant().map(|variant| (&record.key, variant)))
            .collect();
        for record in &self.records {
            for invariant in &record.invariants {
                let (Some(variant), Some(other)) =
                    (resolved.get(&record.key), resolved.get(invariant.other()))
                else {
                    continue;
                };
                if !invariant.holds(variant, other) {
                    return Err(DeclaredError::Invariant {
                        rule: invariant.rule(),
                        flag: invariant.flag().clone(),
                        variant: invariant.variant(),
                        other: invariant.other().clone(),
                        other_variant: invariant.other_variant(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Records the plan in `builder`.
    pub(super) fn apply(self, builder: &mut BaseBuilder) -> Result<(), DeclaredError> {
        for record in self.records {
            let recorded = match record.literal {
                Some(Literal::Variant(variant)) => builder.flag(&record.key, variant),
                Some(Literal::Switch(state)) => builder.switch(&record.key, state),
                None => Ok(()),
            };
            recorded.map_err(DeclaredError::Inconsistent)?;
            for variant in record.settings_absent {
                builder
                    .settings_absent(&record.key, variant)
                    .map_err(DeclaredError::Inconsistent)?;
            }
        }
        Ok(())
    }
}

/// A retired name that is present.
fn retired(catalog: &Catalog, source: &SettingsSource) -> Result<(), DeclaredError> {
    for retired in catalog.retired() {
        if source.get(retired.name().as_str()).is_some() {
            return Err(DeclaredError::Retired {
                name: retired.name().clone(),
                migration_note: retired.migration_note().to_owned(),
            });
        }
    }
    Ok(())
}

/// Every other name under the prefix.
///
/// `select_environment` names no key, so the error names the prefix. It runs
/// after every declared and retired name has had its own diagnostic, which is
/// why a typo is the last thing reported and never masks a specific failure.
fn unknown(catalog: &Catalog, source: &SettingsSource) -> Result<(), DeclaredError> {
    let known: Vec<SettingName> = catalog
        .declarations()
        .map(|declared| declared.setting_name())
        .chain(
            catalog
                .retired()
                .iter()
                .map(|retired| retired.name().clone()),
        )
        .collect();
    let allowed: Vec<&str> = known.iter().map(SettingName::as_str).collect();
    let prefix = catalog.prefix().as_str();
    source
        .select_environment(&allowed, &[prefix])
        .map(drop)
        .map_err(|_| DeclaredError::UnknownName {
            prefix: prefix.to_owned(),
        })
}
