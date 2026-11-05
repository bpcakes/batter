//! The three declaration builders: release flags, mode flags and kill switches.

use std::fmt;

use super::{
    CatalogBuilder,
    draft::{Draft, Shape},
};
use crate::{
    date::Expiry,
    declaration::{Absence, FlipDiscipline, InvariantRule, Kind, Owners, SafeState, StaleRule},
    error::BuildError,
    handle::{Flag, KillSwitch, ModeFlag, ReleaseFlag},
    value::{FlagValue, Toggle, ValueType, check_variants, is_aliased},
    vocabulary::{EnvironmentClass, ProcessKind},
};

/// The setters every declaration kind has.
macro_rules! common_setters {
    () => {
        /// Sets the process kinds that own the declaration; the default is
        /// [`Owners::All`].
        ///
        /// Ownership governs where a value may be presented, which
        /// control-plane entries apply and where the declaration is read live.
        /// `Owners::Only` needs a non-empty, duplicate-free list of this
        /// catalog's kinds. [`Absence::Required`] applies only in owning kinds.
        pub fn owners(mut self, owners: Owners<'_>) -> Self {
            self.draft.owners(owners);
            self
        }

        /// Sets what an absent setting means; the default is
        /// [`Absence::Default`]. [`Absence::Required`] is judged only where a
        /// value may be presented: in an owning kind and, for an `only_in`
        /// flag, inside its class.
        pub fn absence(mut self, absence: Absence) -> Self {
            self.draft.absence(absence);
            self
        }

        /// Sets the flip discipline; the default is [`FlipDiscipline::Rolling`].
        pub fn flip_discipline(mut self, discipline: FlipDiscipline) -> Self {
            self.draft.flip_discipline(discipline);
            self
        }
    };
}

