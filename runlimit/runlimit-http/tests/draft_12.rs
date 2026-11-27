//! Golden `draft-ietf-httpapi-ratelimit-headers-12` fields.
//!
//! The expected values are the draft's own examples where it gives one, with
//! the policy identifier always encoded as a String item.

#[path = "fixtures/decisions.rs"]
mod fixtures;

use std::time::Duration;

use fixtures::{allowance, fixed_policy, gcra_policy, quota};
use runlimit_core::Admitted;
use runlimit_http::{QuotaState, draft_11, draft_12};

fn service_value(name: &str, state: impl Into<QuotaState>) -> String {
    let (field, value) = draft_12::service_limit(name, state).unwrap();
    assert_eq!(field.as_str(), "ratelimit");
    value.to_str().unwrap().to_owned()
}

#[test]
fn encodes_the_draft_12_fixed_window_example() {
    // -12 appendix A.2.1: 99 quota units left for the next 50 seconds of a
    // 100-per-minute policy.
    let policy = fixed_policy(100, Duration::from_mins(1));
    let (field, value) = draft_12::quota_policy("fixedwindow", &policy).unwrap();

    assert_eq!(field.as_str(), "ratelimit-policy");
    assert_eq!(value, "\"fixedwindow\";q=100;w=60");
    assert_eq!(
        service_value("fixedwindow", allowance(100, 99, Duration::from_secs(50))),
        "\"fixedwindow\";a=99;w=50"
    );
}

#[test]
fn encodes_the_draft_12_concurrency_and_throttled_examples() {
    // -12 appendix A.1.3, after 40 of 100 units in the first two seconds.
    assert_eq!(
        service_value("basic", allowance(100, 60, Duration::from_secs(58))),
        "\"basic\";a=60;w=58"
    );
    // -12 appendix A.1.4: an exhausted quota in a throttled response. The
    // backend's 4.2 s retry delay rounds up, so `w` never ends early.
    assert_eq!(
        service_value("default", quota(100, Duration::from_millis(4_200))),
        "\"default\";a=0;w=5"
    );
}

#[test]
fn uses_algorithm_neutral_gcra_policy_metadata() {
    let policy = gcra_policy(10, Duration::from_mins(1));

    assert_eq!(
        draft_12::quota_policy("upload", &policy).unwrap().1,
        "\"upload\";q=10;w=60"
    );
}

#[test]
fn exhausted_and_admitted_outcomes_encode_one_service_value() {
    let available = allowance(10, 4, Duration::from_millis(500));
    let denial = quota(10, Duration::from_millis(1_001));

    assert_eq!(service_value("default", denial), "\"default\";a=0;w=2");
    assert_eq!(
        service_value("default", QuotaState::Exhausted(denial)),
        "\"default\";a=0;w=2"
    );
    assert_eq!(
        service_value("default", Admitted::allowed(available)),
        "\"default\";a=4;w=1"
    );
    assert_eq!(
        service_value("default", Admitted::shadow_denied(denial)),
        "\"default\";a=0;w=2"
    );
}

#[test]
fn rounds_effective_windows_up_to_whole_seconds() {
    assert_eq!(
        service_value("default", allowance(10, 9, Duration::from_nanos(1))),
        "\"default\";a=9;w=1"
    );
    assert_eq!(
        service_value("default", allowance(10, 9, Duration::from_secs(37))),
        "\"default\";a=9;w=37"
    );
    assert_eq!(
        service_value("default", quota(10, Duration::from_millis(36_001))),
        "\"default\";a=0;w=37"
    );
    // -12 section 3.1.2 allows a zero effective window.
    assert_eq!(
        service_value("default", allowance(10, 10, Duration::ZERO)),
        "\"default\";a=10;w=0"
    );
}

#[test]
fn draft_12_differs_from_draft_11_only_in_the_service_limit_keys() {
    let policy = fixed_policy(100, Duration::from_mins(1));
    assert_eq!(
        draft_12::quota_policy("search", &policy),
        draft_11::quota_policy("search", &policy),
    );

    let states = [
        QuotaState::from(allowance(100, 49, Duration::from_millis(36_500))),
        QuotaState::from(allowance(100, 0, Duration::ZERO)),
        QuotaState::from(quota(100, Duration::from_nanos(1))),
    ];
    for state in states {
        let draft_11 = draft_11::service_limit("search", state).unwrap();
        let draft_12 = draft_12::service_limit("search", state).unwrap();
        assert_eq!(draft_11.0, draft_12.0);
        let respelled = draft_11
            .1
            .to_str()
            .unwrap()
            .replace(";r=", ";a=")
            .replace(";t=", ";w=");
        assert_eq!(draft_12.1, respelled.as_str(), "{state:?}");
    }
}
