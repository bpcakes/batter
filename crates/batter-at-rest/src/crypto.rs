use aes_gcm::{Aes256Gcm, KeyInit, Nonce, Tag, aead::AeadInPlace};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{
    ContentDescriptor, Context, Envelope, Error, FORMAT_VERSION, KeyId, Keyring,
    MAX_PLAINTEXT_BYTES, NONCE_BYTES, SECRET_KEY_BYTES, SealedPayload, SealedPayloadRef, TAG_BYTES,
    WrappedKey, key::SecretKey,
};

const HKDF_SALT: &[u8] = b"at-rest:envelope:v1:wrapping-key:salt";
const AAD_MAGIC: &[u8; 4] = b"ATRA";
const HKDF_MAGIC: &[u8; 4] = b"ATRK";
const PAYLOAD_ROLE: u8 = 1;
const WRAPPING_ROLE: u8 = 2;
const HKDF_WRAPPING_ROLE: u8 = 3;

impl Keyring {
    /// Seals bounded bytes under the current wrapping key and explicit context.
    pub fn seal(&self, context: &Context, plaintext: &[u8]) -> Result<SealedPayload, Error> {
        let mut random = SystemRandom;
        self.seal_with_random(context, plaintext, &mut random)
    }

    /// Authenticates and opens a borrowed header/body view under the supplied context.
    ///
    /// No plaintext is returned until both the data-key wrapper and body tags
    /// authenticate successfully.
    pub fn open(
        &self,
        context: &Context,
        sealed: SealedPayloadRef<'_>,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let envelope = sealed.envelope();
        let data_key = self.unwrap_data_key(context, envelope)?;
        let aad = payload_aad(context, envelope.content_descriptor());
        decrypt_bytes(
            &data_key,
            envelope.content_descriptor().payload_nonce(),
            &aad,
            sealed.ciphertext(),
        )
    }

    /// Authenticates the input wrapper and returns its descriptor paired with a
    /// current-key replacement wrapper.
    ///
    /// This metadata-only operation does not read or authenticate a ciphertext
    /// body. Persist the returned complete header with the original body using
    /// a compare-and-swap against the complete input envelope or a revision
    /// covering every header change. The storage owner must also fence obsolete
    /// key policies and enforce the per-derived-key wrapper-encryption budget.
    /// A decoded or reconstructed [`Envelope`] is only structurally validated;
    /// this method does not turn the type into a general body-authentication proof.
    ///
    /// If the wrapper already uses the current key, it is authenticated first
    /// and the exact original envelope is returned without requesting randomness.
    ///
    /// ```
    /// use batter_at_rest::{Context, Envelope, KeyId, Keyring, SealedPayloadRef, SecretKey};
    ///
    /// let old_id = KeyId::new("old")?;
    /// let new_id = KeyId::new("new")?;
    /// let old = Keyring::new(old_id.clone(),
    ///     [(old_id.clone(), SecretKey::from_bytes([7; 32]))])?;
    /// let rotating = Keyring::new(new_id.clone(), [
    ///     (old_id, SecretKey::from_bytes([7; 32])),
    ///     (new_id, SecretKey::from_bytes([9; 32])),
    /// ])?;
    /// let context = Context::for_row("example", "record", b"owner", b"item", "bytes-v1")?;
    /// let original = old.seal(&context, b"payload")?;
    /// let replacement = rotating.rewrap_envelope(&context, original.envelope())?;
    /// let persisted_header = replacement.encode();
    /// let stored = Envelope::decode(&persisted_header)?;
    /// let stored_view = SealedPayloadRef::new(&stored, original.ciphertext())?;
    /// assert_eq!(rotating.open(&context, stored_view)?.as_slice(), b"payload");
    /// # Ok::<(), batter_at_rest::Error>(())
    /// ```
    ///
    /// The complete result cannot be passed directly to the low-level wrapper
    /// composer for a different envelope:
    ///
    /// ```compile_fail,E0308
    /// use batter_at_rest::{Context, KeyId, Keyring, SecretKey};
    /// let id = KeyId::new("current")?;
    /// let keyring = Keyring::new(id.clone(), [(id, SecretKey::from_bytes([7; 32]))])?;
    /// let context = Context::for_row("example", "record", b"owner", b"item", "bytes-v1")?;
    /// let first = keyring.seal(&context, b"first")?;
    /// let second = keyring.seal(&context, b"second")?;
    /// let replacement = keyring.rewrap_envelope(&context, first.envelope())?;
    /// let _wrong = second.envelope().with_wrapped_key(replacement);
    /// # Ok::<(), batter_at_rest::Error>(())
    /// ```
    pub fn rewrap_envelope(
        &self,
        context: &Context,
        envelope: &Envelope,
    ) -> Result<Envelope, Error> {
        let mut random = SystemRandom;
        self.rewrap_envelope_with_random(context, envelope, &mut random)
    }

