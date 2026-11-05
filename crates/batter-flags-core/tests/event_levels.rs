//! Event levels: ordinary evaluations are DEBUG, a gate is DEBUG released and
//! INFO engaged, and only an error evaluation is ever WARN, once per handle and
//! cause.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/observed.rs"]
mod observed;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Reason, SwitchState,
    low_level::{GateReading, gate_switch},
};
use capture::Capture;
use observed::{evaluations, only, views};
use tracing::Level;
use tracing_subscriber::filter::LevelFilter;
use world::{base, world};

/// Ordinary evaluations are DEBUG; a gate is DEBUG released and INFO engaged;
/// nothing but an error evaluation is ever WARN.
#[test]
fn levels_follow_the_surface_and_only_an_error_is_warn() {
    let capture = Capture::new();
    let (h, _, views) = views(&capture);
    let level_of = |capture: &Capture| only(capture).level;
    let engaged = &views[3].1;
    let released = &views[0].1;
    engaged.evaluate(&h.fast_path);
    assert_eq!(level_of(&capture), Level::DEBUG);
    capture.clear();
    engaged.switch(&h.checkout_halt);
    assert_eq!(
        level_of(&capture),
        Level::DEBUG,
        "an ordinary switch read, even engaged"
    );
    capture.clear();
    let gate = gate_switch(engaged, &h.checkout_halt);
    assert_eq!(gate.decision().value(), SwitchState::Engaged);
    assert!(matches!(gate, GateReading::Tripped(_)));
    assert_eq!(
        level_of(&capture),
        Level::INFO,
        "a gate evaluation while engaged"
    );
    capture.clear();
    let gate = gate_switch(released, &h.checkout_halt);
    assert_eq!(gate.decision().value(), SwitchState::Released);
    assert!(matches!(gate, GateReading::Open(_)));
    assert_eq!(
        level_of(&capture),
        Level::DEBUG,
        "a gate evaluation while released"
    );
    capture.clear();

    // A pin, a replay and an override are ordinary evaluations.
    let pin = engaged.pin(&h.fast_path);
    assert_eq!(level_of(&capture), Level::DEBUG);
    capture.clear();
    let _ = released.resolve_pinned(&h.fast_path, &pin);
    assert_eq!(level_of(&capture), Level::DEBUG);
    capture.clear();
    let overridden = engaged.evaluate(&h.new_checkout);
    assert_eq!(overridden.reason(), Reason::Disabled);
    assert_eq!(level_of(&capture), Level::DEBUG);

    // Across every view and handle, a non-error evaluation is never WARN.
    capture.clear();
    for (_, view) in &views {
        let _ = view.evaluate(&h.fast_path);
        let _ = view.evaluate(&h.new_checkout);
        let _ = view.switch(&h.checkout_halt);
        let _ = gate_switch(view, &h.export_halt);
    }
    let seen = evaluations(&capture);
    assert_eq!(seen.len(), 7 * 4);
    for event in &seen {
        let is_error = event.field("feature_flag.result.reason") == Some("error");
        assert!(event.level != Level::WARN || is_error, "{event:?}");
    }
    assert!(
        seen.iter().any(|event| event.level == Level::WARN),
        "the sweep reaches errors, so it can tell a WARN from none"
    );
}

#[test]
fn an_error_is_warn_the_first_time_per_handle_and_cause_then_debug_and_always_counted() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = capture.within(|| base.declared_view());
    let levels = |capture: &Capture| {
        let levels: Vec<_> = evaluations(capture)
            .iter()
            .map(|event| event.level)
            .collect();
        capture.clear();
        levels
    };

    // The same handle and cause: WARN, then DEBUG from then on.
    for _ in 0..3 {
        view.evaluate(&other.fast_path);
    }
    assert_eq!(levels(&capture), [Level::WARN, Level::DEBUG, Level::DEBUG]);
    assert_eq!(base.error_count(ErrorCause::ForeignCatalog), 3);

    // Another handle with the same cause is its own first occurrence...
    view.evaluate(&other.checkout_style);
    assert_eq!(levels(&capture), [Level::WARN]);
    // ...and so is the same handle with another cause.
    view.evaluate(&h.worker_mode);
    view.evaluate(&h.worker_mode);
    assert_eq!(levels(&capture), [Level::WARN, Level::DEBUG]);
    assert_eq!(base.error_count(ErrorCause::NotOwned), 2);

    // The gate surface follows the same rule: a switch handle meets a cause once.
    let _ = gate_switch(&view, &h.export_halt);
    let _ = gate_switch(&view, &h.export_halt);
    assert_eq!(levels(&capture), [Level::WARN, Level::DEBUG]);
    assert_eq!(base.error_count(ErrorCause::NotOwned), 4);

    // The tally is the base's: another process starts over.
    let (catalog, again) = world();
    let second = self::base(catalog, &again.serving, &[]);
    let second_view = capture.within(|| second.declared_view());
    second_view.evaluate(&other.fast_path);
    assert_eq!(
        levels(&capture),
        [Level::WARN],
        "a second base, a second first"
    );
    assert_eq!(second.error_count(ErrorCause::ForeignCatalog), 1);
}

#[test]
fn errors_are_counted_even_when_their_events_are_filtered_out() {
    let capture = Capture::at(LevelFilter::ERROR);
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = capture.within(|| base.declared_view());
    for _ in 0..4 {
        view.evaluate(&other.fast_path);
    }
    assert!(capture.events().is_empty());
    assert_eq!(base.error_count(ErrorCause::ForeignCatalog), 4);
}

#[test]
fn at_info_a_released_gate_adds_no_event_and_an_engaged_one_adds_one() {
    let capture = Capture::at(LevelFilter::INFO);
    let (h, _, views) = views(&capture);
    let (declared, engaged) = (&views[0].1, &views[3].1);
    let _ = gate_switch(declared, &h.checkout_halt);
    assert!(capture.events().is_empty(), "released: DEBUG, filtered");
    let _ = gate_switch(engaged, &h.checkout_halt);
    let event = only(&capture);
    assert_eq!(event.level, Level::INFO);
    assert_eq!(event.field("feature_flag.result.variant"), Some("engaged"));
    // An ordinary evaluation adds nothing at INFO, engaged or not.
    capture.clear();
    engaged.evaluate(&h.fast_path);
    engaged.switch(&h.checkout_halt);
    assert!(capture.events().is_empty());
}
