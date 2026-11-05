//! Pins: what a pin records, and the outcome table of replaying one.
//!
//! The arms that need a stored envelope this build did not write (an unknown
//! envelope version, a malformed body, another kind or value type, a newer
//! variant) are in `pins_serde.rs`.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    ErrorCause, Freshness, Layer, PinOrigin, PinRejection, PinnedResolution, PrePublication,
    Reason, Source, SwitchState, Toggle,
};
use fixtures::Checkout;
use world::{Literal, base, file_snapshot, flag_row, snapshot, switch_row, version, world};

#[test]
fn a_pin_records_the_decision_and_the_origin_of_the_view_it_was_taken_from() {
    let (catalog, h) = world();
    let base = base(
        catalog,
        &h.serving,
        &[Literal::Flag("checkout_style", "express")],
    );
    let published = snapshot(
        &base,
        7,
        [flag_row(&h.checkout_style, Checkout::Express, 6)],
    );
    let restored = file_snapshot(
        &base,
        7,
        [flag_row(&h.checkout_style, Checkout::Express, 6)],
    );
    let revision = base.revision();

    let cases = [
        (
            "declared",
            base.declared_view(),
            PinOrigin::Declared { revision },
            "declared",
        ),
        (
            "not loaded",
            base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh),
            PinOrigin::NotLoaded { revision },
            "not_loaded",
        ),
        (
            "control plane",
            base.control_plane_view(&published, Freshness::Fresh)
                .unwrap(),
            PinOrigin::ControlPlane {
                version: version(7),
            },
            "control_plane",
        ),
        (
            "persisted file",
            base.persisted_file_view(&restored, Freshness::Stale)
                .unwrap(),
            PinOrigin::PersistedFile {
                version: version(7),
            },
            "persisted_file",
        ),
        (
            "test",
            base.test_view(&published, Freshness::Fresh).unwrap(),
            PinOrigin::Test {
                version: version(7),
            },
            "test",
        ),
    ];
    for (label, view, origin, class) in cases {
        let pin = view.pin(&h.checkout_style);
        assert_eq!(pin.key(), Some("checkout_style"), "{label}");
        assert_eq!(pin.variant(), Some("express"), "{label}");
        assert_eq!(pin.origin(), Some(origin), "{label}");
        assert_eq!(origin.class(), class, "{label}");
    }
}

#[test]
fn an_honoured_pin_is_cached_and_keeps_its_origin_as_data() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let early = snapshot(
        &base,
        7,
        [flag_row(&h.checkout_style, Checkout::Express, 6)],
    );
    let late = snapshot(
        &base,
        9,
        [flag_row(&h.checkout_style, Checkout::Classic, 8)],
    );
    let taken = base.control_plane_view(&early, Freshness::Fresh).unwrap();
    let replaying = base.control_plane_view(&late, Freshness::Stale).unwrap();
    let pin = taken.pin(&h.checkout_style);

    let PinnedResolution::Honoured(decision) = replaying.resolve_pinned(&h.checkout_style, &pin)
    else {
        panic!("a compatible pin is honoured");
    };
    assert_eq!(decision.value(), Checkout::Express, "not the live value");
    assert_eq!(decision.variant(), "express");
    assert_eq!(decision.reason(), Reason::Cached);
    assert_eq!(decision.layer(), &Layer::Entry);
    assert_eq!(decision.error(), None);
    assert_eq!(
        decision.source(),
        Source::Pinned {
            origin: PinOrigin::ControlPlane {
                version: version(7)
            }
        },
        "the origin is what was stored, not what the replaying view is"
    );
    // The same pin replays under any view of the catalog, whatever its origin.
    let declared = base.declared_view();
    assert!(matches!(
        declared.resolve_pinned(&h.checkout_style, &pin),
        PinnedResolution::Honoured(_)
    ));
}

#[test]
fn a_governing_switch_engaged_where_the_pin_replays_overrides_an_honoured_pin() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let released = snapshot(
        &base,
        5,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Released, 5),
        ],
    );
    let engaged = snapshot(
        &base,
        6,
        [
            flag_row(&h.new_checkout, Toggle::On, 4),
            switch_row(&h.checkout_halt, SwitchState::Engaged, 6),
        ],
    );
    let pin = base
        .control_plane_view(&released, Freshness::Fresh)
        .unwrap()
        .pin(&h.new_checkout);
    assert_eq!(pin.variant(), Some("on"));

    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    let PinnedResolution::Overridden { decision, by } = view.resolve_pinned(&h.new_checkout, &pin)
    else {
        panic!("an engaged governing switch overrides the pin");
    };
    assert_eq!(by.as_str(), "checkout_halt");
    assert_eq!(decision.value(), Toggle::Off, "the code default");
    assert_eq!(decision.reason(), Reason::Disabled);
    assert_eq!(decision.layer(), &Layer::Override { by });
    assert_eq!(
        decision.source(),
        view.source(),
        "the view's, not the pin's"
    );

    // A flag no switch governs is never overridden.
    let ungoverned = view.pin(&h.fast_path);
    assert!(matches!(
        view.resolve_pinned(&h.fast_path, &ungoverned),
        PinnedResolution::Honoured(_)
    ));
}

