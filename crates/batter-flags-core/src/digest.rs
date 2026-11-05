//! SHA-256 digests in the length-framed form every batter-flags digest uses.
//!
//! Framing: the digest's own label is the first field; every field is its
//! length as a big-endian `u64` followed by its bytes; every list is preceded
//! by its element count as a big-endian `u64`; an optional field is the count
//! 0, or the count 1 followed by the field. Two inputs that differ only in how
//! their bytes split into fields therefore digest differently, which the
//! golden vectors in the tests pin. Every sorted list is ordered by the bytes
//! of its text fields, so an independent implementation can reproduce it.
//!
//! The catalog digest frames, in order: the label `batter.flags.catalog.v1`; the
//! settings prefix; the sorted process-kind names; the
//! sorted environment-class names; the sorted retired names; then the count of
//! declarations and, for each in key order, its key, kind tag, value-type tag,
//! sorted variants, optional default variant, absence tag, owners (`all`, or
//! `only` followed by the sorted kind names), sorted `pinned_by` names,
//! optional `only_in` class, optional safe-state tag, optional stale-rule tag,
//! optional governing-switch key, the count of `requires_settings` entries with
//! each variant and its sorted names in variant order, and the count of
//! invariants with each rule tag, variant, other key and other variant,
//! sorted by those four texts in that order (so `both_or_none` < `conflicts` <
//! `requires`). Expiry, flip discipline and migration notes are not framed.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{
    declaration::{Absence, InvariantRule, Kind, SafeState, StaleRule},
    value::ValueType,
};

pub(crate) const CATALOG_LABEL: &str = "batter.flags.catalog.v1";
pub(crate) const DECLARATION_LABEL: &str = "batter.flags.declaration.v1";
#[cfg(feature = "serde")]
pub(crate) const MANIFEST_LABEL: &str = "batter.flags.manifest.v1";

/// A hasher applying the framing above.
pub(crate) struct Framing {
    hasher: Sha256,
}

impl Framing {
    /// Starts a digest whose first field is `label`.
    pub(crate) fn new(label: &str) -> Self {
        let mut framing = Self {
            hasher: Sha256::new(),
        };
        framing.field(label.as_bytes());
        framing
    }

    /// One length-prefixed field.
    pub(crate) fn field(&mut self, bytes: &[u8]) {
        self.hasher.update((bytes.len() as u64).to_be_bytes());
        self.hasher.update(bytes);
    }

    /// The element count that precedes a list.
    pub(crate) fn count(&mut self, count: usize) {
        self.hasher.update((count as u64).to_be_bytes());
    }

    /// A list of string fields: its count, then each field.
    pub(crate) fn list<'a>(&mut self, items: impl ExactSizeIterator<Item = &'a str>) {
        self.count(items.len());
        for item in items {
            self.field(item.as_bytes());
        }
    }

    /// A list of string fields in sorted order.
    pub(crate) fn sorted_list<'a>(&mut self, items: impl Iterator<Item = &'a str>) {
        let mut sorted: Vec<&str> = items.collect();
        sorted.sort_unstable();
        self.list(sorted.into_iter());
    }

    /// An optional field: the count 0, or the count 1 and the field.
    pub(crate) fn optional(&mut self, item: Option<&str>) {
        match item {
            None => self.count(0),
            Some(item) => {
                self.count(1);
                self.field(item.as_bytes());
            }
        }
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }
}

macro_rules! digest_type {
    ($name:ident, $label:expr, $description:literal) => {
        #[doc = $description]
        ///
        /// A SHA-256 digest in the batter-flags framing, labelled
        #[doc = concat!("`", $label, "`. Displays as 64 lower-case hex digits.")]
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub(crate) fn from_framing(framing: Framing) -> Self {
                Self(framing.finish())
            }

            /// The raw digest bytes.
            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// The digest as 64 lower-case hex digits.
            pub fn to_hex(&self) -> String {
                self.to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({self})", stringify!($name))
            }
        }

        impl std::str::FromStr for $name {
            type Err = DigestParseError;

            /// Parses 64 lower-case hex digits.
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                parse_hex(text).map(Self)
            }
        }

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

/// Text that is not a digest: 64 lower-case hex digits.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
#[error("a digest is written as 64 lower-case hex digits")]
pub struct DigestParseError;

fn parse_hex(text: &str) -> Result<[u8; 32], DigestParseError> {
    fn nibble(digit: u8) -> Result<u8, DigestParseError> {
        match digit {
            b'0'..=b'9' => Ok(digit - b'0'),
            b'a'..=b'f' => Ok(digit - b'a' + 10),
            _ => Err(DigestParseError),
        }
    }
    let digits = text.as_bytes();
    if digits.len() != 64 {
        return Err(DigestParseError);
    }
    let mut bytes = [0_u8; 32];
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Ok(bytes)
}

digest_type!(
    CatalogDigest,
    "batter.flags.catalog.v1",
    "The digest of everything resolution and evaluation depend on in a catalog; the `feature_flag.set.id` of its evaluations."
);
digest_type!(
    DeclarationFingerprint,
    "batter.flags.declaration.v1",
    "The digest of one declaration's key, kind, value-type tag and sorted variants, carried by its handle."
);
#[cfg(feature = "serde")]
digest_type!(
    ManifestDigest,
    "batter.flags.manifest.v1",
    "The digest of a catalog's canonical manifest bytes; the identity of a published catalog."
);

