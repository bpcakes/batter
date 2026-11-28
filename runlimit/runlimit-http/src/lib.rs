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
//! Both revisions share their `RateLimit-Policy` encoding, the
//! [`HeaderField`] output, one policy-name validation and RFC 9651 integer
//! range ([`MAX_POLICY_NAME_LENGTH`], [`MAX_STRUCTURED_FIELD_INTEGER`]) and
//! one [`EncodingError`], all at this crate root, so a policy name, a policy
//! or an available quota that one revision rejects, the other rejects with
//! the same error. Their `RateLimit` encoders differ: `-11`'s
//! takes a [`draft_11::QuotaState`] and reports `r` and `t`, while `-12`'s
//! takes an [`Allowance`](runlimit_core::Allowance) and reports only its
//! exact available quota `a`, since no decision states the effective window
//! and a denial does not state the quota that remains (see [`draft_12`]).
//! Only `-11` therefore encodes a window, so only it can reject one, such as
//! a replenishment delay beyond [`MAX_STRUCTURED_FIELD_INTEGER`] seconds.

mod encoding;
mod state;

pub mod draft_11;
pub mod draft_12;

pub use encoding::{
    EncodingError, HeaderField, MAX_POLICY_NAME_LENGTH, MAX_STRUCTURED_FIELD_INTEGER,
};
