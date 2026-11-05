//! The evaluation event: its attributes against semantic conventions v1.45.0,
//! what each says about its decision and where it is emitted. Levels and the
//! first-WARN rule are in `event_levels.rs`.
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
    Decision, ErrorCause, Freshness, PinRejection, PinnedResolution, Reason, Source, Toggle,
    low_level::gate_switch,
};
use capture::Capture;
use observed::{TARGET, evaluations, only, version_text, views};
use tracing::Level;
use world::{base, snapshot, version, world};

#[test]
fn an_evaluation_event_carries_exactly_the_semantic_convention_attributes() {
    let capture = Capture::new();
    let (h, base, views) = views(&capture);
    let (_, view) = &views[3];
    view.evaluate(&h.fast_path);
    let event = only(&capture);

    assert_eq!((event.level, event.target.as_str()), (Level::DEBUG, TARGET));
    let names: Vec<_> = event.fields.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "feature_flag.key",
            "feature_flag.provider.name",
            "feature_flag.result.reason",
            "feature_flag.result.variant",
            "feature_flag.set.id",
            "feature_flag.version",
        ],
        "an ordinary evaluation records these and no error attribute"
    );
    assert_eq!(event.field("feature_flag.key"), Some("fast_path"));
    assert_eq!(event.field("feature_flag.result.variant"), Some("on"));
    assert_eq!(event.field("feature_flag.result.reason"), Some("static"));
    assert_eq!(event.field("feature_flag.version"), Some("7"));
    assert_eq!(
        event.field("feature_flag.provider.name"),
        Some("batter-flags")
    );
    assert_eq!(
        event.field("feature_flag.set.id"),
        Some(base.catalog().digest().to_hex().as_str())
    );
}

#[test]
fn the_declared_fields_never_include_what_must_not_be_emitted() {
    let capture = Capture::new();
    let (h, _, views) = views(&capture);
    views[0].1.evaluate(&h.fast_path);
    let event = only(&capture);
    assert_eq!(
        event.declared,
        [
            "feature_flag.key",
            "feature_flag.result.variant",
            "feature_flag.result.reason",
            "feature_flag.version",
            "feature_flag.set.id",
            "feature_flag.provider.name",
            "error.type",
            "batter.flags.cause",
        ],
        "the field names are the v1.45.0 attribute names, and `error.type` is the semantic-convention spelling"
    );
    for forbidden in [
        "feature_flag.context.id",
        "feature_flag.result.value",
        "error.message",
        "feature_flag.error.message",
    ] {
        assert!(
            !event.declared.iter().any(|name| name == forbidden),
            "{forbidden}"
        );
    }
}

#[test]
fn the_reason_and_error_spellings_are_semantic_convention_values() {
    let reasons = [
        (Reason::Static, "static"),
        (Reason::Default, "default"),
        (Reason::Disabled, "disabled"),
        (Reason::Cached, "cached"),
        (Reason::Stale, "stale"),
        (Reason::Error, "error"),
    ];
    for (reason, text) in reasons {
        assert_eq!(reason.as_str(), text);
    }
    for cause in ErrorCause::ALL {
        let semconv = [
            "flag_not_found",
            "provider_not_ready",
            "parse_error",
            "type_mismatch",
            "general",
        ];
        assert!(semconv.contains(&cause.error_type()), "{cause:?}");
    }
}

/// What a decision told its caller, for comparing with what its event said.
struct Seen {
    key: &'static str,
    what: &'static str,
    variant: &'static str,
    reason: Reason,
    source: Source,
    error: Option<ErrorCause>,
}

fn seen<T>(key: &'static str, what: &'static str, decision: &Decision<T>) -> Seen {
    Seen {
        key,
        what,
        variant: decision.variant(),
        reason: decision.reason(),
        source: decision.source(),
        error: decision.error(),
    }
}

