//! Governance: a flag whose governing kill switch is engaged serves its code
//! default with `Layer::Override`, whichever layer engaged the switch.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use std::sync::Arc;

use batter_flags_core::{
    CatalogBuilder, Freshness, Layer, PrePublication, Reason, SafeState, SettingsPrefix, StaleRule,
    SwitchState, Toggle,
    low_level::{Candidate, Snapshot},
};
use fixtures::Checkout;
use world::{Literal, base, flag_row, snapshot, switch_row, undecodable_row, version, world};

/// Row: a governing switch engaged, live.
#[test]
fn an_engaged_governing_switch_overrides_the_flag() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let on = snapshot(
        &base,
        5,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Released, 5),
        ],
    );
    let released = base.control_plane_view(&on, Freshness::Fresh).unwrap();
    let served = released.evaluate(&h.new_checkout);
    assert_eq!(
        (served.value(), served.reason(), served.layer()),
        (Toggle::On, Reason::Static, &Layer::Entry)
    );

    let engaged = snapshot(
        &base,
        6,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 6),
        ],
    );
    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    let overridden = view.evaluate(&h.new_checkout);
    assert_eq!(overridden.value(), Toggle::Off, "the code default");
    assert_eq!(overridden.variant(), "off");
    assert_eq!(overridden.reason(), Reason::Disabled);
    let Layer::Override { by } = overridden.layer() else {
        panic!("expected an override layer, got {:?}", overridden.layer());
    };
    assert_eq!(by.as_str(), "checkout_halt");
    assert_eq!(overridden.source(), view.source());
    assert_eq!(overridden.error(), None);

    // A flag no switch governs is untouched, and the switch's own read is.
    assert_eq!(view.evaluate(&h.fast_path).layer(), &Layer::Base);
    assert_eq!(view.switch(&h.checkout_halt).value(), SwitchState::Engaged);
}

#[test]
fn declared_mode_and_a_floor_govern_flags_too() {
    let (catalog, h) = world();
    let engaged = base(
        catalog,
        &h.serving,
        &[
            Literal::Flag("new_checkout", "on"),
            Literal::Switch("checkout_halt", SwitchState::Engaged),
        ],
    );
    let declared = engaged.declared_view().evaluate(&h.new_checkout);
    assert_eq!(
        (declared.value(), declared.reason()),
        (Toggle::Off, Reason::Disabled)
    );
    let floor = engaged
        .not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh)
        .evaluate(&h.new_checkout);
    assert_eq!(
        (floor.value(), floor.reason(), floor.error()),
        (Toggle::Off, Reason::Disabled, None),
        "the floor governs even before anything is published, and before not-ready"
    );
    let unready = engaged
        .not_loaded_view(PrePublication::Unready, Freshness::Fresh)
        .evaluate(&h.new_checkout);
    assert_eq!(
        (unready.reason(), unready.error()),
        (Reason::Disabled, None),
        "an engaged governor answers before the not-ready error does"
    );
}

/// The governor is read through the one kill-switch function, so a held state
/// and a stale rule engage it exactly as they engage the switch itself.
#[test]
fn a_held_or_stale_engaged_governor_overrides_like_a_published_one() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let scope = base.validation_scope();
    let engaged = snapshot(
        &base,
        8,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 8),
        ],
    );
    let held = Arc::new(
        Snapshot::validate(
            Candidate::new(&scope, version(9))
                .rows([
                    flag_row(&h.new_checkout, Toggle::On, 4),
                    undecodable_row("checkout_halt", "switch", "switch", 9),
                ])
                .carrying(&engaged)
                .unwrap(),
        )
        .unwrap(),
    );
    let view = base.control_plane_view(&held, Freshness::Fresh).unwrap();
    assert_eq!(view.evaluate(&h.new_checkout).reason(), Reason::Disabled);

    // Without a carried state the held switch serves its safe state, released.
    let forgotten = snapshot(
        &base,
        10,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            undecodable_row("checkout_halt", "switch", "switch", 10),
        ],
    );
    let view = base
        .control_plane_view(&forgotten, Freshness::Fresh)
        .unwrap();
    assert_eq!(view.evaluate(&h.new_checkout).value(), Toggle::On);
}

#[test]
fn a_stale_view_overrides_when_the_governors_rule_engages_it() {
    let (mut builder, serving, _) =
        CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted").unwrap();
    let halt = builder
        .kill_switch("halt", SafeState::Released, StaleRule::EngageWhenStale)
        .declare()
        .unwrap();
    let hold = builder
        .kill_switch("hold", SafeState::Released, StaleRule::HoldLastKnown)
        .declare()
        .unwrap();
    // The enum's default is its second variant, so "the code default" is not
    // the first variant by accident.
    let strict = builder
        .release_flag("strict_mode", Checkout::Express, fixtures::expiry())
        .governed_by(&halt)
        .declare()
        .unwrap();
    let lenient = builder
        .release_flag("lenient_mode", Checkout::Express, fixtures::expiry())
        .governed_by(&hold)
        .declare()
        .unwrap();
    let base = base(builder.build(), &serving, &[]);
    let published = snapshot(
        &base,
        4,
        [
            flag_row(&strict, Checkout::Classic, 3),
            flag_row(&lenient, Checkout::Classic, 3),
            switch_row(&halt, SwitchState::Released, 3),
            switch_row(&hold, SwitchState::Released, 3),
        ],
    );

    let fresh = base
        .control_plane_view(&published, Freshness::Fresh)
        .unwrap();
    assert_eq!(fresh.evaluate(&strict).value(), Checkout::Classic);

    let stale = base
        .control_plane_view(&published, Freshness::Stale)
        .unwrap();
    let overridden = stale.evaluate(&strict);
    assert_eq!(
        (overridden.value(), overridden.reason()),
        (Checkout::Express, Reason::Disabled),
        "the code default, which is not the first variant"
    );
    let Layer::Override { by } = overridden.layer() else {
        panic!("expected an override layer");
    };
    assert_eq!(by.as_str(), "halt");
    // A governor that holds its last known state does not engage on staleness.
    let held = stale.evaluate(&lenient);
    assert_eq!(
        (held.value(), held.reason()),
        (Checkout::Classic, Reason::Stale)
    );
}
