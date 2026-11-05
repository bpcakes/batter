//! The three declaration builders: release flags, mode flags and kill switches.
//!
//! Ownership is the one choice whose valid values depend on another: only a
//! declaration restricted to some kinds can name kinds that may resolve its
//! pins. Each declaration therefore carries its ownership as a type, [`Everyone`]
//! until `owned_only_by` restricts it to [`Restricted`], so the combinations
//! that make no sense are not methods and do not compile.

use std::{fmt, marker::PhantomData};

use super::{
    CatalogBuilder,
    draft::{Draft, Shape},
};
use crate::{
    date::Expiry,
    declaration::{Kind, SafeState, StaleRule},
    error::BuildError,
    handle::{KillSwitch, ModeFlag, ReleaseFlag},
    value::{FlagValue, ValueType, check_variants, is_aliased},
    vocabulary::{EnvironmentClass, ProcessKind},
};

/// A declaration every process kind owns, which is every declaration until
/// `owned_only_by` restricts it.
///
/// It only names a state and has no values.
pub enum Everyone {}

/// A declaration restricted to some process kinds by `owned_only_by`.
///
/// Only a restricted release or mode flag can name kinds that may resolve its
/// pins without owning it, so `pinned_by` exists only here.
pub enum Restricted {}

/// The setters every declaration kind has.
macro_rules! common_setters {
    () => {
        /// Makes an absent setting a startup failure where a value may be
        /// presented: in an owning kind and, for an `only_in` flag, inside its
        /// class. Without it an absent setting resolves to the code default, or
        /// a switch's safe state. Calling it again changes nothing.
        pub fn required(mut self) -> Self {
            self.draft.require_absence();
            self
        }

        /// Marks the declaration as one whose processes must change together,
        /// which deployment tooling reads; without it processes change one at
        /// a time. See [`FlipDiscipline::Coordinated`](crate::FlipDiscipline::Coordinated).
        /// Calling it again changes nothing.
        pub fn coordinated(mut self) -> Self {
            self.draft.coordinate();
            self
        }
    };
}

/// The setters release and mode flags share.
macro_rules! flag_setters {
    () => {
        /// Restricts the flag to one environment class, such as development.
        ///
        /// Such a flag is declared-only: a setting for it outside the class
        /// fails startup, and the control plane never supplies it.
        /// [`required`](Self::required) applies only inside the class.
        pub fn only_in(mut self, class: &EnvironmentClass) -> Self {
            self.draft.only_in(class);
            self
        }

        /// Names the settings an enabling `variant` needs, outside the flag
        /// prefix.
        ///
        /// Declared mode checks only that each is present and non-empty when
        /// the flag resolves to `variant`; values are never retained or
        /// reported. The code default cannot require settings, and each
        /// variant takes one call with a non-empty, duplicate-free list.
        pub fn requires_settings(mut self, variant: T, names: &[&'static str]) -> Self {
            if is_aliased(variant) {
                self.draft.fail(|key| BuildError::AliasedValue {
                    flag: key.clone(),
                    key,
                    variant: variant.variant(),
                });
                return self;
            }
            self.draft
                .requires_settings(variant.variant(), self.default.variant(), names);
            self
        }

        fn shape(&self, kind: Kind, expiry: Option<Expiry>) -> Shape {
            Shape {
                kind,
                value_type: ValueType::of::<T>(),
                variants: T::VARIANTS,
                default_variant: Some(self.default.variant()),
                expiry,
                safe_state: None,
                stale_rule: None,
            }
        }
    };
}

fn checked_draft<'b, T: FlagValue>(
    builder: &'b mut CatalogBuilder,
    key: &str,
    default: T,
) -> Draft<'b> {
    let mut draft = Draft::new(builder, key, T::VARIANTS);
    if let Err(cause) = check_variants(default) {
        draft.fail(|key| BuildError::InvalidVariants { key, cause });
    }
    draft
}