    /// Low-level compatibility: authenticate a wrapper and return only its
    /// current-key replacement metadata.
    ///
    /// This operation deliberately does not inspect or attest to any ciphertext
    /// body. The caller must preserve and reattach the exact input descriptor and
    /// body. Prefer [`Keyring::rewrap_envelope`] when persisting a complete header.
    /// If the wrapper is
    /// already current, the authenticated original wrapper is returned exactly.
    /// A returned wrapper belongs only to the input descriptor; persistence must
    /// compare-and-swap against the complete input envelope or a storage revision
    /// covering every envelope change. Descriptor-only comparison cannot detect
    /// another rewrap. Persistence must also fence obsolete key policies, either
    /// with a checked policy generation or exclusive maintenance that excludes
    /// stale writers. On conflict, reload both the record and active key policy.
    pub fn rewrap(&self, context: &Context, envelope: &Envelope) -> Result<WrappedKey, Error> {
        let mut random = SystemRandom;
        self.rewrap_with_random(context, envelope, &mut random)
    }

    fn rewrap_envelope_with_random<R: RandomSource>(
        &self,
        context: &Context,
        envelope: &Envelope,
        random: &mut R,
    ) -> Result<Envelope, Error> {
        let wrapped_key = self.rewrap_with_random(context, envelope, random)?;
        Ok(envelope.with_wrapped_key(wrapped_key))
    }

    fn seal_with_random<R: RandomSource>(
        &self,
        context: &Context,
        plaintext: &[u8],
        random: &mut R,
    ) -> Result<SealedPayload, Error> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(Error::PlaintextTooLarge);
        }

        let data_key = random_secret(random)?;
        let payload_nonce = random_array(random)?;
        let wrapping_nonce = random_array(random)?;
        let descriptor = ContentDescriptor::new(payload_nonce);

        let body = encrypt_bytes(
            &data_key,
            descriptor.payload_nonce(),
            &payload_aad(context, &descriptor),
            plaintext,
        )?;
        let wrapped_key = self.wrap_data_key(
            context,
            &descriptor,
            &self.current_id,
            self.current_key()?,
            wrapping_nonce,
            &data_key,
        )?;
        SealedPayload::from_parts(Envelope::from_parts(descriptor, wrapped_key), body)
    }

    fn rewrap_with_random<R: RandomSource>(
        &self,
        context: &Context,
        envelope: &Envelope,
        random: &mut R,
    ) -> Result<WrappedKey, Error> {
        let data_key = self.unwrap_data_key(context, envelope)?;
        if envelope.wrapped_key().key_id() == &self.current_id {
            return Ok(envelope.wrapped_key().clone());
        }

        let wrapping_nonce = random_array(random)?;
        self.wrap_data_key(
            context,
            envelope.content_descriptor(),
            &self.current_id,
            self.current_key()?,
            wrapping_nonce,
            &data_key,
        )
    }

    fn wrap_data_key(
        &self,
        context: &Context,
        descriptor: &ContentDescriptor,
        key_id: &KeyId,
        master_key: &SecretKey,
        wrapping_nonce: [u8; NONCE_BYTES],
        data_key: &[u8; SECRET_KEY_BYTES],
    ) -> Result<WrappedKey, Error> {
        let wrapping_key = derive_wrapping_key(master_key, context)?;
        let aad = wrapping_aad(context, descriptor, key_id);
        let encrypted = encrypt_bytes(&wrapping_key, &wrapping_nonce, &aad, data_key)?;
        let encrypted_data_key = encrypted
            .try_into()
            .map_err(|_| Error::CryptographicFailure)?;
        Ok(WrappedKey::new(
            key_id.clone(),
            wrapping_nonce,
            encrypted_data_key,
        ))
    }

    fn unwrap_data_key(
        &self,
        context: &Context,
        envelope: &Envelope,
    ) -> Result<Zeroizing<[u8; SECRET_KEY_BYTES]>, Error> {
        let wrapped_key = envelope.wrapped_key();
        let master_key = self.key(wrapped_key.key_id())?;
        let wrapping_key = derive_wrapping_key(master_key, context)?;
        let aad = wrapping_aad(context, envelope.content_descriptor(), wrapped_key.key_id());
        let plaintext = decrypt_bytes(
            &wrapping_key,
            wrapped_key.nonce(),
            &aad,
            wrapped_key.encrypted_data_key(),
        )?;
        if plaintext.len() != SECRET_KEY_BYTES {
            return Err(Error::AuthenticationFailed);
        }
        let mut data_key = Zeroizing::new([0_u8; SECRET_KEY_BYTES]);
        data_key.copy_from_slice(&plaintext);
        Ok(data_key)
    }
}

