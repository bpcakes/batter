//! Framework-neutral HTTP response metadata for Runlimit decisions.
//!
//! This crate deliberately does not choose response status codes, bodies, or
//! which application policies should be disclosed. It only serializes
//! caller-selected policy and decision metadata into typed HTTP header fields.
//!
//! The `RateLimit-Policy` and `RateLimit` fields come from
//! `draft-ietf-httpapi-ratelimit-headers`, an Internet-Draft rather than a
//! published RFC. Each revision this crate encodes is its own module, so every
//! call site names the wire format it emits:
//!
//! - [`draft_12`] encodes `-12` (20 November 2026), the current revision. New
//!   code selects it.
//! - [`draft_11`] encodes `-11` (23 May 2026), which expired on 24 November
//!   2026. It stays selectable so an application can move clients that parse
//!   `-11` deliberately, and its removal is tracked as `batter-6u2p`.
//!
//! The two differ only in the `RateLimit` parameter keys: `-11` spells the
//! available quota and the effective window `r` and `t`, `-12` spells them
//! `a` and `w`. Everything else is shared and lives at this crate root: the
//! [`QuotaState`] input, the [`HeaderField`] output, one policy-name
//! validation and RFC 9651 integer range ([`MAX_POLICY_NAME_LENGTH`],
//! [`MAX_STRUCTURED_FIELD_INTEGER`]) and one [`EncodingError`], so both
//! revisions reject the same input with the same error.

mod encoding;
mod state;

pub mod draft_11;
pub mod draft_12;

pub use encoding::{
    EncodingError, HeaderField, MAX_POLICY_NAME_LENGTH, MAX_STRUCTURED_FIELD_INTEGER,
};
pub use state::QuotaState;
