use std::fmt;

use crate::{PolicyFingerprint, SubjectKey};

/// The complete logical identity of one stored rate-limit counter.
///
/// Backends must use both the policy configuration fingerprint and opaque
/// subject key when comparing, ordering, locking, or persisting counters.
/// This storage key deliberately does not implement Serde traits.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CounterKey {
    fingerprint: PolicyFingerprint,
    subject: SubjectKey,
}

impl CounterKey {
    /// Constructs a logical counter key from its fixed-width components.
    pub const fn new(fingerprint: PolicyFingerprint, subject: SubjectKey) -> Self {
        Self {
            fingerprint,
            subject,
        }
    }

    /// Returns the policy configuration fingerprint.
    pub const fn fingerprint(self) -> PolicyFingerprint {
        self.fingerprint
    }

    /// Returns an unbound copy of the opaque subject key.
    ///
    /// This accessor exists for storage-backend integration. Binding the
    /// returned key to a policy is an explicit new binding, independent of
    /// the fingerprint retained by this counter key.
    pub const fn subject(self) -> SubjectKey {
        self.subject
    }

    /// Returns the stable `fingerprint || subject` byte representation.
    ///
    /// This fixed-width encoding deliberately has no framing because both
    /// components are exactly 32 bytes.
    pub fn to_bytes(self) -> [u8; 64] {
        let mut bytes = [0_u8; 64];
        bytes[..32].copy_from_slice(self.fingerprint.as_bytes());
        bytes[32..].copy_from_slice(self.subject.as_bytes());
        bytes
    }
}

impl fmt::Debug for CounterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CounterKey")
            .field("fingerprint", &self.fingerprint)
            .field("subject", &self.subject)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{Check, FixedWindowPolicy, PolicyId, ScopeId, SubjectKey};

    #[test]
    fn fixed_width_encoding_is_fingerprint_then_subject() {
        let policy = FixedWindowPolicy::new(
            PolicyId::new("auth.login").unwrap(),
            ScopeId::new("client").unwrap(),
            8,
            Duration::from_mins(1),
        )
        .unwrap();
        let subject = SubjectKey::from_digest([0x5a; 32]);
        let key = Check::new(subject.bind(&policy)).counter_key();
        let bytes = key.to_bytes();

        assert_eq!(&bytes[..32], policy.fingerprint().as_bytes());
        assert_eq!(&bytes[32..], subject.as_bytes());
        assert!(format!("{key:?}").contains("[REDACTED]"));
        assert!(!format!("{key:?}").contains("5a5a"));
    }
}
