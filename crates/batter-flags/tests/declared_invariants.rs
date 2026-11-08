//! Invariants between flags: R-05.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

#[path = "support/world.rs"]
mod world;

use batter_core::settings::SettingsSource;
use batter_flags::{
    core::{CatalogBuilder, InvariantRule, SettingsPrefix, Toggle},
    declared::{self, DeclaredError},
};
use world::{resolve_serving, resolve_worker, today};

fn message(error: &DeclaredError) -> String {
    error.to_string()
}

#[test]
fn a_requires_invariant_between_flags_one_kind_owns_is_checked() {
    let error = resolve_worker(&[("FLAG_LEDGER_V2", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Invariant { rule: InvariantRule::Requires, flag, other, .. }
            if flag.as_str() == "ledger_v2" && other.as_str() == "ledger_audit"),
        "{error}"
    );
    assert_eq!(
        message(&error),
        "invariant broken: flag `ledger_v2` at `on` requires flag `ledger_audit` at `on`"
    );
    // The audit alone is allowed: the invariant runs one way.
    assert!(resolve_worker(&[("FLAG_LEDGER_AUDIT", "on")]).is_ok());
    // With both, the worker still owes `worker_sync`.
    let error =
        resolve_worker(&[("FLAG_LEDGER_V2", "on"), ("FLAG_LEDGER_AUDIT", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Invariant { other, .. } if other.as_str() == "worker_sync"),
        "{error}"
    );
    assert!(
        resolve_worker(&[
            ("FLAG_LEDGER_V2", "on"),
            ("FLAG_LEDGER_AUDIT", "on"),
            ("FLAG_WORKER_SYNC", "on"),
        ])
        .is_ok()
    );
}

#[test]
fn an_invariant_whose_other_flag_belongs_to_another_kind_is_not_checked() {
    // `serving` owns `ledger_v2` but not `worker_sync`: it cannot know what the
    // worker resolves, so it does not guess from the default. Residual,
    // recorded in the module's documentation.
    assert!(resolve_serving(&[("FLAG_LEDGER_V2", "on"), ("FLAG_LEDGER_AUDIT", "on")]).is_ok());
}

#[test]
fn an_invariant_is_judged_over_a_flag_restricted_to_another_class_at_its_default() {
    let build = || {
        let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
        let serving = builder.process_kind("serving").unwrap();
        let development = builder.environment_class("development").unwrap();
        let hosted = builder.environment_class("hosted").unwrap();
        let debug_panel = builder
            .mode_flag("debug_panel", Toggle::Off)
            .only_in(&development)
            .declare()
            .unwrap();
        let ledger_v2 = builder
            .mode_flag("ledger_v2", Toggle::Off)
            .declare()
            .unwrap();
        builder
            .requires(&ledger_v2, Toggle::On, &debug_panel, Toggle::On)
            .unwrap();
        (builder.build().unwrap(), serving, development, hosted)
    };
    let resolve = |class: &str, pairs: &[(&str, &str)]| {
        let (catalog, serving, development, hosted) = build();
        let class = if class == "hosted" {
            hosted
        } else {
            development
        };
        let source = SettingsSource::from_pairs(
            pairs
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into())),
        )
        .unwrap();
        declared::resolve(catalog, &source, serving, class, today())
    };
    // Hosted: `debug_panel` can never be set, so it evaluates off, and
    // `ledger_v2` cannot be on there.
    let error = resolve("hosted", &[("FLAG_LEDGER_V2", "on")]).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Invariant { flag, other, .. }
            if flag.as_str() == "ledger_v2" && other.as_str() == "debug_panel"),
        "{error}"
    );
    // Development: it holds once `debug_panel` is on, and not while it is off.
    assert!(
        resolve(
            "development",
            &[("FLAG_LEDGER_V2", "on"), ("FLAG_DEBUG_PANEL", "on")]
        )
        .is_ok()
    );
    assert!(matches!(
        resolve("development", &[("FLAG_LEDGER_V2", "on")]).unwrap_err(),
        DeclaredError::Invariant { .. }
    ));
}

