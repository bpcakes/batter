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
/// use batter_sqlx::managed::{ManagedObjectError, ManagedObjectManifest, ManagedRevision, PublicExecute, SourceChecksum};
/// use batter_sqlx::verification::QualifiedName;
///
/// let empty = ManagedObjectManifest::new(ManagedRevision::new(1).expect("valid revision"),
///     QualifiedName::new("app", "managed_objects")?,
///     PublicExecute::Revoke,
///     SourceChecksum::FromStatementStart,
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
/// use batter_sqlx::managed::{ManagedObjectManifest, PublicExecute, SourceChecksum};
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ledger, PublicExecute::Revoke, SourceChecksum::WholeSource, [], []);
/// ```
///
/// The policy has no default, so a manifest cannot silently leave new
/// routines executable by `PUBLIC`:
///
/// ```compile_fail,E0061
/// use batter_sqlx::managed::{ManagedObjectManifest, ManagedRevision, SourceChecksum};
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ManagedRevision::new(1).expect("valid revision"), ledger, SourceChecksum::WholeSource, [], []);
/// ```
///
/// Nor has the checksum rule one, so a set cannot leave each source's
/// leading comments to be handled, or not, where it is declared:
///
/// ```compile_fail,E0061
/// use batter_sqlx::managed::{ManagedObjectManifest, ManagedRevision, PublicExecute};
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ManagedRevision::new(1).expect("valid revision"), ledger, PublicExecute::Revoke, [], []);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedObjectManifest {
    ledger: QualifiedName,
    revision: ManagedRevision,
    public_execute: PublicExecute,
    source_checksum: SourceChecksum,
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
    /// Migration: `ManagedObjectManifest::new` now also requires a
    /// [`SourceChecksum`] rule after the [`PublicExecute`] policy. Pass
    /// [`SourceChecksum::WholeSource`] to keep every existing ledger's checksums
    /// and digest; [`SourceChecksum::FromStatementStart`] keeps leading comments
    /// out of them, which is a set change for every source that has any.
    ///
    /// Every object's source checksum and the set digest are computed here,
    /// under `source_checksum`, for the whole set at once.
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
        source_checksum: SourceChecksum,
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
        for object in &mut objects {
            object.checksum_under(source_checksum);
        }
        let digest = set_digest(public_execute, &objects, &retirements);
        Ok(Self {
            ledger,
            revision,
            public_execute,
            source_checksum,
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

    /// Return the rule under which every object's source is checksummed.
    pub const fn source_checksum(&self) -> SourceChecksum {
        self.source_checksum
    }

    /// Return the declared objects in sorted identity order.
    pub fn objects(&self) -> &[ManagedObject] {
        &self.objects
    }

    /// Return each declared object, in sorted identity order, with the
    /// SHA-256 source checksum its ledger row records and the set digest
    /// covers, computed under [`Self::source_checksum`].
    ///
    /// Migration: this replaces `ManagedObject::checksum`. An object no longer
    /// has a checksum of its own, because the manifest's [`SourceChecksum`]
    /// rule decides which source bytes it covers.
    ///
    /// ```
    /// use batter_sqlx::managed::{ManagedObject, ManagedObjectManifest, ManagedRevision, PublicExecute, SourceChecksum};
    /// use batter_sqlx::verification::{QualifiedName, RoutineSignature};
    /// use sha2::Digest as _;
    ///
    /// let statement = "CREATE OR REPLACE FUNCTION app.now_utc() RETURNS timestamptz LANGUAGE sql RETURN now();";
    /// let manifest = ManagedObjectManifest::new(
    ///     ManagedRevision::new(1).expect("valid revision"),
    ///     QualifiedName::new("app", "managed_objects")?,
    ///     PublicExecute::Revoke,
    ///     SourceChecksum::FromStatementStart,
    ///     [ManagedObject::function(RoutineSignature::new("app", "now_utc", [])?, format!("-- Current time.\n{statement}"))?],
    ///     [],
    /// )?;
    /// let (object, checksum) = manifest.object_checksums().next().expect("one object");
    /// assert_eq!(object.identity(), r#""app"."now_utc"()"#);
    /// // The header comment is not checksummed.
    /// assert_eq!(checksum, &<[u8; 32]>::from(sha2::Sha256::digest(statement)));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn object_checksums(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ManagedObject, &[u8; 32])> + '_ {
        self.objects
            .iter()
            .map(|object| (object, object.checksum()))
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