/// Every event says what its decision says, whichever view served it.
#[test]
fn an_event_reports_its_decision_whichever_view_served_it() {
    let capture = Capture::new();
    let (h, base, views) = views(&capture);
    let (_other_catalog, other) = world();
    let mut compared = 0;
    for (label, view) in &views {
        capture.clear();
        let decisions = [
            seen("fast_path", "flag", &view.evaluate(&h.fast_path)),
            seen(
                "checkout_style",
                "mode flag",
                &view.evaluate(&h.checkout_style),
            ),
            seen(
                "new_checkout",
                "governed flag",
                &view.evaluate(&h.new_checkout),
            ),
            seen("worker_mode", "not owned", &view.evaluate(&h.worker_mode)),
            seen("fast_path", "foreign", &view.evaluate(&other.fast_path)),
            seen("checkout_halt", "switch", &view.switch(&h.checkout_halt)),
            seen(
                "export_halt",
                "not owned switch",
                &view.switch(&h.export_halt),
            ),
            seen(
                "checkout_halt",
                "gate",
                &gate_switch(view, &h.checkout_halt).into_decision(),
            ),
        ];
        let events = evaluations(&capture);
        assert_eq!(
            events.len(),
            decisions.len(),
            "{label}: one event per evaluation"
        );
        for (decision, event) in decisions.iter().zip(&events) {
            let context = format!("{label}, {}", decision.what);
            assert_eq!(
                event.field("feature_flag.key"),
                Some(decision.key),
                "{context}"
            );
            assert_eq!(
                event.field("feature_flag.result.variant"),
                Some(decision.variant),
                "{context}"
            );
            assert_eq!(
                event.field("feature_flag.result.reason"),
                Some(decision.reason.as_str()),
                "{context}"
            );
            assert_eq!(
                event.field("feature_flag.version"),
                Some(version_text(decision.source).as_str()),
                "{context}"
            );
            // `error.type` is set on every error evaluation and on no other.
            assert_eq!(
                event.field("error.type"),
                decision.error.map(ErrorCause::error_type),
                "{context}"
            );
            assert_eq!(
                event.field("error.type").is_some(),
                event.field("feature_flag.result.reason") == Some("error"),
                "{context}"
            );
            assert_eq!(
                event.field("batter.flags.cause"),
                decision.error.and_then(ErrorCause::handle_cause),
                "{context}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 7 * 8);
    assert!(base.error_count(ErrorCause::ProviderNotReady) > 0);
}

#[test]
fn one_event_per_evaluation_even_when_nothing_changed() {
    let capture = Capture::new();
    let (h, _, views) = views(&capture);
    let view = &views[3].1;
    for _ in 0..3 {
        view.evaluate(&h.fast_path);
    }
    assert_eq!(evaluations(&capture).len(), 3);
}

#[test]
fn a_governed_flag_emits_its_own_event_and_not_its_governing_switchs() {
    let capture = Capture::new();
    let (h, _, views) = views(&capture);
    let engaged = &views[3].1;
    let decision = engaged.evaluate(&h.new_checkout);
    assert_eq!(decision.reason(), Reason::Disabled);
    let event = only(&capture);
    assert_eq!(event.field("feature_flag.key"), Some("new_checkout"));
    assert_eq!(event.field("feature_flag.result.reason"), Some("disabled"));
    assert_eq!(event.field("error.type"), None);
}

#[test]
fn an_event_goes_to_the_dispatcher_and_span_captured_with_the_view() {
    let (first, second) = (Capture::new(), Capture::new());
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let view = first.within(|| tracing::info_span!("request").in_scope(|| base.declared_view()));

    // Evaluated elsewhere, under another dispatcher and no span.
    second.within(|| {
        view.evaluate(&h.fast_path);
    });
    let event = only(&first);
    assert_eq!(
        event.parent.as_deref(),
        Some("request"),
        "the captured span"
    );
    assert!(
        second.events().is_empty(),
        "never whatever dispatcher happens to be current: {:?}",
        second.events()
    );

    // A view captured with no dispatcher current emits to nobody.
    let (catalog, h) = world();
    let bare = self::base(catalog, &h.serving, &[]);
    let view = bare.declared_view();
    second.within(|| {
        view.evaluate(&h.fast_path);
    });
    assert!(second.events().is_empty());
}

/// Each library evaluation emits one event: a pin and a replay included, the
/// arms the library can reach without a stored envelope.
#[test]
fn a_pin_and_each_replay_emit_one_event() {
    let capture = Capture::new();
    let (h, _, views) = views(&capture);
    let (_other_catalog, other) = world();
    let view = &views[3].1;

    let pin = view.pin(&h.fast_path);
    assert_eq!(only(&capture).field("feature_flag.key"), Some("fast_path"));
    capture.clear();

    let PinnedResolution::Honoured(honoured) = view.resolve_pinned(&h.fast_path, &pin) else {
        panic!("honoured");
    };
    let event = only(&capture);
    assert_eq!(event.field("feature_flag.result.reason"), Some("cached"));
    assert_eq!(
        event.field("feature_flag.version"),
        Some(version_text(honoured.source()).as_str()),
        "a pin's version is its origin's"
    );
    assert_eq!(event.field("feature_flag.version"), Some("7"));
    capture.clear();

    let check = |resolution: PinnedResolution<'_, Toggle>, arm: PinRejection, cause| {
        assert_eq!(resolution.rejection(), Some(arm));
        let event = only(&capture);
        capture.clear();
        assert_eq!(
            event.field("feature_flag.result.variant"),
            None,
            "{arm:?}: no value"
        );
        assert_eq!(
            event.field("feature_flag.result.reason"),
            Some("error"),
            "{arm:?}"
        );
        assert_eq!(event.field("error.type"), Some(arm.error_type()), "{arm:?}");
        assert_eq!(event.field("batter.flags.cause"), cause, "{arm:?}");
        assert_eq!(
            event.field("feature_flag.version"),
            Some(version_text(view.source()).as_str()),
            "{arm:?}: the replaying view's"
        );
    };
    check(
        view.resolve_pinned(&other.fast_path, &pin),
        PinRejection::ForeignHandle,
        Some("foreign_catalog"),
    );
    check(
        view.resolve_pinned(&h.new_checkout, &pin),
        PinRejection::KeyMismatch,
        None,
    );
}

#[test]
fn a_test_view_reports_the_injected_version() {
    let capture = Capture::new();
    let (h, base, _) = views(&capture);
    let published = snapshot(&base, 41, []);
    let test = capture.within(|| base.test_view(&published, Freshness::Stale).unwrap());
    test.evaluate(&h.fast_path);
    let event = only(&capture);
    assert_eq!(event.field("feature_flag.version"), Some("41"));
    assert_eq!(version(41).get(), 41);
    assert!(matches!(test.source(), Source::Test { .. }));
}
