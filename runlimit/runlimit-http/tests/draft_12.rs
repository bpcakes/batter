//! Golden `draft-ietf-httpapi-ratelimit-headers-12` fields.
//!
//! The expected values follow the draft's own examples where it gives one,
//! with the policy identifier always encoded as a String item. `RateLimit`
//! states only an allowance's available quota: no decision states the
//! effective window `w`, and a denial does not state the quota that remains.

#[path = "fixtures/decisions.rs"]
mod fixtures;

use std::time::Duration;

use fixtures::{allowance, fixed_policy, gcra_policy, quota};
use runlimit_core::{Admitted, AdmittedView, Allowance};
use runlimit_http::{draft_11, draft_12};

fn service_value(name: &str, allowance: Allowance) -> String {
    let (field, value) = draft_12::service_limit(name, allowance).unwrap();
    assert_eq!(field.as_str(), "ratelimit");
    value.to_str().unwrap().to_owned()
}

#[test]
fn encodes_the_draft_12_fixed_window_example() {
    // -12 appendix A.2.1: 99 quota units left of a 100-per-minute policy. The
    // example's `w=50` is the effective window, which no decision states.
    let policy = fixed_policy(100, Duration::from_mins(1));
    let (field, value) = draft_12::quota_policy("fixedwindow", &policy).unwrap();

    assert_eq!(field.as_str(), "ratelimit-policy");
    assert_eq!(value, "\"fixedwindow\";q=100;w=60");
    assert_eq!(
        service_value("fixedwindow", allowance(100, 99, Duration::from_secs(50))),
        "\"fixedwindow\";a=99"
    );
}

#[test]
fn states_the_available_quota_exactly() {
    // -12 appendix A.1.3, after 40 of 100 units in the first two seconds.
    assert_eq!(
        service_value("basic", allowance(100, 60, Duration::from_secs(58))),
        "\"basic\";a=60"
    );
    assert_eq!(
        service_value("basic", allowance(100, 0, Duration::ZERO)),
        "\"basic\";a=0"
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
fn a_drained_gcra_allowance_states_no_window_it_cannot_keep() {
    // A GCRA allowance's delay is the time until the whole burst is back, but
    // one unit returns after a hundredth of it, so `w=100` would make a
    // client pacing on it wait a hundred times longer than it must.
    let drained = allowance(100, 0, Duration::from_secs(100));

    assert_eq!(service_value("upload", drained), "\"upload\";a=0");
}

#[test]
fn an_admitted_allowance_encodes_and_a_shadow_denial_has_no_field() {
    let encode = |admitted: Admitted| match admitted.view() {
        AdmittedView::Allowed { allowance } => Some(service_value("default", allowance)),
        AdmittedView::ShadowDenied { .. } => None,
    };

    assert_eq!(
        encode(Admitted::allowed(allowance(
            10,
            4,
            Duration::from_millis(500)
        )))
        .as_deref(),
        Some("\"default\";a=4")
    );
    assert_eq!(
        encode(Admitted::shadow_denied(quota(10, Duration::from_secs(1)))),
        None
    );
}

#[test]
fn draft_12_states_draft_11_s_available_quota_without_its_window() {
    let policy = fixed_policy(100, Duration::from_mins(1));
    assert_eq!(
        draft_12::quota_policy("search", &policy),
        draft_11::quota_policy("search", &policy),
    );

    for available in [
        allowance(100, 49, Duration::from_millis(36_500)),
        allowance(100, 0, Duration::ZERO),
    ] {
        let draft_11 = draft_11::service_limit("search", available).unwrap();
        let draft_12 = draft_12::service_limit("search", available).unwrap();
        assert_eq!(draft_11.0, draft_12.0);
        let draft_11 = draft_11.1.to_str().unwrap();
        let without_window = draft_11.split(";t=").next().unwrap().replace(";r=", ";a=");
        assert_eq!(draft_12.1, without_window.as_str(), "{draft_11}");
    }
}
