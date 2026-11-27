//! The attempt storage protocol is persistent: PostgreSQL stores attempt rows
//! under the policy fingerprint and the subject key, so these vectors pin
//! every byte of both encodings. They were computed independently of this
//! crate from the documented layouts:
//!
//! - fingerprint: `SHA-256("runlimit/attempt-policy/v1\0" || id || 0 || scope
//!   || 0 || be64(initial) || be64(maximum) || be64(quiet) || be64(lease))`,
//!   all durations in whole milliseconds;
//! - subject: `HMAC-SHA-256(secret, "runlimit/subject-key/v1\0" ||
//!   "attempt/v1\0" || id || 0 || scope || 0 || subject)`.
//!
//! A change that moves either vector strands stored attempt state during a
//! rolling deployment; it needs a new domain version and a storage migration.

use std::time::Duration;

use runlimit_core::{KeyHasher, PolicyId, QuotaPeriod, ScopeId, attempts::AttemptPolicy};

fn policy() -> AttemptPolicy {
    let millis = |value| QuotaPeriod::new(Duration::from_millis(value)).unwrap();
    AttemptPolicy::new(
        PolicyId::new("sign-in").unwrap(),
        ScopeId::new("account").unwrap(),
        millis(1_000),
        millis(60_000),
        millis(600_000),
        millis(30_000),
    )
    .unwrap()
}

#[test]
fn attempt_policy_fingerprint_matches_stable_protocol_vector() {
    assert_eq!(
        policy().fingerprint().to_string(),
        "3807264bbc041e72a6a2a4467ebaacdfe991aa3612937ffd0b28855a9cdd6afd"
    );
}

#[test]
fn attempt_subject_key_matches_stable_protocol_vector() {
    let policy = policy();
    let subject = KeyHasher::new([0x42; 32])
        .unwrap()
        .hash_attempt_for(&policy, b"user@example.test");

    let key = subject.counter_key();
    assert_eq!(key.fingerprint(), policy.fingerprint());
    assert_eq!(
        key.subject().into_bytes(),
        [
            0xc6, 0xdd, 0x31, 0x9a, 0x6e, 0x8d, 0xbc, 0x65, 0xc6, 0xc1, 0xf2, 0xf3, 0xe8, 0xc5,
            0x75, 0x41, 0x5a, 0x95, 0x12, 0x10, 0xec, 0x62, 0xa9, 0xdd, 0xfd, 0xf7, 0x8a, 0x31,
            0x1a, 0x3c, 0x25, 0x80,
        ]
    );
    assert_eq!(
        key.subject(),
        subject.into_unbound_subject_key(),
        "the storage key and the unbound key are one derivation"
    );
}
