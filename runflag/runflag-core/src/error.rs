//! Catalog build errors.

use thiserror::Error;

use crate::{
    declaration::InvariantRule,
    key::{FlagKey, KeyError, SettingName, SettingNameError},
    value::VariantsError,
};

/// Why a vocabulary value, declaration, retired name or catalog was refused.
///
/// Every variant names declared, non-secret identifiers only. A declaration
/// reports the first error it met; [`CatalogBuilder::build`] reports the first
/// catalog-wide error.
///
/// [`CatalogBuilder::build`]: crate::CatalogBuilder::build
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BuildError {
    /// A process-kind name broke the name grammar `[a-z][a-z0-9_]{0,63}`.
    #[error("process kind {name:?}: {cause}")]
    InvalidProcessKind {
        /// The offending name.
        name: Box<str>,
        /// Why it is invalid.
        cause: KeyError,
    },
    /// A process-kind name was already declared.
    #[error("process kind {name:?} is already declared")]
    DuplicateProcessKind {
        /// The repeated name.
        name: Box<str>,
    },
    /// An environment-class name broke the name grammar `[a-z][a-z0-9_]{0,63}`.
    #[error("environment class {name:?}: {cause}")]
    InvalidEnvironmentClass {
        /// The offending name.
        name: Box<str>,
        /// Why it is invalid.
        cause: KeyError,
    },
    /// An environment-class name was already declared.
    #[error("environment class {name:?} is already declared")]
    DuplicateEnvironmentClass {
        /// The repeated name.
        name: Box<str>,
    },
    /// The catalog declared no process kind, so no process could resolve it.
    #[error("a catalog needs at least one process kind")]
    NoProcessKind,
    /// The catalog declared no environment class, so no process could resolve it.
    #[error("a catalog needs at least one environment class")]
    NoEnvironmentClass,
    /// The key broke the key grammar.
    #[error("flag key {key:?}: {cause}")]
    InvalidKey {
        /// The offending key text.
        key: Box<str>,
        /// Why it is invalid.
        cause: KeyError,
    },
    /// The key was already declared in this catalog.
    #[error("flag {key} is already declared")]
    DuplicateKey {
        /// The repeated key.
        key: FlagKey,
    },
    /// A declaration setter was called twice.
    #[error("flag {key}: {method} was called twice")]
    RepeatedCall {
        /// The declaration being built.
        key: FlagKey,
        /// The setter called twice.
        method: &'static str,
    },
    /// The flag value type's variant list was refused.
    #[error("flag {key}: {cause}")]
    InvalidVariants {
        /// The declaration being built.
        key: FlagKey,
        /// Why the variants were refused.
        cause: VariantsError,
    },
    /// A process kind from another catalog was presented.
    #[error("flag {key}: process kind {kind:?} belongs to another catalog")]
    ForeignProcessKind {
        /// The declaration being built.
        key: FlagKey,
        /// The foreign kind's name.
        kind: Box<str>,
    },
    /// An environment class from another catalog was presented.
    #[error("flag {key}: environment class {class:?} belongs to another catalog")]
    ForeignEnvironmentClass {
        /// The declaration being built.
        key: FlagKey,
        /// The foreign class's name.
        class: Box<str>,
    },
    /// A handle from another catalog was presented as a governing switch or
    /// the other side of an invariant.
    #[error("flag {key}: handle {handle} belongs to another catalog")]
    ForeignHandle {
        /// The declaration being built.
        key: FlagKey,
        /// The foreign handle's key.
        handle: FlagKey,
    },
    /// `Owners::Only` listed no kind.
    #[error("flag {key}: Owners::Only needs at least one process kind")]
    EmptyOwners {
        /// The declaration being built.
        key: FlagKey,
    },
    /// A kind was listed twice among owners or `pinned_by`.
    #[error("flag {key}: process kind {kind:?} is listed twice")]
    DuplicateKind {
        /// The declaration being built.
        key: FlagKey,
        /// The repeated kind's name.
        kind: Box<str>,
    },
    /// `pinned_by` listed no kind.
    #[error("flag {key}: pinned_by needs at least one process kind")]
    EmptyPinnedBy {
        /// The declaration being built.
        key: FlagKey,
    },
    /// A `pinned_by` kind already owns the flag.
    #[error("flag {key}: process kind {kind:?} owns the flag and cannot also be in pinned_by")]
    PinnedByOwner {
        /// The declaration being built.
        key: FlagKey,
        /// The kind listed twice over.
        kind: Box<str>,
    },
    /// `pinned_by` was given while every kind owns the flag.
    #[error("flag {key}: pinned_by is meaningless when every process kind owns the flag")]
    PinnedByWithAllOwners {
        /// The declaration being built.
        key: FlagKey,
    },
    /// A kind that owns the flag or may resolve its pins does not own the
    /// governing switch, so it could not read the switch.
    #[error(
        "flag {key}: process kind {kind:?} owns or pins the flag but does not own its governing \
         switch {switch}"
    )]
    GoverningSwitchNotOwned {
        /// The governed flag.
        key: FlagKey,
        /// The governing switch.
        switch: FlagKey,
        /// The kind without the switch.
        kind: Box<str>,
    },
    /// Every kind owns the flag but only some own the governing switch.
    #[error("flag {key} is owned by every process kind but its governing switch {switch} is not")]
    GoverningSwitchRestricted {
        /// The governed flag.
        key: FlagKey,
        /// The governing switch.
        switch: FlagKey,
    },
    /// The same invariant was declared twice.
    #[error("flag {key}: the {rule:?} invariant on {other} is already declared")]
    DuplicateInvariant {
        /// The declaration being built.
        key: FlagKey,
        /// The repeated relation.
        rule: InvariantRule,
        /// The other flag.
        other: FlagKey,
    },
    /// The code defaults of the two flags already break an invariant, so a
    /// process with neither setting present could never start.
    #[error("flag {key}: the code defaults break its {rule:?} invariant on {other}")]
    DefaultsViolateInvariant {
        /// The declaration being built.
        key: FlagKey,
        /// The broken relation.
        rule: InvariantRule,
        /// The other flag.
        other: FlagKey,
    },
    /// A value whose variant name its type's `VARIANTS` does not list was
    /// named in an invariant or `requires_settings`.
    #[error("flag {key}: variant {variant:?} of {flag} is not declared in its VARIANTS")]
    UndeclaredVariant {
        /// The declaration being built.
        key: FlagKey,
        /// The flag whose value type lacks the variant.
        flag: FlagKey,
        /// The undeclared variant name.
        variant: &'static str,
    },
    /// A value named in an invariant or `requires_settings` shares its
    /// variant name with another value of its type, so `from_variant` cannot
    /// return it and the declaration would silently apply to that other value.
    #[error(
        "flag {key}: a value of {flag} names variant {variant:?}, which from_variant maps to a \
         different value"
    )]
    AliasedValue {
        /// The declaration being built.
        key: FlagKey,
        /// The flag whose value type aliases the name.
        flag: FlagKey,
        /// The shared variant name.
        variant: &'static str,
    },
    /// A required setting name broke the setting-name grammar.
    #[error("flag {key}: required setting {name:?}: {cause}")]
    InvalidRequiredSetting {
        /// The declaration being built.
        key: FlagKey,
        /// The offending name.
        name: Box<str>,
        /// Why it is invalid.
        cause: SettingNameError,
    },
    /// A required setting name lies under the flag prefix.
    #[error("flag {key}: required setting {name} lies under the flag prefix")]
    RequiredSettingUnderPrefix {
        /// The declaration being built.
        key: FlagKey,
        /// The offending name.
        name: SettingName,
    },
    /// `requires_settings` listed no name.
    #[error("flag {key}: requires_settings for variant {variant:?} names no setting")]
    EmptyRequiredSettings {
        /// The declaration being built.
        key: FlagKey,
        /// The variant whose list was empty.
        variant: &'static str,
    },
    /// A required setting name was listed twice for one variant.
    #[error("flag {key}: required setting {name} is listed twice")]
    DuplicateRequiredSetting {
        /// The declaration being built.
        key: FlagKey,
        /// The repeated name.
        name: SettingName,
    },
    /// `requires_settings` was called twice for one variant.
    #[error("flag {key}: requires_settings was called twice for variant {variant:?}")]
    DuplicateRequiresSettings {
        /// The declaration being built.
        key: FlagKey,
        /// The repeated variant.
        variant: &'static str,
    },
    /// `requires_settings` named the code default, which must need nothing:
    /// only an enabling variant declares the settings it needs.
    #[error("flag {key}: the code default {variant:?} cannot require settings")]
    RequiredSettingsOnDefault {
        /// The declaration being built.
        key: FlagKey,
        /// The default variant.
        variant: &'static str,
    },
    /// A retired name broke the setting-name grammar.
    #[error("retired name {name:?}: {cause}")]
    InvalidRetiredName {
        /// The offending name.
        name: Box<str>,
        /// Why it is invalid.
        cause: SettingNameError,
    },
    /// A retired name was declared twice.
    #[error("retired name {name} is already declared")]
    DuplicateRetiredName {
        /// The repeated name.
        name: SettingName,
    },
    /// A retired name came without a migration note.
    #[error("retired name {name} needs a migration note")]
    EmptyMigrationNote {
        /// The retired name.
        name: SettingName,
    },
    /// A retired name is also a declared flag's setting name or a setting
    /// some variant requires, so its presence would both fail and serve.
    #[error("setting name {name} is retired and also in use by the catalog")]
    RetiredNameInUse {
        /// The colliding setting name.
        name: SettingName,
    },
}

/// A process kind from another catalog was presented to a catalog query.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("process kind {kind:?} belongs to another catalog")]
pub struct ForeignKind {
    kind: Box<str>,
}

impl ForeignKind {
    pub(crate) fn new(kind: &str) -> Self {
        Self {
            kind: Box::from(kind),
        }
    }

    /// The foreign kind's name.
    pub fn kind(&self) -> &str {
        &self.kind
    }
}
