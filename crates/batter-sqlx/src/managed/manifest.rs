//! Validated manifest and its application-declared release order.

use super::*;

/// A validated, bounded set of managed objects, retirements and policy.
///
/// The ledger is an application-named relation, distinct from any SQLx
/// migration ledger, that apply creates when absent and verification reads.
/// Its exact shape is `identity text PRIMARY KEY`, `kind text`,
/// `source_checksum bytea`, `rendered_checksum bytea`, `applied_at timestamptz`
/// and `release_revision bigint`, all `NOT NULL`. The manifest row carries the
/// revision; object rows carry zero. A legacy ledger without the release column
/// is upgraded only when its recorded digest matches this manifest. Apply refuses
/// any other shape, including a SQLx ledger. Entries and retirements are kept in
/// sorted identity order, which is also apply's execution order within each
/// phase.
///
/// There is no empty manifest: a set must declare at least one object or one
/// retirement.
///
/// ```
/// use batter_sqlx::managed::{ManagedObjectError, ManagedObjectManifest, ManagedRevision, PublicExecute};
/// use batter_sqlx::verification::QualifiedName;
///
/// let empty = ManagedObjectManifest::new(ManagedRevision::new(1).expect("valid revision"),
///     QualifiedName::new("app", "managed_objects")?,
///     PublicExecute::Revoke,
///     [],
///     [],
/// );
/// assert_eq!(empty, Err(ManagedObjectError::EmptyManifest));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A release revision cannot be omitted, including when migrating an old call:
///
/// ```compile_fail,E0061
/// use batter_sqlx::managed::{ManagedObjectManifest, PublicExecute};
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ledger, PublicExecute::Revoke, [], []);
/// ```
///
/// The policy has no default, so a manifest cannot silently leave new
/// routines executable by `PUBLIC`:
///
/// ```compile_fail,E0061
/// use batter_sqlx::managed::{ManagedObjectManifest, ManagedRevision};
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ManagedRevision::new(1).expect("valid revision"), ledger, [], []);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedObjectManifest {
    ledger: QualifiedName,
    revision: ManagedRevision,
    public_execute: PublicExecute,
    objects: Vec<ManagedObject>,
    retirements: Vec<Retirement>,
    digest: ManagedObjectDigest,
}

impl ManagedObjectManifest {
    /// Validate a complete managed object set.
    ///
    /// Migration: `ManagedObjectManifest::new` now requires a `ManagedRevision`
    /// before the ledger argument. No implicit revision is supplied: reused
    /// revisions with different content and lower revisions are refused at apply.
    ///
    /// Inputs are consumed only up to the combined capacity of 10,000 objects
    /// plus retirements. Sources may total at most 64 MiB. Collection stops at
    /// the first object exceeding either bound, before consuming retirements.
    ///
    /// # Errors
    /// Returns [`ManagedObjectError`] for an empty set, a SQLx-reserved or
    /// temporary ledger, a temporary retirement, duplicate entries or
    /// retirements, a retirement of a declared identity, or exceeded capacity.
    pub fn new(
        revision: ManagedRevision,
        ledger: QualifiedName,
        public_execute: PublicExecute,
        objects: impl IntoIterator<Item = ManagedObject>,
        retirements: impl IntoIterator<Item = Retirement>,
    ) -> Result<Self, ManagedObjectError> {
        if ledger.name() == "_sqlx_migrations" {
            return Err(ManagedObjectError::ReservedLedger);
        }
        if temporary_namespace(ledger.schema()) {
            return Err(ManagedObjectError::TemporaryNamespace);
        }
        let mut source_bytes = 0usize;
        let mut collected = Vec::new();
        for object in objects {
            if collected.len() == MAX_MANAGED_OBJECTS {
                return Err(ManagedObjectError::Capacity);
            }
            source_bytes = source_bytes
                .checked_add(object.source.len())
                .filter(|bytes| *bytes <= MAX_MANIFEST_SOURCE_BYTES)
                .ok_or(ManagedObjectError::Capacity)?;
            collected.push(object);
        }
        let mut objects = collected;
        let mut retirements = collect_bounded(
            retirements,
            MAX_MANAGED_OBJECTS.saturating_sub(objects.len()),
        )?;
        if objects.is_empty() && retirements.is_empty() {
            return Err(ManagedObjectError::EmptyManifest);
        }
        objects.sort_by(|left, right| left.rendered.cmp(&right.rendered));
        retirements.sort_by(|left, right| left.rendered.cmp(&right.rendered));
        let mut declared = HashSet::with_capacity(objects.len());
        for object in &objects {
            if !declared.insert(object.rendered.as_str()) {
                return Err(ManagedObjectError::DuplicateObject {
                    identity: object.rendered.clone(),
                });
            }
        }
        let mut retired = HashSet::with_capacity(retirements.len());
        for retirement in &retirements {
            if retirement.identity.temporary() {
                return Err(ManagedObjectError::TemporaryNamespace);
            }
            if declared.contains(retirement.rendered.as_str()) {
                return Err(ManagedObjectError::RetiredObjectDeclared {
                    identity: retirement.rendered.clone(),
                });
            }
            if !retired.insert(retirement.rendered.as_str()) {
                return Err(ManagedObjectError::DuplicateRetirement {
                    identity: retirement.rendered.clone(),
                });
            }
        }
        let digest = set_digest(public_execute, &objects, &retirements);
        Ok(Self {
            ledger,
            revision,
            public_execute,
            objects,
            retirements,
            digest,
        })
    }

    /// Return the application-declared release revision.
    pub const fn revision(&self) -> ManagedRevision {
        self.revision
    }

    /// Return the application-named ledger relation.
    pub fn ledger(&self) -> &QualifiedName {
        &self.ledger
    }

    /// Return the declared `PUBLIC` `EXECUTE` policy.
    pub const fn public_execute(&self) -> PublicExecute {
        self.public_execute
    }

    /// Return the declared objects in sorted identity order.
    pub fn objects(&self) -> &[ManagedObject] {
        &self.objects
    }

    /// Return the retirements in sorted identity order.
    pub fn retirements(&self) -> &[Retirement] {
        &self.retirements
    }

    /// Return the deterministic set digest apply records and verification compares.
    pub const fn digest(&self) -> ManagedObjectDigest {
        self.digest
    }

    /// Session advisory lock key shared by every apply of this ledger.
    pub(crate) fn lock_key(&self) -> i64 {
        let digest = Sha256::new()
            .chain_update(b"batter.managed-objects.lock\0")
            .chain_update(self.ledger.quoted().as_bytes())
            .finalize();
        let mut key = [0; 8];
        key.copy_from_slice(&digest[..8]);
        i64::from_be_bytes(key)
    }
}