#[test]
fn a_pin_taken_while_the_governing_switch_is_engaged_records_the_code_default() {
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
    let pin = view.pin(&h.new_checkout);
    assert_eq!(
        pin.variant(),
        Some("off"),
        "the effective decision is what is pinned"
    );
}

#[test]
fn a_pin_taken_for_a_handle_the_view_cannot_answer_for_records_its_code_default() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = base.declared_view();
    let pin = view.pin(&other.fast_path);
    assert_eq!(pin.variant(), Some("off"));
    assert_eq!(base.error_count(ErrorCause::ForeignCatalog), 1);
}

/// Each arm a pin the library itself took can reach.
#[test]
fn the_arms_reachable_from_a_pin_the_library_took() {
    let (catalog, h) = world();
    let (other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let other_base = self::base(other_catalog, &other.worker, &[]);
    let view = base.declared_view();

    // The handle is checked first: another catalog's handle.
    let pin = view.pin(&h.fast_path);
    assert_eq!(
        view.resolve_pinned(&other.fast_path, &pin).rejection(),
        Some(PinRejection::ForeignHandle)
    );
    // A kind that neither owns the flag nor may resolve its pins.
    let worker_pin = other_base.declared_view().pin(&other.worker_mode);
    assert_eq!(
        view.resolve_pinned(&h.worker_mode, &worker_pin).rejection(),
        Some(PinRejection::NotOwned)
    );
    // A pin of another flag of the same value type, such as a row read for the
    // wrong flag.
    assert_eq!(
        view.resolve_pinned(&h.new_checkout, &pin).rejection(),
        Some(PinRejection::KeyMismatch)
    );
    // A test pin outside a test view, and inside one.
    let published = snapshot(&base, 3, []);
    let restored = file_snapshot(&base, 3, []);
    let test = base.test_view(&published, Freshness::Fresh).unwrap();
    let test_pin = test.pin(&h.fast_path);
    assert_eq!(
        test_pin.origin(),
        Some(PinOrigin::Test {
            version: version(3)
        })
    );
    for (label, view) in [
        ("declared", base.declared_view()),
        (
            "control plane",
            base.control_plane_view(&published, Freshness::Fresh)
                .unwrap(),
        ),
        (
            "persisted file",
            base.persisted_file_view(&restored, Freshness::Fresh)
                .unwrap(),
        ),
        (
            "not loaded",
            base.not_loaded_view(PrePublication::ServeDefaults, Freshness::Fresh),
        ),
    ] {
        assert_eq!(
            view.resolve_pinned(&h.fast_path, &test_pin).rejection(),
            Some(PinRejection::TestProvenance),
            "{label}"
        );
    }
    assert!(matches!(
        test.resolve_pinned(&h.fast_path, &test_pin),
        PinnedResolution::Honoured(_)
    ));
}

#[test]
fn incompatible_is_decided_before_the_governing_switch() {
    let (catalog, h) = world();
    let (other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let other_base = self::base(other_catalog, &other.serving, &[]);
    let engaged = snapshot(
        &base,
        6,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 6)],
    );
    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    // `new_checkout` is governed by an engaged switch, yet each mismatched pin
    // is reported as what is wrong with it.
    let pin = view.pin(&h.fast_path);
    assert_eq!(
        view.resolve_pinned(&h.new_checkout, &pin).rejection(),
        Some(PinRejection::KeyMismatch)
    );
    let foreign = other_base.declared_view().pin(&other.new_checkout);
    assert!(
        matches!(
            view.resolve_pinned(&h.new_checkout, &foreign),
            PinnedResolution::Overridden { .. }
        ),
        "a compatible pin from another process is overridden, never rejected"
    );
    assert_eq!(
        view.resolve_pinned(&other.new_checkout, &foreign)
            .rejection(),
        Some(PinRejection::ForeignHandle),
        "the handle is checked before the pin"
    );
}

