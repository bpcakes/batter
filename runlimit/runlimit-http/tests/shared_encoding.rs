//! Every draft module shares one validation, one integer range and one
//! [`QuotaState`] input, so the same input fails with the same error and
//! converts the same way whichever revision a caller selects.

#[path = "fixtures/decisions.rs"]
mod fixtures;

use std::time::Duration;

use fixtures::{allowance, fixed_policy, gcra_policy, quota};
use runlimit_core::{Admitted, RateLimitPolicy};
use runlimit_http::{
    EncodingError, HeaderField, MAX_POLICY_NAME_LENGTH, MAX_STRUCTURED_FIELD_INTEGER, QuotaState,
    draft_11, draft_12,
};

type Outcome = Result<HeaderField, EncodingError>;

/// Runs one policy through both drafts' `RateLimit-Policy` encoders.
fn policy_fields<P: RateLimitPolicy>(name: &str, policy: &P) -> [Outcome; 2] {
    [
        draft_11::quota_policy(name, policy),
        draft_12::quota_policy(name, policy),
    ]
}

/// Runs one state through both drafts' `RateLimit` encoders.
fn service_fields(name: &str, state: impl Into<QuotaState>) -> [Outcome; 2] {
    let state = state.into();
    [
        draft_11::service_limit(name, state),
        draft_12::service_limit(name, state),
    ]
}

/// Asserts that every draft's encoder rejected its input with `expected`.
fn assert_all_fail(outcomes: [Outcome; 2], expected: &EncodingError) {
    for (draft, outcome) in ["-11", "-12"].into_iter().zip(outcomes) {
        assert_eq!(outcome.as_ref().err(), Some(expected), "draft {draft}");
    }
}

/// Every encoder of every draft for one name.
fn every_encoder(name: &str) -> [Outcome; 4] {
    let policy = fixed_policy(5, Duration::from_secs(1));
    let [policy_11, policy_12] = policy_fields(name, &policy);
    let [service_11, service_12] = service_fields(name, allowance(5, 4, Duration::from_secs(1)));
    [policy_11, policy_12, service_11, service_12]
}

#[test]
fn invalid_policy_names_fail_identically_in_every_draft() {
    let cases = [
        (String::new(), EncodingError::EmptyPolicyName),
        (
            "a".repeat(MAX_POLICY_NAME_LENGTH + 1),
            EncodingError::PolicyNameTooLong {
                actual: MAX_POLICY_NAME_LENGTH + 1,
                maximum: MAX_POLICY_NAME_LENGTH,
            },
        ),
        (
            "bad\r\ninjected: value".to_owned(),
            EncodingError::InvalidPolicyNameCharacter {
                index: 3,
                character: '\r',
            },
        ),
        (
            "café".to_owned(),
            EncodingError::InvalidPolicyNameCharacter {
                index: 3,
                character: 'é',
            },
        ),
        (
            "tab\there".to_owned(),
            EncodingError::InvalidPolicyNameCharacter {
                index: 3,
                character: '\t',
            },
        ),
        (
            "delete\u{7f}".to_owned(),
            EncodingError::InvalidPolicyNameCharacter {
                index: 6,
                character: '\u{7f}',
            },
        ),
    ];

    for (name, expected) in cases {
        for outcome in every_encoder(&name) {
            assert_eq!(outcome.err(), Some(expected.clone()), "name {name:?}");
        }
    }
}

#[test]
fn a_policy_name_is_validated_before_any_number_in_every_draft() {
    let too_large = MAX_STRUCTURED_FIELD_INTEGER + 1;

    assert_all_fail(
        policy_fields("", &fixed_policy(too_large, Duration::from_millis(1_500))),
        &EncodingError::EmptyPolicyName,
    );
    assert_all_fail(
        service_fields("", allowance(too_large, too_large, Duration::from_secs(1))),
        &EncodingError::EmptyPolicyName,
    );
}

#[test]
fn policy_names_are_one_escaped_string_item_in_every_draft() {
    let name = "quoted \"name\" \\ path";
    let escaped = "\"quoted \\\"name\\\" \\\\ path\";";

    for outcome in every_encoder(name) {
        let (_, value) = outcome.unwrap();
        assert!(value.to_str().unwrap().starts_with(escaped), "{value:?}");
    }
    assert_eq!(
        draft_12::quota_policy(name, &fixed_policy(5, Duration::from_secs(1)))
            .unwrap()
            .1,
        "\"quoted \\\"name\\\" \\\\ path\";q=5;w=1"
    );
}

#[test]
fn every_accepted_policy_name_character_produces_valid_headers_in_every_draft() {
    for byte in b' '..=b'~' {
        let name = char::from(byte).to_string();
        for outcome in every_encoder(&name) {
            assert!(outcome.is_ok(), "rejected accepted byte {byte}");
        }
    }
    assert!(
        every_encoder(&"a".repeat(MAX_POLICY_NAME_LENGTH))
            .iter()
            .all(Result::is_ok)
    );
}

#[test]
fn the_structured_field_integer_range_is_shared_by_every_draft() {
    let too_large = EncodingError::StructuredFieldIntegerTooLarge {
        actual: MAX_STRUCTURED_FIELD_INTEGER + 1,
        maximum: MAX_STRUCTURED_FIELD_INTEGER,
    };
    let maximum = fixed_policy(MAX_STRUCTURED_FIELD_INTEGER, Duration::from_secs(1));

    for outcome in policy_fields("maximum", &maximum) {
        assert_eq!(outcome.unwrap().1, "\"maximum\";q=999999999999999;w=1");
    }
    assert_all_fail(
        policy_fields(
            "too-large",
            &fixed_policy(MAX_STRUCTURED_FIELD_INTEGER + 1, Duration::from_secs(1)),
        ),
        &too_large,
    );
    assert_all_fail(
        service_fields(
            "too-large",
            allowance(
                MAX_STRUCTURED_FIELD_INTEGER + 1,
                MAX_STRUCTURED_FIELD_INTEGER + 1,
                Duration::from_secs(1),
            ),
        ),
        &too_large,
    );
    assert_all_fail(
        service_fields(
            "too-late",
            allowance(1, 0, Duration::from_secs(MAX_STRUCTURED_FIELD_INTEGER + 1)),
        ),
        &too_large,
    );
    assert_all_fail(
        service_fields(
            "too-late",
            quota(1, Duration::from_secs(MAX_STRUCTURED_FIELD_INTEGER + 1)),
        ),
        &too_large,
    );
}

#[test]
fn non_whole_second_policy_periods_fail_identically_in_every_draft() {
    let expected = EncodingError::QuotaPeriodNotWholeSeconds {
        actual: Duration::from_millis(1_500),
    };

    assert_all_fail(
        policy_fields("fractional", &fixed_policy(5, Duration::from_millis(1_500))),
        &expected,
    );
    assert_all_fail(
        policy_fields("fractional", &gcra_policy(5, Duration::from_millis(1_500))),
        &expected,
    );
}

#[test]
fn quota_states_convert_from_every_reportable_quota_outcome() {
    let available = allowance(10, 4, Duration::from_millis(500));
    let denial = quota(10, Duration::from_millis(1_001));

    assert_eq!(
        QuotaState::from(available),
        QuotaState::Available(available)
    );
    assert_eq!(QuotaState::from(denial), QuotaState::Exhausted(denial));
    assert_eq!(
        QuotaState::from(Admitted::allowed(available)),
        QuotaState::Available(available)
    );
    assert_eq!(
        QuotaState::from(Admitted::shadow_denied(denial)),
        QuotaState::Exhausted(denial)
    );
}
