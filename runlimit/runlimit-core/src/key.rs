use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;

use crate::{FixedWindowPolicy, RateLimitPolicy};

const KEY_DOMAIN: &[u8] = b"runlimit/subject-key/v1\0";

type HmacSha256 = Hmac<Sha256>;

/// An opaque, fixed-width subject identifier used by storage backends.
///
/// The inner digest is intentionally omitted from [`Debug`] output. Derive
/// keys with [`KeyHasher::hash_for`]; [`SubjectKey::from_digest`] exists only
/// for input that is already a cryptographically opaque 32-byte digest.
///
/// This type deliberately does not implement Serde traits, even when the
/// crate's `serde` feature is enabled, to avoid accidental key disclosure.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubjectKey([u8; 32]);

impl SubjectKey {
    /// Constructs a key from an already-opaque 32-byte digest.
    ///
    /// This constructor does not hash, namespace, or otherwise transform the
    /// input, so it bypasses the per-policy domain separation that
    /// [`KeyHasher::hash_for`] provides. Never pass raw or padded application
    /// identities: the bytes enter storage, comparison, and shard selection
    /// exactly as given. It is intended for tests and for applications that
    /// already derive an opaque, secret-keyed digest of their own.
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Returns the opaque digest bytes for storage and comparison.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Consumes the key and returns its opaque digest bytes.
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Binds this already-opaque digest to the policy that will evaluate it.
    ///
    /// Prefer [`KeyHasher::hash_for`] when Runlimit derives the digest. This
    /// method is the explicit boundary for tests and applications that already
    /// own a cryptographically opaque, secret-keyed digest.
    pub const fn bind<P: RateLimitPolicy + ?Sized>(self, policy: &P) -> PolicySubject<'_, P> {
        PolicySubject {
            policy,
            subject: self,
        }
    }
}

impl fmt::Debug for SubjectKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SubjectKey([REDACTED])")
    }
}

/// An opaque subject key bound to the exact policy that names its namespace.
///
/// This value is the only input accepted by [`crate::Check::new`]. Its fields
/// are private so the normal construction path cannot replace the policy after
/// deriving a key. [`KeyHasher::hash_for`] creates it from a normalized
/// subject. [`PolicySubject::into_unbound_subject_key`] explicitly leaves that
/// path, while [`SubjectKey::bind`] is the corresponding explicit binding
/// operation for an already-opaque digest.
pub struct PolicySubject<'a, P: RateLimitPolicy + ?Sized = FixedWindowPolicy> {
    policy: &'a P,
    subject: SubjectKey,
}

impl<P: RateLimitPolicy + ?Sized> Copy for PolicySubject<'_, P> {}

impl<P: RateLimitPolicy + ?Sized> Clone for PolicySubject<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: RateLimitPolicy + ?Sized> fmt::Debug for PolicySubject<'_, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolicySubject")
            .field("policy_id", &self.policy.id())
            .field("scope_id", &self.policy.scope())
            .field("subject", &self.subject)
            .finish()
    }
}

impl<'a, P: RateLimitPolicy + ?Sized> PolicySubject<'a, P> {
    /// Returns the opaque key without its policy binding.
    ///
    /// This is an escape hatch for adapter boundaries that own policy
    /// selection separately and will immediately bind the returned key to
    /// that policy. Normal application code should pass this value directly
    /// to [`crate::Check::new`]; unbinding it makes an explicit later
    /// [`SubjectKey::bind`] to a different policy possible.
    pub const fn into_unbound_subject_key(self) -> SubjectKey {
        self.subject
    }

    pub(crate) const fn into_parts(self) -> (&'a P, SubjectKey) {
        (self.policy, self.subject)
    }
}

/// Derives opaque subject keys using HMAC-SHA-256.
///
/// Each derivation is domain-separated by the exact policy and scope
/// identifiers of the policy it is derived for. The same normalized subject
/// therefore yields unrelated keys in different policy scopes. There is one
/// derivation method, [`KeyHasher::hash_for`], and it takes the policy the
/// resulting key will be checked against.
///
/// Applications should keep one stable secret per deployment. Rotating it
/// deliberately starts new counters because all derived subject keys change.
///
/// The raw secret is not retained after construction. Instead, the hasher
/// caches key-equivalent, precomputed HMAC state. Cloning a hasher copies that
/// state; each copy zeroizes its SHA-256 state and buffered input when dropped.
/// Treat a live [`KeyHasher`] as secret material. [`Debug`](fmt::Debug) never
/// exposes its state.
pub struct KeyHasher {
    template: HmacSha256,
}

impl Clone for KeyHasher {
    fn clone(&self) -> Self {
        Self {
            template: self.template.clone(),
        }
    }
}

