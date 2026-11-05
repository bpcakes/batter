//! The catalog builder: vocabularies, declarations and retired names.

mod draft;
mod kinds;

use std::{collections::BTreeMap, fmt, sync::Arc};

pub use kinds::{ModeDeclaration, ReleaseDeclaration, SwitchDeclaration};

use crate::{
    catalog::{Catalog, Retired},
    date::Expiry,
    declaration::{Entry, SafeState, StaleRule},
    error::BuildError,
    key::{FlagKey, KeyError, SettingName, SettingsPrefix},
    value::{FlagValue, ToggleLiterals},
    vocabulary::{CatalogId, EnvironmentClass, ProcessKind},
};

/// Builds a [`Catalog`]: the one declaration of every flag, kill switch,
/// process kind, environment class and retired name an application has.
///
/// Each declaration method returns a declaration builder whose `declare`
/// validates it and mints its handle; [`build`](Self::build) validates the
/// whole catalog and returns it.
///
/// ```
/// use batter_flags_core::{
///     CatalogBuilder, CivilDate, Expiry, Owners, SafeState, SettingsPrefix, StaleRule, Toggle,
///     ToggleLiterals,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, ToggleLiterals::TrueFalse);
/// let serving = builder.process_kind("serving")?;
/// let worker = builder.process_kind("worker")?;
/// let _hosted = builder.environment_class("hosted")?;
///
/// let checkout_halt = builder
///     .kill_switch("checkout_halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()?;
/// let new_checkout = builder
///     .release_flag("new_checkout", Toggle::Off, Expiry::on(CivilDate::new(2027, 3, 31)?))
///     .owners(Owners::Only(&[serving.clone()]))
///     .pinned_by(&[worker.clone()])
///     .governed_by(&checkout_halt)
///     .requires_settings(Toggle::On, &["PAYMENT_API_URL"])
///     .declare()?;
/// builder.retired("ENABLE_NEW_CHECKOUT", "set FLAG_NEW_CHECKOUT instead")?;
///
/// let catalog = builder.build()?;
/// assert_eq!(catalog.declaration("new_checkout").unwrap().setting_name().as_str(), "FLAG_NEW_CHECKOUT");
/// assert_eq!(catalog.owned_by(&serving)?.count(), 2);
/// # let _ = new_checkout;
/// # Ok(())
/// # }
/// ```
pub struct CatalogBuilder {
    pub(crate) id: CatalogId,
    pub(crate) prefix: SettingsPrefix,
    pub(crate) toggle_literals: ToggleLiterals,
    pub(crate) process_kinds: Vec<ProcessKind>,
    pub(crate) environment_classes: Vec<EnvironmentClass>,
    /// Declaration order; a handle's index points here.
    pub(crate) entries: Vec<Entry>,
    pub(crate) keys: BTreeMap<FlagKey, usize>,
    pub(crate) retired: BTreeMap<SettingName, Retired>,
}

impl CatalogBuilder {
    /// Starts a catalog whose flags read settings under `prefix` and spell
    /// every [`Toggle`](crate::Toggle) with `toggle_literals`.
    ///
    /// Both are part of the catalog digest and the manifest. Declared mode
    /// refuses every name under the prefix that the catalog does not declare.
    pub fn new(prefix: SettingsPrefix, toggle_literals: ToggleLiterals) -> Self {
        Self {
            id: CatalogId::mint(),
            prefix,
            toggle_literals,
            process_kinds: Vec::new(),
            environment_classes: Vec::new(),
            entries: Vec::new(),
            keys: BTreeMap::new(),
            retired: BTreeMap::new(),
        }
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

    /// Validates the whole catalog and returns it.
    ///
    /// # Errors
    ///
    /// [`BuildError::NoProcessKind`] or [`BuildError::NoEnvironmentClass`]
    /// when a vocabulary is empty, because no process could then resolve the
    /// catalog.
    pub fn build(self) -> Result<Catalog, BuildError> {
        if self.process_kinds.is_empty() {
            return Err(BuildError::NoProcessKind);
        }
        if self.environment_classes.is_empty() {
            return Err(BuildError::NoEnvironmentClass);
        }
        Ok(Catalog::assemble(self))
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
            .field("toggle_literals", &self.toggle_literals)
            .field("declarations", &self.entries.len())
            .finish_non_exhaustive()
    }
}

fn vocabulary_name(name: &str) -> Result<Arc<str>, KeyError> {
    FlagKey::new(name).map(|_| Arc::from(name))
}
