use std::{fmt, str};

use crate::{
    Error, FORMAT_VERSION, MAX_BODY_BYTES, MAX_ENVELOPE_BYTES, MAX_KEY_ID_BYTES, NONCE_BYTES,
    SECRET_KEY_BYTES, TAG_BYTES, key::KeyId,
};

const DESCRIPTOR_BYTES: usize = 2 + NONCE_BYTES;
pub(crate) const WRAPPED_DATA_KEY_BYTES: usize = SECRET_KEY_BYTES + TAG_BYTES;
const MAX_WRAPPED_KEY_BYTES: usize =
    1 + MAX_KEY_ID_BYTES + 1 + NONCE_BYTES + 2 + WRAPPED_DATA_KEY_BYTES;
const MAX_CANONICAL_ENVELOPE_BYTES: usize = 4 + 2 + DESCRIPTOR_BYTES + 2 + MAX_WRAPPED_KEY_BYTES;
const _: () = assert!(MAX_CANONICAL_ENVELOPE_BYTES <= MAX_ENVELOPE_BYTES);
const ENVELOPE_MAGIC: &[u8; 4] = b"ATRE";
const SEALED_MAGIC: &[u8; 4] = b"ATRS";

/// Structurally validated content metadata: cryptographic format and payload nonce.
/// Decoding does not authenticate these values; successful cryptographic operations do.
#[derive(Clone, Eq, PartialEq)]
pub struct ContentDescriptor {
    format_version: u8,
    payload_nonce: [u8; NONCE_BYTES],
}

impl ContentDescriptor {
    pub(crate) fn new(payload_nonce: [u8; NONCE_BYTES]) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            payload_nonce,
        }
    }

    /// Decodes one exact canonical descriptor.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut decoder = Decoder::new(bytes);
        let format_version = decoder.byte()?;
        if format_version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion);
        }
        if usize::from(decoder.byte()?) != NONCE_BYTES {
            return Err(Error::MalformedEncoding);
        }
        let payload_nonce = decoder.array()?;
        decoder.finish()?;
        Ok(Self {
            format_version,
            payload_nonce,
        })
    }

    /// Encodes the descriptor as exactly 14 canonical bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; DESCRIPTOR_BYTES] {
        let mut encoded = [0_u8; DESCRIPTOR_BYTES];
        encoded[0] = self.format_version;
        encoded[1] = NONCE_BYTES as u8;
        encoded[2..].copy_from_slice(&self.payload_nonce);
        encoded
    }

    /// Returns the cryptographic format version.
    #[must_use]
    pub fn format_version(&self) -> u8 {
        self.format_version
    }

    /// Returns the payload nonce; decoding alone does not authenticate it.
    #[must_use]
    pub fn payload_nonce(&self) -> &[u8; NONCE_BYTES] {
        &self.payload_nonce
    }
}

impl fmt::Debug for ContentDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentDescriptor")
            .field("format_version", &self.format_version)
            .field("payload_nonce", &"<redacted>")
            .finish()
    }
}

/// Structurally validated metadata containing an encrypted data key and its tag.
#[derive(Clone, Eq, PartialEq)]
pub struct WrappedKey {
    key_id: KeyId,
    nonce: [u8; NONCE_BYTES],
    encrypted_data_key: [u8; WRAPPED_DATA_KEY_BYTES],
}

impl WrappedKey {
    pub(crate) fn new(
        key_id: KeyId,
        nonce: [u8; NONCE_BYTES],
        encrypted_data_key: [u8; WRAPPED_DATA_KEY_BYTES],
    ) -> Self {
        Self {
            key_id,
            nonce,
            encrypted_data_key,
        }
    }

    /// Decodes one exact canonical wrapper for the supplied descriptor version.
    pub fn decode_for(descriptor: &ContentDescriptor, bytes: &[u8]) -> Result<Self, Error> {
        if descriptor.format_version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let mut decoder = Decoder::new(bytes);
        let key_id_length = usize::from(decoder.byte()?);
        let key_id_bytes = decoder.take(key_id_length)?;
        let key_id = KeyId::new(str::from_utf8(key_id_bytes).map_err(|_| Error::InvalidKeyId)?)?;
        if usize::from(decoder.byte()?) != NONCE_BYTES {
            return Err(Error::MalformedEncoding);
        }
        let nonce = decoder.array()?;
        if usize::from(decoder.u16()?) != WRAPPED_DATA_KEY_BYTES {
            return Err(Error::MalformedEncoding);
        }
        let encrypted_data_key = decoder.array()?;
        decoder.finish()?;
        Ok(Self {
            key_id,
            nonce,
            encrypted_data_key,
        })
    }