/// A release flag being declared; [`declare`](Self::declare) mints its handle.
///
/// Started by [`CatalogBuilder::release_flag`] with a code default and a
/// mandatory [`Expiry`]. A release flag without an expiry does not compile:
///
/// ```compile_fail,E0061
/// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// let _flag = builder.release_flag("new_checkout", Toggle::Off).declare();
/// ```
///
/// Every kind owns it until [`owned_only_by`](Self::owned_only_by) restricts
/// it.
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct ReleaseDeclaration<'b, T: FlagValue, O = Everyone> {
    draft: Draft<'b>,
    default: T,
    expiry: Expiry,
    ownership: PhantomData<fn() -> O>,
}

impl<'b, T: FlagValue> ReleaseDeclaration<'b, T> {
    pub(super) fn new(
        builder: &'b mut CatalogBuilder,
        key: &str,
        default: T,
        expiry: Expiry,
    ) -> Self {
        Self {
            draft: checked_draft(builder, key, default),
            default,
            expiry,
            ownership: PhantomData,
        }
    }

    /// Restricts ownership to `kind`, which starts the list of owners. See
    /// [`ModeDeclaration::owned_only_by`].
    pub fn owned_only_by(mut self, kind: &ProcessKind) -> ReleaseDeclaration<'b, T, Restricted> {
        self.draft.owned_only_by(kind);
        ReleaseDeclaration {
            draft: self.draft,
            default: self.default,
            expiry: self.expiry,
            ownership: PhantomData,
        }
    }
}

impl<T: FlagValue> ReleaseDeclaration<'_, T, Restricted> {
    /// Adds an owner. See [`ModeDeclaration::and_owned_by`].
    pub fn and_owned_by(mut self, kind: &ProcessKind) -> Self {
        self.draft.add_owner(kind);
        self
    }

    /// Adds a kind that may resolve this flag's pins without owning it. See
    /// [`ModeDeclaration::pinned_by`].
    pub fn pinned_by(mut self, kind: &ProcessKind) -> Self {
        self.draft.add_pin_reader(kind);
        self
    }
}

impl<T: FlagValue, O> ReleaseDeclaration<'_, T, O> {
    common_setters!();
    flag_setters!();

    /// Governs the flag by a kill switch of this catalog: while the switch is
    /// engaged the flag serves its code default with reason `disabled`, over
    /// a pin too.
    ///
    /// The build refuses a switch that some kind owning the flag or listed in
    /// its `pinned_by` does not own, so every kind that evaluates or resolves
    /// the flag reads the switch.
    pub fn governed_by(mut self, switch: &KillSwitch) -> Self {
        self.draft.governed_by(switch);
        self
    }

    /// Validates the declaration and mints its handle.
    ///
    /// # Errors
    ///
    /// The first [`BuildError`] the declaration met, or one against the
    /// catalog so far: a duplicate key, a retired setting name, or a
    /// governing switch an owning or `pinned_by` kind does not own.
    pub fn declare(self) -> Result<ReleaseFlag<T>, BuildError> {
        let shape = self.shape(Kind::Release, Some(self.expiry));
        let default = self.default;
        self.draft
            .finish(&shape)
            .map(|handle| ReleaseFlag::mint(handle, default))
    }
}