impl KeyHasher {
    /// Derives an opaque subject bound to an outcome-aware attempt policy.
    /// Application normalization remains caller-owned. This domain is distinct
    /// from ordinary quota subjects even for identical identifiers.
    pub fn hash_attempt_for<'a>(
        &self,
        policy: &'a crate::attempts::AttemptPolicy,
        subject: impl AsRef<[u8]>,
    ) -> crate::attempts::AttemptSubject<'a> {
        let mut mac = self.template.clone();
        mac.update(b"attempt/v1\0");
        mac.update(policy.id().as_str().as_bytes());
        mac.update(&[0]);
        mac.update(policy.scope().as_str().as_bytes());
        mac.update(&[0]);
        mac.update(subject.as_ref());
        crate::attempts::AttemptSubject {
            policy,
            subject: SubjectKey::from_digest(mac.finalize().into_bytes().into()),
        }
    }
    /// Minimum accepted secret length in bytes.
    pub const MINIMUM_SECRET_LENGTH: usize = 32;

    /// Constructs a hasher by precomputing zeroizing keyed HMAC state.
    ///
    /// The supplied raw secret is borrowed only for construction and is not
    /// retained by the returned hasher.
    ///
    /// # Errors
    ///
    /// Returns [`KeyHasherError::SecretTooShort`] unless the secret contains at
    /// least 32 bytes.
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, KeyHasherError> {
        let secret = secret.as_ref();
        if secret.len() < Self::MINIMUM_SECRET_LENGTH {
            return Err(KeyHasherError::SecretTooShort {
                actual: secret.len(),
                minimum: Self::MINIMUM_SECRET_LENGTH,
            });
        }

        let Ok(mut template) = HmacSha256::new_from_slice(secret) else {
            unreachable!("HMAC-SHA-256 accepts keys of every length");
        };
        template.update(KEY_DOMAIN);

        Ok(Self { template })
    }

    /// Hashes a normalized subject and binds it to its policy namespace.
    ///
    /// The returned value retains the exact `policy` reference and can be
    /// passed directly to [`crate::Check::new`]. There is no separate policy
    /// argument at check construction, so the normal path cannot accidentally
    /// pair the derived key with another policy. Deliberately calling
    /// [`PolicySubject::into_unbound_subject_key`] leaves that safe path and
    /// permits an explicit later binding.
    ///
    /// ```compile_fail,E0599
    /// # use std::time::Duration;
    /// # use runlimit_core::{FixedWindowPolicy, KeyHasher, PolicyId, ScopeId};
    /// # let first = FixedWindowPolicy::new(PolicyId::new("first").unwrap(), ScopeId::new("client").unwrap(), 1, Duration::from_secs(1)).unwrap();
    /// # let second = FixedWindowPolicy::new(PolicyId::new("second").unwrap(), ScopeId::new("client").unwrap(), 1, Duration::from_secs(1)).unwrap();
    /// # let hasher = KeyHasher::new([7; 32]).unwrap();
    /// let subject = hasher.hash_for(&first, b"normalized-subject");
    /// let _ = subject.bind(&second);
    /// ```
    ///
    /// Normalization is application-owned: two byte strings are treated as
    /// distinct subjects even if an application considers them equivalent.
    pub fn hash_for<'a, P: RateLimitPolicy + ?Sized>(
        &self,
        policy: &'a P,
        subject: impl AsRef<[u8]>,
    ) -> PolicySubject<'a, P> {
        let mut mac = self.template.clone();
        mac.update(policy.id().as_str().as_bytes());
        mac.update(&[0]);
        mac.update(policy.scope().as_str().as_bytes());
        mac.update(&[0]);
        mac.update(subject.as_ref());
        SubjectKey::from_digest(mac.finalize().into_bytes().into()).bind(policy)
    }
}

impl fmt::Debug for KeyHasher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyHasher([REDACTED])")
    }
}

