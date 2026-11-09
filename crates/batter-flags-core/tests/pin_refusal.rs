//! A refused pin: reading the flag live in its place is possible only through
//! the refusal the replay returned, on that view and for that flag.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Freshness, Layer, PinRejection, PinnedResolution, Reason, SwitchState, Toggle,
};
use capture::Capture;
use world::{Literal, base, flag_row, snapshot, switch_row, world};

/// The one way to get a refusal here: replay a pin of another flag of the same
/// type, such as a row read for the wrong flag.
#[test]
fn a_refused_pin_reads_live_only_through_its_refusal_with_an_event_of_its_own() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let published = snapshot(&base, 7, [flag_row(&h.new_checkout, Toggle::On, 6)]);
    let view = capture.within(|| {
        base.control_plane_view(&published, Freshness::Fresh)
            .unwrap()
    });
    let wrong_row = view.pin(&h.fast_path).unwrap();
    capture.clear();

    let PinnedResolution::Incompatible(refusal) = view.resolve_pinned(&h.new_checkout, &wrong_row)
    else {
        panic!("the pin names another flag");
    };
    assert_eq!(refusal.rejection(), PinRejection::KeyMismatch);
    let refused = capture.events();
    assert_eq!(refused.len(), 1, "the refusal emitted its own event");
    assert_eq!(refused[0].field("feature_flag.result.variant"), None);
    assert_eq!(refused[0].field("error.type"), Some("type_mismatch"));

    // Reading live is a separate evaluation of the same flag on the same view.
    capture.clear();
    let live = refusal.fall_back_to_live();
    assert_eq!(live.value(), Toggle::On);
    assert_eq!(
        (live.reason(), live.layer()),
        (Reason::Static, &Layer::Entry)
    );
    assert_eq!(live.source(), view.source());
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].field("feature_flag.key"), Some("new_checkout"));
    assert_eq!(events[0].field("feature_flag.result.variant"), Some("on"));
    assert_eq!(events[0].field("error.type"), None);
}

#[test]
fn the_fallback_reads_the_view_that_refused_the_pin() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let early = snapshot(&base, 7, [flag_row(&h.new_checkout, Toggle::On, 6)]);
    let late = snapshot(&base, 9, [flag_row(&h.new_checkout, Toggle::Off, 8)]);
    let refusing = base.control_plane_view(&early, Freshness::Fresh).unwrap();
    let other = base.control_plane_view(&late, Freshness::Fresh).unwrap();

    let wrong_row = refusing.pin(&h.fast_path).unwrap();
    let PinnedResolution::Incompatible(refusal) =
        refusing.resolve_pinned(&h.new_checkout, &wrong_row)
    else {
        panic!("another flag's pin is refused");
    };
    // Whatever other views exist, the refusal reads through the one that refused.
    let live = refusal.fall_back_to_live();
    assert_eq!(live.value(), Toggle::On);
    assert_eq!(live.source(), refusing.source());
    assert_ne!(live.source(), other.source());
    assert_eq!(other.evaluate(&h.new_checkout).value(), Toggle::Off);
}

#[test]
fn a_governed_flag_falls_back_to_its_override_not_around_it() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let engaged = snapshot(
        &base,
        6,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 6),
        ],
    );
    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    let wrong_row = view.pin(&h.fast_path).unwrap();
    let PinnedResolution::Incompatible(refusal) = view.resolve_pinned(&h.new_checkout, &wrong_row)
    else {
        panic!("another flag's pin is refused, even where the governor is engaged");
    };
    let live = refusal.fall_back_to_live();
    assert_eq!(live.value(), Toggle::Off);
    assert_eq!(live.reason(), Reason::Disabled);
    assert!(matches!(live.layer(), Layer::Override { .. }));
}

#[test]
fn a_kind_that_only_resolves_pins_cannot_read_the_flag_live_even_through_a_refusal() {
    let (catalog, h) = world();
    let worker = base(catalog, &h.worker, &[]);
    let view = worker.declared_view();
    // `new_checkout` is owned by `serving` and pinned by `worker`.
    let wrong_row = view.pin(&h.fast_path).unwrap();
    let PinnedResolution::Incompatible(refusal) = view.resolve_pinned(&h.new_checkout, &wrong_row)
    else {
        panic!("another flag's pin is refused");
    };
    assert_eq!(refusal.rejection(), PinRejection::KeyMismatch);
    let live = refusal.fall_back_to_live();
    assert_eq!(live.error(), Some(ErrorCause::NotOwned));
    assert_eq!(live.value(), Toggle::Off);
}

#[test]
fn only_a_refusal_carries_the_fallback() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = base.declared_view();
    let pin = view.pin(&h.fast_path).unwrap();
    let honoured = view.resolve_pinned(&h.fast_path, &pin);
    assert_eq!(honoured.rejection(), None);
    // An honoured pin has no refusal to give; the match has nowhere to get one.
    match honoured {
        PinnedResolution::Honoured(decision) => assert_eq!(decision.reason(), Reason::Cached),
        PinnedResolution::Incompatible(_) | PinnedResolution::Overridden { .. } => {
            panic!("a compatible pin is honoured")
        }
    }
    let debug = format!("{:?}", view.resolve_pinned(&h.new_checkout, &pin));
    assert!(
        debug.contains("PinRefusal") && debug.contains("KeyMismatch"),
        "{debug}"
    );
}