/// A mode flag being declared; [`declare`](Self::declare) mints its handle.
///
/// Started by [`CatalogBuilder::mode_flag`] with a code default. A mode flag
/// is permanent: it has no expiry and no governing switch.
///
/// ```
/// use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// builder.process_kind("serving")?;
/// let development = builder.environment_class("development")?;
/// builder.environment_class("hosted")?;
///
/// let ledger_v2 = builder
///     .mode_flag("ledger_v2", Toggle::Off)
///     .required()
///     .coordinated()
///     .declare()?;
/// let ledger_audit = builder.mode_flag("ledger_audit", Toggle::Off).declare()?;
/// let debug_panel = builder
///     .mode_flag("debug_panel", Toggle::Off)
///     .only_in(&development)
///     .declare()?;
///
/// // Relationships need both handles, so the catalog states them.
/// builder.both_or_none(&ledger_audit, &ledger_v2)?;
/// builder.requires(&debug_panel, Toggle::On, &ledger_audit, Toggle::On)?;
/// builder.conflicts(&debug_panel, Toggle::On, &ledger_v2, Toggle::Off)?;
/// let catalog = builder.build()?;
/// assert_eq!(catalog.declaration("debug_panel").unwrap().invariants().len(), 2);
/// # Ok(())
/// # }
/// ```
///
/// A mode flag has no governing switch:
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule, Toggle};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// let halt = builder
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .declare()
///     .unwrap();
/// let _mode = builder.mode_flag("strict_mode", Toggle::Off).governed_by(&halt);
/// ```
///
/// Every kind owns it until [`owned_only_by`](Self::owned_only_by) restricts
/// it.
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct ModeDeclaration<'b, T: FlagValue, O = Everyone> {
    draft: Draft<'b>,
    default: T,
    ownership: PhantomData<fn() -> O>,
}

impl<'b, T: FlagValue> ModeDeclaration<'b, T> {
    pub(super) fn new(builder: &'b mut CatalogBuilder, key: &str, default: T) -> Self {
        Self {
            draft: checked_draft(builder, key, default),
            default,
            ownership: PhantomData,
        }
    }

    /// Restricts ownership to `kind`, which starts the list of owners; without
    /// this every process kind owns the declaration.
    ///
    /// Ownership governs where a value may be presented, which control-plane
    /// entries apply and where the declaration is read live. The kind must
    /// belong to this catalog. The list always has an owner, because this is
    /// how it starts, and this is offered once:
    ///
    /// ```compile_fail,E0599
    /// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
    /// # let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    /// # let serving = builder.process_kind("serving").unwrap();
    /// # let worker = builder.process_kind("worker").unwrap();
    /// let _ = builder
    ///     .mode_flag("strict_mode", Toggle::Off)
    ///     .owned_only_by(&serving)
    ///     .owned_only_by(&worker);
    /// ```
    ///
    /// [`and_owned_by`](ModeDeclaration::and_owned_by) adds an owner afterwards.
    pub fn owned_only_by(mut self, kind: &ProcessKind) -> ModeDeclaration<'b, T, Restricted> {
        self.draft.owned_only_by(kind);
        ModeDeclaration {
            draft: self.draft,
            default: self.default,
            ownership: PhantomData,
        }
    }
}

impl<T: FlagValue> ModeDeclaration<'_, T, Restricted> {
    /// Adds an owner. Adding a kind already listed changes nothing.
    pub fn and_owned_by(mut self, kind: &ProcessKind) -> Self {
        self.draft.add_owner(kind);
        self
    }

    /// Adds a kind that may resolve this flag's pins without owning it. It
    /// never evaluates the flag live, and its settings for the flag are refused
    /// like any non-owner's. Adding a kind already listed changes nothing.
    ///
    /// A kind cannot both own the flag and be listed here: the build refuses it,
    /// because an owner already resolves pins and the list would not say which
    /// was meant.
    ///
    /// This exists only once ownership is restricted, because a flag every kind
    /// owns has no kind left to list:
    ///
    /// ```compile_fail,E0599
    /// # use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
    /// # let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    /// # let worker = builder.process_kind("worker").unwrap();
    /// let _ = builder
    ///     .mode_flag("strict_mode", Toggle::Off)
    ///     .pinned_by(&worker);
    /// ```
    pub fn pinned_by(mut self, kind: &ProcessKind) -> Self {
        self.draft.add_pin_reader(kind);
        self
    }
}

impl<T: FlagValue, O> ModeDeclaration<'_, T, O> {
    common_setters!();
    flag_setters!();

    /// Validates the declaration and mints its handle.
    ///
    /// # Errors
    ///
    /// The first [`BuildError`] the declaration met, or one against the
    /// catalog so far: a duplicate key or a retired setting name.
    pub fn declare(self) -> Result<ModeFlag<T>, BuildError> {
        let shape = self.shape(Kind::Mode, None);
        let default = self.default;
        self.draft
            .finish(&shape)
            .map(|handle| ModeFlag::mint(handle, default))
    }
}

