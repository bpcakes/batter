//! Snapshots: a candidate is rejected only for structural corruption, and every
//! other row is published, counted or held on its own.
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
    Freshness, Layer, Reason, SwitchState, Toggle,
    low_level::{Candidate, CandidateRow, HoldCause, Snapshot, SnapshotRejection, ValidationScope},
};
use capture::Capture;
use fixtures::Checkout;
use world::{Literal, base, flag_row, snapshot, version, world};

const CANONICAL_ON: &str = r#"{"encoding":1,"variant":"on"}"#;
const CANONICAL_EXPRESS: &str = r#"{"encoding":1,"variant":"express"}"#;

fn row(key: &str, kind: &str, value_type: &str, state: &str) -> CandidateRow {
    CandidateRow::new(key, kind, value_type, version(2), state)
}

fn held(snapshot: &Snapshot) -> Vec<(String, HoldCause)> {
    snapshot
        .held()
        .map(|(key, cause)| (key.as_str().to_owned(), cause))
        .collect()
}

/// The causes `rows` are held for, with the world's `serving` base.
fn causes(rows: impl IntoIterator<Item = CandidateRow>) -> Vec<(String, HoldCause)> {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    held(&snapshot(&base, 3, rows))
}

#[test]
fn a_candidate_is_rejected_only_for_structural_corruption() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();

    let versionless = Candidate::new(&scope).row(flag_row(&h.fast_path, Toggle::On, 1));
    assert_eq!(
        Snapshot::validate(&scope, versionless).unwrap_err(),
        SnapshotRejection::MissingVersion
    );

    let repeated = Candidate::new(&scope).at(version(4)).rows([
        flag_row(&h.fast_path, Toggle::On, 1),
        row("retired_flag", "mode", "toggle", CANONICAL_ON),
        flag_row(&h.fast_path, Toggle::Off, 2),
    ]);
    assert_eq!(
        Snapshot::validate(&scope, repeated).unwrap_err(),
        SnapshotRejection::DuplicateKey { row: 2 },
        "the later row is the one that repeats"
    );
    // A repeated key is corrupt whether or not any declaration has it.
    let unknown_twice = Candidate::new(&scope).at(version(4)).rows([
        row("retired_flag", "mode", "toggle", CANONICAL_ON),
        row("retired_flag", "mode", "toggle", CANONICAL_ON),
    ]);
    assert_eq!(
        Snapshot::validate(&scope, unknown_twice).unwrap_err(),
        SnapshotRejection::DuplicateKey { row: 1 }
    );

    let (other_catalog, other) = world();
    let other_base = self::base(other_catalog, &other.serving, &[]);
    let other_scope = other_base.validation_scope();
    let foreign = Candidate::new(&other_scope).at(version(4));
    assert_eq!(
        Snapshot::validate(&scope, foreign).unwrap_err(),
        SnapshotRejection::ForeignScope
    );

    let valid = Candidate::new(&scope).at(version(0));
    assert_eq!(
        Snapshot::validate(&scope, valid).unwrap().version(),
        version(0)
    );
}

#[test]
fn a_rejection_names_a_position_never_candidate_text() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    let secret = row("SECRET_key", "SECRET_kind", "SECRET_type", "SECRET_state");
    let candidate = Candidate::new(&scope)
        .at(version(1))
        .rows([secret.clone(), secret]);
    let rejection = Snapshot::validate(&scope, candidate).unwrap_err();
    for text in [format!("{rejection}"), format!("{rejection:?}")] {
        assert!(!text.contains("SECRET"), "{text}");
    }
    assert_eq!(rejection, SnapshotRejection::DuplicateKey { row: 1 });
}

