//! The catalog builder: vocabularies, declarations and retired names.

mod draft;
mod kinds;
mod relations;

use std::{collections::BTreeMap, fmt, sync::Arc};

pub use kinds::{
    Assigned, Everyone, ModeDeclaration, ReleaseDeclaration, Restricted, SwitchDeclaration,
    Unassigned,
};

use crate::{
    catalog::{Catalog, Retired},
    date::Expiry,
    declaration::{Entry, SafeState, StaleRule},
    error::BuildError,
    key::{FlagKey, KeyError, SettingName, SettingsPrefix},
    value::FlagValue,
    vocabulary::{CatalogId, EnvironmentClass, ProcessKind},
};

/// Builds a [`Catalog`]: the one declaration of every flag, kill switch,
/// process kind, environment class and retired name an application has.
///
/// Each declaration method returns a declaration builder whose `declare`
/// validates it and mints its handle. Relationships between two declared flags
/// are stated on the builder afterwards, with [`requires`](Self::requires),
/// [`conflicts`](Self::conflicts) and [`both_or_none`](Self::both_or_none),
/// because they need both handles; [`build`](Self::build) validates the whole
/// catalog and returns it.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, SafeState, SettingsPrefix, StaleRule, Toggle,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut builder, serving, _hosted) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let worker = builder.process_kind("worker")?;
///
/// let checkout_halt = builder
///     .kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()?;
/// let new_checkout = builder
///     .release_flag("new_checkout", Toggle::Off, Expiry::on(CivilDate::new(2027, 3, 31)?))
///     .owned_only_by(&serving)
///     .pinned_by(&worker)
///     .governed_by(&checkout_halt)
///     .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
///     .declare()?;
/// builder.retired("ENABLE_NEW_CHECKOUT", "set FLAG_NEW_CHECKOUT instead")?;
///
/// let catalog = builder.build();
/// assert_eq!(catalog.declaration("new_checkout").unwrap().setting_name().as_str(), "FLAG_NEW_CHECKOUT");
/// assert_eq!(catalog.owned_by(&serving)?.count(), 2);
/// # let _ = new_checkout;
/// # Ok(())
/// # }
/// ```
pub struct CatalogBuilder {
    pub(crate) id: CatalogId,
    pub(crate) prefix: SettingsPrefix,
    pub(crate) process_kinds: Vec<ProcessKind>,
    pub(crate) environment_classes: Vec<EnvironmentClass>,
    /// Declaration order; a handle's index points here.
    pub(crate) entries: Vec<Entry>,
    pub(crate) keys: BTreeMap<FlagKey, usize>,
    pub(crate) retired: BTreeMap<SettingName, Retired>,
}

impl CatalogBuilder {
    /// Starts a catalog whose flags read settings under `prefix`, with its
    /// first process kind and environment class, and returns their handles.
    ///
    /// A catalog without either could never be resolved by a process, so it
    /// cannot be started without them; add more with
    /// [`process_kind`](Self::process_kind) and
    /// [`environment_class`](Self::environment_class). The prefix is part of
    /// the catalog digest and the manifest. Declared mode refuses every name
    /// under the prefix that the catalog does not declare.
    ///
    /// # Errors
    ///
    /// As for [`process_kind`](Self::process_kind) and
    /// [`environment_class`](Self::environment_class).
    pub fn new(
        prefix: SettingsPrefix,
        process_kind: &str,
        environment_class: &str,
    ) -> Result<(Self, ProcessKind, EnvironmentClass), BuildError> {
        let mut builder = Self {
            id: CatalogId::mint(),
            prefix,
            process_kinds: Vec::new(),
            environment_classes: Vec::new(),
            entries: Vec::new(),
            keys: BTreeMap::new(),
            retired: BTreeMap::new(),
        };
        let kind = builder.process_kind(process_kind)?;
        let class = builder.environment_class(environment_class)?;
        Ok((builder, kind, class))
    }

    /// Declares a process kind and returns the only value that names it.
    ///
    /// # Errors
    ///
    /// [`BuildError::InvalidProcessKind`] for a name outside
    /// `[a-z][a-z0-9_]{0,63}`, [`BuildError::DuplicateProcessKind`] for a name
    /// this catalog already declared.
    pub fn process_kind(&mut self, name: &str) -> Result<ProcessKind, BuildError> {
        let name = vocabulary_name(name).map_err(|cause| BuildError::InvalidProcessKind {
            name: Box::from(name),
            cause,
        })?;
        if self.process_kinds.iter().any(|kind| kind.name() == &*name) {
            return Err(BuildError::DuplicateProcessKind {
                name: Box::from(&*name),
            });
        }
        let kind = ProcessKind::mint(self.id, self.process_kinds.len(), name);
        self.process_kinds.push(kind.clone());
        Ok(kind)
    }

