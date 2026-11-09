//! Comparing two manifests: a kind or value-type change is a new key.

use std::fmt;

use super::{Manifest, ManifestEntry};
use crate::{declaration::Kind, key::FlagKey, value::ValueType};

/// Compares the manifest of a previous build with the current one.
///
/// The rule is that a kind or value-type change is a new key: stored entries,
/// pins and settings written for the old type would otherwise be read as the
/// new one. Adding or removing an enum variant is not a re-type.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle, manifest_diff,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// fn catalog(release: bool) -> Result<batter_flags_core::Catalog, Box<dyn std::error::Error>> {
///     let (mut builder, _, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
///     if release {
///         let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
///         builder.release_flag("checkout", Toggle::Off, expiry).declare()?;
///     } else {
///         builder.mode_flag("checkout", Toggle::Off).declare()?;
///     }
///     Ok(builder.build())
/// }
///
/// let previous = catalog(true)?.manifest();
/// let error = manifest_diff(&previous, &catalog(false)?.manifest()).unwrap_err();
/// assert_eq!(error.retyped()[0].key().as_str(), "checkout");
/// assert!(manifest_diff(&previous, &previous)?.is_empty());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`ManifestDiffError`] listing every key whose kind or value type changed.
pub fn manifest_diff(
    previous: &Manifest,
    current: &Manifest,
) -> Result<ManifestDiff, ManifestDiffError> {
    let mut diff = ManifestDiff {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
    };
    let mut retyped = Vec::new();
    for entry in &previous.flags {
        match current.flag(entry.key.as_str()) {
            None => diff.removed.push(entry.key.clone()),
            Some(next) if retypes(entry, next) => retyped.push(Retype {
                key: entry.key.clone(),
                previous: (entry.kind, entry.value_type),
                current: (next.kind, next.value_type),
            }),
            Some(next) if next != entry => diff.changed.push(entry.key.clone()),
            Some(_) => {}
        }
    }
    for entry in &current.flags {
        if previous.flag(entry.key.as_str()).is_none() {
            diff.added.push(entry.key.clone());
        }
    }
    if retyped.is_empty() {
        Ok(diff)
    } else {
        Err(ManifestDiffError { retyped })
    }
}

fn retypes(previous: &ManifestEntry, current: &ManifestEntry) -> bool {
    previous.kind != current.kind || previous.value_type != current.value_type
}

/// The keys two compatible manifests differ in, each list in key order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestDiff {
    added: Vec<FlagKey>,
    removed: Vec<FlagKey>,
    changed: Vec<FlagKey>,
}

impl ManifestDiff {
    /// Keys only the current manifest declares.
    pub fn added(&self) -> &[FlagKey] {
        &self.added
    }

    /// Keys only the previous manifest declared.
    pub fn removed(&self) -> &[FlagKey] {
        &self.removed
    }

    /// Keys both declare with the same kind and value type but different
    /// declarations, such as an added variant, new owners or a new expiry.
    pub fn changed(&self) -> &[FlagKey] {
        &self.changed
    }

    /// Whether no declaration differs.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Keys whose kind or value type changed between two manifests.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{} flag(s) changed kind or value type; a kind or value-type change needs a new key: {}", .retyped.len(), RetypedKeys(.retyped))]
pub struct ManifestDiffError {
    retyped: Vec<Retype>,
}

impl ManifestDiffError {
    /// Every re-typed key, in key order.
    pub fn retyped(&self) -> &[Retype] {
        &self.retyped
    }
}

struct RetypedKeys<'a>(&'a [Retype]);

impl fmt::Display for RetypedKeys<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, retype) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(
                formatter,
                "{} ({} {} -> {} {})",
                retype.key,
                retype.previous.0.as_str(),
                retype.previous.1.as_str(),
                retype.current.0.as_str(),
                retype.current.1.as_str()
            )?;
        }
        Ok(())
    }
}

/// One key whose kind or value type changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Retype {
    key: FlagKey,
    previous: (Kind, ValueType),
    current: (Kind, ValueType),
}

impl Retype {
    /// The re-typed key.
    pub fn key(&self) -> &FlagKey {
        &self.key
    }

    /// The previous kind and value type.
    pub fn previous(&self) -> (Kind, ValueType) {
        self.previous
    }

    /// The current kind and value type.
    pub fn current(&self) -> (Kind, ValueType) {
        self.current
    }
}
