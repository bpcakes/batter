//! The rules a catalog declares: R-02, R-04, R-05, R-06, R-07 and R-14, and a
//! kind or class that is not the catalog's.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use batter_core::settings::SettingsSource;
use batter_flags::{
    core::{CatalogBuilder, SafeState, SettingsPrefix, StaleRule, Toggle},
    declared::{self, DeclaredError},
};
use world::{
    As, resolve_as, resolve_development, resolve_serving, resolve_worker, source_over, today, world,
};

fn message(error: &DeclaredError) -> String {
    error.to_string()
}

// R-02: absence is a closed state, or the flag is required where a value may
// be presented.

#[test]
fn a_required_flag_must_be_present_where_a_value_may_be_presented() {
    let (catalog, h) = world();
    let empty = SettingsSource::from_pairs([]).unwrap();
    let error = declared::resolve(catalog, &empty, h.serving, h.hosted, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Missing { key, name }
            if key.as_str() == "region_lock" && name.as_str() == "FLAG_REGION_LOCK"),
        "{error}"
    );
    assert_eq!(
        message(&error),
        "setting FLAG_REGION_LOCK for flag `region_lock` is required and absent"
    );
}

#[test]
fn a_required_flag_is_judged_only_in_owning_kinds_and_inside_its_class() {
    // `worker_region` is required of the worker alone: serving does not owe it.
    assert!(resolve_serving(&[]).is_ok());
    let (catalog, h) = world();
    let only_region = source_over(&[("FLAG_REGION_LOCK", "off")], &[]);
    let error = declared::resolve(catalog, &only_region, h.worker, h.hosted, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Missing { key, .. } if key.as_str() == "worker_region"),
        "{error}"
    );
    // `dev_tools` is required only inside `development`.
    let (catalog, h) = world();
    let error =
        declared::resolve(catalog, &only_region, h.serving, h.development, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Missing { key, .. } if key.as_str() == "dev_tools"),
        "{error}"
    );
    assert!(resolve_serving(&[]).is_ok(), "hosted owes no `dev_tools`");
}

#[test]
fn a_required_kill_switch_must_be_present_and_is_never_inferred() {
    let build = || {
        let (mut builder, serving, hosted) =
            CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap(), "serving", "hosted")
                .unwrap();
        builder
            .kill_switch(
                "payments_halt",
                SafeState::Released,
                StaleRule::HoldLastKnown,
            )
            .required()
            .declare()
            .unwrap();
        (builder.build(), serving, hosted)
    };
    let (catalog, serving, hosted) = build();
    let empty = SettingsSource::from_pairs([]).unwrap();
    let error = declared::resolve(catalog, &empty, serving, hosted, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Missing { key, name }
            if key.as_str() == "payments_halt" && name.as_str() == "FLAG_PAYMENTS_HALT"),
        "{error}"
    );
    let (catalog, serving, hosted) = build();
    let released =
        SettingsSource::from_pairs([("FLAG_PAYMENTS_HALT".into(), "released".into())]).unwrap();
    assert!(declared::resolve(catalog, &released, serving, hosted, today()).is_ok());
}

#[test]
fn a_required_flag_that_is_present_resolves_like_any_other() {
    let (resolved, h) = resolve_serving(&[("FLAG_REGION_LOCK", "on")]).unwrap();
    assert_eq!(
        resolved.startup_view().evaluate(&h.region_lock).value(),
        Toggle::On
    );
}

// R-04: an enabling variant declares the settings it requires.

#[test]
fn an_enabling_variant_needs_its_settings_present_and_non_empty() {
    let error = resolve_serving(&[("FLAG_NEW_CHECKOUT", "on")]).unwrap_err();
    match &error {
        DeclaredError::RequiredSettings {
            key,
            variant,
            names,
        } => {
            assert_eq!((key.as_str(), *variant), ("new_checkout", "on"));
            let names: Vec<&str> = names.iter().map(|name| name.as_str()).collect();
            assert_eq!(names, ["PAYMENT_API_REGION", "PAYMENT_API_URL"]);
        }
        other => panic!("expected RequiredSettings, found {other}"),
    }
    // Present but empty is absent.
    let error = resolve_serving(&[
        ("FLAG_NEW_CHECKOUT", "on"),
        ("PAYMENT_API_URL", ""),
        ("PAYMENT_API_REGION", "eu"),
    ])
    .unwrap_err();
    assert!(
        matches!(&error, DeclaredError::RequiredSettings { names, .. }
            if names.len() == 1 && names[0].as_str() == "PAYMENT_API_URL"),
        "{error}"
    );
    // Both present and non-empty: the flag resolves.
    let (resolved, h) = resolve_serving(&[
        ("FLAG_NEW_CHECKOUT", "on"),
        ("PAYMENT_API_URL", "https://payments.example"),
        ("PAYMENT_API_REGION", "eu"),
    ])
    .unwrap();
    assert_eq!(
        resolved.startup_view().evaluate(&h.new_checkout).value(),
        Toggle::On
    );
}

#[test]
fn the_requirement_is_read_from_the_variant_the_flag_resolved_to() {
    // `off` needs nothing, and the enabling variant's settings are not
    // inferred from being present: enablement is the flag's own value.
    let (resolved, h) = resolve_serving(&[
        ("PAYMENT_API_URL", "https://payments.example"),
        ("PAYMENT_API_REGION", "eu"),
    ])
    .unwrap();
    assert_eq!(
        resolved.startup_view().evaluate(&h.new_checkout).value(),
        Toggle::Off
    );
    assert!(resolve_serving(&[("FLAG_NEW_CHECKOUT", "off")]).is_ok());
}