#[test]
fn rows_that_are_not_even_names_are_counted_unknown_and_never_reject_the_candidate() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let long = "a".repeat(200);
    let snapshot = snapshot(
        &base,
        5,
        [
            row("", "mode", "toggle", CANONICAL_ON),
            row("UPPER", "mode", "toggle", CANONICAL_ON),
            row("with space", "mode", "toggle", CANONICAL_ON),
            row("naïve", "mode", "toggle", CANONICAL_ON),
            row(&long, "mode", "toggle", CANONICAL_ON),
            row("1starts_with_digit", "mode", "toggle", CANONICAL_ON),
            row("\u{0}", "\u{0}", "\u{0}", "\u{0}"),
            flag_row(&h.fast_path, Toggle::On, 4),
        ],
    );
    assert_eq!((snapshot.unknown(), snapshot.not_owned()), (7, 0));
    assert_eq!(held(&snapshot), []);
    let view = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();
    assert_eq!(view.evaluate(&h.fast_path).value(), Toggle::On);
}

#[test]
fn unknown_and_not_owned_rows_are_counted_never_held_and_emit_nothing() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope = base.validation_scope();
    // `worker_mode` and `export_halt` belong to `worker` alone.
    let candidate = Candidate::new(&scope).at(version(6)).rows([
        row("retired_flag", "mode", "toggle", CANONICAL_ON),
        row("another_retired_flag", "release", "toggle", "not even json"),
        flag_row(&h.worker_mode, Toggle::On, 5),
        row("export_halt", "switch", "switch", r#"{"encoding":9}"#),
        flag_row(&h.fast_path, Toggle::On, 5),
    ]);
    let capture = Capture::new();
    let snapshot = capture.within(|| Snapshot::validate(&scope, candidate).unwrap());
    assert_eq!((snapshot.unknown(), snapshot.not_owned()), (2, 2));
    assert_eq!(held(&snapshot), [], "neither kind of row is ever held");
    assert!(
        capture.events().is_empty(),
        "validation emits no event, so it cannot WARN: {:?}",
        capture.events()
    );
}

#[test]
fn the_hold_vocabulary_is_closed() {
    let all = [
        (HoldCause::EntryTypeMismatch, "entry_type_mismatch"),
        (HoldCause::UnknownRowEncoding, "unknown_row_encoding"),
        (HoldCause::UndeclaredVariant, "undeclared_variant"),
        (
            HoldCause::RequiresSettingsAbsent,
            "requires_settings_absent",
        ),
        (HoldCause::EnvironmentRestricted, "environment_restricted"),
        (HoldCause::InvariantUnmet, "invariant_unmet"),
    ];
    for (cause, name) in all {
        assert_eq!(cause.as_str(), name);
    }
    // An exhaustive match fails to compile when a cause is added without a test.
    for (cause, _) in all {
        match cause {
            HoldCause::EntryTypeMismatch
            | HoldCause::UnknownRowEncoding
            | HoldCause::UndeclaredVariant
            | HoldCause::RequiresSettingsAbsent
            | HoldCause::EnvironmentRestricted
            | HoldCause::InvariantUnmet => {}
        }
    }
}

struct Case {
    label: &'static str,
    key: &'static str,
    kind: &'static str,
    value_type: &'static str,
    state: &'static str,
    cause: HoldCause,
}

const fn case(
    (label, key, kind, value_type): (&'static str, &'static str, &'static str, &'static str),
    state: &'static str,
    cause: HoldCause,
) -> Case {
    Case {
        label,
        key,
        kind,
        value_type,
        state,
        cause,
    }
}

