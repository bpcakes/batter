//! Relationships between two declared flags.
//!
//! A relationship names both flags by handle, so it can be stated only once
//! both are declared. It therefore belongs to the catalog and not to either
//! declaration: a declaration is consumed by `declare`, and two declarations
//! cannot be held at once because each borrows the builder. Stating it here
//! also makes a reciprocal pair expressible whichever flag was declared first.

use super::CatalogBuilder;
use crate::{
    declaration::{Invariant, InvariantRule},
    error::BuildError,
    handle::{Flag, Handle},
    value::{FlagValue, Toggle, is_aliased},
};

/// One side of a relationship: a declared flag at one of its variants.
struct Side<'a> {
    handle: &'a Handle,
    variant: &'static str,
    /// Whether the typed value names a variant that `from_variant` maps to
    /// another value.
    aliased: bool,
}

fn side<F: Flag>(flag: &F, value: F::Value) -> Side<'_> {
    Side {
        handle: flag.handle(),
        variant: value.variant(),
        aliased: is_aliased(value),
    }
}

impl CatalogBuilder {
    /// While `flag` is `when`, `other` must be `then`.
    ///
    /// Both flags must already be declared. State each direction separately to
    /// make the pair reciprocal; the order of the two calls, and of the two
    /// declarations, changes nothing, because the catalog digest and the
    /// manifest sort every relationship.
    ///
    /// Declared mode checks it once at startup; a control-plane snapshot
    /// holds the published entry that would break it, with
    /// [`HoldCause::InvariantUnmet`](crate::low_level::HoldCause::InvariantUnmet).
    /// The call refuses a relationship the two code defaults already break.
    ///
    /// ```
    /// use batter_flags_core::{CatalogBuilder, SettingsPrefix, Toggle};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
    /// builder.process_kind("serving")?;
    /// builder.environment_class("hosted")?;
    /// let ledger_v2 = builder.mode_flag("ledger_v2", Toggle::Off).declare()?;
    /// let ledger_audit = builder.mode_flag("ledger_audit", Toggle::Off).declare()?;
    ///
    /// builder.requires(&ledger_v2, Toggle::On, &ledger_audit, Toggle::On)?;
    /// builder.requires(&ledger_audit, Toggle::On, &ledger_v2, Toggle::On)?;
    ///
    /// let catalog = builder.build()?;
    /// assert_eq!(catalog.declaration("ledger_v2").unwrap().invariants().len(), 1);
    /// assert_eq!(catalog.declaration("ledger_audit").unwrap().invariants().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`BuildError::ForeignHandle`] for a flag of another catalog,
    /// [`BuildError::SelfInvariant`] for a flag related to itself,
    /// [`BuildError::AliasedValue`] and [`BuildError::UndeclaredVariant`] for a
    /// value the type cannot name unambiguously,
    /// [`BuildError::DuplicateInvariant`] for a relationship already stated and
    /// [`BuildError::DefaultsViolateInvariant`] for one the code defaults
    /// break.
    pub fn requires<A: Flag, B: Flag>(
        &mut self,
        flag: &A,
        when: A::Value,
        other: &B,
        then: B::Value,
    ) -> Result<(), BuildError> {
        self.relate(InvariantRule::Requires, side(flag, when), side(other, then))
    }

    /// `flag` at `when` and `other` at `then` must not both hold.
    ///
    /// The relationship is symmetric, so one call states it; it is stored on
    /// `flag`. Declared mode checks it once at startup, and a control-plane
    /// snapshot holds the published entry that would break it. The call
    /// refuses a relationship the two code defaults already break.
    ///
    /// # Errors
    ///
    /// As for [`requires`](Self::requires).
    pub fn conflicts<A: Flag, B: Flag>(
        &mut self,
        flag: &A,
        when: A::Value,
        other: &B,
        then: B::Value,
    ) -> Result<(), BuildError> {
        self.relate(
            InvariantRule::Conflicts,
            side(flag, when),
            side(other, then),
        )
    }

