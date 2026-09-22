//! Response fields from `draft-ietf-httpapi-ratelimit-headers-11`.
//!
//! The draft defines `RateLimit-Policy` for stable quota policy metadata and
//! `RateLimit` for the service limit currently available to a client. Both are
//! HTTP Structured Field lists whose members identify policies with String
//! items.
//!
//! This module emits one list member at a time. Applications remain
//! responsible for selecting policies, combining multiple members when
//! appropriate, and deciding whether to attach the fields to a response.
//! Partition keys are intentionally unsupported so subject material cannot be
//! exposed accidentally.

use std::time::Duration;

use http::{HeaderName, HeaderValue};
use runlimit_core::{Admitted, AdmittedView, Allowance, QuotaDenial, RateLimitPolicy};
use thiserror::Error;

/// Largest integer representable by an RFC 9651 Structured Field.
pub const MAX_STRUCTURED_FIELD_INTEGER: u64 = 999_999_999_999_999;

/// Maximum accepted byte length of a public policy name.
pub const MAX_POLICY_NAME_LENGTH: usize = 128;

/// A typed HTTP header name and value.
pub type HeaderField = (HeaderName, HeaderValue);

/// The quota state a `RateLimit` field can describe.
///
/// Only an allowance or a quota denial has service-limit metadata. A
/// storage-capacity denial is not representable here, so it cannot reach
/// [`service_limit`] and become an encoding error in the response path.
/// Callers holding a [`runlimit_core::Denial`] match it and pass the
/// [`QuotaDenial`] from its `QuotaExceeded` arm. An [`Admitted`] outcome
/// converts directly: an allowance becomes [`QuotaState::Available`] and a
/// shadow denial becomes [`QuotaState::Exhausted`], so a shadow denial exposes
/// the service value that would have applied if the policy were enforced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaState {
    /// Quota was consumed and the allowance remains available.
    Available(Allowance),
    /// The requested cost exceeded the quota, enforced or shadowed.
    Exhausted(QuotaDenial),
}

impl From<Allowance> for QuotaState {
    fn from(allowance: Allowance) -> Self {
        Self::Available(allowance)
    }
}

impl From<QuotaDenial> for QuotaState {
    fn from(denial: QuotaDenial) -> Self {
        Self::Exhausted(denial)
    }
}

impl From<Admitted> for QuotaState {
    fn from(admitted: Admitted) -> Self {
        match admitted.view() {
            AdmittedView::Allowed { allowance } => Self::Available(allowance),
            AdmittedView::ShadowDenied { denial } => Self::Exhausted(denial),
        }
    }
}

/// Encodes one `RateLimit-Policy` field.
///
/// The result has the form `"name";q=N;w=S`, where `q` is
/// [`RateLimitPolicy::quota`] and `w` is its exact whole-second
/// [`RateLimitPolicy::quota_period`]. The default quota unit from the draft is
/// requests; this helper therefore omits the optional `qu` parameter.
///
/// # Errors
///
/// Returns [`EncodingError`] when the public policy name is unsafe for an
/// HTTP Structured Field String, a numeric value exceeds the Structured Field
/// integer range, or the quota period is not an exact whole number of
/// seconds.
pub fn quota_policy<P: RateLimitPolicy + ?Sized>(
    name: &str,
    policy: &P,
) -> Result<HeaderField, EncodingError> {
    let name = encode_policy_name(name)?;
    let quota = structured_integer(policy.quota().get())?;
    let period = policy.quota_period();
    if !period.millis().is_multiple_of(1_000) {
        return Err(EncodingError::QuotaPeriodNotWholeSeconds {
            actual: period.duration(),
        });
    }
    let period = structured_integer(period.millis() / 1_000)?;
    let value = header_value(format!("{name};q={quota};w={period}"));

    Ok((HeaderName::from_static("ratelimit-policy"), value))
}

