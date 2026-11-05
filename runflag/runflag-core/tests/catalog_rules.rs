//! Catalog build rules: required settings, retired names, invariants and
//! flag value types.

#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Checkout, expiry, vocabulary_builder};
use runflag_core::{
    BuildError, CatalogBuilder, FlagValue, InvariantRule, ModeDeclaration, ReleaseDeclaration,
    SettingName, SettingNameError, Toggle, VariantsError,
};

fn new_checkout(builder: &mut CatalogBuilder) -> ReleaseDeclaration<'_, Toggle> {
    builder.release_flag("new_checkout", Toggle::Off, expiry())
}

fn style(builder: &mut CatalogBuilder) -> ModeDeclaration<'_, Checkout> {
    builder.mode_flag("checkout_style", Checkout::Classic)
}

fn cause<H: std::fmt::Debug>(result: Result<H, BuildError>) -> VariantsError {
    match result {
        Err(BuildError::InvalidVariants { cause, .. }) => cause,
        other => panic!("expected invalid variants, got {other:?}"),
    }
}

#[test]
fn required_settings_are_validated_names_outside_the_prefix() {
    let mut builder = vocabulary_builder();
    let under = new_checkout(&mut builder)
        .requires_settings(Toggle::On, &["FLAG_PAYMENT_URL"])
        .declare();
    assert!(
        matches!(under, Err(BuildError::RequiredSettingUnderPrefix { name, .. }) if name.as_str() == "FLAG_PAYMENT_URL")
    );
    let invalid = new_checkout(&mut builder)
        .requires_settings(Toggle::On, &["payment_url"])
        .declare();
    assert!(matches!(
        invalid,
        Err(BuildError::InvalidRequiredSetting {
            cause: SettingNameError::InvalidStart { .. },
            ..
        })
    ));
    let empty = new_checkout(&mut builder)
        .requires_settings(Toggle::On, &[])
        .declare();
    assert!(matches!(
        empty,
        Err(BuildError::EmptyRequiredSettings { variant: "on", .. })
    ));
    let repeated = new_checkout(&mut builder)
        .requires_settings(Toggle::On, &["PAYMENT_URL", "PAYMENT_URL"])
        .declare();
    assert!(matches!(
        repeated,
        Err(BuildError::DuplicateRequiredSetting { .. })
    ));
    let default = new_checkout(&mut builder)
        .requires_settings(Toggle::Off, &["PAYMENT_URL"])
        .declare();
    assert!(matches!(
        default,
        Err(BuildError::RequiredSettingsOnDefault { variant: "off", .. })
    ));
    let twice = new_checkout(&mut builder)
        .requires_settings(Toggle::On, &["PAYMENT_URL"])
        .requires_settings(Toggle::On, &["PAYMENT_KEY"])
        .declare();
    assert!(matches!(
        twice,
        Err(BuildError::DuplicateRequiresSettings { variant: "on", .. })
    ));
    // A name sharing the prefix's letters but not its namespace is outside it.
    new_checkout(&mut builder)
        .requires_settings(Toggle::On, &["PAYMENT_URL", "FLAGSHIP_URL"])
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    let declared = catalog.declaration("new_checkout").unwrap();
    let required: Vec<_> = declared.requires_settings().collect();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0].0, "on");
    let names: Vec<&str> = required[0].1.iter().map(SettingName::as_str).collect();
    assert_eq!(names, ["FLAGSHIP_URL", "PAYMENT_URL"]);
}

#[test]
fn retired_names_are_validated_and_never_in_use() {
    let mut builder = vocabulary_builder();
    assert!(matches!(
        builder.retired("enable_x", "note"),
        Err(BuildError::InvalidRetiredName { .. })
    ));
    assert!(matches!(
        builder.retired("ENABLE_X", "  "),
        Err(BuildError::EmptyMigrationNote { .. })
    ));
    builder.retired("ENABLE_X", "set FLAG_X instead").unwrap();
    assert!(matches!(
        builder.retired("ENABLE_X", "again"),
        Err(BuildError::DuplicateRetiredName { .. })
    ));
    // A retired name under the prefix is a declared flag's name only once declared.
    builder
        .retired("FLAG_OLD_MODE", "use FLAG_STRICT_MODE")
        .unwrap();
    let collides = builder.mode_flag("old_mode", Toggle::Off).declare();
    assert!(
        matches!(collides, Err(BuildError::RetiredNameInUse { name }) if name.as_str() == "FLAG_OLD_MODE")
    );
    builder
        .mode_flag("strict_mode", Toggle::Off)
        .declare()
        .unwrap();
    assert!(matches!(
        builder.retired("FLAG_STRICT_MODE", "note"),
        Err(BuildError::RetiredNameInUse { .. })
    ));
    let required = builder
        .release_flag("new_checkout", Toggle::Off, expiry())
        .requires_settings(Toggle::On, &["ENABLE_X"]);
    assert!(
        matches!(required.declare(), Err(BuildError::RetiredNameInUse { name }) if name.as_str() == "ENABLE_X")
    );
    builder
        .release_flag("new_checkout", Toggle::Off, expiry())
        .requires_settings(Toggle::On, &["PAYMENT_URL"])
        .declare()
        .unwrap();
    assert!(matches!(
        builder.retired("PAYMENT_URL", "note"),
        Err(BuildError::RetiredNameInUse { .. })
    ));
    let catalog = builder.build().unwrap();
    let retired: Vec<&str> = catalog
        .retired()
        .iter()
        .map(|r| r.name().as_str())
        .collect();
    assert_eq!(retired, ["ENABLE_X", "FLAG_OLD_MODE"]);
    assert_eq!(catalog.retired()[0].migration_note(), "set FLAG_X instead");
}