/// A kill switch being declared; [`declare`](Self::declare) mints its handle.
///
/// Started by [`CatalogBuilder::kill_switch`] with its [`SafeState`] and
/// [`StaleRule`]. A kill switch is permanent and never per-subject: it has no
/// percentage, no subject targeting, no pins and no environment restriction.
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// let _halt = builder
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .percentage(50)
///     .declare();
/// ```
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// # let worker = builder.process_kind("worker").unwrap();
/// let _halt = builder
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .owned_only_by(&worker)
///     .pinned_by(&worker)
///     .declare();
/// ```
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule};
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// # let development = builder.environment_class("development").unwrap();
/// let _halt = builder
///     .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
///     .only_in(&development)
///     .declare();
/// ```
///
/// Every kind owns it until [`owned_only_by`](Self::owned_only_by) restricts it.
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct SwitchDeclaration<'b, O = Everyone> {
    draft: Draft<'b>,
    safe_state: SafeState,
    stale_rule: StaleRule,
    ownership: PhantomData<fn() -> O>,
}

impl<'b> SwitchDeclaration<'b> {
    pub(super) fn new(
        builder: &'b mut CatalogBuilder,
        key: &str,
        safe_state: SafeState,
        stale_rule: StaleRule,
    ) -> Self {
        Self {
            draft: Draft::new(builder, key, &[]),
            safe_state,
            stale_rule,
            ownership: PhantomData,
        }
    }

    /// Restricts ownership to `kind`, which starts the list of owners. See
    /// [`ModeDeclaration::owned_only_by`].
    pub fn owned_only_by(mut self, kind: &ProcessKind) -> SwitchDeclaration<'b, Restricted> {
        self.draft.owned_only_by(kind);
        SwitchDeclaration {
            draft: self.draft,
            safe_state: self.safe_state,
            stale_rule: self.stale_rule,
            ownership: PhantomData,
        }
    }
}

impl SwitchDeclaration<'_, Restricted> {
    /// Adds an owner. See [`ModeDeclaration::and_owned_by`].
    pub fn and_owned_by(mut self, kind: &ProcessKind) -> Self {
        self.draft.add_owner(kind);
        self
    }
}

impl<O> SwitchDeclaration<'_, O> {
    common_setters!();

    /// Validates the declaration and mints its handle.
    ///
    /// # Errors
    ///
    /// The first [`BuildError`] the declaration met, or one against the
    /// catalog so far: a duplicate key or a retired setting name.
    pub fn declare(self) -> Result<KillSwitch, BuildError> {
        let shape = Shape {
            kind: Kind::Switch,
            value_type: ValueType::Switch,
            variants: &[],
            default_variant: None,
            expiry: None,
            safe_state: Some(self.safe_state),
            stale_rule: Some(self.stale_rule),
        };
        let safe_state = self.safe_state;
        self.draft
            .finish(&shape)
            .map(|handle| KillSwitch::mint(handle, safe_state))
    }
}

impl<T: FlagValue, O> fmt::Debug for ReleaseDeclaration<'_, T, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_draft(formatter, "ReleaseDeclaration", &self.draft)
    }
}

impl<T: FlagValue, O> fmt::Debug for ModeDeclaration<'_, T, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_draft(formatter, "ModeDeclaration", &self.draft)
    }
}

impl<O> fmt::Debug for SwitchDeclaration<'_, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_draft(formatter, "SwitchDeclaration", &self.draft)
    }
}

fn debug_draft(formatter: &mut fmt::Formatter<'_>, name: &str, draft: &Draft<'_>) -> fmt::Result {
    formatter
        .debug_struct(name)
        .field("key", &draft.key().map(crate::FlagKey::as_str))
        .finish_non_exhaustive()
}
