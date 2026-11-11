//! Replaying a stored pin: one event per call, no variant when the pin is
//! refused, the `error.type` of each arm, and no stored text in any event.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![cfg(feature = "serde")]

#[path = "support/capture.rs"]
mod capture;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_flags_core::{PinRejection, Pinned, PinnedResolution};
use capture::Capture;
use fixtures::Checkout;
use serde_json::{Value, json};
use tracing::Level;
use world::{base, world};

const MARKER: &str = "SECRET-MARKER";

fn pin(value: Value) -> Pinned<Checkout> {
    serde_json::from_value(value).unwrap()
}

fn stored(key: &str, kind: &str, value_type: &str, variant: &str, origin: Value) -> Value {
    json!({
        "v": 1, "key": key, "kind": kind, "value_type": value_type,
        "variant": variant, "origin": origin,
    })
}

fn control_plane() -> Value {
    json!({"class": "control_plane", "version": 7})
}

/// Each arm a stored pin can reach, every stored string carrying the marker
/// wherever the arm allows it.
fn arms() -> Vec<(PinRejection, Pinned<Checkout>)> {
    let secret = |name: &str| format!("{MARKER}-{name}");
    vec![
        (
            PinRejection::UnsupportedVersion { v: 2 },
            pin(json!({"v": 2, "key": secret("key"), "payload": secret("payload")})),
        ),
        (
            PinRejection::Malformed,
            pin(json!({"v": 1, "key": secret("key"), "origin": secret("origin")})),
        ),
        (
            PinRejection::KeyMismatch,
            pin(stored(
                &secret("key"),
                &secret("kind"),
                &secret("type"),
                &secret("variant"),
                control_plane(),
            )),
        ),
        (
            PinRejection::KindChanged,
            pin(stored(
                "checkout_style",
                &secret("kind"),
                &secret("type"),
                &secret("variant"),
                control_plane(),
            )),
        ),
        (
            PinRejection::ValueTypeChanged,
            pin(stored(
                "checkout_style",
                "mode",
                &secret("type"),
                &secret("variant"),
                control_plane(),
            )),
        ),
        (
            PinRejection::UnknownVariant,
            pin(stored(
                "checkout_style",
                "mode",
                "enum",
                &secret("variant"),
                control_plane(),
            )),
        ),
        (
            PinRejection::TestProvenance,
            pin(stored(
                "checkout_style",
                "mode",
                "enum",
                "classic",
                json!({"class": "test", "version": 9}),
            )),
        ),
    ]
}

#[test]
fn every_refused_stored_pin_emits_one_event_with_its_arms_error_type_and_no_stored_text() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = capture.within(|| base.declared_view());

    for (arm, pin) in arms() {
        capture.clear();
        let resolution = view.resolve_pinned(&h.checkout_style, &pin);
        assert_eq!(resolution.rejection(), Some(arm), "{arm:?}");
        let events = capture.events();
        assert_eq!(events.len(), 1, "{arm:?}: {events:?}");
        let event = &events[0];
        assert_eq!(event.name, "feature_flag.evaluation", "{arm:?}");
        assert_eq!(event.target, "batter::flags", "{arm:?}");
        assert_eq!(event.level, Level::WARN, "{arm:?}: the first of its cause");
        assert_eq!(event.field("feature_flag.result.variant"), None, "{arm:?}");
        assert_eq!(event.field("feature_flag.result.reason"), Some("error"));
        assert_eq!(event.field("error.type"), Some(arm.error_type()), "{arm:?}");
        assert_eq!(event.field("feature_flag.key"), Some("checkout_style"));
        assert!(
            !event.everything().contains(MARKER),
            "{arm:?}: {}",
            event.everything()
        );
        for text in [
            format!("{resolution:?}"),
            format!("{pin:?}"),
            format!("{arm}"),
        ] {
            assert!(!text.contains(MARKER), "{arm:?}: {text}");
        }
    }
}

#[test]
fn a_cause_meets_a_handle_once_at_warn_whatever_text_the_pin_carried() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = capture.within(|| base.declared_view());
    let variant = |text: &str| {
        pin(stored(
            "checkout_style",
            "mode",
            "enum",
            text,
            control_plane(),
        ))
    };
    for text in ["turbo", "instant", "Express"] {
        assert_eq!(
            view.resolve_pinned(&h.checkout_style, &variant(text))
                .rejection(),
            Some(PinRejection::UnknownVariant),
            "{text}"
        );
    }
    let levels: Vec<_> = capture.events().iter().map(|event| event.level).collect();
    assert_eq!(levels, [Level::WARN, Level::DEBUG, Level::DEBUG]);
}

#[test]
fn an_honoured_stored_pin_reports_its_origins_version() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = capture.within(|| base.declared_view());
    let honoured = pin(stored(
        "checkout_style",
        "mode",
        "enum",
        "express",
        json!({"class": "persisted_file", "version": 31}),
    ));
    let PinnedResolution::Honoured(decision) = view.resolve_pinned(&h.checkout_style, &honoured)
    else {
        panic!("a compatible pin is honoured");
    };
    assert_eq!(decision.variant(), "express");
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, Level::DEBUG);
    assert_eq!(events[0].field("feature_flag.version"), Some("31"));
    assert_eq!(
        events[0].field("feature_flag.result.reason"),
        Some("cached")
    );
    assert_eq!(
        events[0].field("feature_flag.result.variant"),
        Some("express")
    );
    assert_eq!(events[0].field("error.type"), None);
}
