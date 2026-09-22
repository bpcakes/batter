use thiserror::Error;

/// A value-free failure from key validation, format validation, or cryptography.
///
/// Variants intentionally carry no source value. `Display` and `Debug` can
/// therefore be sent through ordinary error plumbing without exposing key
/// material, plaintext, or application context identifiers.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum Error {
    /// A master key did not contain exactly 32 bytes.
    #[error("invalid secret-key length")]
    InvalidSecretKey,
    /// An imported MAC key did not contain exactly 32 bytes.
    #[error("invalid MAC-key length")]
    InvalidMacKey,
    /// A wrapping-key identifier was empty, oversized, or noncanonical.
    #[error("invalid key identifier")]
    InvalidKeyId,
    /// Keyring construction received the same identifier more than once.
    #[error("duplicate key identifier")]
    DuplicateKeyId,
    /// The configured current key does not exist in the supplied entries.
    #[error("current key is unavailable")]
    CurrentKeyUnavailable,
    /// Opening or rewrapping requires a key absent from this keyring.
    #[error("required key is unavailable")]
    KeyUnavailable,
    /// A context component was empty, oversized, or noncanonical.
    #[error("invalid encryption context")]
    InvalidContext,
    /// The plaintext exceeds the format's documented limit.
    #[error("plaintext exceeds the supported limit")]
    PlaintextTooLarge,
    /// Encoded data is truncated, inconsistent, oversized, or noncanonical.
    #[error("malformed at-rest encoding")]
    MalformedEncoding,
    /// Encoded data names a cryptographic format this crate does not support.
    #[error("unsupported at-rest format version")]
    UnsupportedVersion,
    /// Bytes remain after an otherwise complete canonical value.
    #[error("trailing data after at-rest encoding")]
    TrailingData,
    /// Ciphertext, associated data, a wrapped data key, or a MAC failed authentication.
    #[error("at-rest authentication failed")]
    AuthenticationFailed,
    /// The operating-system random source could not fill a required value.
    #[error("operating-system randomness is unavailable")]
    RandomnessUnavailable,
    /// A fixed cryptographic construction could not be initialized.
    #[error("at-rest cryptographic operation failed")]
    CryptographicFailure,
}