    /// Either both toggles are `on`, or neither is.
    ///
    /// It exists only between toggles, which the signature enforces:
    ///
    /// ```compile_fail,E0271
    /// # use batter_flags_core::{CatalogBuilder, FlagValue, SettingsPrefix, Toggle};
    /// # #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    /// # enum Checkout { Classic, Express }
    /// # impl FlagValue for Checkout {
    /// #     const VARIANTS: &'static [&'static str] = &["classic", "express"];
    /// #     fn variant(self) -> &'static str { match self { Self::Classic => "classic", Self::Express => "express" } }
    /// #     fn from_variant(v: &str) -> Option<Self> { match v { "classic" => Some(Self::Classic), "express" => Some(Self::Express), _ => None } }
    /// # }
    /// # let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    /// let toggle = builder.mode_flag("toggle", Toggle::Off).declare().unwrap();
    /// let checkout = builder.mode_flag("checkout", Checkout::Classic).declare().unwrap();
    /// let _ = builder.both_or_none(&toggle, &checkout);
    /// ```
    ///
    /// For enum flags state each direction with [`requires`](Self::requires).
    /// The relationship is symmetric, so one call states it; it is stored on
    /// `flag`. The call refuses a pair whose code defaults already break it.
    ///
    /// # Errors
    ///
    /// As for [`requires`](Self::requires).
    pub fn both_or_none<A, B>(&mut self, flag: &A, other: &B) -> Result<(), BuildError>
    where
        A: Flag<Value = Toggle>,
        B: Flag<Value = Toggle>,
    {
        self.relate(
            InvariantRule::BothOrNone,
            side(flag, Toggle::On),
            side(other, Toggle::On),
        )
    }

    /// Validates one relationship and stores it on `flag`'s entry.
    fn relate(
        &mut self,
        rule: InvariantRule,
        flag: Side<'_>,
        other: Side<'_>,
    ) -> Result<(), BuildError> {
        let key = flag.handle.key().clone();
        for side in [&flag, &other] {
            if side.handle.catalog() != self.id {
                return Err(BuildError::ForeignHandle {
                    key,
                    handle: side.handle.key().clone(),
                });
            }
        }
        if flag.handle.index() == other.handle.index() {
            return Err(BuildError::SelfInvariant { key });
        }
        for side in [&flag, &other] {
            if side.aliased {
                return Err(BuildError::AliasedValue {
                    key,
                    flag: side.handle.key().clone(),
                    variant: side.variant,
                });
            }
            if !self.entries[side.handle.index()]
                .variants
                .contains(&side.variant)
            {
                return Err(BuildError::UndeclaredVariant {
                    key,
                    flag: side.handle.key().clone(),
                    variant: side.variant,
                });
            }
        }
        let invariant = Invariant {
            rule,
            flag: key.clone(),
            variant: flag.variant,
            other: other.handle.key().clone(),
            other_variant: other.variant,
        };
        let (entry, other_entry) = (
            &self.entries[flag.handle.index()],
            &self.entries[other.handle.index()],
        );
        if entry.invariants.contains(&invariant) {
            return Err(BuildError::DuplicateInvariant {
                key,
                rule,
                other: invariant.other,
            });
        }
        let defaults = (
            entry.default_variant.unwrap_or_default(),
            other_entry.default_variant.unwrap_or_default(),
        );
        if !invariant.holds(defaults.0, defaults.1) {
            return Err(BuildError::DefaultsViolateInvariant {
                key,
                rule,
                other: invariant.other,
            });
        }
        let entry = &mut self.entries[flag.handle.index()];
        let mut invariants = std::mem::take(&mut entry.invariants).into_vec();
        invariants.push(invariant);
        invariants.sort_unstable();
        entry.invariants = invariants.into_boxed_slice();
        Ok(())
    }
}
