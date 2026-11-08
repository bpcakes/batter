//! Fixtures shared by the batter-flags-core integration tests.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]

use batter_flags_core::{
    Catalog, CatalogBuilder, CivilDate, EnvironmentClass, Expiry, FlagValue, KillSwitch,
    ProcessKind, ReleaseDeclaration, ReleaseFlag, SafeState, SettingsPrefix, StaleRule, Toggle,
};

/// A two-variant application enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Checkout {
    Classic,
    Express,
}

impl FlagValue for Checkout {
    const VARIANTS: &'static [&'static str] = &["classic", "express"];

    fn variant(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Express => "express",
        }
    }

    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "classic" => Some(Self::Classic),
            "express" => Some(Self::Express),
            _ => None,
        }
    }
}

/// [`Checkout`] with one more variant, as a later build would declare it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CheckoutWide {
    Classic,
    Express,
    Instant,
}

impl FlagValue for CheckoutWide {
    const VARIANTS: &'static [&'static str] = &["classic", "express", "instant"];

    fn variant(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Express => "express",
            Self::Instant => "instant",
        }
    }

    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "classic" => Some(Self::Classic),
            "express" => Some(Self::Express),
            "instant" => Some(Self::Instant),
            _ => None,
        }
    }
}

pub fn date(year: u16, month: u8, day: u8) -> CivilDate {
    CivilDate::new(year, month, day).unwrap()
}

pub fn expiry() -> Expiry {
    Expiry::on(date(2027, 3, 31))
}

/// A builder under `FLAG_` with no vocabulary.
pub fn builder() -> CatalogBuilder {
    CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap())
}

/// A builder with the process kinds `serving` and `worker` and the
/// environment classes `development` and `hosted`.
pub fn vocabulary_builder() -> CatalogBuilder {
    let mut builder = builder();
    builder.process_kind("serving").unwrap();
    builder.process_kind("worker").unwrap();
    builder.environment_class("development").unwrap();
    builder.environment_class("hosted").unwrap();
    builder
}

