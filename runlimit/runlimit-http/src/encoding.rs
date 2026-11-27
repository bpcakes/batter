//! Version-neutral RFC 9651 Structured Field encoding shared by every draft
//! module.
//!
//! Each draft module spells its own field names and parameter keys; this
//! module owns everything those spellings have in common: the policy-name
//! String item, the Integer range, the whole-second policy window, and the one
//! error type. Every draft therefore rejects the same input with the same
//! error.

use std::time::Duration;

use http::{HeaderName, HeaderValue};
use runlimit_core::QuotaPeriod;
use std::fmt::Write as _;
use thiserror::Error;

/// Largest integer representable by an RFC 9651 Structured Field.
///
/// Migration: this replaces `runlimit_http::draft_11::MAX_STRUCTURED_FIELD_INTEGER`;
/// every draft module shares this one limit.
pub const MAX_STRUCTURED_FIELD_INTEGER: u64 = 999_999_999_999_999;

/// Maximum accepted byte length of a public policy name.
///
/// Migration: this replaces `runlimit_http::draft_11::MAX_POLICY_NAME_LENGTH`;
/// every draft module shares this one limit.
pub const MAX_POLICY_NAME_LENGTH: usize = 128;

/// A typed HTTP header name and value.
///
/// Migration: this replaces `runlimit_http::draft_11::HeaderField`; every
/// draft module returns this one type.
pub type HeaderField = (HeaderName, HeaderValue);

/// One Structured Field list member: a policy-name String item followed by
/// the Integer parameters a draft module appends in its own order.
///
/// Only [`Member::policy`] creates a member, so every parameter follows a
/// validated name and every member a draft module emits shares that
/// validation.
pub(crate) struct Member {
    encoded: String,
}

impl Member {
    /// Starts a member with the public policy name as a Structured Field
    /// String, escaping `"` and `\`.
    ///
    /// Both drafts accept a String identifier, so this never emits a Token.
    pub(crate) fn policy(name: &str) -> Result<Self, EncodingError> {
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
        Ok(Self { encoded })
    }

    /// Appends `;key=value` after checking the Structured Field Integer range.
    ///
    /// `key` is a draft module's literal lowercase parameter key.
    pub(crate) fn integer(mut self, key: &'static str, value: u64) -> Result<Self, EncodingError> {
        if value > MAX_STRUCTURED_FIELD_INTEGER {
            return Err(EncodingError::StructuredFieldIntegerTooLarge {
                actual: value,
                maximum: MAX_STRUCTURED_FIELD_INTEGER,
            });
        }
        self.encoded.push(';');
        self.encoded.push_str(key);
        self.encoded.push('=');
        // Writing into a String cannot fail.
        let _ = write!(self.encoded, "{value}");
        Ok(self)
    }

    /// Pairs the encoded member with its field name.
    ///
    /// Every byte is either visible ASCII validated by [`Member::policy`] or a
    /// literal from the Structured Field grammar, all of which `HeaderValue`
    /// accepts, so this conversion cannot fail.
    pub(crate) fn into_field(self, name: HeaderName) -> HeaderField {
        let value = HeaderValue::try_from(self.encoded)
            .expect("validated policy names and Structured Field literals are visible ASCII");
        (name, value)
    }
}

/// The exact whole seconds of a policy's quota period.
///
/// Both drafts define the `RateLimit-Policy` window as a non-zero Integer
/// number of seconds without sub-second precision. A [`QuotaPeriod`] is
/// already nonzero, so the one remaining rule is an exact whole second.
pub(crate) fn policy_window(period: QuotaPeriod) -> Result<u64, EncodingError> {
    if period.millis().is_multiple_of(1_000) {
        Ok(period.millis() / 1_000)
    } else {
        Err(EncodingError::QuotaPeriodNotWholeSeconds {
            actual: period.duration(),
        })
    }
}

/// Failure to encode `RateLimit-Policy` or `RateLimit` response metadata.
///
/// Every draft module returns this one type and validates its input in the
/// same order, so the same input fails with the same error whichever revision
/// the caller selected.
///
/// Migration: this replaces `runlimit_http::draft_11::EncodingError`; its
/// variants are unchanged.
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
