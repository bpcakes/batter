//! The reference manifest as JSON, and helpers for decoding a tampered copy.
#![allow(dead_code, reason = "shared support; each test crate uses part of it")]

use batter_flags_core::Manifest;
use serde_json::Value;

/// The canonical bytes of the reference manifest, reviewed by hand.
pub const REFERENCE_MANIFEST: &str = include_str!("reference_manifest.json");

/// A change to the reference manifest that one decode check must refuse.
pub type Tamper = fn(&mut Value);

pub fn reference_value() -> Value {
    serde_json::from_str(REFERENCE_MANIFEST).unwrap()
}

pub fn decode(value: &Value) -> Result<Manifest, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

pub fn flag_mut<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    value["flags"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["key"] == key)
        .unwrap()
}

/// Decodes the reference manifest after each tamper and requires the refusal
/// message that names the broken rule.
pub fn assert_refused(cases: &[(&str, Tamper)]) {
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