impl DeclarationFingerprint {
    /// The fingerprint of a declaration: label, key, kind tag, value-type tag,
    /// then the variants as a sorted list.
    pub(crate) fn compute(key: &str, kind: &str, value_type: &str, variants: &[&str]) -> Self {
        let mut framing = Framing::new(DECLARATION_LABEL);
        framing.field(key.as_bytes());
        framing.field(kind.as_bytes());
        framing.field(value_type.as_bytes());
        let mut sorted: Vec<&str> = variants.to_vec();
        sorted.sort_unstable();
        framing.list(sorted.iter().copied());
        Self::from_framing(framing)
    }
}

/// The inputs of a catalog digest, as text. [`CatalogDigest::compute`] sorts
/// every list itself, so callers may pass them in any order.
pub(crate) struct DigestInput<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) process_kinds: Vec<&'a str>,
    pub(crate) environment_classes: Vec<&'a str>,
    pub(crate) retired: Vec<&'a str>,
    pub(crate) entries: Vec<EntryView<'a>>,
}

/// One declaration's digest inputs, as text.
pub(crate) struct EntryView<'a> {
    pub(crate) key: &'a str,
    pub(crate) kind: Kind,
    pub(crate) value_type: ValueType,
    pub(crate) variants: Vec<&'a str>,
    pub(crate) default_variant: Option<&'a str>,
    pub(crate) absence: Absence,
    /// `None` when every kind owns the declaration.
    pub(crate) owners: Option<Vec<&'a str>>,
    pub(crate) pinned_by: Vec<&'a str>,
    pub(crate) only_in: Option<&'a str>,
    pub(crate) safe_state: Option<SafeState>,
    pub(crate) stale_rule: Option<StaleRule>,
    pub(crate) governed_by: Option<&'a str>,
    pub(crate) requires_settings: Vec<(&'a str, Vec<&'a str>)>,
    /// Rule, variant, other key and other variant.
    pub(crate) invariants: Vec<(InvariantRule, &'a str, &'a str, &'a str)>,
}

impl CatalogDigest {
    pub(crate) fn compute(mut input: DigestInput<'_>) -> Self {
        let mut framing = Framing::new(CATALOG_LABEL);
        framing.field(input.prefix.as_bytes());
        framing.sorted_list(input.process_kinds.into_iter());
        framing.sorted_list(input.environment_classes.into_iter());
        framing.sorted_list(input.retired.into_iter());
        input.entries.sort_unstable_by_key(|entry| entry.key);
        framing.count(input.entries.len());
        for entry in input.entries {
            frame_entry(&mut framing, entry);
        }
        Self::from_framing(framing)
    }
}

fn frame_entry(framing: &mut Framing, mut entry: EntryView<'_>) {
    framing.field(entry.key.as_bytes());
    framing.field(entry.kind.as_str().as_bytes());
    framing.field(entry.value_type.as_str().as_bytes());
    framing.sorted_list(entry.variants.into_iter());
    framing.optional(entry.default_variant);
    framing.field(entry.absence.as_str().as_bytes());
    match entry.owners {
        None => framing.field(b"all"),
        Some(kinds) => {
            framing.field(b"only");
            framing.sorted_list(kinds.into_iter());
        }
    }
    framing.sorted_list(entry.pinned_by.into_iter());
    framing.optional(entry.only_in);
    framing.optional(entry.safe_state.map(SafeState::as_str));
    framing.optional(entry.stale_rule.map(StaleRule::as_str));
    framing.optional(entry.governed_by);
    entry
        .requires_settings
        .sort_unstable_by_key(|(variant, _)| *variant);
    framing.count(entry.requires_settings.len());
    for (variant, names) in entry.requires_settings {
        framing.field(variant.as_bytes());
        framing.sorted_list(names.into_iter());
    }
    entry.invariants.sort_unstable();
    framing.count(entry.invariants.len());
    for (rule, variant, other, other_variant) in entry.invariants {
        framing.field(rule.as_str().as_bytes());
        framing.field(variant.as_bytes());
        framing.field(other.as_bytes());
        framing.field(other_variant.as_bytes());
    }
}

#[cfg(feature = "serde")]
impl ManifestDigest {
    /// The digest of canonical manifest bytes: the label, then the bytes as
    /// one field.
    pub(crate) fn compute(canonical: &[u8]) -> Self {
        let mut framing = Framing::new(MANIFEST_LABEL);
        framing.field(canonical);
        Self::from_framing(framing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_boundaries_change_the_digest() {
        let mut joined = Framing::new("test");
        joined.field(b"abc");
        let mut split = Framing::new("test");
        split.field(b"ab");
        split.field(b"c");
        assert_ne!(joined.finish(), split.finish());
    }

    #[test]
    fn fingerprints_sort_variants_and_render_hex() {
        let fingerprint = DeclarationFingerprint::compute("k", "release", "enum", &["b", "a"]);
        let sorted = DeclarationFingerprint::compute("k", "release", "enum", &["a", "b"]);
        assert_eq!(fingerprint, sorted);
        let hex = fingerprint.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            format!("{fingerprint:?}"),
            format!("DeclarationFingerprint({hex})")
        );
        assert_eq!(hex.parse::<DeclarationFingerprint>(), Ok(fingerprint));
    }

    #[test]
    fn digests_parse_only_lower_case_hex() {
        assert_eq!("".parse::<CatalogDigest>(), Err(DigestParseError));
        assert_eq!(
            "0".repeat(63).parse::<CatalogDigest>(),
            Err(DigestParseError)
        );
        assert_eq!(
            "A".repeat(64).parse::<CatalogDigest>(),
            Err(DigestParseError)
        );
        assert_eq!(
            "g".repeat(64).parse::<CatalogDigest>(),
            Err(DigestParseError)
        );
        let parsed: CatalogDigest = "0f".repeat(32).parse().unwrap();
        assert_eq!(parsed.as_bytes(), &[0x0f; 32]);
    }
}
