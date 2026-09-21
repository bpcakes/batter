#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod context;
mod crypto;
mod error;
mod format;
mod key;
mod mac;

pub use context::Context;
pub use error::Error;
pub use format::{ContentDescriptor, Envelope, SealedPayload, SealedPayloadRef, WrappedKey};
pub use key::{KeyId, Keyring, SecretKey};
pub use mac::MacKey;

/// The only cryptographic format version implemented by this crate.
pub const FORMAT_VERSION: u8 = 1;
/// Required byte length of a configured master key and random data key.
pub const SECRET_KEY_BYTES: usize = 32;
/// Required byte length of an imported HMAC-SHA256 key and its tag.
pub const MAC_BYTES: usize = 32;
/// Required AES-GCM nonce length.
pub const NONCE_BYTES: usize = 12;
/// Required AES-GCM authentication-tag length.
pub const TAG_BYTES: usize = 16;
/// Maximum persisted key-ID length.
pub const MAX_KEY_ID_BYTES: usize = 128;
/// Maximum namespace, purpose, or payload-schema label length.
pub const MAX_CONTEXT_LABEL_BYTES: usize = 64;
/// Maximum owner, resource, or tenant binding component length.
pub const MAX_BINDING_COMPONENT_BYTES: usize = 128;
/// Maximum canonical encoded context length.
pub const MAX_CONTEXT_BYTES: usize = 512;
/// Maximum canonical encoded envelope-header length.
pub const MAX_ENVELOPE_BYTES: usize = 256;
/// Maximum plaintext accepted by [`Keyring::seal`].
pub const MAX_PLAINTEXT_BYTES: usize = 64 * 1024 * 1024;
/// Maximum ciphertext-body length, including its authentication tag.
pub const MAX_BODY_BYTES: usize = MAX_PLAINTEXT_BYTES + TAG_BYTES;
