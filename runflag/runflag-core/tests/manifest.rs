//! The manifest: pinned canonical bytes, validating decode and the diff rule.
#![cfg(feature = "serde")]

#[path = "support/fixtures.rs"]
mod fixtures;

use fixtures::{Change, Checkout, CheckoutWide, expiry, reference, vocabulary_builder};
use runflag_core::{Catalog, Kind, Manifest, ManifestOwners, Toggle, ValueType, manifest_diff};
use serde_json::{Value, json};

/// The canonical bytes of the reference manifest, reviewed by hand.
const REFERENCE_MANIFEST: &str = include_str!("support/reference_manifest.json");
/// Computed independently over [`REFERENCE_MANIFEST`] in the documented framing.
const REFERENCE_MANIFEST_DIGEST: &str =
    "decc48bf9dbba6699db75daf43682138d9241844b26b70959a3c632b2e6d88df";

fn reference_value() -> Value {
    serde_json::from_str(REFERENCE_MANIFEST).unwrap()
}

fn decode(value: &Value) -> Result<Manifest, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

fn flag_mut<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    value["flags"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["key"] == key)
        .unwrap()
}

#[test]
fn the_reference_manifest_bytes_and_digest_are_pinned() {
    let catalog = reference(Change::None);
    let manifest = catalog.manifest();
    assert_eq!(
        String::from_utf8(manifest.to_canonical_json()).unwrap(),
        REFERENCE_MANIFEST.trim_end()
    );
    assert_eq!(manifest.digest().to_hex(), REFERENCE_MANIFEST_DIGEST);
    assert_eq!(catalog.manifest_digest(), manifest.digest());
    assert_eq!(manifest.catalog_digest(), catalog.digest());
}

#[test]
fn a_manifest_exports_every_declaration_for_tooling() {
    let manifest = reference(Change::None).manifest();
    assert_eq!(manifest.prefix().as_str(), "FLAG_");
    assert_eq!(manifest.process_kinds(), ["serving", "worker"]);
    assert_eq!(manifest.environment_classes(), ["development", "hosted"]);
    let entry = manifest.flag("new_checkout").unwrap();
    assert_eq!(entry.setting_name().as_str(), "FLAG_NEW_CHECKOUT");
    assert_eq!(
        (entry.kind(), entry.value_type()),
        (Kind::Release, ValueType::Toggle)
    );
    assert_eq!(
        entry.owners(),
        &ManifestOwners::Only(vec!["serving".to_owned()])
    );
    assert_eq!(entry.pinned_by(), ["worker"]);
    assert_eq!(entry.governed_by().unwrap().as_str(), "checkout_halt");
    assert_eq!(entry.expiry().unwrap().to_string(), "2027-03-31");
    assert_eq!(entry.requires_settings()[0].settings().len(), 2);
    assert_eq!(
        manifest.flag("debug_panel").unwrap().only_in(),
        Some("development")
    );
    assert_eq!(
        manifest.retired()[0].migration_note(),
        "set FLAG_NEW_CHECKOUT instead"
    );
    assert!(manifest.flag("missing").is_none());
}

#[test]
fn decoding_round_trips_and_normalizes_list_order() {
    let manifest = reference(Change::None).manifest();
    let decoded: Manifest = serde_json::from_str(REFERENCE_MANIFEST).unwrap();
    assert_eq!(decoded, manifest);
    let mut shuffled = reference_value();
    shuffled["flags"].as_array_mut().unwrap().reverse();
    shuffled["process_kinds"] = json!(["worker", "serving"]);
    flag_mut(&mut shuffled, "new_checkout")["variants"] = json!(["on", "off"]);
    shuffled["added_by_a_newer_build"] = json!(true);
    let decoded = decode(&shuffled).unwrap();
    assert_eq!(decoded, manifest);
    assert_eq!(decoded.digest().to_hex(), REFERENCE_MANIFEST_DIGEST);
}

#[test]
fn only_version_one_decodes() {
    let mut value = reference_value();
    value["v"] = json!(2);
    let error = decode(&value).unwrap_err();
    assert!(
        error.contains("manifest version 2 is not supported"),
        "{error}"
    );
    value.as_object_mut().unwrap().remove("v");
    assert!(decode(&value).is_err());
}

type Tamper = fn(&mut Value);