#[test]
fn invariants_name_this_catalog_and_hold_at_the_defaults() {
    let mut other = vocabulary_builder();
    let foreign = other
        .mode_flag("payments_v2", Toggle::Off)
        .declare()
        .unwrap();
    let mut builder = vocabulary_builder();
    let payments = builder
        .mode_flag("payments_v2", Toggle::Off)
        .declare()
        .unwrap();
    let on_by_default = builder
        .mode_flag("legacy_api", Toggle::On)
        .declare()
        .unwrap();

    let foreign = style(&mut builder)
        .requires(Checkout::Express, &foreign, Toggle::On)
        .declare();
    assert!(
        matches!(foreign, Err(BuildError::ForeignHandle { handle, .. }) if handle.as_str() == "payments_v2")
    );
    let duplicate = style(&mut builder)
        .requires(Checkout::Express, &payments, Toggle::On)
        .requires(Checkout::Express, &payments, Toggle::On)
        .declare();
    assert!(matches!(
        duplicate,
        Err(BuildError::DuplicateInvariant {
            rule: InvariantRule::Requires,
            ..
        })
    ));
    let requires = style(&mut builder)
        .requires(Checkout::Classic, &payments, Toggle::On)
        .declare();
    assert!(matches!(
        requires,
        Err(BuildError::DefaultsViolateInvariant {
            rule: InvariantRule::Requires,
            ..
        })
    ));
    let conflicts = style(&mut builder)
        .conflicts(Checkout::Classic, &on_by_default, Toggle::On)
        .declare();
    assert!(matches!(
        conflicts,
        Err(BuildError::DefaultsViolateInvariant {
            rule: InvariantRule::Conflicts,
            ..
        })
    ));
    let both = builder
        .mode_flag("fast_path", Toggle::Off)
        .both_or_none(&on_by_default)
        .declare();
    assert!(matches!(
        both,
        Err(BuildError::DefaultsViolateInvariant {
            rule: InvariantRule::BothOrNone,
            ..
        })
    ));

    style(&mut builder)
        .requires(Checkout::Express, &payments, Toggle::On)
        .conflicts(Checkout::Express, &on_by_default, Toggle::On)
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    let invariants = catalog.declaration("checkout_style").unwrap().invariants();
    assert_eq!(invariants.len(), 2);
    // Invariants sort by rule tag text: conflicts before requires.
    assert_eq!(invariants[0].rule(), InvariantRule::Conflicts);
    assert_eq!(invariants[1].rule(), InvariantRule::Requires);
    assert_eq!(
        (invariants[1].variant(), invariants[1].other().as_str()),
        ("express", "payments_v2")
    );
    assert!(invariants[1].holds("classic", "off"));
    assert!(invariants[1].holds("express", "on"));
    assert!(!invariants[1].holds("express", "off"));
    assert!(!invariants[0].holds("express", "on"));
}