/// Encodes one `RateLimit` field.
///
/// An available quota uses the immediately available allowance and rounds
/// its replenishment delay up to whole seconds. An exhausted quota uses `r=0`
/// and rounds the backend's retry delay up in the same way. Neither creates a
/// `Retry-After` field or chooses an HTTP status.
///
/// # Errors
///
/// Returns [`EncodingError`] when the public policy name is unsafe or a
/// numeric value exceeds the Structured Field integer range.
pub fn service_limit(
    name: &str,
    state: impl Into<QuotaState>,
) -> Result<HeaderField, EncodingError> {
    let name = encode_policy_name(name)?;
    let (available, effective_window) = match state.into() {
        QuotaState::Available(allowance) => (
            allowance.available(),
            allowance.replenishes_after().seconds(),
        ),
        QuotaState::Exhausted(denial) => (0, denial.retry_after().seconds()),
    };
    let available = structured_integer(available)?;
    let effective_window = structured_integer(effective_window)?;
    let value = header_value(format!("{name};r={available};t={effective_window}"));

    Ok((HeaderName::from_static("ratelimit"), value))
}

fn encode_policy_name(name: &str) -> Result<String, EncodingError> {
    if name.is_empty() {
        return Err(EncodingError::EmptyPolicyName);
    }
    if name.len() > MAX_POLICY_NAME_LENGTH {
        return Err(EncodingError::PolicyNameTooLong {
            actual: name.len(),
            maximum: MAX_POLICY_NAME_LENGTH,
        });
    }

    let mut encoded = String::with_capacity(name.len() + 2);
    encoded.push('"');
    for (index, character) in name.char_indices() {
        if !matches!(character, '\u{20}'..='\u{7e}') {
            return Err(EncodingError::InvalidPolicyNameCharacter { index, character });
        }
        if matches!(character, '"' | '\\') {
            encoded.push('\\');
        }
        encoded.push(character);
    }
    encoded.push('"');
    Ok(encoded)
}

fn structured_integer(value: u64) -> Result<u64, EncodingError> {
    if value > MAX_STRUCTURED_FIELD_INTEGER {
        return Err(EncodingError::StructuredFieldIntegerTooLarge {
            actual: value,
            maximum: MAX_STRUCTURED_FIELD_INTEGER,
        });
    }
    Ok(value)
}

/// Converts an encoded member into a header value.
///
/// Every byte is either visible ASCII validated by [`encode_policy_name`] or a
/// literal from the Structured Field grammar, all of which `HeaderValue`
/// accepts, so this conversion cannot fail.
fn header_value(value: String) -> HeaderValue {
    HeaderValue::try_from(value)
        .expect("validated policy names and Structured Field literals are visible ASCII")
}

