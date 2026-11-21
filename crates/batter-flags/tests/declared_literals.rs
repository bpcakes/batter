//! Strict literals, absence and unknown names: R-01, R-02 and R-03.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use std::{error::Error as _, ffi::OsString, os::unix::ffi::OsStringExt};

use batter_core::settings::SettingsSource;
use batter_flags::{
    ValueSource,
    core::{Layer, Reason, Source, SwitchState, Toggle},
    declared::{self, DeclaredError, ValueProblem},
    low_level,
};
use world::{As, Checkout, resolve_as, resolve_serving, source_over, today, world};

/// What a rejected value reports: the flag, the setting and the problem.
fn invalid(error: &DeclaredError) -> (&str, &str, ValueProblem) {
    match error {
        DeclaredError::InvalidValue {
            key, name, problem, ..
        } => (key.as_str(), name.as_str(), *problem),
        other => panic!("expected InvalidValue, found {other}"),
    }
}

#[test]
fn an_empty_source_resolves_every_flag_to_its_default() {
    let (resolved, h) = resolve_serving(&[]).unwrap();
    let decision = resolved.startup_view().evaluate(&h.fast_path);
    assert_eq!(decision.value(), Toggle::Off);
    assert_eq!(decision.reason(), Reason::Default);
    assert_eq!(decision.layer(), &Layer::Base);
    assert!(matches!(
        decision.source(),
        Source::Declared { revision } if revision == resolved.revision()
    ));
    let report = resolved.report();
    let entry = |key: &str| {
        report
            .entries()
            .iter()
            .find(|entry| entry.key().as_str() == key)
            .unwrap()
    };
    assert_eq!(entry("fast_path").source(), ValueSource::Absent);
    assert_eq!(entry("fast_path").variant(), "off");
    assert_eq!(entry("checkout_halt").variant(), "released");
    assert_eq!(entry("region_lock").source(), ValueSource::Literal);
}

#[test]
fn a_toggle_accepts_true_false_on_and_off_in_any_ascii_case() {
    for (text, expected) in [
        ("true", Toggle::On),
        ("TRUE", Toggle::On),
        ("True", Toggle::On),
        ("on", Toggle::On),
        ("ON", Toggle::On),
        ("oN", Toggle::On),
        ("false", Toggle::Off),
        ("FALSE", Toggle::Off),
        ("off", Toggle::Off),
        ("Off", Toggle::Off),
    ] {
        let (resolved, h) = resolve_serving(&[("FLAG_FAST_PATH", text)]).unwrap();
        let decision = resolved.startup_view().evaluate(&h.fast_path);
        assert_eq!(decision.value(), expected, "{text}");
        assert_eq!(decision.reason(), Reason::Static, "{text}");
        assert_eq!(decision.layer(), &Layer::Entry, "{text}");
    }
}

#[test]
fn a_toggle_rejects_everything_else_naming_the_flag_and_never_the_value() {
    for text in [
        "1",
        "0",
        "yes",
        "no",
        "y",
        "n",
        "t",
        "f",
        "tru",
        "truee",
        "enabled",
        "disabled",
        "null",
        " true",
        "true ",
        "on ",
        "true\n",
        "\u{0442}rue",
        "оn",
    ] {
        let error = resolve_serving(&[("FLAG_FAST_PATH", text)]).unwrap_err();
        let (key, name, problem) = invalid(&error);
        assert_eq!((key, name), ("fast_path", "FLAG_FAST_PATH"), "{text:?}");
        assert_eq!(problem, ValueProblem::NotAToggle, "{text:?}");
        assert!(error.source().is_none(), "{text:?}");
    }
}

#[test]
fn a_blank_value_is_reported_as_blank_for_every_kind_of_flag() {
    for name in [
        "FLAG_FAST_PATH",
        "FLAG_CHECKOUT_STYLE",
        "FLAG_CHECKOUT_HALT",
    ] {
        for text in ["", " ", "\t", " \n "] {
            let error = resolve_serving(&[(name, text)]).unwrap_err();
            let (_, reported, problem) = invalid(&error);
            assert_eq!((reported, problem), (name, ValueProblem::Blank), "{text:?}");
        }
    }
}

#[test]
fn an_enum_flag_accepts_exactly_a_declared_variant_name() {
    for (text, expected) in [
        ("classic", Checkout::Classic),
        ("express", Checkout::Express),
    ] {
        let (resolved, h) = resolve_serving(&[("FLAG_CHECKOUT_STYLE", text)]).unwrap();
        let decision = resolved.startup_view().evaluate(&h.checkout_style);
        assert_eq!(decision.value(), expected);
        assert_eq!(decision.reason(), Reason::Static);
    }
    for text in [
        "Express",
        "EXPRESS",
        "eXpress",
        " express",
        "express ",
        "express,classic",
        "fast",
        "on",
        "0",
    ] {
        let error = resolve_serving(&[("FLAG_CHECKOUT_STYLE", text)]).unwrap_err();
        let (key, _, problem) = invalid(&error);
        assert_eq!(key, "checkout_style");
        assert_eq!(problem, ValueProblem::NotADeclaredVariant, "{text:?}");
    }
}

