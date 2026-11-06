//! Why declared-mode resolution failed.

use std::{env::VarError, fmt};

use batter_flags_core::{FlagKey, InvariantRule, SettingName, low_level::BaseError};

/// What is wrong with a setting's value.
///
/// Never the value itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ValueProblem {
    /// The value is empty or only whitespace.
    #[error("is blank")]
    Blank,
    /// The value is not valid Unicode.
    #[error("is not valid Unicode")]
    NotUnicode,
    /// The value is not exactly one of the flag's declared variant names.
    #[error("is not one of the flag's declared variants")]
    NotADeclaredVariant,
    /// The value is not `true`, `false`, `on` or `off` in any ASCII letter
    /// case.
    #[error("is not a toggle value (true, false, on or off)")]
    NotAToggle,
    /// The value is not exactly `engaged` or `released`.
    #[error("is not a switch state (engaged or released)")]
    NotASwitchState,
}

/// Why [`resolve`](super::resolve) refused a catalog and its settings.
///
/// Every message names declared things only: flag keys, setting names, the
/// process kind and environment class, the prefix and a retired name's
/// migration note. None carries a setting's value, and neither `{}` nor
/// `{:?}` can show one. The one retained value is the cause of a non-Unicode
/// setting, reachable only through [`source`](std::error::Error::source), which
/// is explicit access to the original `VarError`, as for the settings errors
/// of `batter-core`; a reporter that prints the error chain prints it.
///
/// Resolution reports the first failure it meets, in a fixed order: the kind
/// and class, every declared setting in key order, retired names, unknown names
/// under the prefix, required settings, then invariants.
///
/// ```
/// use batter_flags::{core::{CatalogBuilder, CivilDate, Expiry, SettingsPrefix, Toggle}, declared};
/// use batter_core::settings::SettingsSource;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut catalog = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?);
/// let serving = catalog.process_kind("serving")?;
/// let hosted = catalog.environment_class("hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// catalog.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let catalog = catalog.build()?;
///
/// // A mistyped name under the prefix never resolves silently to a default.
/// let source = SettingsSource::from_pairs([("FLAG_NEW_CHECKOT".into(), "on".into())])?;
/// let today = CivilDate::new(2026, 11, 5)?;
/// let error = declared::resolve(catalog, &source, serving, hosted, today).unwrap_err();
/// assert_eq!(
///     error.to_string(),
///     "a setting under the prefix `FLAG_` names no declared or retired flag",
/// );
/// # Ok(())
/// # }
/// ```
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum DeclaredError {
    /// The process kind was declared by another catalog.
    #[error("process kind `{kind}` belongs to another catalog")]
    ForeignKind {
        /// The kind's declared name.
        kind: String,
    },
    /// The environment class was declared by another catalog.
    #[error("environment class `{class}` belongs to another catalog")]
    ForeignClass {
        /// The class's declared name.
        class: String,
    },
    /// A setting under the catalog's prefix is neither a declared flag's name
    /// nor a retired one. The source reports no key, so only the prefix is
    /// named.
    #[error("a setting under the prefix `{prefix}` names no declared or retired flag")]
    UnknownName {
        /// The catalog's settings prefix.
        prefix: String,
    },
    /// A retired setting name is present.
    #[error("retired setting {name} is present: {migration_note}")]
    Retired {
        /// The retired name.
        name: SettingName,
        /// What the catalog says to do instead.
        migration_note: String,
    },
    /// A value was supplied for a flag this process kind does not own.
    #[error(
        "setting {name} was supplied for flag `{key}`, which process kind `{kind}` does not own"
    )]
    NotOwned {
        /// The flag's key.
        key: FlagKey,
        /// The setting's name.
        name: SettingName,
        /// This process's kind.
        kind: String,
    },
    /// A value was supplied for a flag restricted to another environment
    /// class.
    #[error(
        "setting {name} was supplied for flag `{key}`, which is restricted to environment class `{class}`"
    )]
    OutsideClass {
        /// The flag's key.
        key: FlagKey,
        /// The setting's name.
        name: SettingName,
        /// The one class the flag is restricted to.
        class: String,
    },
    /// A flag declared with `Absence::Required` has no setting where a value
    /// may be presented.
    #[error("setting {name} for flag `{key}` is required and absent")]
    Missing {
        /// The flag's key.
        key: FlagKey,
        /// The setting's name.
        name: SettingName,
    },
    /// A setting's value is not a literal the flag accepts.
    #[error("setting {name} for flag `{key}` {problem}")]
    InvalidValue {
        /// The flag's key.
        key: FlagKey,
        /// The setting's name.
        name: SettingName,
        /// What is wrong with the value.
        problem: ValueProblem,
        /// The original non-Unicode value, retained for trusted inspection.
        #[source]
        cause: Option<VarError>,
    },
    /// A flag resolved to a variant whose required settings are absent or
    /// empty.
    #[error("flag `{key}` resolved to variant `{variant}`, which requires settings that are absent or empty: {}", listed(.names))]
    RequiredSettings {
        /// The flag's key.
        key: FlagKey,
        /// The enabling variant it resolved to.
        variant: &'static str,
        /// The names of the settings that are absent or empty, in name order.
        names: Vec<SettingName>,
    },
    /// Two flags this process kind owns resolved to variants that break a
    /// declared invariant.
    ///
    /// The fields are the invariant as declared, not the values that broke it.
    #[error("{}", invariant_text(*.rule, .flag, .variant, .other, .other_variant))]
    Invariant {
        /// The relation.
        rule: InvariantRule,
        /// The flag the invariant is declared on.
        flag: FlagKey,
        /// That flag's variant the invariant speaks about.
        variant: &'static str,
        /// The other flag.
        other: FlagKey,
        /// The other flag's variant.
        other_variant: &'static str,
    },
    /// The catalog refused a literal that resolution had accepted. This is a
    /// defect in this crate or in `batter-flags-core`, never a settings
    /// problem.
    #[error("resolution and the catalog disagree: {0}")]
    Inconsistent(BaseError),
}

fn listed(names: &[SettingName]) -> String {
    let text: Vec<&str> = names.iter().map(SettingName::as_str).collect();
    text.join(", ")
}

fn invariant_text(
    rule: InvariantRule,
    flag: &FlagKey,
    variant: &str,
    other: &FlagKey,
    other_variant: &str,
) -> String {
    match rule {
        InvariantRule::Requires => format!(
            "invariant broken: flag `{flag}` at `{variant}` requires flag `{other}` at `{other_variant}`"
        ),
        InvariantRule::Conflicts => format!(
            "invariant broken: flag `{flag}` at `{variant}` conflicts with flag `{other}` at `{other_variant}`"
        ),
        InvariantRule::BothOrNone => {
            format!("invariant broken: flags `{flag}` and `{other}` must both be on or both be off")
        }
    }
}

impl fmt::Debug for DeclaredError {
    /// The same text as `Display`: the variants hold declared names only, but
    /// a derived `Debug` would also print the retained `VarError`, which holds
    /// the value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DeclaredError({self})")
    }
}
