//! Flag keys, the settings prefix and the setting names derived from them.

use std::{fmt, sync::Arc};

use thiserror::Error;

/// Maximum length of a flag key in bytes.
pub const MAX_KEY_LENGTH: usize = 64;
/// Maximum length of a settings prefix in bytes, its trailing underscore included.
pub const MAX_PREFIX_LENGTH: usize = 64;
/// Maximum length of a setting name in bytes.
pub const MAX_SETTING_NAME_LENGTH: usize = 128;

/// A validated flag key: `[a-z][a-z0-9_]{0,63}`.
///
/// Keys are the catalog-wide identity of a declaration. Their setting name is
/// the catalog's [`SettingsPrefix`] followed by the key in upper case, which
/// is injective because upper-casing is one-to-one on the key alphabet.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FlagKey(Arc<str>);

impl FlagKey {
    /// Validates and constructs a key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is empty, longer than 64 bytes, does not
    /// start with a lower-case ASCII letter, or contains a character outside
    /// `[a-z0-9_]`.
    pub fn new(key: &str) -> Result<Self, KeyError> {
        validate_key(key)?;
        Ok(Self(Arc::from(key)))
    }

    /// The key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The setting name under `prefix`: the prefix followed by the key in
    /// upper case.
    pub fn setting_name(&self, prefix: &SettingsPrefix) -> SettingName {
        let mut name = String::with_capacity(prefix.as_str().len() + self.0.len());
        name.push_str(prefix.as_str());
        name.push_str(&self.0.to_ascii_uppercase());
        SettingName(name.into())
    }
}

impl fmt::Debug for FlagKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "FlagKey({:?})", &*self.0)
    }
}

impl fmt::Display for FlagKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for FlagKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// `text` when it is a valid name, otherwise a placeholder: stored or decoded
/// text is the application's and may be anything, so diagnostics show it only
/// when it cannot be more than a name.
pub(crate) fn name_or_placeholder(text: &str) -> String {
    if FlagKey::new(text).is_ok() {
        text.to_owned()
    } else {
        String::from("<not a name>")
    }
}

fn validate_key(key: &str) -> Result<(), KeyError> {
    let mut characters = key.chars();
    let Some(first) = characters.next() else {
        return Err(KeyError::Empty);
    };
    if key.len() > MAX_KEY_LENGTH {
        return Err(KeyError::TooLong {
            actual: key.len(),
            maximum: MAX_KEY_LENGTH,
        });
    }
    if !first.is_ascii_lowercase() {
        return Err(KeyError::InvalidStart { character: first });
    }
    for (index, character) in key.char_indices().skip(1) {
        if !(character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_') {
            return Err(KeyError::InvalidCharacter { index, character });
        }
    }
    Ok(())
}

/// An invalid flag key.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum KeyError {
    /// The key was empty.
    #[error("flag key must not be empty")]
    Empty,
    /// The key exceeded [`MAX_KEY_LENGTH`].
    #[error("flag key is {actual} bytes; the maximum is {maximum}")]
    TooLong {
        /// Supplied byte length.
        actual: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The key did not start with a lower-case ASCII letter.
    #[error("flag key must start with a lower-case ASCII letter, not {character:?}")]
    InvalidStart {
        /// The first character.
        character: char,
    },
    /// The key contained a character outside `[a-z0-9_]`.
    #[error(
        "flag key contains invalid character {character:?} at byte index {index}; \
         use lower-case ASCII letters, digits or '_'"
    )]
    InvalidCharacter {
        /// Zero-based byte index of the invalid character.
        index: usize,
        /// Invalid character.
        character: char,
    },
}

/// The validated settings prefix dedicated to one catalog's flags:
/// `[A-Z][A-Z0-9_]{0,62}_`.
///
/// Every setting name the catalog derives starts with it, and declared mode
/// rejects every other name under it, so the trailing underscore keeps the
/// namespace from matching unrelated settings that merely share its letters.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SettingsPrefix(Arc<str>);

impl SettingsPrefix {
    /// Validates and constructs a prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when the prefix is empty, longer than 64 bytes, does
    /// not start with an upper-case ASCII letter, does not end with `_`, or
    /// contains a character outside `[A-Z0-9_]`.
    pub fn new(prefix: &str) -> Result<Self, PrefixError> {
        validate_prefix(prefix)?;
        Ok(Self(Arc::from(prefix)))
    }

    /// The prefix text, trailing underscore included.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `name` lies under this prefix.
    pub fn covers(&self, name: &str) -> bool {
        name.starts_with(self.as_str())
    }
}

impl fmt::Debug for SettingsPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SettingsPrefix({:?})", &*self.0)
    }
}

impl fmt::Display for SettingsPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_prefix(prefix: &str) -> Result<(), PrefixError> {
    let mut characters = prefix.chars();
    let Some(first) = characters.next() else {
        return Err(PrefixError::Empty);
    };
    if prefix.len() > MAX_PREFIX_LENGTH {
        return Err(PrefixError::TooLong {
            actual: prefix.len(),
            maximum: MAX_PREFIX_LENGTH,
        });
    }
    if !first.is_ascii_uppercase() {
        return Err(PrefixError::InvalidStart { character: first });
    }
    for (index, character) in prefix.char_indices().skip(1) {
        if !is_setting_character(character) {
            return Err(PrefixError::InvalidCharacter { index, character });
        }
    }
    if !prefix.ends_with('_') {
        return Err(PrefixError::MissingTrailingUnderscore);
    }
    Ok(())
}

fn is_setting_character(character: char) -> bool {
    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
}