#[test]
fn each_hold_cause_is_reported_for_its_own_row() {
    use HoldCause::{EntryTypeMismatch, UndeclaredVariant, UnknownRowEncoding};
    let style = |label| (label, "checkout_style", "mode", "enum");
    let halt = |label| (label, "checkout_halt", "switch", "switch");
    let cases = [
        case(
            ("kind differs", "checkout_style", "release", "enum"),
            CANONICAL_EXPRESS,
            EntryTypeMismatch,
        ),
        case(
            ("value type differs", "checkout_style", "mode", "toggle"),
            CANONICAL_EXPRESS,
            EntryTypeMismatch,
        ),
        case(
            (
                "a switch row for a flag",
                "checkout_style",
                "switch",
                "switch",
            ),
            r#"{"encoding":1,"switch":"engaged"}"#,
            EntryTypeMismatch,
        ),
        case(
            ("a flag row for a switch", "checkout_halt", "mode", "toggle"),
            CANONICAL_ON,
            EntryTypeMismatch,
        ),
        case(
            style("a higher encoding"),
            r#"{"encoding":2,"variant":"express"}"#,
            UnknownRowEncoding,
        ),
        case(
            style("an extra field"),
            r#"{"encoding":1,"variant":"express","weight":10}"#,
            UnknownRowEncoding,
        ),
        case(style("not json"), "express", UnknownRowEncoding),
        case(
            style("a variant of the wrong shape"),
            r#"{"encoding":1,"variant":1}"#,
            UnknownRowEncoding,
        ),
        case(
            halt("a switch state that is neither literal"),
            r#"{"encoding":1,"switch":"tripped"}"#,
            UnknownRowEncoding,
        ),
        case(
            halt("a switch row with a variant"),
            r#"{"encoding":1,"variant":"on"}"#,
            UnknownRowEncoding,
        ),
        case(
            style("a variant no declaration lists"),
            r#"{"encoding":1,"variant":"turbo"}"#,
            UndeclaredVariant,
        ),
        case(
            style("a variant spelled in another case"),
            r#"{"encoding":1,"variant":"Express"}"#,
            UndeclaredVariant,
        ),
    ];
    for case in cases {
        let candidate = row(case.key, case.kind, case.value_type, case.state);
        assert_eq!(
            causes([candidate]),
            [(case.key.to_owned(), case.cause)],
            "{}",
            case.label
        );
    }
}