/// The setters release and mode flags share.
macro_rules! flag_setters {
    () => {
        /// Lists the process kinds that may resolve pins of this flag without
        /// owning it. They never evaluate it live, and their settings for it
        /// are refused like any non-owner's.
        ///
        /// The list must be non-empty, duplicate-free, disjoint from the
        /// owners, and is refused when every kind owns the flag.
        pub fn pinned_by(mut self, kinds: &[ProcessKind]) -> Self {
            self.draft.pinned_by(kinds);
            self
        }

        /// Restricts the flag to one environment class, such as development.
        ///
        /// Such a flag is declared-only: a setting for it outside the class
        /// fails startup, and the control plane never supplies it.
        /// [`Absence::Required`] applies only inside the class.
        pub fn only_in(mut self, class: &EnvironmentClass) -> Self {
            self.draft.only_in(class);
            self
        }

        /// While this flag is `when`, `other` must be `variant`.
        ///
        /// Declared mode checks it once at startup. The build refuses an
        /// invariant the two code defaults already break.
        pub fn requires<F: Flag>(mut self, when: T, other: &F, variant: F::Value) -> Self {
            self.invariant(InvariantRule::Requires, when, other, variant);
            self
        }

        /// This flag at `when` and `other` at `variant` must not both hold.
        ///
        /// Declared mode checks it once at startup. The build refuses an
        /// invariant the two code defaults already break.
        pub fn conflicts<F: Flag>(mut self, when: T, other: &F, variant: F::Value) -> Self {
            self.invariant(InvariantRule::Conflicts, when, other, variant);
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

        fn invariant<F: Flag>(
            &mut self,
            rule: InvariantRule,
            when: T,
            other: &F,
            variant: F::Value,
        ) {
            if is_aliased(when) {
                return self.draft.fail(|key| BuildError::AliasedValue {
                    flag: key.clone(),
                    key,
                    variant: when.variant(),
                });
            }
            if is_aliased(variant) {
                return self.draft.fail(|key| BuildError::AliasedValue {
                    key,
                    flag: other.key().clone(),
                    variant: variant.variant(),
                });
            }
            self.draft.invariant(
                rule,
                (when.variant(), self.default.variant()),
                other.handle(),
                variant.variant(),
            );
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

/// The toggle-only setter release and mode flags share.
macro_rules! toggle_setters {
    () => {
        /// Either this toggle and `other` are both `on`, or neither is.
        ///
        /// Declared mode checks it once at startup. The build refuses the
        /// invariant when the two code defaults already break it. It exists
        /// only between toggles; for enum flags state each direction with
        /// [`requires`](Self::requires).
        pub fn both_or_none<F: Flag<Value = Toggle>>(mut self, other: &F) -> Self {
            self.invariant(InvariantRule::BothOrNone, Toggle::On, other, Toggle::On);
            self
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
/// `both_or_none` relates two toggles only, so an enum flag has none:
///
/// ```compile_fail,E0599
/// # use batter_flags_core::{CatalogBuilder, CivilDate, Expiry, FlagValue, SettingsPrefix, Toggle};
/// # #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// # enum Checkout { Classic, Express }
/// # impl FlagValue for Checkout {
/// #     const VARIANTS: &'static [&'static str] = &["classic", "express"];
/// #     fn variant(self) -> &'static str { match self { Self::Classic => "classic", Self::Express => "express" } }
/// #     fn from_variant(v: &str) -> Option<Self> { match v { "classic" => Some(Self::Classic), "express" => Some(Self::Express), _ => None } }
/// # }
/// # let mut builder =
/// #     CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
/// # let expiry = Expiry::on(CivilDate::new(2027, 1, 1).unwrap());
/// let toggle = builder.release_flag("toggle", Toggle::Off, expiry).declare().unwrap();
/// let _checkout = builder
///     .release_flag("checkout", Checkout::Classic, expiry)
///     .both_or_none(&toggle);
/// ```
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct ReleaseDeclaration<'b, T: FlagValue> {
    draft: Draft<'b>,
    default: T,
    expiry: Expiry,
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
        }
    }

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
        self.draft.finish(&shape).map(ReleaseFlag::mint)
    }
}

impl ReleaseDeclaration<'_, Toggle> {
    toggle_setters!();
}

/// A mode flag being declared; [`declare`](Self::declare) mints its handle.
///
/// Started by [`CatalogBuilder::mode_flag`] with a code default. A mode flag
/// is permanent: it has no expiry and no governing switch.
///
/// ```
/// use batter_flags_core::{Absence, CatalogBuilder, FlipDiscipline, SettingsPrefix, Toggle};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// builder.process_kind("serving")?;
/// let development = builder.environment_class("development")?;
/// builder.environment_class("hosted")?;
///
/// let ledger_v2 = builder
///     .mode_flag("ledger_v2", Toggle::Off)
///     .absence(Absence::Required)
///     .flip_discipline(FlipDiscipline::Coordinated)
///     .declare()?;
/// let ledger_audit = builder
///     .mode_flag("ledger_audit", Toggle::Off)
///     .both_or_none(&ledger_v2)
///     .declare()?;
/// builder
///     .mode_flag("debug_panel", Toggle::Off)
///     .only_in(&development)
///     .requires(Toggle::On, &ledger_audit, Toggle::On)
///     .conflicts(Toggle::On, &ledger_v2, Toggle::Off)
///     .declare()?;
/// let catalog = builder.build()?;
/// assert_eq!(catalog.declaration("debug_panel").unwrap().invariants().len(), 2);
/// # Ok(())
/// # }
/// ```
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
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct ModeDeclaration<'b, T: FlagValue> {
    draft: Draft<'b>,
    default: T,
}

impl<'b, T: FlagValue> ModeDeclaration<'b, T> {
    pub(super) fn new(builder: &'b mut CatalogBuilder, key: &str, default: T) -> Self {
        Self {
            draft: checked_draft(builder, key, default),
            default,
        }
    }

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
        self.draft.finish(&shape).map(ModeFlag::mint)
    }
}

impl ModeDeclaration<'_, Toggle> {
    toggle_setters!();
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
///     .pinned_by(&[worker])
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
#[must_use = "a declaration takes effect only when `declare` is called"]
pub struct SwitchDeclaration<'b> {
    draft: Draft<'b>,
    safe_state: SafeState,
    stale_rule: StaleRule,
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
        }
    }

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
        self.draft.finish(&shape).map(KillSwitch::mint)
    }
}

impl<T: FlagValue> fmt::Debug for ReleaseDeclaration<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_draft(formatter, "ReleaseDeclaration", &self.draft)
    }
}

impl<T: FlagValue> fmt::Debug for ModeDeclaration<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_draft(formatter, "ModeDeclaration", &self.draft)
    }
}

impl fmt::Debug for SwitchDeclaration<'_> {
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