    /// Encodes this wrapper without duplicating the descriptor's format version.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let key_id = self.key_id.as_str().as_bytes();
        let mut encoded =
            Vec::with_capacity(1 + key_id.len() + 1 + NONCE_BYTES + 2 + WRAPPED_DATA_KEY_BYTES);
        encoded.push(u8::try_from(key_id.len()).expect("validated key ID fits u8"));
        encoded.extend_from_slice(key_id);
        encoded.push(NONCE_BYTES as u8);
        encoded.extend_from_slice(&self.nonce);
        encoded.extend_from_slice(&(WRAPPED_DATA_KEY_BYTES as u16).to_be_bytes());
        encoded.extend_from_slice(&self.encrypted_data_key);
        encoded
    }

    /// Returns the configured master-key ID needed to open this wrapper.
    #[must_use]
    pub fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    pub(crate) fn nonce(&self) -> &[u8; NONCE_BYTES] {
        &self.nonce
    }

    pub(crate) fn encrypted_data_key(&self) -> &[u8; WRAPPED_DATA_KEY_BYTES] {
        &self.encrypted_data_key
    }
}

impl fmt::Debug for WrappedKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WrappedKey")
            .field("key_id", &"<redacted>")
            .field("nonce", &"<redacted>")
            .field("encrypted_data_key", &"<redacted>")
            .finish()
    }
}

/// A structurally validated header with immutable content and rotatable key metadata.
/// Construction and decoding do not verify authentication tags.
#[derive(Clone, Eq, PartialEq)]
pub struct Envelope {
    descriptor: ContentDescriptor,
    wrapped_key: WrappedKey,
}

impl Envelope {
    /// Combines independently validated descriptor and wrapper parts.
    #[must_use]
    pub fn from_parts(descriptor: ContentDescriptor, wrapped_key: WrappedKey) -> Self {
        Self {
            descriptor,
            wrapped_key,
        }
    }

    /// Decodes one exact canonical envelope header.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(Error::MalformedEncoding);
        }
        let mut decoder = Decoder::new(bytes);
        decoder.magic(ENVELOPE_MAGIC)?;
        let descriptor_bytes = decoder.length_prefixed_u16()?;
        let descriptor = ContentDescriptor::decode(descriptor_bytes)?;
        let wrapped_bytes = decoder.length_prefixed_u16()?;
        let wrapped_key = WrappedKey::decode_for(&descriptor, wrapped_bytes)?;
        decoder.finish()?;
        Ok(Self {
            descriptor,
            wrapped_key,
        })
    }

    /// Encodes the complete canonical envelope header.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let descriptor = self.descriptor.encode();
        let wrapped_key = self.wrapped_key.encode();
        let mut encoded = Vec::with_capacity(4 + 2 + descriptor.len() + 2 + wrapped_key.len());
        encoded.extend_from_slice(ENVELOPE_MAGIC);
        push_u16_length(&mut encoded, &descriptor);
        push_u16_length(&mut encoded, &wrapped_key);
        debug_assert!(encoded.len() <= MAX_ENVELOPE_BYTES);
        encoded
    }

    /// Returns the immutable content descriptor.
    #[must_use]
    pub fn content_descriptor(&self) -> &ContentDescriptor {
        &self.descriptor
    }

    /// Returns the rotatable wrapped-key metadata.
    #[must_use]
    pub fn wrapped_key(&self) -> &WrappedKey {
        &self.wrapped_key
    }

    /// Returns a new header with the same descriptor and replacement wrapped key.
    #[must_use]
    pub fn with_wrapped_key(&self, wrapped_key: WrappedKey) -> Self {
        Self {
            descriptor: self.descriptor.clone(),
            wrapped_key,
        }
    }
}

impl fmt::Debug for Envelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Envelope")
            .field("descriptor", &self.descriptor)
            .field("wrapped_key", &self.wrapped_key)
            .finish()
    }
}

