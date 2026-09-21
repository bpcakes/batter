use std::{collections::BTreeMap, fmt, sync::Arc};

use zeroize::Zeroizing;

use crate::{Error, MAX_KEY_ID_BYTES, SECRET_KEY_BYTES};

/// An opaque, validated 32-byte master key.
///
/// The owned bytes are zeroized on drop. This cannot erase copies retained by
/// the caller before construction.
pub struct SecretKey {
    bytes: Zeroizing<[u8; SECRET_KEY_BYTES]>,
}

impl SecretKey {
    /// Takes ownership of an exact-size master key.
    #[must_use]
    pub fn from_bytes(bytes: [u8; SECRET_KEY_BYTES]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// Copies an exact-size byte slice into owned, zeroizing storage.
    ///
    /// The input slice remains the caller's responsibility.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let bytes = bytes.try_into().map_err(|_| Error::InvalidSecretKey)?;
        Ok(Self::from_bytes(bytes))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; SECRET_KEY_BYTES] {
        &self.bytes
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretKey(<redacted>)")
    }
}

/// A validated identifier stored beside a wrapped data key.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyId(String);

impl KeyId {
    /// Validates and owns a 1–128 byte case-sensitive identifier.
    ///
    /// Accepted bytes are ASCII letters, digits, `.`, `_`, `:`, and `-`.
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.is_empty()
            || value.len() > MAX_KEY_ID_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        {
            return Err(Error::InvalidKeyId);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the identifier for persistence or key-selection configuration.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyId(<redacted>)")
    }
}

impl AsRef<str> for KeyId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<&str> for KeyId {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// An immutable collection of wrapping master keys with one current key.
///
/// Cloning a keyring shares the same secret allocations rather than copying
/// key bytes. Construction consumes an entry sequence so duplicate IDs can be
/// rejected before a map would silently overwrite them.
#[derive(Clone)]
pub struct Keyring {
    pub(crate) current_id: KeyId,
    pub(crate) keys: Arc<BTreeMap<KeyId, Arc<SecretKey>>>,
}

impl Keyring {
    /// Builds a keyring and proves that the current ID is present and unique.
    pub fn new<I>(current_id: KeyId, entries: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (KeyId, SecretKey)>,
    {
        let mut keys = BTreeMap::new();
        for (key_id, secret_key) in entries {
            if keys.insert(key_id, Arc::new(secret_key)).is_some() {
                return Err(Error::DuplicateKeyId);
            }
        }
        if !keys.contains_key(&current_id) {
            return Err(Error::CurrentKeyUnavailable);
        }
        Ok(Self {
            current_id,
            keys: Arc::new(keys),
        })
    }

    /// Returns the ID used for new wrappers.
    #[must_use]
    pub fn current_key_id(&self) -> &KeyId {
        &self.current_id
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_current_key_for_test(&self, key_id: KeyId, secret_key: SecretKey) -> Self {
        let mut keys = self.keys.as_ref().clone();
        keys.insert(key_id.clone(), Arc::new(secret_key));
        Self {
            current_id: key_id,
            keys: Arc::new(keys),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn without_historical_key_for_test(&self, key_id: &KeyId) -> Self {
        assert_ne!(
            &self.current_id, key_id,
            "the current wrapping key cannot be removed"
        );
        let mut keys = self.keys.as_ref().clone();
        keys.remove(key_id);
        Self {
            current_id: self.current_id.clone(),
            keys: Arc::new(keys),
        }
    }

    pub(crate) fn key(&self, key_id: &KeyId) -> Result<&SecretKey, Error> {
        self.keys
            .get(key_id)
            .map(AsRef::as_ref)
            .ok_or(Error::KeyUnavailable)
    }

    pub(crate) fn current_key(&self) -> Result<&SecretKey, Error> {
        self.key(&self.current_id)
    }
}

impl fmt::Debug for Keyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Keyring")
            .field("current_id", &"<redacted>")
            .field("key_count", &self.keys.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
