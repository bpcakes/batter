//! Golden `draft-ietf-httpapi-ratelimit-headers-11` fields.
//!
//! `-11` is expired but stays selectable for clients that parse it, so these
//! values pin its wire format byte for byte.

#[path = "fixtures/decisions.rs"]
mod fixtures;

use std::time::Duration;

use fixtures::{allowance, fixed_policy, gcra_policy, quota};
use runlimit_core::Admitted;
use runlimit_http::{QuotaState, draft_11};

fn service_value(state: impl Into<QuotaState>) -> String {
    let (name, value) = draft_11::service_limit("default", state).unwrap();
    assert_eq!(name.as_str(), "ratelimit");
    value.to_str().unwrap().to_owned()
}

#[test]
fn encodes_exact_draft_11_golden_fields() {
    let policy = fixed_policy(100, Duration::from_mins(1));
    let policy_field = draft_11::quota_policy("search", &policy).unwrap();
    let service_field =
        draft_11::service_limit("search", allowance(100, 49, Duration::from_secs(37))).unwrap();

    assert_eq!(policy_field.0.as_str(), "ratelimit-policy");
    assert_eq!(policy_field.1, "\"search\";q=100;w=60");
    assert_eq!(service_field.0.as_str(), "ratelimit");
    assert_eq!(service_field.1, "\"search\";r=49;t=37");
}

#[test]
fn uses_algorithm_neutral_gcra_policy_metadata() {
    let policy = gcra_policy(10, Duration::from_mins(1));

    assert_eq!(
        draft_11::quota_policy("upload", &policy).unwrap().1,
        "\"upload\";q=10;w=60"
    );
}

#[test]
fn exhausted_quota_is_an_r_zero_service_value() {
    let denial = quota(10, Duration::from_millis(1_001));

    assert_eq!(service_value(denial), "\"default\";r=0;t=2");
    assert_eq!(
        service_value(QuotaState::Exhausted(denial)),
        "\"default\";r=0;t=2"
    );
}

#[test]
fn admitted_outcomes_encode_without_a_decision_round_trip() {
    let allowance = allowance(10, 4, Duration::from_millis(500));
    let denial = quota(10, Duration::from_millis(1_001));

    assert_eq!(
        service_value(Admitted::allowed(allowance)),
        "\"default\";r=4;t=1"
    );
    assert_eq!(
        service_value(Admitted::shadow_denied(denial)),
        "\"default\";r=0;t=2"
    );
}

#[test]
fn rounds_effective_windows_up_to_whole_seconds() {
    assert_eq!(
        service_value(allowance(10, 9, Duration::from_nanos(1))),
        "\"default\";r=9;t=1"
    );
    assert_eq!(
        service_value(allowance(10, 9, Duration::from_secs(37))),
        "\"default\";r=9;t=37"
    );
    assert_eq!(
        service_value(quota(10, Duration::from_millis(36_001))),
        "\"default\";r=0;t=37"
    );
    assert_eq!(
        service_value(allowance(10, 10, Duration::ZERO)),
        "\"default\";r=10;t=0"
    );
}
