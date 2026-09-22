use std::{fmt, sync::Arc};

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{Error, MAC_BYTES};

type HmacSha256 = Hmac<Sha256>;

/// An opaque, independently owned HMAC-SHA256 key.
///
/// Imported bytes are final key material: construction never applies a hidden
/// derivation or domain prefix. Call [`Self::derive_hkdf`] explicitly when a
/// new key should be derived. The owned bytes are zeroized on final drop;
/// cloning shares the same allocation rather than copying the secret.
#[derive(Clone)]
pub struct MacKey {
    bytes: Arc<Zeroizing<[u8; MAC_BYTES]>>,
}

impl MacKey {
    /// Takes ownership of exact-size, final HMAC-SHA256 key material.
    #[must_use]
    pub fn from_bytes(bytes: [u8; MAC_BYTES]) -> Self {
        Self::from_zeroizing(Zeroizing::new(bytes))
    }

    /// Copies exact-size, final key material into owned, zeroizing storage.
    ///
    /// The input slice remains the caller's responsibility. No key derivation
    /// is applied.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != MAC_BYTES {
            return Err(Error::InvalidMacKey);
        }
        let mut owned = Zeroizing::new([0_u8; MAC_BYTES]);
        owned.copy_from_slice(bytes);
        Ok(Self::from_zeroizing(owned))
    }

    /// Derives a new 32-byte HMAC key with standard HKDF-SHA256.
    ///
    /// The salt and info are used exactly as supplied; this crate adds no
    /// prefix. The input key material remains the caller's responsibility.
    #[must_use]
    pub fn derive_hkdf(input_key_material: &[u8], salt: Option<&[u8]>, info: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(salt, input_key_material);
        let mut bytes = Zeroizing::new([0_u8; MAC_BYTES]);
        hkdf.expand(info, &mut bytes[..])
            .expect("a 32-byte HKDF-SHA256 output is always valid");
        Self::from_zeroizing(bytes)
    }

    /// Computes HMAC-SHA256 over one byte string.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> [u8; MAC_BYTES] {
        self.sign_segments([message])
    }

    /// Computes HMAC-SHA256 over the exact concatenation of byte segments.
    ///
    /// Segment boundaries are not encoded. A caller that needs canonical
    /// framing must supply those framing bytes as segments.
    #[must_use]
    pub fn sign_segments<'a, I>(&self, segments: I) -> [u8; MAC_BYTES]
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut mac = self.computation();
        for segment in segments {
            mac.update(segment);
        }
        let output = mac.finalize().into_bytes();
        let mut tag = [0_u8; MAC_BYTES];
        tag.copy_from_slice(&output);
        tag
    }

    /// Verifies an HMAC-SHA256 tag for one byte string in constant time.
    pub fn verify(&self, message: &[u8], expected: &[u8]) -> Result<(), Error> {
        self.verify_segments([message], expected)
    }

    /// Verifies an HMAC-SHA256 tag over the exact concatenation of segments.
    ///
    /// Verification uses the HMAC implementation's constant-time tag check.
    pub fn verify_segments<'a, I>(&self, segments: I, expected: &[u8]) -> Result<(), Error>
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut mac = self.computation();
        for segment in segments {
            mac.update(segment);
        }
        mac.verify_slice(expected)
            .map_err(|_| Error::AuthenticationFailed)
    }

    fn computation(&self) -> HmacSha256 {
        <HmacSha256 as Mac>::new_from_slice(self.bytes.as_ref().as_ref())
            .expect("HMAC accepts keys of any size")
    }

    fn from_zeroizing(bytes: Zeroizing<[u8; MAC_BYTES]>) -> Self {
        Self {
            bytes: Arc::new(bytes),
        }
    }
}

impl fmt::Debug for MacKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MacKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests;