#[test]
fn a_pinned_by_kind_replays_a_pin_but_never_evaluates_the_flag_live() {
    let (catalog, h) = world();
    let (worker_catalog, worker) = world();
    let serving_base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let worker_base = base(worker_catalog, &worker.worker, &[]);
    let pin = serving_base.declared_view().pin(&h.new_checkout);
    assert_eq!(pin.variant(), Some("on"));

    let view = worker_base.declared_view();
    let PinnedResolution::Honoured(decision) = view.resolve_pinned(&worker.new_checkout, &pin)
    else {
        panic!("a pinned_by kind replays the pin");
    };
    assert_eq!(decision.value(), Toggle::On);
    let live = view.evaluate(&worker.new_checkout);
    assert_eq!(live.error(), Some(ErrorCause::NotOwned));
    assert_eq!(live.value(), Toggle::Off);
}

#[test]
fn pins_compare_by_decision_and_by_envelope() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("fast_path", "on")]);
    let first = snapshot(&base, 5, [flag_row(&h.fast_path, Toggle::On, 4)]);
    let second = snapshot(&base, 6, [flag_row(&h.fast_path, Toggle::On, 4)]);
    let off = snapshot(&base, 7, [flag_row(&h.fast_path, Toggle::Off, 7)]);
    let pin = |snapshot| {
        base.control_plane_view(snapshot, Freshness::Fresh)
            .unwrap()
            .pin(&h.fast_path)
    };
    let (a, b, c) = (pin(&first), pin(&second), pin(&off));
    assert!(
        a.same_decision(&b),
        "the same key and variant, whatever the origin"
    );
    assert_ne!(a, b, "the whole envelope is part of a keyed request");
    assert_eq!(a, pin(&first));
    assert!(!a.same_decision(&c));
}

#[test]
fn every_arm_maps_to_its_cause_and_semantic_convention_error_type() {
    let arms = [
        (
            PinRejection::ForeignHandle,
            ErrorCause::ForeignCatalog,
            "flag_not_found",
        ),
        (
            PinRejection::NotOwned,
            ErrorCause::NotOwned,
            "flag_not_found",
        ),
        (
            PinRejection::UnsupportedVersion { v: 2 },
            ErrorCause::PinUnsupportedVersion,
            "parse_error",
        ),
        (
            PinRejection::Malformed,
            ErrorCause::PinMalformed,
            "parse_error",
        ),
        (
            PinRejection::KeyMismatch,
            ErrorCause::PinKeyMismatch,
            "type_mismatch",
        ),
        (
            PinRejection::KindChanged,
            ErrorCause::PinKindChanged,
            "type_mismatch",
        ),
        (
            PinRejection::ValueTypeChanged,
            ErrorCause::PinValueTypeChanged,
            "type_mismatch",
        ),
        (
            PinRejection::UnknownVariant,
            ErrorCause::PinUnknownVariant,
            "parse_error",
        ),
        (
            PinRejection::TestProvenance,
            ErrorCause::PinTestProvenance,
            "general",
        ),
    ];
    for (arm, cause, error_type) in arms {
        assert_eq!(arm.cause(), cause, "{arm:?}");
        assert_eq!(arm.error_type(), error_type, "{arm:?}");
        assert_eq!(cause.error_type(), error_type, "{arm:?}");
        // Every message is fixed text: no arm carries anything from the pin.
        assert!(!arm.to_string().is_empty());
    }
    // An exhaustive match fails to compile when an arm is added without a row above.
    for (arm, _, _) in arms {
        match arm {
            PinRejection::ForeignHandle
            | PinRejection::NotOwned
            | PinRejection::UnsupportedVersion { .. }
            | PinRejection::Malformed
            | PinRejection::KeyMismatch
            | PinRejection::KindChanged
            | PinRejection::ValueTypeChanged
            | PinRejection::UnknownVariant
            | PinRejection::TestProvenance => {}
        }
    }
}

#[test]
fn the_closed_error_vocabulary_has_one_type_per_cause() {
    let types: Vec<_> = ErrorCause::ALL
        .iter()
        .map(|cause| cause.error_type())
        .collect();
    assert_eq!(
        types,
        [
            "flag_not_found",
            "flag_not_found",
            "provider_not_ready",
            "parse_error",
            "parse_error",
            "type_mismatch",
            "type_mismatch",
            "type_mismatch",
            "parse_error",
            "general",
        ]
    );
    let handle_causes: Vec<_> = ErrorCause::ALL
        .iter()
        .filter_map(|cause| cause.handle_cause())
        .collect();
    assert_eq!(handle_causes, ["foreign_catalog", "not_owned"]);
}
