use std::{fmt, str::FromStr};

use thiserror::Error;

/// Maximum encoded length of a policy or scope identifier.
pub const MAX_IDENTIFIER_LENGTH: usize = 128;

fn validate(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_LENGTH {
        return Err(IdentifierError::TooLong {
            actual: value.len(),
            maximum: MAX_IDENTIFIER_LENGTH,
        });
    }

    for (index, character) in value.char_indices() {
        if !(character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | ':' | '/'))
        {
            return Err(IdentifierError::InvalidCharacter { index, character });
        }
    }

    Ok(())
}

/// An invalid policy or scope identifier.
///
/// Identifiers are deliberately restricted to short ASCII tokens. This keeps
/// storage keys portable and avoids visually confusable policy namespaces.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IdentifierError {
    /// The identifier was empty.
    #[error("identifier must not be empty")]
    Empty,
    /// The identifier exceeded [`MAX_IDENTIFIER_LENGTH`].
    #[error("identifier is {actual} bytes; the maximum is {maximum}")]
    TooLong {
        /// Supplied byte length.
        actual: usize,
        /// Maximum accepted byte length.
        maximum: usize,
    },
    /// The identifier contained a byte outside the accepted token alphabet.
    #[error(
        "identifier contains invalid character {character:?} at byte index {index}; \
         use ASCII letters, digits, '-', '.', '_', ':', or '/'"
    )]
    InvalidCharacter {
        /// Zero-based byte index of the invalid character.
        index: usize,
        /// Invalid character.
        character: char,
    },
}

macro_rules! identifier_type {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// Values contain between 1 and 128 bytes and may use ASCII letters,
        /// digits, `-`, `.`, `_`, `:`, and `/`. Identifiers are case-sensitive.
        ///
        /// With the `serde` feature, this type uses a Serde string and applies
        /// the same validation when deserializing.
        #[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Box<str>);

        impl $name {
            /// Validates and constructs an identifier.
            ///
            /// # Errors
            ///
            /// Returns an error when the value is empty, longer than 128
            /// bytes, or contains a character outside the documented token
            /// alphabet.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate(&value)?;
                Ok(Self(value.into_boxed_str()))
            }

            /// Returns the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdentifierError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier_type!(
    PolicyId,
    "A stable application-defined rate-limit policy identifier."
);
identifier_type!(
    ScopeId,
    "A stable application-defined scope within a rate-limit policy."
);

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{IdentifierError, MAX_IDENTIFIER_LENGTH, PolicyId, ScopeId};

    #[test]
    fn accepts_portable_policy_and_scope_tokens() {
        let policy = PolicyId::new("auth/login:v2").unwrap();
        let scope: ScopeId = "client-ip_64".parse().unwrap();

        assert_eq!(policy.as_str(), "auth/login:v2");
        assert_eq!(scope.as_str(), "client-ip_64");
        assert_eq!(policy.to_string(), "auth/login:v2");
    }

    #[test]
    fn rejects_empty_identifiers() {
        assert_eq!(PolicyId::new(""), Err(IdentifierError::Empty));
        assert_eq!(ScopeId::new(""), Err(IdentifierError::Empty));
    }

    #[test]
    fn rejects_long_identifiers() {
        let value = "a".repeat(MAX_IDENTIFIER_LENGTH + 1);

        assert_eq!(
            PolicyId::new(value),
            Err(IdentifierError::TooLong {
                actual: MAX_IDENTIFIER_LENGTH + 1,
                maximum: MAX_IDENTIFIER_LENGTH,
            })
        );
    }

    #[test]
    fn rejects_whitespace_unicode_and_control_characters() {
        assert_eq!(
            PolicyId::new("auth login"),
            Err(IdentifierError::InvalidCharacter {
                index: 4,
                character: ' ',
            })
        );
        assert_eq!(
            ScopeId::new("café"),
            Err(IdentifierError::InvalidCharacter {
                index: 3,
                character: 'é',
            })
        );
        assert!(matches!(
            ScopeId::new("client\0ip"),
            Err(IdentifierError::InvalidCharacter {
                index: 6,
                character: '\0',
            })
        ));
    }

    #[test]
    fn identifiers_are_case_sensitive_and_orderable() {
        let upper = PolicyId::new("Login").unwrap();
        let lower = PolicyId::new("login").unwrap();
        let values = BTreeSet::from([lower.clone(), upper.clone()]);

        assert_ne!(upper, lower);
        assert_eq!(values.len(), 2);
    }
}
