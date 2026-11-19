//! Pins replayed by a process whose settings differ from the one that took
//! them: a variant this process cannot serve is refused, not honoured.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{Freshness, PinRejection, PinnedResolution, SwitchState, Toggle};
use world::{Literal, base, flag_row, snapshot, switch_row, world};

/// A pin taken where an enabling variant's settings were present meets a
/// process whose base records them absent: a row selecting that variant is
/// held there, so the pin is refused too, before any governing switch.
#[test]
fn a_pin_whose_variant_needs_settings_the_replaying_process_lacks_is_refused() {
    let (catalog, h) = world();
    let (lacking_catalog, lacking) = world();
    let taking = base(catalog, &h.serving, &[]);
    let published = snapshot(&taking, 4, [flag_row(&h.new_checkout, Toggle::On, 4)]);
    let taken = taking
        .control_plane_view(&published, Freshness::Fresh)
        .unwrap();
    let pin = taken.pin(&h.new_checkout).unwrap();
    assert_eq!(pin.variant(), Some("on"));

    let replaying = base(
        lacking_catalog,
        &lacking.serving,
        &[Literal::SettingsAbsent("new_checkout", "on")],
    );
    let engaged = snapshot(
        &replaying,
        6,
        [switch_row(&lacking.checkout_halt, SwitchState::Engaged, 6)],
    );
    for (label, view) in [
        ("declared", replaying.declared_view()),
        (
            "engaged governor",
            replaying
                .control_plane_view(&engaged, Freshness::Fresh)
                .unwrap(),
        ),
    ] {
        assert_eq!(
            view.resolve_pinned(&lacking.new_checkout, &pin).rejection(),
            Some(PinRejection::RequiresSettingsAbsent),
            "{label}"
        );
    }
    // The variant that needs nothing still replays on the same process.
    let off = taking.declared_view().pin(&h.new_checkout).unwrap();
    assert!(matches!(
        replaying
            .declared_view()
            .resolve_pinned(&lacking.new_checkout, &off),
        PinnedResolution::Honoured(_)
    ));
}
