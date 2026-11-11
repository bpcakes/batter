//! The manifest: pinned canonical bytes and decoding round trips.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]
#![cfg(feature = "serde")]

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/manifest_json.rs"]
mod manifest_json;

use batter_flags_core::{Kind, Manifest, ManifestOwners, ValueType};
use fixtures::{Change, reference};
use manifest_json::{REFERENCE_MANIFEST, decode, flag_mut, reference_value};
use serde_json::json;

/// Computed independently over [`REFERENCE_MANIFEST`] in the documented framing.
const REFERENCE_MANIFEST_DIGEST: &str =
    "8db7fc16f32be7147da10236408c28e370286204b22407097956c94b8ad21380";

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
