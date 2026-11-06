//! The built catalog: the one validated declaration an application has.

use std::{collections::BTreeMap, fmt, sync::Arc};

use crate::{
    builder::CatalogBuilder,
    date::CivilDate,
    declaration::{Declared, Entry},
    digest::{CatalogDigest, DigestInput},
    error::ForeignKind,
    key::{FlagKey, SettingName, SettingsPrefix},
    vocabulary::{CatalogId, EnvironmentClass, ProcessKind},
};

/// A validated catalog of flags and kill switches.
///
/// Built once by [`CatalogBuilder::build`]. It is not `Clone`: a process
/// consumes its catalog exactly once, by resolving it in declared mode or by
/// building test flags from it, so two diverging copies of one declaration
/// cannot exist.
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, SettingsPrefix};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// # builder.process_kind("serving").unwrap();
/// # builder.environment_class("hosted").unwrap();
/// let catalog = builder.build().unwrap();
/// let _copy = catalog.clone();
/// ```
///
/// A consumer that takes the catalog leaves nothing to reuse:
///
/// ```compile_fail,E0382
/// # use batter_flags_core::{Catalog, CatalogBuilder, SettingsPrefix};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// # builder.process_kind("serving").unwrap();
/// # builder.environment_class("hosted").unwrap();
/// fn consume(_catalog: Catalog) {}
///
/// let catalog = builder.build().unwrap();
/// consume(catalog);
/// consume(catalog);
/// ```
pub struct Catalog {
    pub(crate) id: CatalogId,
    pub(crate) prefix: SettingsPrefix,
    pub(crate) process_kinds: Box<[ProcessKind]>,
    pub(crate) environment_classes: Box<[EnvironmentClass]>,
    /// Declaration order; a handle's index points here.
    pub(crate) entries: Box<[Entry]>,
    pub(crate) by_key: BTreeMap<FlagKey, usize>,
    /// Sorted by name.
    pub(crate) retired: Box<[Retired]>,
    pub(crate) digest: CatalogDigest,
}

impl Catalog {
    pub(crate) fn assemble(builder: CatalogBuilder) -> Self {
        let CatalogBuilder {
            id,
            prefix,
            process_kinds,
            environment_classes,
            entries,
            keys,
            retired,
        } = builder;
        let retired: Box<[Retired]> = retired.into_values().collect();
        let digest = CatalogDigest::compute(DigestInput {
            prefix: prefix.as_str(),
            process_kinds: process_kinds.iter().map(ProcessKind::name).collect(),
            environment_classes: environment_classes
                .iter()
                .map(EnvironmentClass::name)
                .collect(),
            retired: retired
                .iter()
                .map(|retired| retired.name().as_str())
                .collect(),
            entries: entries.iter().map(Entry::digest_view).collect(),
        });
        Self {
            id,
            prefix,
            process_kinds: process_kinds.into_boxed_slice(),
            environment_classes: environment_classes.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
            by_key: keys,
            retired,
            digest,
        }
    }

    /// The settings prefix every flag's setting name starts with.
    pub fn prefix(&self) -> &SettingsPrefix {
        &self.prefix
    }

    /// The digest of everything resolution and evaluation depend on.
    ///
    /// It changes with the prefix, the vocabularies, the retired names and
    /// every declaration's behaviour-bearing fields, and
    /// not with expiry, flip discipline, migration notes or declaration order.
    pub fn digest(&self) -> CatalogDigest {
        self.digest
    }

    /// The declared process kinds, in declaration order.
    pub fn process_kinds(&self) -> &[ProcessKind] {
        &self.process_kinds
    }

    /// The declared environment classes, in declaration order.
    pub fn environment_classes(&self) -> &[EnvironmentClass] {
        &self.environment_classes
    }

    /// Every declaration, in key order.
    pub fn declarations(&self) -> impl ExactSizeIterator<Item = Declared<'_>> + '_ {
        self.by_key
            .values()
            .map(|&index| Declared::new(&self.prefix, &self.entries[index]))
    }

    /// The declaration with `key`, if any.
    ///
    /// This reads declaration metadata for tooling and tests. Values are only
    /// ever read through the typed handles the builder minted.
    pub fn declaration(&self, key: &str) -> Option<Declared<'_>> {
        let key = FlagKey::new(key).ok()?;
        self.by_key
            .get(&key)
            .map(|&index| Declared::new(&self.prefix, &self.entries[index]))
    }

    /// The retired setting names, in name order.
    pub fn retired(&self) -> &[Retired] {
        &self.retired
    }

    /// The keys `kind` owns, in key order, so a root's test can assert what
    /// its kind evaluates live. Kinds that only resolve pins of a flag
    /// through `pinned_by` do not own it.
    ///
    /// # Errors
    ///
    /// [`ForeignKind`] when `kind` belongs to another catalog.
    pub fn owned_by<'a>(
        &'a self,
        kind: &'a ProcessKind,
    ) -> Result<impl Iterator<Item = FlagKey> + 'a, ForeignKind> {
        if kind.catalog() != self.id {
            return Err(ForeignKind::new(kind.name()));
        }
        Ok(self
            .by_key
            .iter()
            .filter(move |&(_, &index)| self.entries[index].owners.contains(kind))
            .map(|(key, _)| key.clone()))
    }

    /// The release flags whose last valid day lies before `on`, in key order.
    ///
    /// Expiry never changes a decision and never fails startup; this list is
    /// for the people who clean release flags up. This crate reads no clock:
    /// tests pass a fixed date, and an application's own CI gate passes the
    /// date it reads.
    ///
    /// ```
    /// use batter_flags_core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    /// builder.process_kind("serving")?;
    /// builder.environment_class("hosted")?;
    /// let expiry = Expiry::on(CivilDate::new(2026, 12, 31)?);
    /// builder.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
    /// let catalog = builder.build()?;
    ///
    /// assert_eq!(catalog.expired(CivilDate::new(2026, 12, 31)?).count(), 0);
    /// let expired: Vec<_> = catalog.expired(CivilDate::new(2027, 1, 1)?).collect();
    /// assert_eq!(expired[0].key().as_str(), "new_checkout");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The wall-clock gate an application's CI runs:
    ///
    /// ```no_run
    /// # fn application_catalog() -> batter_flags_core::Catalog { unimplemented!() }
    /// use std::time::{SystemTime, UNIX_EPOCH};
    ///
    /// use batter_flags_core::CivilDate;
    ///
    /// let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    /// let today = CivilDate::from_unix_time(since_epoch).unwrap();
    /// let catalog = application_catalog();
    /// let expired: Vec<String> = catalog
    ///     .expired(today)
    ///     .map(|declared| declared.key().to_string())
    ///     .collect();
    /// assert!(expired.is_empty(), "release flags past their expiry: {expired:?}");
    /// ```
    pub fn expired(&self, on: CivilDate) -> impl Iterator<Item = Declared<'_>> + '_ {
        self.declarations().filter(move |declared| {
            declared
                .expiry()
                .is_some_and(|expiry| expiry.is_expired_on(on))
        })
    }
}

impl fmt::Debug for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Catalog")
            .field("prefix", &self.prefix)
            .field("digest", &self.digest)
            .field("declarations", &self.entries.len())
            .finish_non_exhaustive()
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
