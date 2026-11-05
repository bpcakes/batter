//! Manifest decoding: every tampered reference manifest is refused with the
//! message of the rule it breaks.
#![cfg(feature = "serde")]

#[path = "support/manifest_json.rs"]
mod manifest_json;

use manifest_json::{Tamper, assert_refused, decode, flag_mut, reference_value};
use serde_json::{Value, json};

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
    let cases: [(&str, Tamper); 9] = [
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
            "both_or_none invariant on new_checkout must relate both toggles at on",
            |v| {
                flag_mut(v, "fast_path")["invariants"][0]["variant"] = json!("off");
            },
        ),
        (
            "both_or_none invariant on new_checkout must relate both toggles at on",
            |v| {
                let invariant = &mut flag_mut(v, "fast_path")["invariants"][0];
                invariant["variant"] = json!("off");
                invariant["other_variant"] = json!("off");
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

#[test]
fn decoding_refuses_malformed_vocabularies_retired_names_and_ownership() {
    let cases: [(&str, Tamper); 11] = [
        ("process kind \"serving\" is listed twice", |v| {
            v["process_kinds"] = json!(["serving", "serving", "worker"]);
        }),
        ("process kind \"Serving\"", |v| {
            v["process_kinds"] = json!(["Serving", "worker"]);
        }),
        ("environment class \"hosted\" is listed twice", |v| {
            v["environment_classes"] = json!(["development", "hosted", "hosted"]);
        }),
        ("retired name ENABLE_NEW_CHECKOUT is listed twice", |v| {
            let retired = v["retired"][0].clone();
            v["retired"].as_array_mut().unwrap().push(retired);
        }),
        (
            "retired name ENABLE_NEW_CHECKOUT has no migration note",
            |v| {
                v["retired"][0]["migration_note"] = json!("   ");
            },
        ),
        ("owners must be a non-empty list of distinct kinds", |v| {
            flag_mut(v, "export_halt")["owners"] = json!({ "only": [] });
        }),
        ("owners must be a non-empty list of distinct kinds", |v| {
            flag_mut(v, "export_halt")["owners"] = json!({ "only": ["worker", "worker"] });
        }),
        ("pinned_by kinds must be distinct non-owners", |v| {
            flag_mut(v, "new_checkout")["pinned_by"] = json!(["serving"]);
        }),
        ("pinned_by kinds must be distinct non-owners", |v| {
            flag_mut(v, "fast_path")["pinned_by"] = json!(["worker"]);
        }),
        ("an owner or pinned_by kind is not declared", |v| {
            flag_mut(v, "new_checkout")["pinned_by"] = json!(["reporting"]);
        }),
        ("environment class \"staging\" is not declared", |v| {
            flag_mut(v, "debug_panel")["only_in"] = json!("staging");
        }),
    ];
    assert_refused(&cases);
}

#[test]
fn decoding_refuses_malformed_flag_fields_and_invariants() {
    let cases: [(&str, Tamper); 13] = [
        ("a kill switch has the value type switch", |v| {
            flag_mut(v, "checkout_halt")["value_type"] = json!("toggle");
        }),
        ("only a kill switch has the value type switch", |v| {
            flag_mut(v, "fast_path")["value_type"] = json!("switch");
        }),
        ("only a kill switch has a safe state or a stale rule", |v| {
            flag_mut(v, "fast_path")["safe_state"] = json!("engaged");
        }),
        ("a flag needs at least one variant", |v| {
            flag_mut(v, "checkout_style")["variants"] = json!([]);
        }),
        ("variant \"classic\" is invalid or listed twice", |v| {
            flag_mut(v, "checkout_style")["variants"] = json!(["classic", "classic", "express"]);
        }),
        ("variant \"Classic\" is invalid or listed twice", |v| {
            flag_mut(v, "checkout_style")["variants"] = json!(["Classic", "express"]);
        }),
        (
            "settings are required by an undeclared or default variant",
            |v| {
                flag_mut(v, "new_checkout")["requires_settings"][0]["variant"] = json!("off");
            },
        ),
        (
            "settings are required by an undeclared or default variant",
            |v| {
                flag_mut(v, "new_checkout")["requires_settings"][0]["variant"] = json!("maybe");
            },
        ),
        (
            "an invariant names checkout_style which is not another flag",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["other"] = json!("checkout_style");
            },
        ),
        (
            "an invariant names checkout_halt which is not another flag",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["other"] = json!("checkout_halt");
            },
        ),
        (
            "the invariant on new_checkout names an undeclared variant",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["variant"] = json!("instant");
            },
        ),
        (
            "the invariant on new_checkout names an undeclared variant",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["other_variant"] = json!("maybe");
            },
        ),
        (
            "the invariant on new_checkout names an undeclared variant",
            |v| {
                flag_mut(v, "checkout_style")["invariants"][0]["rule"] = json!("both_or_none");
            },
        ),
    ];
    assert_refused(&cases);
}
