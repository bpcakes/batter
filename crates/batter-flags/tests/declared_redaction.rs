//! Nothing a setting holds is ever shown: every error and report formats
//! without it, and resolution reads the source it is given, never the process
//! environment.
//!
//! R-01 (a failure names the flag, never the value), R-11 (the report is
//! redacted) and, for `resolve` only, R-13. That library code never reads the
//! process environment is a property of every crate's source, which a later
//! task enforces; it remains unverified here.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use std::{
    error::Error as _,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    process::{Command, Stdio},
};

use batter_core::settings::SettingsSource;
use batter_flags::{
    declared::{self, DeclaredError},
    low_level,
};
use world::{As, resolve_as, resolve_serving, source_over, today, world};

/// A value no declared name contains and no message may.
const MARKER: &str = "s3cr3t-marker-4821";

/// Every way the library formats a value.
fn formats(value: &(impl std::fmt::Display + std::fmt::Debug)) -> [String; 3] {
    [
        format!("{value}"),
        format!("{value:?}"),
        format!("{value:#?}"),
    ]
}

fn assert_marker_free(label: &str, value: &(impl std::fmt::Display + std::fmt::Debug)) {
    for text in formats(value) {
        assert!(!text.contains(MARKER), "{label}: {text}");
    }
}

#[test]
fn every_failure_formats_without_the_value_that_caused_it() {
    let marked = format!("{MARKER}-value");
    // One failing resolution per variant that a setting's value can reach.
    let cases: Vec<(&str, Result<_, DeclaredError>)> = vec![
        // The value is in a declared setting.
        (
            "invalid toggle",
            resolve_serving(&[("FLAG_FAST_PATH", &marked)]),
        ),
        (
            "invalid enum",
            resolve_serving(&[("FLAG_CHECKOUT_STYLE", &marked)]),
        ),
        (
            "invalid switch",
            resolve_serving(&[("FLAG_CHECKOUT_HALT", &marked)]),
        ),
        // The value is where it may not be.
        (
            "not owned",
            resolve_serving(&[("FLAG_WORKER_MODE", &marked)]),
        ),
        (
            "outside class",
            resolve_serving(&[("FLAG_DEBUG_PANEL", &marked)]),
        ),
        // The value is in a retired or unknown setting, or a name carries it.
        (
            "retired",
            resolve_serving(&[("FLAG_OLD_CHECKOUT", &marked)]),
        ),
        (
            "retired global",
            resolve_serving(&[("LEGACY_FEATURE_SWITCH", &marked)]),
        ),
        ("unknown value", resolve_serving(&[("FLAG_TYPO", &marked)])),
        (
            "unknown name",
            resolve_serving(&[(&format!("FLAG_{MARKER}"), "on")]),
        ),
        // The value is in a required setting that is present while another is not.
        (
            "required settings",
            resolve_serving(&[("FLAG_NEW_CHECKOUT", "on"), ("PAYMENT_API_URL", &marked)]),
        ),
        (
            "invariant",
            resolve_serving(&[("FLAG_SYNC_IN", "on"), ("DATABASE_URL", &marked)]),
        ),
    ]
    .into_iter()
    .map(|(label, result)| (label, result.map(|(resolved, _)| resolved)))
    .collect();
    assert_eq!(cases.len(), 11);
    for (label, result) in &cases {
        let error = result.as_ref().unwrap_err();
        assert_marker_free(label, error);
        if let Some(cause) = error.source() {
            // Only a non-Unicode value retains a cause, covered below.
            panic!("{label}: unexpected retained cause {cause}");
        }
    }
}

#[test]
fn a_failure_with_no_setting_to_blame_formats_cleanly_too() {
    let (catalog, h) = world();
    let empty = SettingsSource::from_pairs([]).unwrap();
    let missing = declared::resolve(catalog, &empty, h.serving, h.hosted, today()).unwrap_err();
    assert!(matches!(missing, DeclaredError::Missing { .. }));
    assert_marker_free("missing", &missing);

    let (catalog, h) = world();
    let (_other, foreign) = world();
    let source = source_over(
        &world::required(As::ServingHosted),
        &[("DATABASE_URL", MARKER)],
    );
    let kind = declared::resolve(catalog, &source, foreign.serving, h.hosted, today()).unwrap_err();
    assert!(matches!(kind, DeclaredError::ForeignKind { .. }));
    assert_marker_free("foreign kind", &kind);
    let (catalog, h) = world();
    let class =
        declared::resolve(catalog, &source, h.serving, foreign.hosted, today()).unwrap_err();
    assert!(matches!(class, DeclaredError::ForeignClass { .. }));
    assert_marker_free("foreign class", &class);
}