#[test]
fn a_conflicts_invariant_fails_only_while_both_sides_hold() {
    let error =
        resolve_serving(&[("FLAG_LEGACY_EXPORT", "on"), ("FLAG_BULK_EXPORT", "on")]).unwrap_err();
    assert!(
        matches!(
            &error,
            DeclaredError::Invariant {
                rule: InvariantRule::Conflicts,
                ..
            }
        ),
        "{error}"
    );
    assert!(message(&error).contains("`legacy_export` at `on` conflicts with flag `bulk_export`"));
    assert!(resolve_serving(&[("FLAG_LEGACY_EXPORT", "on")]).is_ok());
    assert!(resolve_serving(&[("FLAG_BULK_EXPORT", "on")]).is_ok());
    assert!(resolve_serving(&[("FLAG_LEGACY_EXPORT", "on"), ("FLAG_BULK_EXPORT", "off")]).is_ok());
}

#[test]
fn a_both_or_none_invariant_fails_when_exactly_one_is_on() {
    for one in ["FLAG_SYNC_IN", "FLAG_SYNC_OUT"] {
        let error = resolve_serving(&[(one, "on")]).unwrap_err();
        assert!(
            matches!(
                &error,
                DeclaredError::Invariant {
                    rule: InvariantRule::BothOrNone,
                    ..
                }
            ),
            "{one}: {error}"
        );
    }
    assert!(
        message(&resolve_serving(&[("FLAG_SYNC_IN", "on")]).unwrap_err())
            .contains("both be on or both be off")
    );
    assert!(resolve_serving(&[("FLAG_SYNC_IN", "on"), ("FLAG_SYNC_OUT", "true")]).is_ok());
    assert!(resolve_serving(&[]).is_ok());
}

// R-06: a development-only flag outside its class.

#[test]
fn a_governed_flag_is_judged_at_its_default_while_its_switch_resolves_engaged() {
    let resolve = |safe: batter_flags::core::SafeState, pairs: &[(&str, &str)]| {
        let mut builder = CatalogBuilder::new(SettingsPrefix::new("FLAG_").unwrap());
        let serving = builder.process_kind("serving").unwrap();
        let hosted = builder.environment_class("hosted").unwrap();
        let halt = builder
            .kill_switch("halt", safe, batter_flags::core::StaleRule::HoldLastKnown)
            .declare()
            .unwrap();
        let governed = builder
            .release_flag(
                "governed",
                Toggle::Off,
                batter_flags::core::Expiry::on(
                    batter_flags::core::CivilDate::new(2027, 3, 31).unwrap(),
                ),
            )
            .governed_by(&halt)
            .declare()
            .unwrap();
        let dependent = builder
            .mode_flag("dependent", Toggle::Off)
            .declare()
            .unwrap();
        builder
            .requires(&dependent, Toggle::On, &governed, Toggle::On)
            .unwrap();
        let source = SettingsSource::from_pairs(
            pairs
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into())),
        )
        .unwrap();
        declared::resolve(builder.build().unwrap(), &source, serving, hosted, today())
    };
    let both = [("FLAG_DEPENDENT", "on"), ("FLAG_GOVERNED", "on")];
    let released = batter_flags::core::SafeState::Released;
    assert!(resolve(released, &both).is_ok());
    // Engaged by its literal, the switch serves `governed` at its default off.
    let engaged = [both[0], both[1], ("FLAG_HALT", "engaged")];
    let error = resolve(released, &engaged).unwrap_err();
    assert!(
        matches!(&error, DeclaredError::Invariant { flag, other, .. }
            if flag.as_str() == "dependent" && other.as_str() == "governed"),
        "{error}"
    );
    // Engaged by its safe state, with no literal, the same.
    assert!(resolve(batter_flags::core::SafeState::Engaged, &both).is_err());
}