#[test]
fn flag_value_types_are_checked_at_declaration() {
    macro_rules! value {
        ($name:ident, $variants:expr, $variant:expr, $parse:expr) => {
            #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
            struct $name;
            impl FlagValue for $name {
                const VARIANTS: &'static [&'static str] = $variants;
                fn variant(self) -> &'static str {
                    $variant
                }
                fn from_variant(variant: &str) -> Option<Self> {
                    ($parse)(variant).then_some(Self)
                }
            }
        };
    }
    value!(Empty, &[], "a", |_| false);
    value!(Upper, &["A"], "A", |v| v == "A");
    value!(Twice, &["a", "a"], "a", |v| v == "a");
    value!(Lossy, &["a", "b"], "a", |_| true);
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum Partial {
        Listed,
        Unlisted,
    }
    impl FlagValue for Partial {
        const VARIANTS: &'static [&'static str] = &["listed"];
        fn variant(self) -> &'static str {
            match self {
                Self::Listed => "listed",
                Self::Unlisted => "unlisted",
            }
        }
        fn from_variant(variant: &str) -> Option<Self> {
            (variant == "listed").then_some(Self::Listed)
        }
    }
    let mut builder = vocabulary_builder();
    assert_eq!(
        cause(builder.mode_flag("a", Empty).declare()),
        VariantsError::Empty
    );
    assert!(matches!(
        cause(builder.mode_flag("a", Upper).declare()),
        VariantsError::Invalid { .. }
    ));
    assert!(matches!(
        cause(builder.mode_flag("a", Twice).declare()),
        VariantsError::Duplicate { .. }
    ));
    assert!(matches!(
        cause(builder.release_flag("a", Lossy, expiry()).declare()),
        VariantsError::NotRoundTrip { .. }
    ));
    assert!(matches!(
        cause(builder.mode_flag("a", Partial::Unlisted).declare()),
        VariantsError::DefaultUndeclared { .. }
    ));

    // A value outside VARIANTS cannot be named by an invariant or required settings.
    let partial = builder
        .mode_flag("partial", Partial::Listed)
        .declare()
        .unwrap();
    let toggle = builder.mode_flag("toggle", Toggle::Off).declare().unwrap();
    let when = builder
        .mode_flag("a", Partial::Listed)
        .requires(Partial::Unlisted, &toggle, Toggle::On)
        .declare();
    assert!(matches!(
        when,
        Err(BuildError::UndeclaredVariant {
            variant: "unlisted",
            ..
        })
    ));
    let other = builder
        .mode_flag("a", Toggle::Off)
        .requires(Toggle::On, &partial, Partial::Unlisted)
        .declare();
    assert!(
        matches!(other, Err(BuildError::UndeclaredVariant { flag, variant: "unlisted", .. }) if flag.as_str() == "partial")
    );
    let settings = builder
        .mode_flag("a", Partial::Listed)
        .requires_settings(Partial::Unlisted, &["PAYMENT_URL"])
        .declare();
    assert!(matches!(
        settings,
        Err(BuildError::UndeclaredVariant {
            variant: "unlisted",
            ..
        })
    ));
}

/// Two values that share the variant name `first`, as a copy-pasted match
/// arm would produce. Every listed name round-trips, so only a supplied
/// value's identity can be wrong.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Shadowed {
    First,
    Second,
    Shadow,
}

impl FlagValue for Shadowed {
    const VARIANTS: &'static [&'static str] = &["first", "second"];

    fn variant(self) -> &'static str {
        match self {
            Self::First | Self::Shadow => "first",
            Self::Second => "second",
        }
    }

    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "first" => Some(Self::First),
            "second" => Some(Self::Second),
            _ => None,
        }
    }
}

#[test]
fn typed_values_keep_their_identity() {
    let mut builder = vocabulary_builder();
    // A default that shares a listed name with another value is refused.
    let default = builder.mode_flag("a", Shadowed::Shadow).declare();
    assert_eq!(
        cause(default),
        VariantsError::AliasedDefault {
            variant: "first".into()
        }
    );
    let shadowed = builder
        .mode_flag("shadowed", Shadowed::First)
        .declare()
        .unwrap();
    let toggle = builder.mode_flag("toggle", Toggle::Off).declare().unwrap();
    // So are aliased values on either side of an invariant and in required settings.
    let when = builder
        .mode_flag("a", Shadowed::First)
        .requires(Shadowed::Shadow, &toggle, Toggle::On)
        .declare();
    assert!(matches!(
        when,
        Err(BuildError::AliasedValue { key, flag, variant: "first" })
            if key.as_str() == "a" && flag.as_str() == "a"
    ));
    let other = builder
        .mode_flag("a", Toggle::Off)
        .conflicts(Toggle::On, &shadowed, Shadowed::Shadow)
        .declare();
    assert!(matches!(
        other,
        Err(BuildError::AliasedValue { flag, variant: "first", .. }) if flag.as_str() == "shadowed"
    ));
    let settings = builder
        .mode_flag("a", Shadowed::Second)
        .requires_settings(Shadowed::Shadow, &["PAYMENT_URL"])
        .declare();
    assert!(matches!(
        settings,
        Err(BuildError::AliasedValue {
            variant: "first",
            ..
        })
    ));
    // The value that owns the name is accepted in each position.
    builder
        .mode_flag("a", Shadowed::Second)
        .requires(Shadowed::First, &toggle, Toggle::Off)
        .requires_settings(Shadowed::First, &["PAYMENT_URL"])
        .declare()
        .unwrap();
}

#[test]
fn required_absence_combines_with_ownership_and_class_restrictions() {
    let mut builder = fixtures::builder();
    let serving = builder.process_kind("serving").unwrap();
    builder.process_kind("worker").unwrap();
    let development = builder.environment_class("development").unwrap();
    builder.environment_class("hosted").unwrap();
    builder
        .mode_flag("debug_panel", Toggle::Off)
        .only_in(&development)
        .absence(runflag_core::Absence::Required)
        .declare()
        .unwrap();
    builder
        .mode_flag("serving_mode", Toggle::Off)
        .owners(runflag_core::Owners::Only(std::slice::from_ref(&serving)))
        .absence(runflag_core::Absence::Required)
        .declare()
        .unwrap();
    let catalog = builder.build().unwrap();
    let required = catalog
        .declarations()
        .filter(|declared| declared.absence() == runflag_core::Absence::Required)
        .count();
    assert_eq!(required, 2);
}
