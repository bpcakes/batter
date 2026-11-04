//! Flag values: closed, ordered lists of declared variant names.

use std::{any::TypeId, fmt, hash::Hash};

use crate::key::KeyError;

/// A value a release or mode flag can take: one of a closed, ordered list of
/// lower-case variant names.
///
/// Every value is a non-secret declared name. There are no strings, numbers or
/// JSON values, and `bool` is deliberately not a `FlagValue`: a toggle is
/// [`Toggle`], whose spelling each catalog fixes once with [`ToggleLiterals`].
///
/// Implement it for an application enum by listing the variant names in
/// declaration order and mapping both ways. The catalog checks at build time
/// that the list is non-empty, that every name follows the key grammar
/// `[a-z][a-z0-9_]{0,63}`, that the names are unique and that
/// `from_variant(value.variant()) == Some(value)` for each listed name.
///
/// ```
/// use runflag_core::FlagValue;
///
/// #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// enum Checkout {
///     Classic,
///     Express,
/// }
///
/// impl FlagValue for Checkout {
///     const VARIANTS: &'static [&'static str] = &["classic", "express"];
///
///     fn variant(self) -> &'static str {
///         match self {
///             Self::Classic => "classic",
///             Self::Express => "express",
///         }
///     }
///
///     fn from_variant(variant: &str) -> Option<Self> {
///         match variant {
///             "classic" => Some(Self::Classic),
///             "express" => Some(Self::Express),
///             _ => None,
///         }
///     }
/// }
/// ```
///
/// A string is not a flag value, so a flag of strings does not compile:
///
/// ```compile_fail,E0277
/// use runflag_core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, ToggleLiterals};
///
/// let mut builder = CatalogBuilder::new(
///     SettingsPrefix::new("FLAG_").unwrap(),
///     ToggleLiterals::TrueFalse,
/// );
/// let expiry = Expiry::on(CivilDate::new(2027, 1, 1).unwrap());
/// let _flag = builder
///     .release_flag::<String>("greeting", String::from("hello"), expiry)
///     .declare();
/// ```
///
/// Nor is `bool`: a toggle is [`Toggle`], spelled once per catalog.
///
/// ```compile_fail,E0277
/// use runflag_core::{CatalogBuilder, SettingsPrefix, ToggleLiterals};
///
/// let mut builder = CatalogBuilder::new(
///     SettingsPrefix::new("FLAG_").unwrap(),
///     ToggleLiterals::TrueFalse,
/// );
/// let _flag = builder.mode_flag("strict_mode", false).declare();
/// ```
pub trait FlagValue: Copy + Eq + Hash + fmt::Debug + Send + Sync + 'static {
    /// The closed, ordered list of variant names.
    const VARIANTS: &'static [&'static str];

    /// This value's variant name, one of [`VARIANTS`](Self::VARIANTS).
    fn variant(self) -> &'static str;

    /// The value for a variant name, or `None` for an undeclared name.
    fn from_variant(variant: &str) -> Option<Self>;
}

/// The two-state value of a toggle flag.
///
/// Its Rust variants are fixed; the literal each catalog reads and reports
/// for them (`true`/`false` or `on`/`off`) is the catalog's [`ToggleLiterals`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Toggle {
    /// The closed state.
    Off,
    /// The open state.
    On,
}

impl FlagValue for Toggle {
    const VARIANTS: &'static [&'static str] = &["off", "on"];

    fn variant(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }

    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            _ => None,
        }
    }
}

/// The spelling of every [`Toggle`] in one catalog.
///
/// Fixed once per catalog: every toggle setting is read, every toggle variant
/// reported and every toggle entry stored with this spelling.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ToggleLiterals {
    /// `false` for [`Toggle::Off`] and `true` for [`Toggle::On`].
    TrueFalse,
    /// `off` for [`Toggle::Off`] and `on` for [`Toggle::On`].
    OnOff,
}

impl ToggleLiterals {
    /// The literals in [`Toggle::VARIANTS`] order: the closed state first.
    pub fn literals(self) -> [&'static str; 2] {
        match self {
            Self::TrueFalse => ["false", "true"],
            Self::OnOff => ["off", "on"],
        }
    }

    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrueFalse => "true_false",
            Self::OnOff => "on_off",
        }
    }
}