/// An invalid subject-key hasher configuration.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum KeyHasherError {
    /// The supplied secret was shorter than the security minimum.
    #[error("key-hashing secret is {actual} bytes; at least {minimum} bytes are required")]
    SecretTooShort {
        /// Supplied secret length.
        actual: usize,
        /// Minimum accepted secret length.
        minimum: usize,
    },
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{KeyHasher, KeyHasherError, SubjectKey};
    use crate::{Check, FixedWindowPolicy, PolicyId, ScopeId};

    fn hasher() -> KeyHasher {
        KeyHasher::new([0x42; 32]).unwrap()
    }

    fn policy(id: &str, scope: &str) -> FixedWindowPolicy {
        FixedWindowPolicy::new(
            PolicyId::new(id).unwrap(),
            ScopeId::new(scope).unwrap(),
            8,
            Duration::from_mins(1),
        )
        .unwrap()
    }

    #[test]
    fn rejects_short_secrets() {
        assert_eq!(
            KeyHasher::new([0; 31]).unwrap_err(),
            KeyHasherError::SecretTooShort {
                actual: 31,
                minimum: 32,
            }
        );
    }

    #[test]
    fn accepts_secrets_longer_than_the_minimum() {
        assert!(KeyHasher::new([0; 64]).is_ok());
    }

    #[test]
    fn hashing_is_deterministic_within_a_namespace() {
        let policy = policy("auth.login", "identity");
        let first = hasher().hash_for(&policy, b"user@example.test");
        let second = hasher().hash_for(&policy, b"user@example.test");

        assert_eq!(first.subject, second.subject);
    }

    #[test]
    fn derived_subject_keeps_the_exact_policy_value() {
        let first = policy("auth.login", "client");
        let second = policy("auth.login", "client");
        let check = Check::new(hasher().hash_for(&first, b"user@example.test"));

        assert_eq!(
            std::ptr::from_ref(check.policy()),
            std::ptr::from_ref(&first)
        );
        assert_ne!(
            std::ptr::from_ref(check.policy()),
            std::ptr::from_ref(&second)
        );
    }

    #[test]
    fn unbinding_is_an_explicit_policy_substitution_escape_hatch() {
        let first = policy("auth.login", "client");
        let second = policy("auth.login", "identity");
        let subject = hasher()
            .hash_for(&first, b"user@example.test")
            .into_unbound_subject_key();
        let check = Check::new(subject.bind(&second));

        assert_eq!(check.subject(), subject);
        assert_eq!(
            std::ptr::from_ref(check.policy()),
            std::ptr::from_ref(&second)
        );
    }

    #[test]
    fn policy_and_scope_domain_separate_subjects() {
        let hasher = hasher();
        let login_identity_policy = policy("auth.login", "identity");
        let signup_identity_policy = policy("auth.signup", "identity");
        let login_client_policy = policy("auth.login", "client");
        let login_identity = hasher.hash_for(&login_identity_policy, b"user@example.test");
        let signup_identity = hasher.hash_for(&signup_identity_policy, b"user@example.test");
        let login_client = hasher.hash_for(&login_client_policy, b"user@example.test");

        assert_ne!(login_identity.subject, signup_identity.subject);
        assert_ne!(login_identity.subject, login_client.subject);
    }

    #[test]
    fn subjects_and_secrets_change_the_digest() {
        let policy = policy("auth.login", "identity");
        let first = hasher().hash_for(&policy, b"first");
        let second = hasher().hash_for(&policy, b"second");
        let other_secret = KeyHasher::new([0x24; 32])
            .unwrap()
            .hash_for(&policy, b"first");

        assert_ne!(first.subject, second.subject);
        assert_ne!(first.subject, other_secret.subject);
    }

    #[test]
    fn subject_key_debug_output_is_redacted() {
        let key = SubjectKey::from_digest([0xab; 32]);
        let output = format!("{key:?}");

        assert_eq!(output, "SubjectKey([REDACTED])");
        assert!(!output.contains("ab"));
        assert_eq!(key.as_bytes(), &[0xab; 32]);
        assert_eq!(key.into_bytes(), [0xab; 32]);
    }

    #[test]
    fn policy_subject_debug_output_is_redacted() {
        let policy = policy("auth.login", "identity");
        let subject = SubjectKey::from_digest([0xab; 32]).bind(&policy);

        assert_eq!(
            format!("{subject:?}"),
            "PolicySubject { policy_id: PolicyId(\"auth.login\"), scope_id: \
             ScopeId(\"identity\"), subject: SubjectKey([REDACTED]) }"
        );
    }

    #[test]
    fn hasher_debug_output_is_redacted() {
        assert_eq!(format!("{:?}", hasher()), "KeyHasher([REDACTED])");
    }

    #[test]
    fn hashing_matches_stable_protocol_vectors() {
        let policy = policy("auth.login", "identity");

        assert_eq!(
            hasher()
                .hash_for(&policy, b"user@example.test")
                .subject
                .into_bytes(),
            [
                0x7e, 0x8c, 0x35, 0x4d, 0x1a, 0x9b, 0x8c, 0x11, 0xeb, 0xf5, 0xfd, 0x5f, 0xcb, 0x82,
                0x58, 0x6f, 0xda, 0xce, 0xbe, 0xf1, 0xff, 0x15, 0x82, 0x9f, 0xe0, 0xb0, 0x79, 0xd1,
                0x31, 0x22, 0xbc, 0x21,
            ]
        );
        assert_eq!(
            KeyHasher::new([0x24; 80])
                .unwrap()
                .hash_for(&policy, b"user@example.test")
                .subject
                .into_bytes(),
            [
                0x23, 0xa7, 0x30, 0xd0, 0x57, 0x8e, 0xec, 0x28, 0xe3, 0xf5, 0x7d, 0xd3, 0x96, 0x32,
                0xd8, 0xd8, 0x7b, 0x99, 0x87, 0x79, 0x56, 0xc9, 0xcd, 0x7d, 0xfe, 0x26, 0x84, 0x7b,
                0x17, 0x61, 0x8b, 0xd0,
            ]
        );
    }

    #[test]
    fn cloned_hasher_uses_independently_zeroizing_keyed_state() {
        let policy = policy("auth.login", "identity");
        let original = hasher();
        let cloned = original.clone();
        let expected = original.hash_for(&policy, b"user@example.test");

        drop(original);

        assert_eq!(
            cloned.hash_for(&policy, b"user@example.test").subject,
            expected.subject,
            "dropping the original must not invalidate the clone"
        );
        assert_eq!(format!("{cloned:?}"), "KeyHasher([REDACTED])");
    }
}