trait RandomSource {
    fn fill(&mut self, destination: &mut [u8]) -> Result<(), Error>;
}

struct SystemRandom;

impl RandomSource for SystemRandom {
    fn fill(&mut self, destination: &mut [u8]) -> Result<(), Error> {
        OsRng
            .try_fill_bytes(destination)
            .map_err(|_| Error::RandomnessUnavailable)
    }
}

fn random_secret<R: RandomSource>(
    random: &mut R,
) -> Result<Zeroizing<[u8; SECRET_KEY_BYTES]>, Error> {
    let mut bytes = Zeroizing::new([0_u8; SECRET_KEY_BYTES]);
    random.fill(&mut bytes[..])?;
    Ok(bytes)
}

fn random_array<R: RandomSource, const N: usize>(random: &mut R) -> Result<[u8; N], Error> {
    let mut bytes = [0_u8; N];
    random.fill(&mut bytes)?;
    Ok(bytes)
}

fn derive_wrapping_key(
    master_key: &SecretKey,
    context: &Context,
) -> Result<Zeroizing<[u8; SECRET_KEY_BYTES]>, Error> {
    let hkdf = Hkdf::<Sha256>::new(Some(HKDF_SALT), master_key.as_bytes());
    let mut info = Zeroizing::new(Vec::with_capacity(64));
    info.extend_from_slice(HKDF_MAGIC);
    info.extend_from_slice(&[FORMAT_VERSION, HKDF_WRAPPING_ROLE]);
    push_frame(&mut info, context.namespace());
    push_frame(&mut info, context.purpose());

    let mut key = Zeroizing::new([0_u8; SECRET_KEY_BYTES]);
    hkdf.expand(&info, &mut key[..])
        .map_err(|_| Error::CryptographicFailure)?;
    Ok(key)
}

fn payload_aad(context: &Context, descriptor: &ContentDescriptor) -> Zeroizing<Vec<u8>> {
    let mut aad = Zeroizing::new(Vec::with_capacity(8 + context.encoded().len()));
    aad.extend_from_slice(AAD_MAGIC);
    aad.extend_from_slice(&[PAYLOAD_ROLE, descriptor.format_version()]);
    push_frame(&mut aad, context.encoded());
    aad
}

fn wrapping_aad(
    context: &Context,
    descriptor: &ContentDescriptor,
    key_id: &KeyId,
) -> Zeroizing<Vec<u8>> {
    let mut aad = Zeroizing::new(Vec::with_capacity(
        24 + key_id.as_str().len() + context.encoded().len(),
    ));
    aad.extend_from_slice(AAD_MAGIC);
    aad.extend_from_slice(&[WRAPPING_ROLE, descriptor.format_version()]);
    push_frame(&mut aad, key_id.as_str().as_bytes());
    push_frame(&mut aad, descriptor.payload_nonce());
    push_frame(&mut aad, context.encoded());
    aad
}

fn push_frame(target: &mut Vec<u8>, bytes: &[u8]) {
    let length = u16::try_from(bytes.len()).expect("validated AAD component fits u16");
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(bytes);
}

fn encrypt_bytes(
    key: &[u8; SECRET_KEY_BYTES],
    nonce: &[u8; NONCE_BYTES],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::CryptographicFailure)?;
    let capacity = plaintext
        .len()
        .checked_add(TAG_BYTES)
        .ok_or(Error::CryptographicFailure)?;
    let mut encrypted = Zeroizing::new(Vec::with_capacity(capacity));
    encrypted.extend_from_slice(plaintext);
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), aad, &mut encrypted)
        .map_err(|_| Error::CryptographicFailure)?;
    encrypted.extend_from_slice(&tag);
    Ok(std::mem::take(&mut *encrypted))
}

fn decrypt_bytes(
    key: &[u8; SECRET_KEY_BYTES],
    nonce: &[u8; NONCE_BYTES],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if ciphertext_and_tag.len() < TAG_BYTES {
        return Err(Error::AuthenticationFailed);
    }
    let split = ciphertext_and_tag.len() - TAG_BYTES;
    let (ciphertext, tag) = ciphertext_and_tag.split_at(split);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| Error::CryptographicFailure)?;
    let mut plaintext = Zeroizing::new(ciphertext.to_vec());
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            aad,
            &mut plaintext,
            Tag::from_slice(tag),
        )
        .map_err(|_| Error::AuthenticationFailed)?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests;
