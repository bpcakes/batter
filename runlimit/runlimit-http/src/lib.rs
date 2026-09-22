//! Framework-neutral HTTP response metadata for Runlimit decisions.
//!
//! This crate deliberately does not choose response status codes, bodies, or
//! which application policies should be disclosed. It only serializes
//! caller-selected policy and decision metadata into typed HTTP header fields.
//!
//! [`draft_11`] implements
//! `draft-ietf-httpapi-ratelimit-headers-11`, an active Internet-Draft rather
//! than a published RFC. Its versioned module path makes that unstable wire
//! contract explicit.

pub mod draft_11;