/// Decodes the reference manifest after each tamper and requires the refusal
/// message that names the broken rule.
fn assert_refused(cases: &[(&str, Tamper)]) {
    for (expected, tamper) in cases {
        let mut value = reference_value();
        tamper(&mut value);
        let error = decode(&value).expect_err(expected);
        assert!(
            error.contains("invalid manifest") && error.contains(expected),
            "{expected}: {error}"
        );
    }
}

#[test]
fn decoding_refuses_inconsistent_declarations() {
    let cases: [(&str, Tamper); 16] = [
        ("the fingerprint does not match", |v| {
            flag_mut(v, "fast_path")["fingerprint"] = json!("00".repeat(32));
        }),
        ("the catalog digest does not match", |v| {
            v["catalog_digest"] = json!("00".repeat(32));
        }),
        ("does not derive from the prefix", |v| {
            flag_mut(v, "fast_path")["setting_name"] = json!("FLAG_OTHER");
        }),
        ("a kill switch carries a field", |v| {
            flag_mut(v, "checkout_halt")["variants"] = json!(["on"]);
        }),
        ("needs a safe state and a stale rule", |v| {
            flag_mut(v, "checkout_halt")["stale_rule"] = Value::Null;
        }),
        ("only a release flag, has an expiry", |v| {
            flag_mut(v, "fast_path")["expiry"] = Value::Null;
        }),
        ("only a release flag, has an expiry", |v| {
            flag_mut(v, "debug_panel")["expiry"] = json!("2027-01-01");
        }),
        ("a toggle has exactly the variants", |v| {
            flag_mut(v, "debug_panel")["variants"] = json!(["off", "on", "auto"]);
        }),
        ("the default is not a declared variant", |v| {
            flag_mut(v, "checkout_style")["default"] = json!("instant");
        }),
        ("kind is not declared", |v| {
            flag_mut(v, "export_halt")["owners"] = json!({ "only": ["reporting"] });
        }),
        ("only by a kill switch", |v| {
            flag_mut(v, "new_checkout")["governed_by"] = json!("fast_path");
        }),
        ("names outside the prefix", |v| {
            flag_mut(v, "new_checkout")["requires_settings"] =
                json!([{ "variant": "on", "settings": ["FLAG_X"] }]);
        }),
        ("is listed twice", |v| {
            let first = v["flags"][0].clone();
            v["flags"].as_array_mut().unwrap().push(first);
        }),
        ("a pinned_by kind is listed twice", |v| {
            flag_mut(v, "new_checkout")["pinned_by"] = json!(["worker", "worker"]);
        }),
        ("requires settings for one variant twice", |v| {
            let entry = flag_mut(v, "new_checkout");
            let required = entry["requires_settings"][0].clone();
            entry["requires_settings"]
                .as_array_mut()
                .unwrap()
                .push(required);
        }),
        ("an invariant is listed twice", |v| {
            let entry = flag_mut(v, "checkout_style");
            let invariant = entry["invariants"][0].clone();
            entry["invariants"].as_array_mut().unwrap().push(invariant);
        }),
    ];
    assert_refused(&cases);
    let mut value = reference_value();
    flag_mut(&mut value, "fast_path")["key"] = json!("Fast");
    assert!(
        decode(&value).is_err(),
        "names decode through their validating constructors"
    );
}

#[test]
fn decoding_refuses_manifests_no_builder_could_produce() {
    let cases: [(&str, Tamper); 7] = [
        ("needs at least one process kind", |v| {
            v["process_kinds"] = json!([]);
        }),
        ("needs at least one environment class", |v| {
            v["environment_classes"] = json!([]);
        }),
        (
            "setting name FLAG_FAST_PATH is retired and also in use",
            |v| {
                let retired = json!({ "name": "FLAG_FAST_PATH", "migration_note": "gone" });
                v["retired"].as_array_mut().unwrap().push(retired);
            },
        ),
        (
            "setting name PAYMENT_API_URL is retired and also in use",
            |v| {
                let retired = json!({ "name": "PAYMENT_API_URL", "migration_note": "gone" });
                v["retired"].as_array_mut().unwrap().push(retired);
            },
        ),
        (
            "\"worker\" owns or pins the flag but not its governing switch",
            |v| {
                flag_mut(v, "checkout_halt")["owners"] = json!({ "only": ["serving"] });
            },
        ),
        (
            "owned by every process kind but its governing switch",
            |v| {
                flag_mut(v, "checkout_halt")["owners"] = json!({ "only": ["serving", "worker"] });
                let entry = flag_mut(v, "new_checkout");
                entry["owners"] = json!("all");
                entry["pinned_by"] = json!([]);
            },
        ),
        (
            "the code defaults break its invariant on new_checkout",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["variant"] = json!("classic");
            },
        ),
    ];
    assert_refused(&cases);
}