/// An owned envelope header and bounded ciphertext body, including its tag.
/// Construction and decoding validate structure, not cryptographic authenticity.
#[derive(Eq, PartialEq)]
pub struct SealedPayload {
    envelope: Envelope,
    ciphertext: Vec<u8>,
}

impl SealedPayload {
    /// Combines a validated envelope with a bounded ciphertext body.
    pub fn from_parts(envelope: Envelope, ciphertext: Vec<u8>) -> Result<Self, Error> {
        validate_body(&ciphertext)?;
        Ok(Self {
            envelope,
            ciphertext,
        })
    }

    /// Decodes one exact canonical composite payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut decoder = Decoder::new(bytes);
        decoder.magic(SEALED_MAGIC)?;
        let envelope = Envelope::decode(decoder.length_prefixed_u16()?)?;
        let body_length = usize::try_from(decoder.u32()?).map_err(|_| Error::MalformedEncoding)?;
        validate_body_length(body_length)?;
        let ciphertext = decoder.take(body_length)?;
        decoder.finish()?;
        Self::from_parts(envelope, ciphertext.to_vec())
    }

    /// Encodes the envelope and body into one canonical portable value.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let envelope = self.envelope.encode();
        let body_length =
            u32::try_from(self.ciphertext.len()).expect("validated ciphertext body fits u32");
        let mut encoded = Vec::with_capacity(4 + 2 + envelope.len() + 4 + self.ciphertext.len());
        encoded.extend_from_slice(SEALED_MAGIC);
        push_u16_length(&mut encoded, &envelope);
        encoded.extend_from_slice(&body_length.to_be_bytes());
        encoded.extend_from_slice(&self.ciphertext);
        encoded
    }

    /// Returns the validated envelope header.
    #[must_use]
    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// Returns the ciphertext body, including its unverified authentication tag.
    #[must_use]
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// Borrows split storage without concatenating or cloning ciphertext.
    #[must_use]
    pub fn as_ref(&self) -> SealedPayloadRef<'_> {
        SealedPayloadRef {
            envelope: &self.envelope,
            ciphertext: &self.ciphertext,
        }
    }
}

impl fmt::Debug for SealedPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedPayload")
            .field("envelope", &self.envelope)
            .field("ciphertext", &"<redacted>")
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

/// A borrowed decrypt view over a structurally validated header and bounded body.
/// Construction does not authenticate the header or body.
#[derive(Clone, Copy)]
pub struct SealedPayloadRef<'a> {
    envelope: &'a Envelope,
    ciphertext: &'a [u8],
}

impl<'a> SealedPayloadRef<'a> {
    /// Checks body length and borrows an independently stored header and body.
    pub fn new(envelope: &'a Envelope, ciphertext: &'a [u8]) -> Result<Self, Error> {
        validate_body(ciphertext)?;
        Ok(Self {
            envelope,
            ciphertext,
        })
    }

    /// Returns the borrowed envelope header.
    #[must_use]
    pub fn envelope(self) -> &'a Envelope {
        self.envelope
    }

    /// Returns the borrowed ciphertext body.
    #[must_use]
    pub fn ciphertext(self) -> &'a [u8] {
        self.ciphertext
    }
}

impl fmt::Debug for SealedPayloadRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedPayloadRef")
            .field("envelope", self.envelope)
            .field("ciphertext", &"<redacted>")
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

fn validate_body(ciphertext: &[u8]) -> Result<(), Error> {
    validate_body_length(ciphertext.len())
}

fn validate_body_length(length: usize) -> Result<(), Error> {
    if !(TAG_BYTES..=MAX_BODY_BYTES).contains(&length) {
        return Err(Error::MalformedEncoding);
    }
    Ok(())
}

fn push_u16_length(target: &mut Vec<u8>, value: &[u8]) {
    let length = u16::try_from(value.len()).expect("validated encoding component fits u16");
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(value);
}

struct Decoder<'a> {
    remaining: &'a [u8],
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::MalformedEncoding)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        if length > self.remaining.len() {
            return Err(Error::MalformedEncoding);
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn magic(&mut self, expected: &[u8]) -> Result<(), Error> {
        if self.take(expected.len())? != expected {
            return Err(Error::MalformedEncoding);
        }
        Ok(())
    }

    fn length_prefixed_u16(&mut self) -> Result<&'a [u8], Error> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }

    fn finish(self) -> Result<(), Error> {
        if !self.remaining.is_empty() {
            return Err(Error::TrailingData);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