#[test]
fn a_kind_that_does_not_own_the_flag_owes_no_settings() {
    // The worker replays pins of `new_checkout` and never evaluates it.
    assert!(resolve_worker(&[]).is_ok());
}

// R-05: invariants between flags.

/// A flag restricted to another class is not set in this process, but it is
/// still evaluated, at its code default: an invariant that depends on it is
/// judged over that default, not skipped.
#[test]
fn a_development_only_flag_fails_outside_its_class() {
    let error = resolve_serving(&[("FLAG_DEBUG_PANEL", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::OutsideClass { key, class, .. }
            if key.as_str() == "debug_panel" && class == "development"),
        "{error}"
    );
    // The value is irrelevant: even `off` presents a value where none may be.
    assert!(matches!(
        resolve_serving(&[("FLAG_DEBUG_PANEL", "off")]).unwrap_err(),
        DeclaredError::OutsideClass { .. }
    ));
    // Inside the class it resolves; absent it is the default everywhere.
    let (resolved, h) = resolve_development(&[("FLAG_DEBUG_PANEL", "on")]).unwrap();
    assert_eq!(
        resolved.startup_view().evaluate(&h.debug_panel).value(),
        Toggle::On
    );
    assert!(resolve_serving(&[]).is_ok());
}

// R-07: a value for a kind that does not own the flag.

#[test]
fn a_value_for_a_flag_the_kind_does_not_own_fails() {
    let error = resolve_serving(&[("FLAG_WORKER_MODE", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::NotOwned { key, kind, .. }
            if key.as_str() == "worker_mode" && kind == "serving"),
        "{error}"
    );
    assert!(resolve_worker(&[("FLAG_WORKER_MODE", "on")]).is_ok());
    // A kill switch the kind does not own.
    assert!(matches!(
        resolve_serving(&[("FLAG_EXPORT_HALT", "engaged")]).unwrap_err(),
        DeclaredError::NotOwned { .. }
    ));
    assert!(resolve_worker(&[("FLAG_EXPORT_HALT", "released")]).is_ok());
}

#[test]
fn a_pinning_kind_does_not_own_the_flag_it_replays() {
    // `new_checkout` is `pinned_by` the worker: a pin is replayed there, but a
    // value is never presented.
    let error = resolve_worker(&[("FLAG_NEW_CHECKOUT", "off")]).unwrap_err();
    assert!(matches!(error, DeclaredError::NotOwned { .. }), "{error}");
}

#[test]
fn ownership_is_judged_before_the_value_is_read() {
    // A blank value for a flag the kind does not own reports what the operator
    // has to fix: where the value was put.
    assert!(matches!(
        resolve_serving(&[("FLAG_WORKER_MODE", "")]).unwrap_err(),
        DeclaredError::NotOwned { .. }
    ));
    assert!(matches!(
        resolve_serving(&[("FLAG_DEBUG_PANEL", "maybe")]).unwrap_err(),
        DeclaredError::OutsideClass { .. }
    ));
}

// R-14: retired names.

#[test]
fn a_retired_name_fails_with_its_migration_note() {
    let error = resolve_serving(&[("FLAG_OLD_CHECKOUT", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Retired { name, migration_note }
            if name.as_str() == "FLAG_OLD_CHECKOUT" && migration_note == "use FLAG_NEW_CHECKOUT instead"),
        "{error}"
    );
    assert_eq!(
        message(&error),
        "retired setting FLAG_OLD_CHECKOUT is present: use FLAG_NEW_CHECKOUT instead"
    );
}

#[test]
fn a_retired_name_outside_the_prefix_fails_whatever_its_value() {
    for value in ["1", "", "anything"] {
        let error = resolve_serving(&[("LEGACY_FEATURE_SWITCH", value)]).unwrap_err();
        assert!(
            matches!(&error, DeclaredError::Retired { name, .. } if name.as_str() == "LEGACY_FEATURE_SWITCH"),
            "{error}"
        );
    }
}

// A kind or class that is not the catalog's.

#[test]
fn a_process_kind_from_another_catalog_is_refused() {
    let (catalog, h) = world();
    let (_other, foreign) = world();
    let source = source_over(&world::required(As::ServingHosted), &[]);
    let error =
        declared::resolve(catalog, &source, foreign.serving, h.hosted, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::ForeignKind { kind } if kind == "serving"),
        "{error}"
    );
    assert_eq!(
        message(&error),
        "process kind `serving` belongs to another catalog"
    );
}

#[test]
fn an_environment_class_from_another_catalog_is_refused() {
    let (catalog, h) = world();
    let (_other, foreign) = world();
    let source = source_over(&world::required(As::ServingHosted), &[]);
    let error =
        declared::resolve(catalog, &source, h.serving, foreign.hosted, today()).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::ForeignClass { class } if class == "hosted"),
        "{error}"
    );
}

#[test]
fn the_names_resolution_reads_are_declared_and_retired_ones_in_name_order() {
    let (resolved, _) = resolve_as(As::ServingHosted, &[]).unwrap();
    let names: Vec<String> = resolved
        .setting_names()
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    for expected in [
        "FLAG_CHECKOUT_HALT",
        "FLAG_FAST_PATH",
        "FLAG_OLD_CHECKOUT",
        "LEGACY_FEATURE_SWITCH",
        "FLAG_REGION_LOCK",
    ] {
        assert!(names.iter().any(|name| name == expected), "{expected}");
    }
    // The settings a variant requires are not flag settings.
    assert!(!names.iter().any(|name| name.starts_with("PAYMENT_")));
}