fn catalog_with(declare: impl FnOnce(&mut runflag_core::CatalogBuilder)) -> Catalog {
    let mut builder = vocabulary_builder();
    builder.mode_flag("steady", Toggle::Off).declare().unwrap();
    declare(&mut builder);
    builder.build().unwrap()
}

#[test]
fn a_kind_or_value_type_change_is_a_new_key() {
    let release = catalog_with(|b| {
        drop(
            b.release_flag("checkout", Checkout::Classic, expiry())
                .declare()
                .unwrap(),
        );
    });
    let mode = catalog_with(|b| {
        drop(
            b.mode_flag("checkout", Checkout::Classic)
                .declare()
                .unwrap(),
        );
    });
    let toggle = catalog_with(|b| {
        drop(
            b.release_flag("checkout", Toggle::Off, expiry())
                .declare()
                .unwrap(),
        );
    });
    let switch = catalog_with(|b| {
        drop(
            b.kill_switch(
                "checkout",
                runflag_core::SafeState::Released,
                runflag_core::StaleRule::HoldLastKnown,
            )
            .declare()
            .unwrap(),
        );
    });
    for (current, kind, value_type) in [
        (&mode, Kind::Mode, ValueType::Enum),
        (&toggle, Kind::Release, ValueType::Toggle),
        (&switch, Kind::Switch, ValueType::Switch),
    ] {
        let error = manifest_diff(&release.manifest(), &current.manifest()).unwrap_err();
        let retype = &error.retyped()[0];
        assert_eq!(retype.key().as_str(), "checkout");
        assert_eq!(retype.previous(), (Kind::Release, ValueType::Enum));
        assert_eq!(retype.current(), (kind, value_type));
        assert!(
            error.to_string().contains("checkout (release enum ->"),
            "{error}"
        );
    }
}

#[test]
fn a_new_variant_owners_or_expiry_is_a_compatible_change() {
    let narrow = catalog_with(|b| {
        drop(
            b.mode_flag("checkout", Checkout::Classic)
                .declare()
                .unwrap(),
        );
    });
    let wide = catalog_with(|b| {
        drop(
            b.mode_flag("checkout", CheckoutWide::Classic)
                .declare()
                .unwrap(),
        );
    });
    let diff = manifest_diff(&narrow.manifest(), &wide.manifest()).unwrap();
    assert_eq!(diff.changed().len(), 1);
    assert_eq!(diff.changed()[0].as_str(), "checkout");
    assert!(diff.added().is_empty() && diff.removed().is_empty());
    let back = manifest_diff(&wide.manifest(), &narrow.manifest()).unwrap();
    assert_eq!(
        back.changed()[0].as_str(),
        "checkout",
        "removing a variant is not a re-type"
    );

    let base = reference(Change::None).manifest();
    let later = reference(Change::NewCheckoutExpiry(fixtures::date(2030, 1, 1)));
    let diff = manifest_diff(&base, &later.manifest()).unwrap();
    assert_eq!(
        diff.changed()
            .iter()
            .map(runflag_core::FlagKey::as_str)
            .collect::<Vec<_>>(),
        ["new_checkout"]
    );
    assert!(manifest_diff(&base, &base).unwrap().is_empty());
}

#[test]
fn added_and_removed_keys_are_listed() {
    let before = catalog_with(|b| drop(b.mode_flag("old_mode", Toggle::Off).declare().unwrap()));
    let after = catalog_with(|b| drop(b.mode_flag("new_mode", Toggle::Off).declare().unwrap()));
    let diff = manifest_diff(&before.manifest(), &after.manifest()).unwrap();
    assert_eq!(diff.added()[0].as_str(), "new_mode");
    assert_eq!(diff.removed()[0].as_str(), "old_mode");
    assert!(diff.changed().is_empty());
    assert!(!diff.is_empty());
}