/// The one change the parametrized reference catalog applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    /// The reference itself.
    None,
    Prefix(&'static str),
    ExtraKind,
    ExtraClass,
    ExtraRetired,
    NewCheckoutOwnedByAll,
    NewCheckoutUnpinned,
    NewCheckoutUngoverned,
    NewCheckoutExpiry(CivilDate),
    WithoutRequiresSettings,
    DebugPanelEverywhere,
    DebugPanelRolling,
    /// Drops the `checkout_style` invariant and keeps the default.
    WithoutInvariant,
    /// Drops the `checkout_style` invariant and makes `express` the default.
    ExpressDefaultWithoutInvariant,
    ExportHaltAbsenceDefault,
    ExportHaltSafeState(SafeState),
    ExportHaltStaleRule(StaleRule),
    MigrationNote(&'static str),
    /// The same catalog declared in another order.
    ReversedOrder,
}

/// The reference catalog the golden vectors pin, with `change` applied.
///
/// It declares every field a declaration can carry: two kill switches (one
/// restricted, required, coordinated and engaging when stale), a governed and
/// pinned release toggle with required settings, an enum mode flag with a
/// `requires` invariant, a development-only mode toggle with a `conflicts`
/// invariant, a release toggle with a `both_or_none` invariant and a retired
/// name.
pub fn reference(change: Change) -> Catalog {
    let prefix = match change {
        Change::Prefix(prefix) => prefix,
        _ => "FLAG_",
    };
    let prefix = SettingsPrefix::new(prefix).unwrap();
    let mut builder = CatalogBuilder::new(prefix);
    let reversed = change == Change::ReversedOrder;
    let (serving, worker, development);
    if reversed {
        worker = builder.process_kind("worker").unwrap();
        serving = builder.process_kind("serving").unwrap();
        builder.environment_class("hosted").unwrap();
        development = builder.environment_class("development").unwrap();
        retire(&mut builder, change);
    } else {
        serving = builder.process_kind("serving").unwrap();
        worker = builder.process_kind("worker").unwrap();
        development = builder.environment_class("development").unwrap();
        builder.environment_class("hosted").unwrap();
    }
    if change == Change::ExtraKind {
        builder.process_kind("reporting").unwrap();
    }
    if change == Change::ExtraClass {
        builder.environment_class("staging").unwrap();
    }
    let checkout_halt = if reversed {
        let halt = checkout_halt(&mut builder);
        export_halt(&mut builder, &worker, change);
        halt
    } else {
        export_halt(&mut builder, &worker, change);
        checkout_halt(&mut builder)
    };
    let new_checkout = new_checkout(&mut builder, (&serving, &worker), &checkout_halt, change);
    let fast_path = if reversed {
        checkout_style(&mut builder, &new_checkout, change);
        fast_path(&mut builder, &new_checkout)
    } else {
        let fast_path = fast_path(&mut builder, &new_checkout);
        checkout_style(&mut builder, &new_checkout, change);
        fast_path
    };
    debug_panel(&mut builder, &fast_path, &development, change);
    if !reversed {
        retire(&mut builder, change);
    }
    if change == Change::ExtraRetired {
        builder
            .retired("LEGACY_EXPORT", "remove it; exports are always on")
            .unwrap();
    }
    builder.build().unwrap()
}

fn retire(builder: &mut CatalogBuilder, change: Change) {
    let note = match change {
        Change::MigrationNote(note) => note,
        _ => "set FLAG_NEW_CHECKOUT instead",
    };
    builder.retired("ENABLE_NEW_CHECKOUT", note).unwrap();
}

fn checkout_halt(builder: &mut CatalogBuilder) -> KillSwitch {
    builder
        .kill_switch(
            "checkout_halt",
            SafeState::Released,
            StaleRule::HoldLastKnown,
        )
        .declare()
        .unwrap()
}

fn export_halt(builder: &mut CatalogBuilder, worker: &ProcessKind, change: Change) {
    let safe_state = match change {
        Change::ExportHaltSafeState(state) => state,
        _ => SafeState::Engaged,
    };
    let stale_rule = match change {
        Change::ExportHaltStaleRule(rule) => rule,
        _ => StaleRule::EngageWhenStale,
    };
    let mut export_halt = builder
        .kill_switch("export_halt", safe_state, stale_rule)
        .owned_only_by(worker)
        .coordinated();
    if change != Change::ExportHaltAbsenceDefault {
        export_halt = export_halt.required();
    }
    export_halt.declare().unwrap();
}

/// Applies what every variant of the reference `new_checkout` shares, whichever
/// ownership state its declaration is in.
fn finish_new_checkout<O>(
    mut flag: ReleaseDeclaration<'_, Toggle, O>,
    checkout_halt: &KillSwitch,
    change: Change,
) -> ReleaseFlag<Toggle> {
    if change != Change::WithoutRequiresSettings {
        flag = flag.requires_settings(Toggle::On, &["PAYMENT_API_URL", "PAYMENT_API_KEY"]);
    }
    if change == Change::NewCheckoutUngoverned {
        flag.declare().unwrap()
    } else {
        flag.governed_by(checkout_halt).declare().unwrap()
    }
}

fn new_checkout(
    builder: &mut CatalogBuilder,
    (serving, worker): (&ProcessKind, &ProcessKind),
    checkout_halt: &KillSwitch,
    change: Change,
) -> ReleaseFlag<Toggle> {
    let last_valid_day = match change {
        Change::NewCheckoutExpiry(day) => day,
        _ => date(2027, 3, 31),
    };
    let flag = builder.release_flag("new_checkout", Toggle::Off, Expiry::on(last_valid_day));
    if change == Change::NewCheckoutOwnedByAll {
        return finish_new_checkout(flag, checkout_halt, change);
    }
    let owned = flag.owned_only_by(serving);
    if change == Change::NewCheckoutUnpinned {
        finish_new_checkout(owned, checkout_halt, change)
    } else {
        finish_new_checkout(owned.pinned_by(worker), checkout_halt, change)
    }
}

fn fast_path(
    builder: &mut CatalogBuilder,
    new_checkout: &ReleaseFlag<Toggle>,
) -> ReleaseFlag<Toggle> {
    let flag = builder
        .release_flag("fast_path", Toggle::Off, Expiry::on(date(2026, 12, 31)))
        .declare()
        .unwrap();
    builder.both_or_none(&flag, new_checkout).unwrap();
    flag
}

fn checkout_style(
    builder: &mut CatalogBuilder,
    new_checkout: &ReleaseFlag<Toggle>,
    change: Change,
) {
    let default = match change {
        Change::ExpressDefaultWithoutInvariant => Checkout::Express,
        _ => Checkout::Classic,
    };
    let flag = builder
        .mode_flag("checkout_style", default)
        .declare()
        .unwrap();
    let without = [
        Change::WithoutInvariant,
        Change::ExpressDefaultWithoutInvariant,
    ];
    if !without.contains(&change) {
        builder
            .requires(&flag, Checkout::Express, new_checkout, Toggle::On)
            .unwrap();
    }
}

fn debug_panel(
    builder: &mut CatalogBuilder,
    fast_path: &ReleaseFlag<Toggle>,
    development: &EnvironmentClass,
    change: Change,
) {
    let mut flag = builder.mode_flag("debug_panel", Toggle::Off);
    if change != Change::DebugPanelRolling {
        flag = flag.coordinated();
    }
    let flag = if change == Change::DebugPanelEverywhere {
        flag.declare().unwrap()
    } else {
        flag.only_in(development).declare().unwrap()
    };
    builder
        .conflicts(&flag, Toggle::On, fast_path, Toggle::On)
        .unwrap();
}