    /// Declares an environment class and returns the only value that names it.
    ///
    /// # Errors
    ///
    /// [`BuildError::InvalidEnvironmentClass`] for a name outside
    /// `[a-z][a-z0-9_]{0,63}`, [`BuildError::DuplicateEnvironmentClass`] for a
    /// name this catalog already declared.
    pub fn environment_class(&mut self, name: &str) -> Result<EnvironmentClass, BuildError> {
        let name = vocabulary_name(name).map_err(|cause| BuildError::InvalidEnvironmentClass {
            name: Box::from(name),
            cause,
        })?;
        if self
            .environment_classes
            .iter()
            .any(|class| class.name() == &*name)
        {
            return Err(BuildError::DuplicateEnvironmentClass {
                name: Box::from(&*name),
            });
        }
        let class = EnvironmentClass::mint(self.id, self.environment_classes.len(), name);
        self.environment_classes.push(class.clone());
        Ok(class)
    }

    /// Starts a temporary release flag with its code default and its
    /// mandatory expiry, the last valid UTC day.
    ///
    /// Expiry never changes a decision; [`Catalog::expired`] lists the flags
    /// past it.
    pub fn release_flag<T: FlagValue>(
        &mut self,
        key: &str,
        default: T,
        expiry: Expiry,
    ) -> ReleaseDeclaration<'_, T> {
        ReleaseDeclaration::new(self, key, default, expiry)
    }

    /// Starts a permanent mode flag, such as a permission gate or an
    /// operating mode, with its code default.
    pub fn mode_flag<T: FlagValue>(&mut self, key: &str, default: T) -> ModeDeclaration<'_, T> {
        ModeDeclaration::new(self, key, default)
    }

    /// Starts a permanent kill switch with the state it serves where no entry
    /// exists and what it does while its view is stale.
    ///
    /// Both are mandatory because each choice carries a disclosed hazard: see
    /// [`SafeState`] and [`StaleRule`].
    pub fn kill_switch(
        &mut self,
        key: &str,
        safe_state: SafeState,
        stale_rule: StaleRule,
    ) -> SwitchDeclaration<'_> {
        SwitchDeclaration::new(self, key, safe_state, stale_rule)
    }

    /// Retires a setting name: declared mode fails startup while it is
    /// present and reports `migration_note`.
    ///
    /// The name may lie under the flag prefix or outside it.
    ///
    /// # Errors
    ///
    /// [`BuildError::InvalidRetiredName`] for a name outside the setting-name
    /// grammar, [`BuildError::EmptyMigrationNote`] for a blank note,
    /// [`BuildError::DuplicateRetiredName`] for a name retired twice and
    /// [`BuildError::RetiredNameInUse`] for a declared flag's setting name or
    /// a setting a variant requires.
    pub fn retired(&mut self, name: &str, migration_note: &str) -> Result<(), BuildError> {
        let name = SettingName::new(name).map_err(|cause| BuildError::InvalidRetiredName {
            name: Box::from(name),
            cause,
        })?;
        if migration_note.trim().is_empty() {
            return Err(BuildError::EmptyMigrationNote { name });
        }
        if self.retired.contains_key(&name) {
            return Err(BuildError::DuplicateRetiredName { name });
        }
        if self.name_in_use(&name) {
            return Err(BuildError::RetiredNameInUse { name });
        }
        let retired = Retired {
            name: name.clone(),
            migration_note: Arc::from(migration_note),
        };
        self.retired.insert(name, retired);
        Ok(())
    }

    /// Returns the catalog. Every declaration and relationship was validated
    /// as it was stated, and the constructor named a process kind and an
    /// environment class, so nothing is left to refuse.
    pub fn build(self) -> Catalog {
        Catalog::assemble(self)
    }

    fn name_in_use(&self, name: &SettingName) -> bool {
        self.entries.iter().any(|entry| {
            entry.key.setting_name(&self.prefix) == *name
                || entry
                    .requires_settings
                    .iter()
                    .any(|required| required.names.contains(name))
        })
    }
}

impl fmt::Debug for CatalogBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogBuilder")
            .field("prefix", &self.prefix)
            .field("declarations", &self.entries.len())
            .finish_non_exhaustive()
    }
}

fn vocabulary_name(name: &str) -> Result<Arc<str>, KeyError> {
    FlagKey::new(name).map(|_| Arc::from(name))
}
