//! The stored form of a pin: spellings, decoding that never fails on what a
//! newer edge wrote, and the replay arms that need a stored envelope.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![cfg(feature = "serde")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{
    Freshness, PinOrigin, PinRejection, Pinned, PinnedResolution, Revision, SwitchState, Toggle,
};
use fixtures::Checkout;
use serde_json::{Value, json};
use world::{Literal, base, flag_row, snapshot, switch_row, version, world};

const EMPTY_REVISION: &str = "95f804b8ecdfc355842bf949b70925985beb1110bd0c3ef6de4a716181804412";

fn revision() -> Revision {
    EMPTY_REVISION.parse().unwrap()
}

fn envelope(origin: Value) -> Value {
    json!({
        "v": 1,
        "key": "checkout_style",
        "kind": "mode",
        "value_type": "enum",
        "variant": "express",
        "origin": origin,
    })
}

fn control_plane() -> Value {
    json!({"class": "control_plane", "version": 7})
}

fn decode(value: Value) -> Pinned<Checkout> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn every_origin_class_is_stored_in_its_own_spelling_and_round_trips() {
    let cases = [
        (
            PinOrigin::Declared {
                revision: revision(),
            },
            format!(r#"{{"class":"declared","revision":"{EMPTY_REVISION}"}}"#),
        ),
        (
            PinOrigin::NotLoaded {
                revision: revision(),
            },
            format!(r#"{{"class":"not_loaded","revision":"{EMPTY_REVISION}"}}"#),
        ),
        (
            PinOrigin::ControlPlane {
                version: version(7),
            },
            r#"{"class":"control_plane","version":7}"#.to_owned(),
        ),
        (
            PinOrigin::PersistedFile {
                version: version(7),
            },
            r#"{"class":"persisted_file","version":7}"#.to_owned(),
        ),
        (
            PinOrigin::Test {
                version: version(7),
            },
            r#"{"class":"test","version":7}"#.to_owned(),
        ),
    ];
    for (origin, text) in cases {
        assert_eq!(serde_json::to_string(&origin).unwrap(), text);
        let pin = decode(envelope(serde_json::from_str(&text).unwrap()));
        assert_eq!(pin.origin(), Some(origin), "{text}");
        // And the whole envelope, byte for byte.
        let stored = serde_json::to_string(&pin).unwrap();
        assert_eq!(
            stored,
            format!(
                r#"{{"v":1,"key":"checkout_style","kind":"mode","value_type":"enum","variant":"express","origin":{text}}}"#
            )
        );
        let again: Pinned<Checkout> = serde_json::from_str(&stored).unwrap();
        assert_eq!(again, pin);
    }
}

#[test]
fn a_pin_a_view_took_stores_and_replays_unchanged() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let published = snapshot(
        &base,
        7,
        [flag_row(&h.checkout_style, Checkout::Express, 6)],
    );
    let view = base
        .control_plane_view(&published, Freshness::Fresh)
        .unwrap();
    let pin = view.pin(&h.checkout_style);
    let stored = serde_json::to_value(&pin).unwrap();
    assert_eq!(stored, envelope(control_plane()));
    let replayed: Pinned<Checkout> = serde_json::from_value(stored).unwrap();
    let PinnedResolution::Honoured(decision) = view.resolve_pinned(&h.checkout_style, &replayed)
    else {
        panic!("a pin the view took is honoured by it");
    };
    assert_eq!(decision.value(), Checkout::Express);
}

#[test]
fn unknown_fields_inside_envelope_version_one_are_ignored() {
    let mut value = envelope(json!({"class": "control_plane", "version": 7, "note": "x"}));
    value["weight"] = json!(10);
    let pin = decode(value);
    assert_eq!(pin.key(), Some("checkout_style"));
    assert_eq!(
        pin.origin(),
        Some(PinOrigin::ControlPlane {
            version: version(7)
        })
    );
}

#[test]
fn an_origin_whose_fields_disagree_with_its_class_is_malformed() {
    let origins = [
        json!({"class": "declared", "version": 7}),
        json!({"class": "declared", "revision": EMPTY_REVISION, "version": 7}),
        json!({"class": "declared"}),
        json!({"class": "not_loaded", "version": 7}),
        json!({"class": "control_plane", "revision": EMPTY_REVISION}),
        json!({"class": "control_plane", "version": 7, "revision": EMPTY_REVISION}),
        json!({"class": "control_plane"}),
        json!({"class": "persisted_file", "revision": EMPTY_REVISION}),
        json!({"class": "test", "revision": EMPTY_REVISION}),
        json!({"class": "test"}),
        json!({"class": "unknown", "version": 7}),
        json!({"class": 7, "version": 7}),
        json!({"version": 7}),
        json!({"class": "control_plane", "version": "7"}),
        json!({"class": "control_plane", "version": -1}),
        json!({"class": "control_plane", "version": 1.5}),
        json!({"class": "declared", "revision": EMPTY_REVISION.to_uppercase()}),
        json!({"class": "declared", "revision": &EMPTY_REVISION[1..]}),
        json!({"class": "declared", "revision": 7}),
        json!("control_plane"),
        json!(null),
        json!([]),
    ];
    for origin in origins {
        let pin = decode(envelope(origin.clone()));
        assert_eq!(pin.key(), None, "{origin}");
        assert_eq!(pin.variant(), None, "{origin}");
        assert_eq!(pin.origin(), None, "{origin}");
        let (catalog, h) = world();
        let base = base(catalog, &h.serving, &[]);
        assert_eq!(
            base.declared_view()
                .resolve_pinned(&h.checkout_style, &pin)
                .rejection(),
            Some(PinRejection::Malformed),
            "{origin}"
        );
    }
}

#[test]
fn a_body_missing_or_mistyping_a_field_is_malformed() {
    for field in ["key", "kind", "value_type", "variant", "origin"] {
        let mut missing = envelope(control_plane());
        missing.as_object_mut().unwrap().remove(field);
        let mut mistyped = envelope(control_plane());
        mistyped[field] = json!(7);
        for value in [missing, mistyped] {
            let (catalog, h) = world();
            let base = base(catalog, &h.serving, &[]);
            let pin = decode(value);
            assert_eq!(
                base.declared_view()
                    .resolve_pinned(&h.checkout_style, &pin)
                    .rejection(),
                Some(PinRejection::Malformed),
                "{field}"
            );
        }
    }
}

#[test]
fn an_unknown_envelope_version_is_kept_verbatim_and_refused_at_replay() {
    for v in [0_u64, 2, 99, u64::MAX] {
        let mut value = envelope(control_plane());
        value["v"] = json!(v);
        value["something_new"] = json!({"nested": [1, 2, 3]});
        let pin = decode(value.clone());
        assert_eq!((pin.key(), pin.variant(), pin.origin()), (None, None, None));
        assert_eq!(
            serde_json::to_value(&pin).unwrap(),
            value,
            "storing it again destroys nothing a newer edge wrote"
        );
        assert!(
            !pin.same_decision(&pin),
            "an unreadable pin records no decision"
        );
        let (catalog, h) = world();
        let base = base(catalog, &h.serving, &[]);
        assert_eq!(
            base.declared_view()
                .resolve_pinned(&h.checkout_style, &pin)
                .rejection(),
            Some(PinRejection::UnsupportedVersion { v })
        );
    }
}

#[test]
fn a_value_that_is_not_a_pin_envelope_is_a_decode_error() {
    let rejected = [
        json!(null),
        json!("pin"),
        json!(7),
        json!([1]),
        json!({}),
        json!({"key": "checkout_style"}),
        json!({"v": "1"}),
        json!({"v": 1.0}),
        json!({"v": -1}),
        json!({"v": null}),
    ];
    for value in rejected {
        assert!(
            serde_json::from_value::<Pinned<Checkout>>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn a_variant_from_a_newer_edge_is_a_typed_outcome_never_a_decode_error() {
    for variant in ["instant", "Express", "", "express ", "e\u{0}"] {
        let mut value = envelope(control_plane());
        value["variant"] = json!(variant);
        let pin = decode(value);
        assert_eq!(pin.variant(), Some(variant), "the text stays as stored");
        let (catalog, h) = world();
        let base = base(catalog, &h.serving, &[]);
        assert_eq!(
            base.declared_view()
                .resolve_pinned(&h.checkout_style, &pin)
                .rejection(),
            Some(PinRejection::UnknownVariant),
            "{variant:?}"
        );
    }
}

#[test]
fn a_stored_pin_is_checked_against_the_running_declaration_in_order() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = base.declared_view();
    let with = |key: &str, kind: &str, value_type: &str, variant: &str| {
        decode(json!({
            "v": 1, "key": key, "kind": kind, "value_type": value_type,
            "variant": variant, "origin": control_plane(),
        }))
    };
    let replay = |pin: &Pinned<Checkout>| view.resolve_pinned(&h.checkout_style, pin);
    let arm = Some;

    // Each field alone.
    assert_eq!(
        replay(&with("fast_path", "mode", "enum", "express")).rejection(),
        arm(PinRejection::KeyMismatch)
    );
    assert_eq!(
        replay(&with("checkout_style", "release", "enum", "express")).rejection(),
        arm(PinRejection::KindChanged)
    );
    assert_eq!(
        replay(&with("checkout_style", "mode", "toggle", "express")).rejection(),
        arm(PinRejection::ValueTypeChanged)
    );
    assert_eq!(
        replay(&with("checkout_style", "mode", "enum", "turbo")).rejection(),
        arm(PinRejection::UnknownVariant)
    );
    // The first wrong field is the one reported.
    assert_eq!(
        replay(&with("other", "release", "toggle", "turbo")).rejection(),
        arm(PinRejection::KeyMismatch)
    );
    assert_eq!(
        replay(&with("checkout_style", "release", "toggle", "turbo")).rejection(),
        arm(PinRejection::KindChanged)
    );
    assert_eq!(
        replay(&with("checkout_style", "mode", "toggle", "turbo")).rejection(),
        arm(PinRejection::ValueTypeChanged)
    );
    assert!(matches!(
        replay(&with("checkout_style", "mode", "enum", "classic")),
        PinnedResolution::Honoured(_)
    ));
}

#[test]
fn the_handle_is_checked_before_a_stored_pin_is_read() {
    let (catalog, h) = world();
    let (_other_catalog, other) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = base.declared_view();
    assert_eq!(
        view.resolve_pinned(&other.checkout_style, &decode(json!({"v": 2})))
            .rejection(),
        Some(PinRejection::ForeignHandle)
    );
    // `worker_mode` belongs to `worker` alone, so a serving process never reads
    // the pin.
    assert_eq!(
        view.resolve_pinned(&h.worker_mode, &decode_toggle(json!({"v": 2})))
            .rejection(),
        Some(PinRejection::NotOwned)
    );
}

#[test]
fn a_stored_pin_that_is_incompatible_is_never_overridden() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[Literal::Flag("new_checkout", "on")]);
    let engaged = snapshot(
        &base,
        6,
        [switch_row(&h.checkout_halt, SwitchState::Engaged, 6)],
    );
    let view = base.control_plane_view(&engaged, Freshness::Fresh).unwrap();
    let pins = [
        (
            decode_toggle(json!({"v": 2})),
            PinRejection::UnsupportedVersion { v: 2 },
        ),
        (
            decode_toggle(json!({"v": 1, "key": "new_checkout"})),
            PinRejection::Malformed,
        ),
        (
            decode_toggle(toggle_envelope("new_checkout", "turbo")),
            PinRejection::UnknownVariant,
        ),
        (
            decode_toggle(toggle_envelope("fast_path", "on")),
            PinRejection::KeyMismatch,
        ),
    ];
    for (pin, rejection) in pins {
        assert_eq!(
            view.resolve_pinned(&h.new_checkout, &pin).rejection(),
            Some(rejection)
        );
    }
    let compatible = decode_toggle(toggle_envelope("new_checkout", "on"));
    assert!(matches!(
        view.resolve_pinned(&h.new_checkout, &compatible),
        PinnedResolution::Overridden { .. }
    ));
}

fn toggle_envelope(key: &str, variant: &str) -> Value {
    json!({
        "v": 1, "key": key, "kind": "release", "value_type": "toggle",
        "variant": variant, "origin": control_plane(),
    })
}

fn decode_toggle(value: Value) -> Pinned<Toggle> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_test_origin_pin_replays_only_under_a_test_view() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let published = snapshot(&base, 3, []);
    let pin = decode_toggle(json!({
        "v": 1, "key": "fast_path", "kind": "release", "value_type": "toggle",
        "variant": "on", "origin": {"class": "test", "version": 99},
    }));
    let test = base.test_view(&published, Freshness::Fresh).unwrap();
    assert!(matches!(
        test.resolve_pinned(&h.fast_path, &pin),
        PinnedResolution::Honoured(_)
    ));
    let live = base
        .control_plane_view(&published, Freshness::Fresh)
        .unwrap();
    assert_eq!(
        live.resolve_pinned(&h.fast_path, &pin).rejection(),
        Some(PinRejection::TestProvenance)
    );
}

#[test]
fn debug_output_shows_names_never_stored_text() {
    let secret = decode(json!({
        "v": 1, "key": "SECRET key", "kind": "mode", "value_type": "enum",
        "variant": "SECRET variant", "origin": control_plane(),
    }));
    let unusable = decode(json!({"v": 7, "key": "SECRET key", "payload": "SECRET"}));
    let malformed = decode(json!({"v": 1, "key": "SECRET key", "origin": "SECRET"}));
    for pin in [secret, unusable, malformed] {
        let text = format!("{pin:?} {pin:#?}");
        assert!(!text.contains("SECRET"), "{text}");
    }
    let named = decode(envelope(control_plane()));
    assert!(format!("{named:?}").contains("checkout_style"));
}