/// The closed library tag of a declaration's value type.
///
/// It is never a Rust type name: every toggle flag is `toggle`, every
/// application enum is `enum`, and a kill switch is `switch`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ValueType {
    /// A [`Toggle`].
    Toggle,
    /// An application enum implementing [`FlagValue`].
    Enum,
    /// A kill switch, which has no [`FlagValue`].
    Switch,
}

impl ValueType {
    /// The stable tag the digest and manifest carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Enum => "enum",
            Self::Switch => "switch",
        }
    }

    /// The tag of a flag value type.
    pub fn of<T: FlagValue>() -> Self {
        if TypeId::of::<T>() == TypeId::of::<Toggle>() {
            Self::Toggle
        } else {
            Self::Enum
        }
    }
}

/// Why a [`FlagValue`] implementation's variant list was refused.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum VariantsError {
    /// `VARIANTS` was empty.
    #[error("a flag value needs at least one variant")]
    Empty,
    /// A variant name broke the key grammar.
    #[error("variant {variant:?} is not a valid name: {cause}")]
    Invalid {
        /// The offending name.
        variant: Box<str>,
        /// Why it is invalid.
        cause: KeyError,
    },
    /// A variant name appeared twice.
    #[error("variant {variant:?} is listed twice")]
    Duplicate {
        /// The repeated name.
        variant: Box<str>,
    },
    /// `from_variant` and `variant` disagreed for a listed name.
    #[error("variant {variant:?} does not round-trip through from_variant and variant")]
    NotRoundTrip {
        /// The name that failed to round-trip.
        variant: Box<str>,
    },
    /// The default value's variant is not in `VARIANTS`.
    #[error("default variant {variant:?} is not declared in VARIANTS")]
    DefaultUndeclared {
        /// The default's variant name.
        variant: Box<str>,
    },
}

/// Checks a flag value type's variant list and returns the index of
/// `default` within it.
pub(crate) fn check_variants<T: FlagValue>(default: T) -> Result<usize, VariantsError> {
    if T::VARIANTS.is_empty() {
        return Err(VariantsError::Empty);
    }
    for (index, variant) in T::VARIANTS.iter().enumerate() {
        if let Err(cause) = crate::key::FlagKey::new(variant) {
            return Err(VariantsError::Invalid {
                variant: Box::from(*variant),
                cause,
            });
        }
        if T::VARIANTS[..index].contains(variant) {
            return Err(VariantsError::Duplicate {
                variant: Box::from(*variant),
            });
        }
        if T::from_variant(variant).map(FlagValue::variant) != Some(*variant) {
            return Err(VariantsError::NotRoundTrip {
                variant: Box::from(*variant),
            });
        }
    }
    let name = default.variant();
    T::VARIANTS
        .iter()
        .position(|variant| *variant == name)
        .ok_or_else(|| VariantsError::DefaultUndeclared {
            variant: Box::from(name),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum Broken {
        A,
    }

    impl FlagValue for Broken {
        const VARIANTS: &'static [&'static str] = &["a", "b"];

        fn variant(self) -> &'static str {
            "a"
        }

        fn from_variant(variant: &str) -> Option<Self> {
            (variant == "a").then_some(Self::A)
        }
    }

    #[test]
    fn toggle_tags_and_literals_are_fixed() {
        assert_eq!(ValueType::of::<Toggle>(), ValueType::Toggle);
        assert_eq!(ValueType::of::<Broken>(), ValueType::Enum);
        assert_eq!(ToggleLiterals::TrueFalse.literals(), ["false", "true"]);
        assert_eq!(ToggleLiterals::OnOff.literals(), ["off", "on"]);
        assert_eq!(check_variants(Toggle::On), Ok(1));
        assert_eq!(check_variants(Toggle::Off), Ok(0));
    }

    #[test]
    fn inconsistent_value_types_are_refused() {
        assert_eq!(
            check_variants(Broken::A),
            Err(VariantsError::NotRoundTrip {
                variant: Box::from("b")
            })
        );
    }
}
