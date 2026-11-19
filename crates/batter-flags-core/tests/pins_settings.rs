//! Pins replayed by a process whose settings or environment class differ from
//! the one that took them: a variant this process cannot serve is refused, not
//! honoured, whether it owns the flag or only replays its pins.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    Freshness, PinRejection, PinnedResolution, SwitchState, Toggle,
    low_level::{BaseBuilder, BaseError},
};
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

/// A `pinned_by` kind never evaluates the flag, but it serves the pins it
/// replays, so the settings it lacks are recorded and refuse them too.
#[test]
fn a_pin_only_kind_lacking_a_variants_settings_refuses_its_pin() {
    let (catalog, h) = world();
    let (lacking_catalog, lacking) = world();
    let taking = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let pin = taking.declared_view().pin(&h.new_checkout).unwrap();
    let worker = base(
        lacking_catalog,
        &lacking.worker,
        &[Literal::SettingsAbsent("new_checkout", "on")],
    );
    assert_eq!(
        worker
            .declared_view()
            .resolve_pinned(&lacking.new_checkout, &pin)
            .rejection(),
        Some(PinRejection::RequiresSettingsAbsent)
    );
    // The variant that needs nothing still replays on the pin-only kind.
    let (off_catalog, o) = world();
    let off = base(off_catalog, &o.serving, &[])
        .declared_view()
        .pin(&o.new_checkout)
        .unwrap();
    assert!(matches!(
        worker
            .declared_view()
            .resolve_pinned(&lacking.new_checkout, &off),
        PinnedResolution::Honoured(_)
    ));
}

/// An `only_in` flag evaluates at its code default outside its class, so a
/// pin taken inside the class for another variant is refused there, and one
/// for the default still replays.
#[test]
fn a_pin_of_a_restricted_flag_is_refused_outside_its_class() {
    let (catalog, h) = world();
    let (hosted_catalog, hosted) = world();
    let development = base(catalog, &h.serving, &[Literal::Flag("debug_panel", "on")]);
    let on = development.declared_view().pin(&h.debug_panel).unwrap();
    let outside = base(
        hosted_catalog,
        &hosted.serving,
        &[Literal::OutsideClass("debug_panel")],
    );
    assert_eq!(
        outside
            .declared_view()
            .resolve_pinned(&hosted.debug_panel, &on)
            .rejection(),
        Some(PinRejection::OutsideClass)
    );
    let (default_catalog, d) = world();
    let off = base(default_catalog, &d.serving, &[])
        .declared_view()
        .pin(&d.debug_panel)
        .unwrap();
    assert_eq!(off.variant(), Some("off"));
    assert!(matches!(
        outside
            .declared_view()
            .resolve_pinned(&hosted.debug_panel, &off),
        PinnedResolution::Honoured(_)
    ));
}

/// Both records name a flag the kind owns or replays pins of; an
/// unrestricted flag has no class to run outside.
#[test]
fn the_replay_records_refuse_kinds_and_flags_they_cannot_describe() {
    let (catalog, h) = world();
    let mut builder = BaseBuilder::new(catalog, h.worker.clone()).unwrap();
    let key = |name: &str| builder.catalog().declaration(name).unwrap().key().clone();
    let (fast_path, debug_panel, worker_mode) =
        (key("fast_path"), key("debug_panel"), key("worker_mode"));
    let (catalog, s) = world();
    let mut serving = BaseBuilder::new(catalog, s.serving.clone()).unwrap();
    assert_eq!(
        builder.outside_class(&fast_path),
        Err(BaseError::NotRestricted)
    );
    assert_eq!(builder.outside_class(&debug_panel), Ok(()));
    assert_eq!(builder.outside_class(&debug_panel), Ok(()));
    assert_eq!(
        serving.outside_class(&worker_mode),
        Err(BaseError::NotOwned)
    );
}