#[test]
fn a_kill_switch_accepts_exactly_engaged_or_released() {
    for (text, expected) in [
        ("engaged", SwitchState::Engaged),
        ("released", SwitchState::Released),
    ] {
        let (resolved, h) = resolve_serving(&[("FLAG_CHECKOUT_HALT", text)]).unwrap();
        let flags = resolved.into_flags();
        let view = low_level::capture_now(&flags);
        let decision = view.switch(&h.checkout_halt);
        assert_eq!(decision.value(), expected, "{text}");
        assert_eq!(decision.layer(), &Layer::Entry, "{text}");
        assert_eq!(decision.reason(), Reason::Static, "{text}");
    }
    for text in [
        "Engaged", "ENGAGED", "on", "off", "true", "false", "1", "engaged ", "release",
    ] {
        let error = resolve_serving(&[("FLAG_CHECKOUT_HALT", text)]).unwrap_err();
        let (key, _, problem) = invalid(&error);
        assert_eq!(key, "checkout_halt");
        assert_eq!(problem, ValueProblem::NotASwitchState, "{text:?}");
    }
}

#[test]
fn a_kill_switch_with_no_setting_serves_its_safe_state() {
    let (resolved, h) = resolve_as(As::WorkerHosted, &[]).unwrap();
    let flags = resolved.into_flags();
    let view = low_level::capture_now(&flags);
    // Released where the safe state is released, engaged where it is engaged.
    assert_eq!(view.switch(&h.checkout_halt).value(), SwitchState::Released);
    assert_eq!(view.switch(&h.export_halt).value(), SwitchState::Engaged);
    assert_eq!(view.switch(&h.export_halt).layer(), &Layer::Base);
    assert_eq!(view.switch(&h.export_halt).reason(), Reason::Default);
}

#[test]
fn a_non_unicode_value_fails_and_retains_its_cause_for_explicit_inspection() {
    let raw = OsString::from_vec(b"on\xff-secret-marker".to_vec());
    let (catalog, h) = world();
    let source = SettingsSource::from_pairs([
        ("FLAG_REGION_LOCK".into(), "off".into()),
        ("FLAG_FAST_PATH".into(), raw),
    ])
    .unwrap();
    let error = declared::resolve(catalog, &source, h.serving, h.hosted, today()).unwrap_err();
    let (key, name, problem) = invalid(&error);
    assert_eq!((key, name), ("fast_path", "FLAG_FAST_PATH"));
    assert_eq!(problem, ValueProblem::NotUnicode);
    // Formatting never shows the bytes; the retained cause is explicit access.
    for formatted in [
        format!("{error}"),
        format!("{error:?}"),
        format!("{error:#?}"),
    ] {
        assert!(!formatted.contains("secret-marker"), "{formatted}");
    }
    let cause = error.source().expect("the non-Unicode cause is retained");
    assert!(cause.to_string().contains("secret-marker"));
}

#[test]
fn a_mistyped_name_under_the_prefix_never_resolves_silently() {
    for typo in [
        "FLAG_NEW_CHECKOT",
        "FLAG_FAST_PATH_EXTRA",
        "FLAG_",
        "FLAG_fast_path",
        "FLAG_UNKNOWN_SECRET_NAME",
    ] {
        let error = resolve_serving(&[(typo, "on")]).unwrap_err();
        assert!(
            matches!(&error, DeclaredError::UnknownName { prefix } if prefix == "FLAG_"),
            "{typo}: {error}"
        );
        if typo != "FLAG_" {
            assert!(
                !error.to_string().contains(typo),
                "the name is never echoed: {typo}"
            );
        }
    }
}

#[test]
fn names_outside_the_prefix_are_not_this_catalogs() {
    // Another namespace: not rejected, not read.
    let (resolved, _) = resolve_serving(&[
        ("PATH", "/usr/bin"),
        ("HOME", "/home/someone"),
        ("FLAGS_OTHER", "on"),
        ("flag_fast_path", "on"),
        ("DATABASE_URL", "postgres://user:password@host/db"),
    ])
    .unwrap();
    let report = resolved.report();
    // Only the flag the world's sources always carry came from a setting.
    assert!(
        report
            .entries()
            .iter()
            .filter(|entry| entry.key().as_str() != "region_lock")
            .all(|entry| entry.source() != ValueSource::Literal)
    );
}

#[test]
fn the_specific_diagnostic_comes_before_the_catch_all() {
    // A declared value that is wrong, and a typo: the declared one is reported.
    let error = resolve_serving(&[("FLAG_FAST_PATH", "1"), ("FLAG_TYPO", "on")]).unwrap_err();
    assert!(
        matches!(error, DeclaredError::InvalidValue { .. }),
        "{error}"
    );
    // A retired name and a typo: the retired one, with its note.
    let error = resolve_serving(&[("FLAG_OLD_CHECKOUT", "on"), ("FLAG_TYPO", "on")]).unwrap_err();
    assert!(matches!(error, DeclaredError::Retired { .. }), "{error}");
}

#[test]
fn the_first_declared_failure_in_key_order_is_the_one_reported() {
    let error =
        resolve_serving(&[("FLAG_FAST_PATH", "1"), ("FLAG_CHECKOUT_STYLE", "wrong")]).unwrap_err();
    // `checkout_style` sorts before `fast_path`.
    assert_eq!(invalid(&error).0, "checkout_style");
}

#[test]
fn resolution_reads_the_source_it_is_given_and_nothing_else() {
    let (catalog, h) = world();
    let source = source_over(
        &world::required(As::ServingHosted),
        &[("FLAG_FAST_PATH", "on")],
    );
    let resolved = declared::resolve(catalog, &source, h.serving, h.hosted, today()).unwrap();
    assert_eq!(
        resolved.startup_view().evaluate(&h.fast_path).value(),
        Toggle::On
    );
}
