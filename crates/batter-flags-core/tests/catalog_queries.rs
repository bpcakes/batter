//! Catalog queries: declarations, ownership, expiry, setting names and
//! branded handles.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;

use batter_flags_core::{
    Absence, CatalogBuilder, FlipDiscipline, Kind, Owners, SafeState, StaleRule, Toggle, ValueType,
};
use fixtures::{Change, builder, date, reference, vocabulary_builder};

fn halt(builder: &mut CatalogBuilder) -> batter_flags_core::KillSwitch {
    builder
        .kill_switch("halt", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap()
}

#[test]
fn declarations_report_their_kind_and_policy() {
    let catalog = reference(Change::None);
    let keys: Vec<&str> = catalog.declarations().map(|d| d.key().as_str()).collect();
    assert_eq!(
        keys,
        [
            "checkout_halt",
            "checkout_style",
            "debug_panel",
            "export_halt",
            "fast_path",
            "new_checkout"
        ]
    );
    let new_checkout = catalog.declaration("new_checkout").unwrap();
    assert_eq!(
        (new_checkout.kind(), new_checkout.value_type()),
        (Kind::Release, ValueType::Toggle)
    );
    assert_eq!(new_checkout.variants(), ["off", "on"]);
    assert_eq!(new_checkout.default_variant(), Some("off"));
    assert_eq!(
        new_checkout.expiry().unwrap().last_valid_day(),
        date(2027, 3, 31)
    );
    assert_eq!(new_checkout.absence(), Absence::Default);
    assert_eq!(new_checkout.flip_discipline(), FlipDiscipline::Rolling);
    assert_eq!(new_checkout.safe_state(), None);
    let style = catalog.declaration("checkout_style").unwrap();
    assert_eq!(
        (style.kind(), style.value_type(), style.expiry()),
        (Kind::Mode, ValueType::Enum, None)
    );
    assert_eq!(style.owners(), Owners::All);
    assert!(catalog.declaration("missing").is_none());
    assert!(catalog.declaration("Not-A-Key").is_none());
}

#[test]
fn kill_switches_report_their_safe_state_and_stale_rule() {
    let catalog = reference(Change::None);
    let export = catalog.declaration("export_halt").unwrap();
    assert_eq!(
        (export.kind(), export.value_type()),
        (Kind::Switch, ValueType::Switch)
    );
    assert!(export.variants().is_empty());
    assert_eq!(export.default_variant(), None);
    assert_eq!(export.safe_state(), Some(SafeState::Engaged));
    assert_eq!(export.stale_rule(), Some(StaleRule::EngageWhenStale));
    assert_eq!(export.absence(), Absence::Required);
    assert_eq!(export.flip_discipline(), FlipDiscipline::Coordinated);
    assert_eq!(
        catalog
            .declaration("debug_panel")
            .unwrap()
            .only_in()
            .unwrap()
            .name(),
        "development"
    );
}

#[test]
fn ownership_is_queried_per_kind() {
    let (mut builder, serving, _) = builder("serving", "hosted");
    let worker = builder.process_kind("worker").unwrap();
    builder
        .mode_flag("everywhere", Toggle::Off)
        .declare()
        .unwrap();
    builder
        .mode_flag("serving_only", Toggle::Off)
        .owned_only_by(&serving)
        .declare()
        .unwrap();
    let catalog = builder.build();
    let owned = |kind| {
        catalog
            .owned_by(kind)
            .unwrap()
            .map(|key| key.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(owned(&serving), ["everywhere", "serving_only"]);
    assert_eq!(owned(&worker), ["everywhere"]);
    let mut other = vocabulary_builder();
    let foreign = other.process_kind("reporting").unwrap();
    assert_eq!(
        catalog.owned_by(&foreign).err().unwrap().kind(),
        "reporting"
    );
}

#[test]
fn expiry_is_compared_with_the_date_the_caller_passes() {
    let catalog = reference(Change::None);
    let expired = |year, month, day| {
        catalog
            .expired(date(year, month, day))
            .map(|d| d.key().to_string())
            .collect::<Vec<_>>()
    };
    assert!(expired(2026, 12, 31).is_empty());
    assert_eq!(expired(2027, 1, 1), ["fast_path"]);
    assert_eq!(expired(2027, 4, 1), ["fast_path", "new_checkout"]);
    assert_eq!(
        expired(9999, 12, 31).len(),
        2,
        "mode flags and switches never expire"
    );

    // An expiry already past never fails the build.
    let mut builder = vocabulary_builder();
    let ancient = batter_flags_core::Expiry::on(date(1, 1, 1));
    builder
        .release_flag("stale_release", Toggle::Off, ancient)
        .declare()
        .unwrap();
    assert_eq!(builder.build().expired(date(1, 1, 2)).count(), 1);
}

#[test]
fn setting_names_are_injective_over_the_key_alphabet() {
    const ALPHABET: [char; 5] = ['a', 'z', '0', '9', '_'];
    let prefix = batter_flags_core::SettingsPrefix::new("FLAG_").unwrap();
    let mut seen = std::collections::BTreeMap::new();
    for first in ['a', 'z'] {
        for second in ALPHABET {
            for third in ALPHABET {
                let key =
                    batter_flags_core::FlagKey::new(&format!("{first}{second}{third}")).unwrap();
                let name = key.setting_name(&prefix);
                let recovered = name
                    .as_str()
                    .strip_prefix("FLAG_")
                    .unwrap()
                    .to_ascii_lowercase();
                assert_eq!(recovered, key.as_str());
                assert!(seen.insert(name, key).is_none());
            }
        }
    }
    assert_eq!(seen.len(), 2 * 25);
    let catalog = reference(Change::None);
    let names: std::collections::BTreeSet<_> =
        catalog.declarations().map(|d| d.setting_name()).collect();
    assert_eq!(names.len(), catalog.declarations().len());
}

#[test]
fn handles_are_branded_by_their_catalog() {
    use batter_flags_core::Flag as _;
    let mut first = vocabulary_builder();
    let mut second = vocabulary_builder();
    let a = first
        .mode_flag("strict_mode", Toggle::Off)
        .declare()
        .unwrap();
    let b = second
        .mode_flag("strict_mode", Toggle::Off)
        .declare()
        .unwrap();
    assert_eq!(a.key(), b.key());
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "fingerprints describe the declaration"
    );
    assert_ne!(a, b, "handles from different catalogs are distinct");
    assert_eq!(a, a.clone());
    assert_eq!(format!("{a:?}"), "ModeFlag(\"strict_mode\")");
    let switch = halt(&mut first);
    assert_eq!(format!("{switch:?}"), "KillSwitch(\"halt\")");
    assert_eq!(switch.key().as_str(), "halt");
}

#[test]
fn catalogs_and_handles_cross_threads() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<batter_flags_core::Catalog>();
    shareable::<batter_flags_core::ReleaseFlag<Toggle>>();
    shareable::<batter_flags_core::ModeFlag<fixtures::Checkout>>();
    shareable::<batter_flags_core::KillSwitch>();
    shareable::<batter_flags_core::ProcessKind>();
    shareable::<batter_flags_core::EnvironmentClass>();
    shareable::<batter_flags_core::BuildError>();
}