/// Failure to encode draft-11 response metadata.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EncodingError {
    /// The public policy name was empty.
    #[error("RateLimit policy name must not be empty")]
    EmptyPolicyName,
    /// The public policy name exceeded [`MAX_POLICY_NAME_LENGTH`].
    #[error("RateLimit policy name is {actual} bytes; the maximum is {maximum}")]
    PolicyNameTooLong {
        /// Supplied byte length.
        actual: usize,
        /// Largest accepted byte length.
        maximum: usize,
    },
    /// The public policy name contained a character outside visible ASCII.
    #[error(
        "RateLimit policy name contains invalid character {character:?} at byte index {index}; \
         use visible ASCII"
    )]
    InvalidPolicyNameCharacter {
        /// Zero-based byte index of the invalid character.
        index: usize,
        /// Invalid character.
        character: char,
    },
    /// A value exceeded the RFC 9651 Structured Field integer range.
    #[error("Structured Field integer {actual} exceeds maximum {maximum}")]
    StructuredFieldIntegerTooLarge {
        /// Supplied integer.
        actual: u64,
        /// Largest representable integer.
        maximum: u64,
    },
    /// The policy's replenishment period was not an exact whole second.
    #[error("RateLimit-Policy quota period {actual:?} is not an exact whole number of seconds")]
    QuotaPeriodNotWholeSeconds {
        /// Supplied quota period.
        actual: Duration,
    },
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use runlimit_core::{
        Admitted, Allowance, Capacity, FixedWindowPolicy, GcraPolicy, PolicyId, QuotaDenial,
        ScopeId,
    };

    use super::{
        EncodingError, MAX_POLICY_NAME_LENGTH, MAX_STRUCTURED_FIELD_INTEGER, QuotaState,
        quota_policy, service_limit,
    };

    fn fixed_policy(limit: u64, window: Duration) -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new("api.search").unwrap(),
            ScopeId::new("client").unwrap(),
            limit,
            window,
        )
        .unwrap()
    }

    fn allowance(capacity: u64, available: u64, replenishes_after: Duration) -> Allowance {
        Allowance::new(
            Capacity::new(capacity).unwrap(),
            available,
            replenishes_after,
        )
        .unwrap()
    }

    fn quota(capacity: u64, retry_after: Duration) -> QuotaDenial {
        QuotaDenial::new(Capacity::new(capacity).unwrap(), retry_after)
    }

    #[test]
    fn encodes_exact_draft_11_golden_fields() {
        let policy = fixed_policy(100, Duration::from_mins(1));
        let policy_field = quota_policy("search", &policy).unwrap();
        let service_field =
            service_limit("search", allowance(100, 49, Duration::from_secs(37))).unwrap();

        assert_eq!(policy_field.0.as_str(), "ratelimit-policy");
        assert_eq!(policy_field.1, "\"search\";q=100;w=60");
        assert_eq!(service_field.0.as_str(), "ratelimit");
        assert_eq!(service_field.1, "\"search\";r=49;t=37");
    }

    #[test]
    fn uses_algorithm_neutral_gcra_policy_metadata() {
        let policy = GcraPolicy::new(
            PolicyId::new("api.upload").unwrap(),
            ScopeId::new("account").unwrap(),
            10,
            Duration::from_mins(1),
            4,
        )
        .unwrap();

        assert_eq!(
            quota_policy("upload", &policy).unwrap().1,
            "\"upload\";q=10;w=60"
        );
    }

    #[test]
    fn exhausted_quota_is_an_r_zero_service_value() {
        let denial = quota(10, Duration::from_millis(1_001));

        assert_eq!(
            service_limit("default", denial).unwrap().1,
            "\"default\";r=0;t=2"
        );
        assert_eq!(
            service_limit("default", QuotaState::Exhausted(denial))
                .unwrap()
                .1,
            "\"default\";r=0;t=2"
        );
    }

    #[test]
    fn admitted_outcomes_convert_without_a_decision_round_trip() {
        let allowance = allowance(10, 4, Duration::from_millis(500));
        let denial = quota(10, Duration::from_millis(1_001));

        assert_eq!(
            QuotaState::from(Admitted::allowed(allowance)),
            QuotaState::Available(allowance)
        );
        assert_eq!(
            QuotaState::from(Admitted::shadow_denied(denial)),
            QuotaState::Exhausted(denial)
        );
        assert_eq!(
            service_limit("default", Admitted::allowed(allowance))
                .unwrap()
                .1,
            "\"default\";r=4;t=1"
        );
        assert_eq!(
            service_limit("default", Admitted::shadow_denied(denial))
                .unwrap()
                .1,
            "\"default\";r=0;t=2"
        );
    }

    #[test]
    fn rounds_allowed_effective_windows_up_to_whole_seconds() {
        assert_eq!(
            service_limit("default", allowance(10, 9, Duration::from_nanos(1)))
                .unwrap()
                .1,
            "\"default\";r=9;t=1"
        );
    }

    #[test]
    fn escapes_structured_field_strings_without_header_injection() {
        let policy = fixed_policy(5, Duration::from_secs(1));

        assert_eq!(
            quota_policy("quoted \"name\" \\ path", &policy).unwrap().1,
            "\"quoted \\\"name\\\" \\\\ path\";q=5;w=1"
        );
        assert_eq!(
            quota_policy("bad\r\ninjected: value", &policy),
            Err(EncodingError::InvalidPolicyNameCharacter {
                index: 3,
                character: '\r',
            })
        );
        assert_eq!(
            quota_policy("café", &policy),
            Err(EncodingError::InvalidPolicyNameCharacter {
                index: 3,
                character: 'é',
            })
        );
    }

    #[test]
    fn every_accepted_policy_name_character_produces_valid_headers() {
        let policy = fixed_policy(5, Duration::from_secs(1));

        for byte in b' '..=b'~' {
            let name = char::from(byte).to_string();
            assert!(
                quota_policy(&name, &policy).is_ok(),
                "RateLimit-Policy rejected accepted byte {byte}"
            );
            assert!(
                service_limit(&name, allowance(5, 4, Duration::from_secs(1))).is_ok(),
                "RateLimit rejected accepted byte {byte}"
            );
        }
    }

    #[test]
    fn rejects_empty_and_oversized_policy_names() {
        let policy = fixed_policy(5, Duration::from_secs(1));

        assert_eq!(
            quota_policy("", &policy),
            Err(EncodingError::EmptyPolicyName)
        );
        assert_eq!(
            quota_policy(&"a".repeat(MAX_POLICY_NAME_LENGTH + 1), &policy),
            Err(EncodingError::PolicyNameTooLong {
                actual: MAX_POLICY_NAME_LENGTH + 1,
                maximum: MAX_POLICY_NAME_LENGTH,
            })
        );
    }

    #[test]
    fn accepts_the_structured_field_integer_maximum_and_rejects_larger_values() {
        let maximum = fixed_policy(MAX_STRUCTURED_FIELD_INTEGER, Duration::from_secs(1));
        let too_large = fixed_policy(MAX_STRUCTURED_FIELD_INTEGER + 1, Duration::from_secs(1));

        assert_eq!(
            quota_policy("maximum", &maximum).unwrap().1,
            "\"maximum\";q=999999999999999;w=1"
        );
        assert_eq!(
            quota_policy("too-large", &too_large),
            Err(EncodingError::StructuredFieldIntegerTooLarge {
                actual: MAX_STRUCTURED_FIELD_INTEGER + 1,
                maximum: MAX_STRUCTURED_FIELD_INTEGER,
            })
        );
        assert_eq!(
            service_limit(
                "too-large",
                allowance(
                    MAX_STRUCTURED_FIELD_INTEGER + 1,
                    MAX_STRUCTURED_FIELD_INTEGER + 1,
                    Duration::from_secs(1),
                ),
            ),
            Err(EncodingError::StructuredFieldIntegerTooLarge {
                actual: MAX_STRUCTURED_FIELD_INTEGER + 1,
                maximum: MAX_STRUCTURED_FIELD_INTEGER,
            })
        );
    }

    #[test]
    fn rejects_non_whole_second_policy_periods() {
        let policy = fixed_policy(5, Duration::from_millis(1_500));

        assert_eq!(
            quota_policy("fractional", &policy),
            Err(EncodingError::QuotaPeriodNotWholeSeconds {
                actual: Duration::from_millis(1_500),
            })
        );
    }

    #[test]
    fn rejects_effective_windows_above_the_structured_field_integer_maximum() {
        assert_eq!(
            service_limit(
                "default",
                allowance(1, 0, Duration::from_secs(MAX_STRUCTURED_FIELD_INTEGER + 1)),
            ),
            Err(EncodingError::StructuredFieldIntegerTooLarge {
                actual: MAX_STRUCTURED_FIELD_INTEGER + 1,
                maximum: MAX_STRUCTURED_FIELD_INTEGER,
            })
        );
    }
}