/// An invalid settings prefix.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PrefixError {
    /// The prefix was empty.
    #[error("settings prefix must not be empty")]
    Empty,
    /// The prefix exceeded [`MAX_PREFIX_LENGTH`].
    #[error("settings prefix is {actual} bytes; the maximum is {maximum}")]
    TooLong {
        /// Supplied byte length.
        actual: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The prefix did not start with an upper-case ASCII letter.
    #[error("settings prefix must start with an upper-case ASCII letter, not {character:?}")]
    InvalidStart {
        /// The first character.
        character: char,
    },
    /// The prefix contained a character outside `[A-Z0-9_]`.
    #[error(
        "settings prefix contains invalid character {character:?} at byte index {index}; \
         use upper-case ASCII letters, digits or '_'"
    )]
    InvalidCharacter {
        /// Zero-based byte index of the invalid character.
        index: usize,
        /// Invalid character.
        character: char,
    },
    /// The prefix did not end with `_`.
    #[error("settings prefix must end with '_' so that it names a whole namespace")]
    MissingTrailingUnderscore,
}

/// A setting name: a derived flag name, a retired name or a name a variant
/// requires: `[A-Z][A-Z0-9_]{0,127}`.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SettingName(Arc<str>);

impl SettingName {
    /// Validates and constructs a setting name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty, longer than 128 bytes, does
    /// not start with an upper-case ASCII letter, or contains a character
    /// outside `[A-Z0-9_]`.
    pub fn new(name: &str) -> Result<Self, SettingNameError> {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return Err(SettingNameError::Empty);
        };
        if name.len() > MAX_SETTING_NAME_LENGTH {
            return Err(SettingNameError::TooLong {
                actual: name.len(),
                maximum: MAX_SETTING_NAME_LENGTH,
            });
        }
        if !first.is_ascii_uppercase() {
            return Err(SettingNameError::InvalidStart { character: first });
        }
        for (index, character) in name.char_indices().skip(1) {
            if !is_setting_character(character) {
                return Err(SettingNameError::InvalidCharacter { index, character });
            }
        }
        Ok(Self(Arc::from(name)))
    }

    /// The name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SettingName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SettingName({:?})", &*self.0)
    }
}

impl fmt::Display for SettingName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for SettingName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// An invalid setting name.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SettingNameError {
    /// The name was empty.
    #[error("setting name must not be empty")]
    Empty,
    /// The name exceeded [`MAX_SETTING_NAME_LENGTH`].
    #[error("setting name is {actual} bytes; the maximum is {maximum}")]
    TooLong {
        /// Supplied byte length.
        actual: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The name did not start with an upper-case ASCII letter.
    #[error("setting name must start with an upper-case ASCII letter, not {character:?}")]
    InvalidStart {
        /// The first character.
        character: char,
    },
    /// The name contained a character outside `[A-Z0-9_]`.
    #[error(
        "setting name contains invalid character {character:?} at byte index {index}; \
         use upper-case ASCII letters, digits or '_'"
    )]
    InvalidCharacter {
        /// Zero-based byte index of the invalid character.
        index: usize,
        /// Invalid character.
        character: char,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_grammar() {
        assert!(FlagKey::new("a").is_ok());
        assert!(FlagKey::new("new_checkout_2").is_ok());
        assert_eq!(FlagKey::new(""), Err(KeyError::Empty));
        assert_eq!(
            FlagKey::new("Checkout"),
            Err(KeyError::InvalidStart { character: 'C' })
        );
        assert_eq!(
            FlagKey::new("2fa"),
            Err(KeyError::InvalidStart { character: '2' })
        );
        assert_eq!(
            FlagKey::new("new-checkout"),
            Err(KeyError::InvalidCharacter {
                index: 3,
                character: '-'
            })
        );
        let long = "a".repeat(MAX_KEY_LENGTH + 1);
        assert_eq!(
            FlagKey::new(&long),
            Err(KeyError::TooLong {
                actual: MAX_KEY_LENGTH + 1,
                maximum: MAX_KEY_LENGTH
            })
        );
        assert!(FlagKey::new(&"a".repeat(MAX_KEY_LENGTH)).is_ok());
    }

    #[test]
    fn prefixes_name_a_whole_namespace() {
        assert!(SettingsPrefix::new("FLAG_").is_ok());
        assert!(SettingsPrefix::new("APP_FLAGS_").is_ok());
        assert_eq!(SettingsPrefix::new(""), Err(PrefixError::Empty));
        assert_eq!(
            SettingsPrefix::new("FLAG"),
            Err(PrefixError::MissingTrailingUnderscore)
        );
        assert_eq!(
            SettingsPrefix::new("flag_"),
            Err(PrefixError::InvalidStart { character: 'f' })
        );
        assert_eq!(
            SettingsPrefix::new("FLAG-"),
            Err(PrefixError::InvalidCharacter {
                index: 4,
                character: '-'
            })
        );
    }

    #[test]
    fn setting_names_derive_by_upper_casing_the_key() {
        let prefix = SettingsPrefix::new("FLAG_").unwrap();
        let key = FlagKey::new("new_checkout_2").unwrap();
        assert_eq!(key.setting_name(&prefix).as_str(), "FLAG_NEW_CHECKOUT_2");
        assert!(prefix.covers("FLAG_NEW_CHECKOUT_2"));
        assert!(!prefix.covers("FLAGSHIP"));
        assert_eq!(
            SettingName::new("PAYMENT_API_URL").unwrap().as_str(),
            "PAYMENT_API_URL"
        );
        assert_eq!(SettingName::new(""), Err(SettingNameError::Empty));
        assert_eq!(
            SettingName::new("payment"),
            Err(SettingNameError::InvalidStart { character: 'p' })
        );
    }
}