/// When several causes apply, the first in the documented order is reported.
#[test]
fn when_several_causes_apply_the_first_in_the_documented_order_is_reported() {
    use HoldCause::{EntryTypeMismatch, EnvironmentRestricted, UnknownRowEncoding};
    // restricted > type mismatch > unknown encoding
    let all_three = row("debug_panel", "release", "enum", r#"{"encoding":2}"#);
    assert_eq!(
        causes([all_three]),
        [("debug_panel".to_owned(), EnvironmentRestricted)]
    );
    let mismatch_and_encoding = row("fast_path", "mode", "toggle", r#"{"encoding":2}"#);
    assert_eq!(
        causes([mismatch_and_encoding]),
        [("fast_path".to_owned(), EntryTypeMismatch)]
    );
    // An unknown encoding is decided before the variant is looked up.
    let encoding_and_variant = row(
        "checkout_style",
        "mode",
        "enum",
        r#"{"encoding":2,"variant":"turbo"}"#,
    );
    assert_eq!(
        causes([encoding_and_variant]),
        [("checkout_style".to_owned(), UnknownRowEncoding)]
    );
}

#[test]
fn a_variant_whose_required_settings_were_absent_is_held_wherever_it_is_selected() {
    let (catalog, h) = world();
    let absent = base(
        catalog,
        &h.serving,
        &[Literal::SettingsAbsent("new_checkout", "on")],
    );
    let snapshot = snapshot(&absent, 3, [flag_row(&h.new_checkout, Toggle::On, 2)]);
    assert_eq!(
        held(&snapshot),
        [("new_checkout".to_owned(), HoldCause::RequiresSettingsAbsent)]
    );
    let view = absent
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();
    let decision = view.evaluate(&h.new_checkout);
    assert_eq!(
        (decision.value(), decision.reason(), decision.layer()),
        (Toggle::Off, Reason::Default, &Layer::Base),
        "the held entry serves the declared base"
    );
    // The variant that needs nothing is published as usual.
    let off = self::snapshot(&absent, 4, [flag_row(&h.new_checkout, Toggle::Off, 2)]);
    assert_eq!(held(&off), []);
    assert_eq!(
        absent
            .control_plane_view(&off, Freshness::Fresh)
            .unwrap()
            .evaluate(&h.new_checkout)
            .layer(),
        &Layer::Entry
    );

    // The same row is published where the settings were present.
    let (catalog, h) = world();
    let present = base(catalog, &h.serving, &[]);
    let served = self::snapshot(&present, 3, [flag_row(&h.new_checkout, Toggle::On, 2)]);
    assert_eq!(held(&served), []);
    let decision = present
        .control_plane_view(&served, Freshness::Fresh)
        .unwrap()
        .evaluate(&h.new_checkout);
    assert_eq!(
        (decision.value(), decision.layer()),
        (Toggle::On, &Layer::Entry)
    );
}

#[test]
fn a_restricted_declaration_is_held_for_every_process_kind_and_state() {
    for kind in 0..2 {
        let (catalog, h) = world();
        let kind = if kind == 0 { &h.serving } else { &h.worker };
        let base = base(catalog, kind, &[]);
        let rows = [
            flag_row(&h.debug_panel, Toggle::On, 2),
            flag_row(&h.debug_panel, Toggle::Off, 2),
        ];
        for row in rows {
            let snapshot = snapshot(&base, 3, [row]);
            assert_eq!(
                held(&snapshot),
                [("debug_panel".to_owned(), HoldCause::EnvironmentRestricted)]
            );
            let view = base
                .control_plane_view(&snapshot, Freshness::Fresh)
                .unwrap();
            let decision = view.evaluate(&h.debug_panel);
            assert_eq!(
                (decision.value(), decision.reason(), decision.layer()),
                (Toggle::Off, Reason::Default, &Layer::Base)
            );
        }
    }
}

#[test]
fn one_bad_row_never_takes_another_flag_down() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let snapshot = snapshot(
        &base,
        9,
        [
            row("checkout_style", "mode", "enum", "garbage"),
            row("checkout_halt", "switch", "switch", r#"{"encoding":3}"#),
            flag_row(&h.debug_panel, Toggle::On, 2),
            row("retired_flag", "mode", "toggle", "garbage"),
            flag_row(&h.fast_path, Toggle::On, 8),
            flag_row(&h.new_checkout, Toggle::On, 8),
        ],
    );
    assert_eq!(held(&snapshot).len(), 3);
    let view = base
        .control_plane_view(&snapshot, Freshness::Fresh)
        .unwrap();
    assert_eq!(view.evaluate(&h.fast_path).value(), Toggle::On);
    assert_eq!(view.evaluate(&h.new_checkout).value(), Toggle::On);
    assert_eq!(view.evaluate(&h.checkout_style).value(), Checkout::Classic);
    assert_eq!(view.switch(&h.checkout_halt).value(), SwitchState::Released);
}

#[test]
fn debug_output_never_shows_row_state_or_a_key_that_is_not_a_name() {
    let (catalog, h) = world();
    let base = base(catalog, &h.serving, &[]);
    let scope: ValidationScope = base.validation_scope();
    let secret = row("SECRET key", "SECRET_kind", "SECRET_type", "SECRET_state");
    let named = row("fast_path", "release", "toggle", "SECRET_state");
    let candidate = Candidate::new(&scope)
        .at(version(1))
        .rows([secret.clone(), named.clone()]);
    let rendered = [
        format!("{secret:?}"),
        format!("{named:?}"),
        format!("{candidate:?}"),
        format!("{scope:?}"),
        format!("{:?}", Snapshot::validate(&scope, candidate).unwrap()),
    ];
    for text in rendered {
        assert!(!text.contains("SECRET"), "{text}");
    }
    assert_eq!(
        secret.state_text(),
        "SECRET_state",
        "the state is readable on purpose"
    );
}
