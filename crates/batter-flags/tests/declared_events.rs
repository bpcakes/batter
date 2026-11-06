//! What resolution reports through `tracing`: the expiry WARN, once, against
//! the date the root passed; and nothing else.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/capture.rs"]
mod capture;
#[path = "../../../test-support/dispatch.rs"]
mod test_dispatch;
#[path = "support/world.rs"]
mod world;

use batter_core::settings::SettingsSource;
use batter_flags::{
    core::{CatalogBuilder, CivilDate, Expiry, Reason, SettingsPrefix, Toggle},
    declared,
};
use capture::Capture;
use tracing::Level;
use world::{As, required, resolve_serving, source_over, today, world};

/// A catalog with two release flags that expire on different days and one that
/// never does, and the kind and class to resolve it as.
fn expiring() -> (
    batter_flags::core::Catalog,
    batter_flags::core::ProcessKind,
    batter_flags::core::EnvironmentClass,
) {
    let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
    let serving = builder.process_kind("serving").unwrap();
    let hosted = builder.environment_class("hosted").unwrap();
    let on = |year, month, day| Expiry::on(CivilDate::new(year, month, day).unwrap());
    builder
        .release_flag("new_checkout", Toggle::Off, on(2027, 3, 31))
        .declare()
        .unwrap();
    builder
        .release_flag("fast_path", Toggle::Off, on(2026, 12, 31))
        .declare()
        .unwrap();
    builder
        .release_flag("slow_path", Toggle::Off, on(2026, 6, 30))
        .declare()
        .unwrap();
    builder
        .mode_flag("strict_mode", Toggle::Off)
        .declare()
        .unwrap();
    (builder.build().unwrap(), serving, hosted)
}

fn resolve_on(date: CivilDate, capture: &Capture) -> Vec<capture::Event> {
    let (catalog, serving, hosted) = expiring();
    let source = SettingsSource::from_pairs([]).unwrap();
    capture.clear();
    let resolved = capture
        .within(|| declared::resolve(catalog, &source, serving, hosted, date))
        .unwrap();
    drop(resolved);
    capture.events()
}

#[test]
fn flags_past_their_expiry_are_reported_once_in_one_warn_event() {
    let capture = Capture::new();
    // Last valid days are inclusive: on 2026-12-31 only `slow_path` is past.
    let events = resolve_on(CivilDate::new(2026, 12, 31).unwrap(), &capture);
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.level, Level::WARN);
    assert_eq!(event.target, "batter::flags");
    assert_eq!(event.name, "flags.expired");
    assert_eq!(event.field("batter.flags.expired"), Some("slow_path"));
    assert_eq!(event.field("batter.flags.expired.count"), Some("1"));
    assert_eq!(event.field("batter.flags.today"), Some("2026-12-31"));

    // The day after, two are past, listed in key order in the same one event.
    let events = resolve_on(CivilDate::new(2027, 1, 1).unwrap(), &capture);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].field("batter.flags.expired"),
        Some("fast_path,slow_path")
    );
    assert_eq!(events[0].field("batter.flags.expired.count"), Some("2"));

    // Past every expiry: all three release flags, never the mode flag.
    let events = resolve_on(CivilDate::new(2030, 1, 1).unwrap(), &capture);
    assert_eq!(
        events[0].field("batter.flags.expired"),
        Some("fast_path,new_checkout,slow_path")
    );
}

#[test]
fn no_flag_past_its_expiry_means_no_event() {
    let capture = Capture::new();
    assert!(resolve_on(CivilDate::new(2026, 6, 30).unwrap(), &capture).is_empty());
    assert!(resolve_on(CivilDate::new(2025, 1, 1).unwrap(), &capture).is_empty());
}

#[test]
fn expiry_never_changes_what_resolves_or_fails_startup() {
    let (catalog, serving, hosted) = expiring();
    let resolved = declared::resolve(
        catalog,
        &SettingsSource::from_pairs([("FLAG_SLOW_PATH".into(), "on".into())]).unwrap(),
        serving,
        hosted,
        CivilDate::new(2099, 1, 1).unwrap(),
    )
    .unwrap();
    // The flag is long expired and still serves what its setting says.
    let report = resolved.report();
    let slow = report
        .entries()
        .iter()
        .find(|entry| entry.key().as_str() == "slow_path")
        .unwrap();
    assert_eq!(slow.variant(), "on");
}

#[test]
fn the_event_names_keys_only_and_no_value() {
    let capture = Capture::new();
    let (catalog, serving, hosted) = expiring();
    let source = SettingsSource::from_pairs([
        ("FLAG_SLOW_PATH".into(), "on".into()),
        (
            "DATABASE_URL".into(),
            "postgres://user:s3cr3t-marker@host/db".into(),
        ),
    ])
    .unwrap();
    let resolved = capture
        .within(|| {
            declared::resolve(
                catalog,
                &source,
                serving,
                hosted,
                CivilDate::new(2099, 1, 1).unwrap(),
            )
        })
        .unwrap();
    drop(resolved);
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert!(!events[0].everything().contains("s3cr3t-marker"));
}

#[test]
fn a_clean_resolution_emits_nothing() {
    let capture = Capture::new();
    let (catalog, h) = world();
    let source = source_over(&required(As::ServingHosted), &[("FLAG_FAST_PATH", "on")]);
    let resolved = capture
        .within(|| declared::resolve(catalog, &source, h.serving, h.hosted, today()))
        .unwrap();
    assert!(capture.events().is_empty(), "{:?}", capture.events());
    drop(resolved);
}

#[test]
fn the_startup_view_evaluates_under_the_dispatcher_current_at_the_call() {
    let capture = Capture::new();
    let (resolved, h) = resolve_serving(&[("FLAG_FAST_PATH", "on")]).unwrap();
    let decision = capture.within(|| resolved.startup_view().evaluate(&h.fast_path));
    assert_eq!(decision.reason(), Reason::Static);
    let events = capture.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "feature_flag.evaluation");
    assert_eq!(events[0].level, Level::DEBUG);
    assert_eq!(events[0].field("feature_flag.key"), Some("fast_path"));
    assert_eq!(
        events[0].field("feature_flag.version").map(str::len),
        Some(64),
        "a declared decision's version is the revision's hex"
    );
}
