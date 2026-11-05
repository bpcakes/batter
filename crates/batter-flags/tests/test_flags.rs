//! The fixture path: injected values report `Source::Test`, derive reasons like
//! a loaded view and apply the stale rules, and a mistake in a fixture is
//! refused when it is built.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use batter_flags::{
    core::{
        Freshness, Layer, PinRejection, PinnedResolution, Reason, Source, SwitchState, Toggle,
        Version,
    },
    low_level,
    test::TestFlags,
};
use world::{Checkout, world};

const VERSIONS: [u64; 3] = [1, 7, 4_242];
const FRESHNESS: [Freshness; 2] = [Freshness::Fresh, Freshness::Stale];

#[test]
fn every_decision_reports_the_injected_version_and_freshness() {
    for number in VERSIONS {
        for freshness in FRESHNESS {
            let (catalog, h) = world();
            let flags = TestFlags::builder(catalog, h.serving.clone())
                .unwrap()
                .set(&h.fast_path, Toggle::On)
                .release(&h.checkout_halt)
                .version(Version::new(number))
                .freshness(freshness)
                .build()
                .unwrap();
            let view = flags.view();
            let expected = Source::Test {
                version: Version::new(number),
                freshness,
            };
            assert_eq!(view.source(), expected);
            assert_eq!(view.freshness(), freshness);
            // Set, unset, and a switch: every layer reports the view's source.
            assert_eq!(view.evaluate(&h.fast_path).source(), expected);
            assert_eq!(view.evaluate(&h.checkout_style).source(), expected);
            assert_eq!(view.switch(&h.checkout_halt).source(), expected);
        }
    }
}

#[test]
fn the_defaults_are_version_one_and_fresh() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        flags.view().source(),
        Source::Test {
            version: Version::new(1),
            freshness: Freshness::Fresh
        }
    );
}

#[test]
fn an_injected_value_is_an_entry_and_an_unset_flag_serves_its_default() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .set(&h.checkout_style, Checkout::Express)
        .build()
        .unwrap();
    let view = flags.view();
    let on = view.evaluate(&h.fast_path);
    assert_eq!(
        (on.value(), on.reason(), on.layer()),
        (Toggle::On, Reason::Static, &Layer::Entry)
    );
    let express = view.evaluate(&h.checkout_style);
    assert_eq!(express.value(), Checkout::Express);
    let unset = view.evaluate(&h.region_lock);
    assert_eq!(
        (unset.value(), unset.reason(), unset.layer()),
        (Toggle::Off, Reason::Default, &Layer::Base)
    );
}

#[test]
fn a_stale_fixture_reports_its_entries_as_stale() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .freshness(Freshness::Stale)
        .build()
        .unwrap();
    let decision = flags.view().evaluate(&h.fast_path);
    assert_eq!(
        (decision.value(), decision.reason()),
        (Toggle::On, Reason::Stale)
    );
}

#[test]
fn an_injected_stale_engages_an_engage_when_stale_switch_without_a_database() {
    let (catalog, h) = world();
    let stale = TestFlags::builder(catalog, h.worker.clone())
        .unwrap()
        .release(&h.export_halt)
        .release(&h.checkout_halt)
        .freshness(Freshness::Stale)
        .build()
        .unwrap();
    let view = stale.view();
    // `export_halt` engages when stale even though its published state is released.
    let export = view.switch(&h.export_halt);
    assert_eq!(export.value(), SwitchState::Engaged);
    assert_eq!(export.layer(), &Layer::StaleRule);
    assert_eq!(export.reason(), Reason::Stale);
    // `checkout_halt` holds its last known state.
    let checkout = view.switch(&h.checkout_halt);
    assert_eq!(checkout.value(), SwitchState::Released);
    assert_eq!(checkout.layer(), &Layer::Entry);

    // Fresh, the same fixture serves the published states.
    let (catalog, h) = world();
    let fresh = TestFlags::builder(catalog, h.worker.clone())
        .unwrap()
        .release(&h.export_halt)
        .build()
        .unwrap();
    assert_eq!(
        fresh.view().switch(&h.export_halt).value(),
        SwitchState::Released
    );
}

#[test]
fn a_stale_view_never_releases_an_engaged_switch() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .engage(&h.checkout_halt)
        .freshness(Freshness::Stale)
        .build()
        .unwrap();
    assert_eq!(
        flags.view().switch(&h.checkout_halt).value(),
        SwitchState::Engaged
    );
}

#[test]
fn a_floor_engages_whatever_is_published() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .floor(&h.checkout_halt)
        .release(&h.checkout_halt)
        .build()
        .unwrap();
    let decision = flags.view().switch(&h.checkout_halt);
    assert_eq!(decision.value(), SwitchState::Engaged);
    assert_eq!(decision.layer(), &Layer::Floor);
    assert_eq!(decision.reason(), Reason::Static);
}

#[test]
fn an_engaged_governing_switch_overrides_the_flag_it_governs() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.new_checkout, Toggle::On)
        .engage(&h.checkout_halt)
        .build()
        .unwrap();
    let decision = flags.view().evaluate(&h.new_checkout);
    assert_eq!(
        (decision.value(), decision.reason()),
        (Toggle::Off, Reason::Disabled)
    );
    assert!(matches!(decision.layer(), Layer::Override { .. }));
}

#[test]
fn the_flags_a_surface_reads_are_the_same_fixture() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .version(Version::new(9))
        .build()
        .unwrap();
    let from_flags = low_level::capture_now(flags.flags());
    assert_eq!(from_flags.source(), flags.view().source());
    assert_eq!(from_flags.evaluate(&h.fast_path).value(), Toggle::On);
    // Cloned flags read the same fixture.
    let clone = flags.flags().clone();
    assert_eq!(
        low_level::capture_now(&clone).source(),
        flags.view().source()
    );
}

#[test]
fn a_pin_taken_from_test_flags_replays_only_under_a_test_view() {
    let (catalog, h) = world();
    let test = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .build()
        .unwrap();
    let pin = test.view().pin(&h.fast_path);
    // Under a test view it replays.
    let view = test.view();
    assert!(matches!(
        view.resolve_pinned(&h.fast_path, &pin),
        PinnedResolution::Honoured(_)
    ));
    // Under a declared view of a real process it is refused: a test pin never
    // stands in for one a surface served.
    let (catalog, h2) = world();
    let source = world::source_over(&world::required(world::As::ServingHosted), &[]);
    let resolved = batter_flags::declared::resolve(
        catalog,
        &source,
        h2.serving.clone(),
        h2.hosted.clone(),
        world::today(),
    )
    .unwrap();
    let real = resolved.into_flags();
    let real_view = low_level::capture_now(&real);
    let refused = real_view.resolve_pinned(&h2.fast_path, &pin);
    assert_eq!(refused.rejection(), Some(PinRejection::TestProvenance));
}

#[test]
fn fixtures_format_without_values() {
    let (catalog, h) = world();
    let flags = TestFlags::builder(catalog, h.serving.clone())
        .unwrap()
        .set(&h.fast_path, Toggle::On)
        .build()
        .unwrap();
    for text in [
        format!("{flags:?}"),
        format!("{:?}", flags.flags()),
        format!("{:#?}", flags.view()),
    ] {
        assert!(!text.is_empty());
    }
}