#[test]
fn a_non_unicode_value_is_the_one_cause_kept_and_it_is_only_reachable_explicitly() {
    let raw = OsString::from_vec([MARKER.as_bytes(), b"\xff"].concat());
    let (catalog, h) = world();
    let source = SettingsSource::from_pairs([
        ("FLAG_REGION_LOCK".into(), "off".into()),
        ("FLAG_FAST_PATH".into(), raw),
    ])
    .unwrap();
    let error = declared::resolve(catalog, &source, h.serving, h.hosted, today()).unwrap_err();
    assert_marker_free("non-unicode", &error);
    let cause = error.source().unwrap();
    assert!(
        cause.to_string().contains(MARKER),
        "explicit access shows it"
    );
}

#[test]
fn a_resolved_catalog_and_its_report_never_show_what_the_source_held() {
    let (resolved, _h) = resolve_serving(&[
        ("FLAG_NEW_CHECKOUT", "on"),
        ("PAYMENT_API_URL", &format!("https://{MARKER}.example")),
        ("PAYMENT_API_REGION", MARKER),
        ("DATABASE_URL", &format!("postgres://user:{MARKER}@host/db")),
        ("PATH", MARKER),
    ])
    .unwrap();
    assert_marker_free("resolved", &DebugOnly(&resolved));
    let report = resolved.report();
    assert_marker_free("report", &report);
    for entry in report.entries() {
        assert_marker_free("entry", entry);
    }
    let view = resolved.startup_view();
    assert_marker_free("startup view", &DebugOnly(&view));
    let flags = resolved.into_flags();
    assert_marker_free("flags", &DebugOnly(&flags));
    let live = low_level::capture_now(&flags);
    assert_marker_free("view", &DebugOnly(&live));
}

/// A value that is only `Debug`, shown the same way for all three formats.
struct DebugOnly<'a, T>(&'a T);

impl<T: std::fmt::Debug> std::fmt::Debug for DebugOnly<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<T: std::fmt::Debug> std::fmt::Display for DebugOnly<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

// R-13 for `resolve` alone: the process environment is not a source.

const CHILD: &str = "BATTER_FLAGS_REDACTION_CHILD";

/// The child half: it runs in a process whose environment was cleared and then
/// given only a decoy flag setting, and reports what resolution did.
#[test]
fn child_resolves_from_its_source_only() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    // The decoy is a real flag setting in the process environment. The
    // injected source is empty, so a resolver that read the environment would
    // resolve the flag to `on`.
    assert_eq!(std::env::var("FLAG_FAST_PATH").as_deref(), Ok("on"));
    let (resolved, _h) = resolve_serving(&[]).unwrap();
    let report = resolved.report();
    let fast_path = report
        .entries()
        .iter()
        .find(|entry| entry.key().as_str() == "fast_path")
        .unwrap();
    println!("child: fast_path={}", fast_path);
    // And a failure under the cleared environment formats without the value.
    let error = resolve_serving(&[("FLAG_FAST_PATH", MARKER)]).unwrap_err();
    println!("child: error={error}");
    println!("child: debug={error:?}");
}

#[test]
fn resolution_ignores_the_process_environment() {
    if std::env::var_os(CHILD).is_some() {
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_resolves_from_its_source_only",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(CHILD, "1")
        .env("FLAG_FAST_PATH", "on")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("child: fast_path=fast_path=off (absent)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("child: error=setting FLAG_FAST_PATH"),
        "{stdout}"
    );
    for text in [&stdout, &stderr] {
        assert!(!text.contains(MARKER), "{text}");
    }
}

#[test]
fn two_resolutions_of_one_source_agree() {
    // Determinism the child test relies on.
    let first = resolve_as(As::WorkerHosted, &[("FLAG_FAST_PATH", "on")])
        .unwrap()
        .0;
    let second = resolve_as(As::WorkerHosted, &[("FLAG_FAST_PATH", "on")])
        .unwrap()
        .0;
    assert_eq!(first.revision(), second.revision());
}
