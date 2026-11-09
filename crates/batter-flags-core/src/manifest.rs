//! The machine-readable export of a catalog and the digest of its bytes.

mod diff;
mod entry;
mod holds;
mod validate;
mod wire;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

pub use diff::{ManifestDiff, ManifestDiffError, Retype, manifest_diff};
pub use entry::{
    ManifestEntry, ManifestInvariant, ManifestOwners, ManifestRequiredSettings, ManifestRetired,
};
pub use holds::{InvariantHold, InvariantJudgement};

use crate::{
    catalog::Catalog,
    digest::{CatalogDigest, DigestInput, ManifestDigest},
    key::SettingsPrefix,
    vocabulary::{EnvironmentClass, ProcessKind},
};

/// The versioned JSON export of a catalog for deployment tooling: names,
/// setting names, kinds, owners, `pinned_by` kinds, environment classes,
/// defaults, expiry, retired names, invariants and flip discipline.
///
/// Every list is sorted, so the canonical bytes, and with them the
/// [`ManifestDigest`], depend only on the declarations, never on their order.
/// The manifest covers everything the [`CatalogDigest`] does plus expiry,
/// flip discipline and migration notes.
///
/// Decoding is a validating constructor: it accepts version 1 only, ignores
/// unknown fields, normalizes list order, and checks the structure each kind
/// requires, the references between entries, uniqueness, every declaration
/// fingerprint and the catalog digest. A decoded manifest satisfies every
/// catalog rule its data can express: non-empty vocabularies, retired names
/// that no setting uses, governing switches owned by every kind that owns or
/// pins the flag, and invariants that hold at the code defaults. Only rules
/// about Rust values, such as variant round-trips, are left to the builder.
///
/// ```
/// use batter_flags_core::{CatalogBuilder, CivilDate, Expiry, Manifest, SettingsPrefix, Toggle};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut builder, _, _) = CatalogBuilder::new(SettingsPrefix::new("FLAG_")?, "serving", "hosted")?;
/// let expiry = Expiry::on(CivilDate::new(2027, 3, 31)?);
/// builder.release_flag("new_checkout", Toggle::Off, expiry).declare()?;
/// let catalog = builder.build();
///
/// let manifest = catalog.manifest();
/// let bytes = manifest.to_canonical_json();
/// let decoded: Manifest = serde_json::from_slice(&bytes)?;
/// assert_eq!(decoded, manifest);
/// assert_eq!(decoded.digest(), catalog.manifest_digest());
/// assert_eq!(decoded.flag("new_checkout").unwrap().setting_name().as_str(), "FLAG_NEW_CHECKOUT");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "validate::WireManifest")]
pub struct Manifest {
    v: Version,
    prefix: SettingsPrefix,
    catalog_digest: CatalogDigest,
    process_kinds: Vec<String>,
    environment_classes: Vec<String>,
    flags: Vec<ManifestEntry>,
    retired: Vec<ManifestRetired>,
}

/// The only manifest version this build writes and reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Version;

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(Manifest::VERSION)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = u64::deserialize(deserializer)?;
        if version == Manifest::VERSION {
            Ok(Self)
        } else {
            Err(D::Error::custom(format_args!(
                "manifest version {version} is not supported; this build reads version {}",
                Manifest::VERSION
            )))
        }
    }
}

impl Manifest {
    /// The manifest version, written as the `v` field.
    pub const VERSION: u64 = 1;

    /// The settings prefix.
    pub fn prefix(&self) -> &SettingsPrefix {
        &self.prefix
    }

    /// The catalog digest of the exported catalog.
    pub fn catalog_digest(&self) -> CatalogDigest {
        self.catalog_digest
    }

    /// The declared process-kind names, sorted.
    pub fn process_kinds(&self) -> &[String] {
        &self.process_kinds
    }

    /// The declared environment-class names, sorted.
    pub fn environment_classes(&self) -> &[String] {
        &self.environment_classes
    }

    /// Every declaration, in key order.
    pub fn flags(&self) -> &[ManifestEntry] {
        &self.flags
    }

    /// The declaration with `key`, if any.
    pub fn flag(&self, key: &str) -> Option<&ManifestEntry> {
        self.flags
            .binary_search_by(|entry| entry.key.as_str().cmp(key))
            .ok()
            .map(|index| &self.flags[index])
    }

    /// The retired names with their migration notes, in name order.
    pub fn retired(&self) -> &[ManifestRetired] {
        &self.retired
    }

    /// The canonical bytes: compact JSON with fields in declaration order and
    /// every list sorted.
    ///
    /// # Panics
    ///
    /// Never: every field serializes to JSON without a fallible map key or
    /// writer.
    pub fn to_canonical_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a manifest always serializes to JSON")
    }

    /// The digest of the canonical bytes.
    pub fn digest(&self) -> ManifestDigest {
        ManifestDigest::compute(&self.to_canonical_json())
    }

    fn digest_input(&self) -> DigestInput<'_> {
        DigestInput {
            prefix: self.prefix.as_str(),
            process_kinds: self.process_kinds.iter().map(String::as_str).collect(),
            environment_classes: self
                .environment_classes
                .iter()
                .map(String::as_str)
                .collect(),
            retired: self
                .retired
                .iter()
                .map(|retired| retired.name.as_str())
                .collect(),
            entries: self.flags.iter().map(ManifestEntry::digest_view).collect(),
        }
    }
}

impl Catalog {
    /// The versioned JSON export of this catalog (feature `serde`).
    pub fn manifest(&self) -> Manifest {
        Manifest {
            v: Version,
            prefix: self.prefix.clone(),
            catalog_digest: self.digest,
            process_kinds: kind_names(&self.process_kinds),
            environment_classes: sorted(
                self.environment_classes
                    .iter()
                    .map(|class| EnvironmentClass::name(class).to_owned()),
            ),
            flags: self
                .by_key
                .values()
                .map(|&index| ManifestEntry::from_entry(&self.prefix, &self.entries[index]))
                .collect(),
            retired: self
                .retired
                .iter()
                .map(|retired| ManifestRetired {
                    name: retired.name().clone(),
                    migration_note: retired.migration_note().to_owned(),
                })
                .collect(),
        }
    }

    /// The digest of this catalog's canonical manifest bytes (feature
    /// `serde`): the identity of a published catalog. Unlike the
    /// [`digest`](Self::digest), it also changes with expiry, flip discipline
    /// and migration notes.
    pub fn manifest_digest(&self) -> ManifestDigest {
        self.manifest().digest()
    }
}

pub(super) fn kind_names(kinds: &[ProcessKind]) -> Vec<String> {
    sorted(kinds.iter().map(|kind| kind.name().to_owned()))
}

pub(super) fn sorted(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut items: Vec<String> = items.collect();
    items.sort_unstable();
    items
}
