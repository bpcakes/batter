//! Control-plane invariants under governance: a flag whose governing kill
//! switch is engaged serves its code default, so a flag requiring it is held;
//! a stale rule engaging a governor holds only in a stale view; and a base an
//! engaged governor would break is reported before any snapshot.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    Catalog, CatalogBuilder, Freshness, Layer, ModeFlag, ProcessKind, SettingsPrefix, Toggle,
    low_level::{HoldCause, Snapshot},
};
use world::{Literal, base, flag_row, snapshot};

fn held(snapshot: &Snapshot) -> Vec<(String, HoldCause)> {
    snapshot
        .held(Freshness::Fresh)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect()
}

fn unmet(keys: &[&str]) -> Vec<(String, HoldCause)> {
    keys.iter()
        .map(|key| ((*key).to_owned(), HoldCause::InvariantUnmet))
        .collect()
}

/// `dependent` on requires `governed` on; `governed` is a release flag
/// governed by `halt`, a switch that engages when stale.
fn governed_world() -> (
    Catalog,
    ProcessKind,
    ModeFlag<Toggle>,
    batter_flags_core::ReleaseFlag<Toggle>,
    batter_flags_core::KillSwitch,
) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    builder.environment_class("hosted").unwrap();
    let halt = builder
        .kill_switch(
            "halt",
            batter_flags_core::SafeState::Released,
            batter_flags_core::StaleRule::EngageWhenStale,
        )
        .declare()
        .unwrap();
    let governed = builder
        .release_flag("governed", Toggle::Off, fixtures::expiry())
        .governed_by(&halt)
        .declare()
        .unwrap();
    let dependent = builder
        .mode_flag("dependent", Toggle::Off)
        .declare()
        .unwrap();
    builder
        .requires(&dependent, Toggle::On, &governed, Toggle::On)
        .unwrap();
    (builder.build().unwrap(), serving, dependent, governed, halt)
}

#[test]
fn a_flag_requiring_one_its_engaged_switch_overrides_is_held_too() {
    let (catalog, serving, dependent, governed, halt) = governed_world();
    let base = base(catalog, &serving, &[]);
    let rows = |state| {
        [
            flag_row(&dependent, Toggle::On, 2),
            flag_row(&governed, Toggle::On, 2),
            world::switch_row(&halt, state, 3),
        ]
    };
    // Released: both are served on.
    let released = snapshot(&base, 3, rows(batter_flags_core::SwitchState::Released));
    assert_eq!(held(&released), []);
    // Engaged: `governed` serves its default, so `dependent` is held.
    let engaged = snapshot(&base, 4, rows(batter_flags_core::SwitchState::Engaged));
    assert_eq!(held(&engaged), unmet(&["dependent"]));
    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    assert_eq!(view.evaluate(&governed).value(), Toggle::Off);
    assert_eq!(view.evaluate(&dependent).value(), Toggle::Off);
    // A settings floor engages it the same way.
    let (catalog, serving, dependent, governed, halt) = governed_world();
    let floored = world::base(
        catalog,
        &serving,
        &[Literal::Switch(
            "halt",
            batter_flags_core::SwitchState::Engaged,
        )],
    );
    let read = snapshot(
        &floored,
        3,
        [
            flag_row(&dependent, Toggle::On, 2),
            flag_row(&governed, Toggle::On, 2),
            world::switch_row(&halt, batter_flags_core::SwitchState::Released, 3),
        ],
    );
    assert_eq!(held(&read), unmet(&["dependent"]));
}

#[test]
fn a_stale_rule_that_engages_a_governing_switch_holds_only_in_a_stale_view() {
    let (catalog, serving, dependent, governed, halt) = governed_world();
    let base = base(catalog, &serving, &[]);
    let read = snapshot(
        &base,
        3,
        [
            flag_row(&dependent, Toggle::On, 2),
            flag_row(&governed, Toggle::On, 2),
            world::switch_row(&halt, batter_flags_core::SwitchState::Released, 3),
        ],
    );
    assert_eq!(held(&read), [], "a fresh view's holds");
    let fresh = base.control_plane_view(&read, Freshness::Fresh).unwrap();
    assert_eq!(fresh.evaluate(&dependent).value(), Toggle::On);
    let stale = base.control_plane_view(&read, Freshness::Stale).unwrap();
    assert_eq!(
        stale.switch(&halt).value(),
        batter_flags_core::SwitchState::Engaged
    );
    assert_eq!(stale.evaluate(&governed).value(), Toggle::Off);
    let decision = stale.evaluate(&dependent);
    assert_eq!(
        (decision.value(), decision.layer()),
        (Toggle::Off, &Layer::Base),
        "the dependent is held at its base while the switch is engaged by staleness"
    );
}

#[test]
fn a_base_an_engaged_governor_would_break_is_reported_before_any_snapshot() {
    let (catalog, serving, ..) = governed_world();
    // Without settings both serve their defaults, which always meet.
    assert!(
        base(catalog, &serving, &[])
            .governed_invariant_breaks()
            .is_empty()
    );
    // Settings turn both on: engaging `halt` would serve `dependent` on alone.
    let (catalog, serving, ..) = governed_world();
    let both = base(
        catalog,
        &serving,
        &[
            Literal::Flag("dependent", "on"),
            Literal::Flag("governed", "on"),
        ],
    );
    let breaks = both.governed_invariant_breaks();
    assert_eq!(breaks.len(), 1);
    assert_eq!(
        (breaks[0].flag().as_str(), breaks[0].other().as_str()),
        ("dependent", "governed")
    );
}

#[test]
fn a_stale_views_extra_holds_are_reported_for_that_view() {
    let (catalog, serving, dependent, governed, halt) = governed_world();
    let base = base(catalog, &serving, &[]);
    let read = snapshot(
        &base,
        3,
        [
            flag_row(&dependent, Toggle::On, 2),
            flag_row(&governed, Toggle::On, 2),
            world::switch_row(&halt, batter_flags_core::SwitchState::Released, 3),
        ],
    );
    assert_eq!(held(&read), []);
    let stale: Vec<_> = read
        .held(Freshness::Stale)
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect();
    assert_eq!(stale, unmet(&["dependent"]));
}
