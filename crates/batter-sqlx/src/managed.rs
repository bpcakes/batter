//! Consumer-declared PostgreSQL routines and triggers applied after migrations.
//!
//! A routine body edited as a copy inside an append-only migration log hides
//! every change in a restated whole definition, and two branches that restate
//! one routine silently race. A [`ManagedObjectManifest`] instead declares the
//! current source of each managed routine and of each trigger that binds one,
//! plus explicit [`Retirement`]s. Versioned migrations keep tables, columns,
//! constraints, policies and data changes.
//!
//! The manifest is pure: it validates every entry without a connection. The
//! same value then drives both library-owned phases:
//!
//! - [`crate::PgLease::migrate_and_apply`] runs the application-selected SQLx
//!   migrator and then applies the manifest in one transaction on the same
//!   connection. There is no apply-only entry point, so objects can never be
//!   applied to a schema the migrations have not reached.
//! - [`crate::verification::VerificationPlan::with_managed_objects`] compares
//!   the ledger written by apply with the manifest and recomputes every live
//!   catalog definition, read-only, inside the verifier's snapshot.
//!
//! Batter applies and verifies the declared set only. The object source,
//! ownership transfer, grants other than the declared [`PublicExecute`] policy,
//! migration selection and provisioning stay with the application.
//!
//! ```
//! use batter_sqlx::managed::{ManagedObject, ManagedObjectManifest, PublicExecute, Retirement};
//! use batter_sqlx::verification::{Identifier, QualifiedName, RoutineSignature, RoutineType};
//!
//! let touch = RoutineSignature::new("app", "touch_updated_at", [])?;
//! let manifest = ManagedObjectManifest::new(
//!     QualifiedName::new("app", "managed_objects")?,
//!     PublicExecute::Revoke,
//!     [
//!         ManagedObject::function(
//!             touch,
//!             "CREATE OR REPLACE FUNCTION app.touch_updated_at() RETURNS trigger
//!              LANGUAGE plpgsql AS $$ BEGIN NEW.updated_at := now(); RETURN NEW; END $$;",
//!         )?,
//!         ManagedObject::trigger(
//!             QualifiedName::new("app", "records")?,
//!             Identifier::new("records_touch")?,
//!             "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records
//!              FOR EACH ROW EXECUTE FUNCTION app.touch_updated_at();",
//!         )?,
//!     ],
//!     [Retirement::function(RoutineSignature::new(
//!         "app",
//!         "touch_record",
//!         [RoutineType::new("pg_catalog", "int8")?],
//!     )?)],
//! )?;
//! assert_eq!(manifest.objects().len(), 2);
//! println!("managed object set {}", manifest.digest());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod apply;
mod catalog;
mod source;
#[cfg(test)]
mod tests;

pub(crate) use apply::migrate_and_apply;
pub use apply::{AppliedManagedObjects, ManagedApplyError};
pub(crate) use catalog::{
    CatalogError, LedgerRow, LedgerShape, LiveObject, RenderSettings, inspect_ledger_shape,
    ledger_relation, load_ledger_rows, public_execute_holders, render_hashes, rendered_set_digest,
    resolve_live,
};

use crate::verification::{Identifier, QualifiedName, RoutineSignature, temporary_namespace};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::HashSet, fmt};

/// Combined bound on declared objects plus retirements in one manifest.
pub(crate) const MAX_MANAGED_OBJECTS: usize = 10_000;
/// Bound on one object's source statement.
pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Bound on the sum of every object's source statement.
pub(crate) const MAX_MANIFEST_SOURCE_BYTES: usize = 64 * 1024 * 1024;
/// Ledger identity of the row that records the applied set digest.
pub(crate) const SET_ROW: &str = "manifest";

/// The PostgreSQL object kind a managed entry or retirement names.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManagedObjectKind {
    /// A routine created with `CREATE OR REPLACE FUNCTION`.
    Function,
    /// A routine created with `CREATE OR REPLACE PROCEDURE`.
    Procedure,
    /// A table trigger created with `CREATE OR REPLACE TRIGGER`.
    Trigger,
}

impl ManagedObjectKind {
    /// Return the lowercase name recorded in the ledger's `kind` column.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Procedure => "procedure",
            Self::Trigger => "trigger",
        }
    }
}

/// Structural identity shared by entries and retirements.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum Identity {
    Routine(RoutineSignature),
    Trigger {
        table: QualifiedName,
        name: Identifier,
    },
}

impl Identity {
    fn render(&self) -> String {
        match self {
            Self::Routine(signature) => signature.as_str().to_owned(),
            Self::Trigger { table, name } => format!("{} ON {}", name.quoted(), table.quoted()),
        }
    }

    fn temporary(&self) -> bool {
        match self {
            Self::Routine(signature) => {
                temporary_namespace(signature.schema())
                    || signature
                        .arguments()
                        .iter()
                        .any(|argument| temporary_namespace(argument.schema()))
            }
            Self::Trigger { table, .. } => temporary_namespace(table.schema()),
        }
    }
}

/// One validated managed routine or trigger and its complete source statement.
///
/// Construction checks, without a connection, that the source is exactly one
/// `CREATE OR REPLACE` statement of the declared kind whose header names the
/// declared, schema-qualified identity. Quotes, comments, dollar quotes and
/// `BEGIN ATOMIC` bodies are recognized the way PostgreSQL and `psql` do;
/// argument lists, return types and bodies are left to PostgreSQL, and apply
/// rejects a statement that does not create exactly the declared identity.
///
/// A routine identity is the structural [`RoutineSignature`] PostgreSQL uses to
/// tell overloads apart, so it lists the exact catalog types of the input
/// arguments. A trigger identity is its table and name.
///
/// ```
/// use batter_sqlx::managed::{ManagedObject, ManagedObjectError};
/// use batter_sqlx::verification::{RoutineSignature, RoutineType};
///
/// let signature = RoutineSignature::new("app", "total", [RoutineType::new("pg_catalog", "int8")?])?;
/// let object = ManagedObject::function(
///     signature.clone(),
///     "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
///      LANGUAGE sql IMMUTABLE RETURN amount * 2;",
/// )?;
/// assert_eq!(object.identity(), r#""app"."total"("pg_catalog"."int8")"#);
///
/// // The header must name the declared routine, schema-qualified.
/// assert_eq!(
///     ManagedObject::function(signature.clone(), "CREATE OR REPLACE FUNCTION total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;"),
///     Err(ManagedObjectError::SourceIdentity),
/// );
/// // One statement only.
/// assert_eq!(
///     ManagedObject::function(signature, "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1; DROP TABLE app.records;"),
///     Err(ManagedObjectError::SourceStatement),
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Eq, PartialEq)]
pub struct ManagedObject {
    kind: ManagedObjectKind,
    identity: Identity,
    rendered: String,
    source: Cow<'static, str>,
    checksum: [u8; 32],
}

impl ManagedObject {
    /// Declare a function from its complete `CREATE OR REPLACE FUNCTION` statement.
    ///
    /// # Errors
    /// Returns a source or namespace [`ManagedObjectError`] when the statement is
    /// not exactly one function definition naming `signature`.
    pub fn function(
        signature: RoutineSignature,
        source: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ManagedObjectError> {
        Self::new(
            ManagedObjectKind::Function,
            Identity::Routine(signature),
            source.into(),
        )
    }

    /// Declare a procedure from its complete `CREATE OR REPLACE PROCEDURE` statement.
    ///
    /// # Errors
    /// Returns a source or namespace [`ManagedObjectError`] when the statement is
    /// not exactly one procedure definition naming `signature`.
    pub fn procedure(
        signature: RoutineSignature,
        source: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ManagedObjectError> {
        Self::new(
            ManagedObjectKind::Procedure,
            Identity::Routine(signature),
            source.into(),
        )
    }

    /// Declare a trigger from its complete `CREATE OR REPLACE TRIGGER` statement.
    ///
    /// The statement's `ON` clause must name `table` schema-qualified. Constraint
    /// triggers cannot be replaced in place by PostgreSQL and are rejected.
    ///
    /// # Errors
    /// Returns a source or namespace [`ManagedObjectError`] when the statement is
    /// not exactly one trigger definition naming `name` on `table`.
    pub fn trigger(
        table: QualifiedName,
        name: Identifier,
        source: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ManagedObjectError> {
        Self::new(
            ManagedObjectKind::Trigger,
            Identity::Trigger { table, name },
            source.into(),
        )
    }

    fn new(
        kind: ManagedObjectKind,
        identity: Identity,
        source: Cow<'static, str>,
    ) -> Result<Self, ManagedObjectError> {
        if identity.temporary() {
            return Err(ManagedObjectError::TemporaryNamespace);
        }
        source::validate(kind, &identity, &source)?;
        let checksum = Sha256::digest(source.as_bytes()).into();
        Ok(Self {
            kind,
            rendered: identity.render(),
            identity,
            source,
            checksum,
        })
    }

    /// Return the declared object kind.
    pub const fn kind(&self) -> ManagedObjectKind {
        self.kind
    }

    /// Return the canonical identity recorded in the ledger and in findings.
    ///
    /// Routines use [`RoutineSignature::as_str`]; triggers use
    /// `"name" ON "schema"."table"` with every identifier quoted.
    pub fn identity(&self) -> &str {
        &self.rendered
    }

    /// Return the source statement exactly as declared.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the SHA-256 checksum of the exact source bytes.
    pub const fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }

    pub(crate) const fn structural_identity(&self) -> &Identity {
        &self.identity
    }
}

impl fmt::Debug for ManagedObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedObject")
            .field("kind", &self.kind)
            .field("identity", &self.rendered)
            .field("checksum", &Hex(&self.checksum))
            .field("source_bytes", &self.source.len())
            .finish()
    }
}

/// An explicit instruction to drop a routine or trigger the application no longer declares.
///
/// Removing an entry from a manifest only stops managing it; the object stays
/// in the database. Apply drops a retired object only when it still exists,
/// triggers before routines, and never with `CASCADE`, so an undeclared
/// dependent object makes the whole apply fail instead of disappearing.
/// A retirement may not name an identity the same manifest declares: replacing
/// a routine whose return type or argument names change belongs in a versioned
/// migration that drops it, after which apply creates the new definition.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Retirement {
    kind: ManagedObjectKind,
    identity: Identity,
    rendered: String,
}

impl Retirement {
    /// Retire a function by its structural signature.
    pub fn function(signature: RoutineSignature) -> Self {
        Self::new(ManagedObjectKind::Function, Identity::Routine(signature))
    }

    /// Retire a procedure by its structural signature.
    pub fn procedure(signature: RoutineSignature) -> Self {
        Self::new(ManagedObjectKind::Procedure, Identity::Routine(signature))
    }

    /// Retire a trigger by its table and name.
    pub fn trigger(table: QualifiedName, name: Identifier) -> Self {
        Self::new(
            ManagedObjectKind::Trigger,
            Identity::Trigger { table, name },
        )
    }

    fn new(kind: ManagedObjectKind, identity: Identity) -> Self {
        Self {
            kind,
            rendered: identity.render(),
            identity,
        }
    }

    /// Return the retired object kind.
    pub const fn kind(&self) -> ManagedObjectKind {
        self.kind
    }

    /// Return the canonical identity, rendered as for [`ManagedObject::identity`].
    pub fn identity(&self) -> &str {
        &self.rendered
    }

    pub(crate) const fn structural_identity(&self) -> &Identity {
        &self.identity
    }
}

/// Whether apply removes PostgreSQL's default `EXECUTE` grant to `PUBLIC`.
///
/// PostgreSQL grants `EXECUTE` on every new routine to `PUBLIC`. There is no
/// default choice: a manifest states either that Batter revokes it from every
/// managed routine, or that the application owns that grant itself.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PublicExecute {
    /// Apply revokes `EXECUTE` from `PUBLIC` on every managed routine on which
    /// `PUBLIC` holds it, and verification reports any managed routine that
    /// `PUBLIC` can still execute. No other grant is made or removed.
    Revoke,
    /// Batter neither revokes nor checks `PUBLIC`'s `EXECUTE`; PostgreSQL
    /// defaults, default privileges and the application's own grants decide it.
    Unmanaged,
}

/// SHA-256 digest of a manifest's policy, entries and retirements.
///
/// The digest covers the [`PublicExecute`] policy, every entry's kind, identity
/// and source checksum, and every retirement's kind and identity, in sorted
/// identity order. It does not cover the ledger name, so a build can embed it
/// in release evidence independently of where the ledger lives. Its `Display`
/// form is lowercase hexadecimal.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ManagedObjectDigest([u8; 32]);

impl ManagedObjectDigest {
    /// Return the raw digest bytes recorded in the ledger.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ManagedObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&Hex(&self.0), formatter)
    }
}

impl fmt::Debug for ManagedObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ManagedObjectDigest({self})")
    }
}

struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

impl fmt::Debug for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// A validated, bounded set of managed objects, retirements and policy.
///
/// The ledger is an application-named relation, distinct from any SQLx
/// migration ledger, that apply creates when absent and verification reads.
/// Its exact shape is `identity text PRIMARY KEY`, `kind text`,
/// `source_checksum bytea`, `rendered_checksum bytea` and
/// `applied_at timestamptz`, all `NOT NULL`. Apply refuses any other relation
/// of that name, including a SQLx ledger. Entries and retirements are kept in
/// sorted identity order, which is also apply's execution order within each
/// phase.
///
/// There is no empty manifest: a set must declare at least one object or one
/// retirement.
///
/// ```
/// use batter_sqlx::managed::{ManagedObjectError, ManagedObjectManifest, PublicExecute};
/// use batter_sqlx::verification::QualifiedName;
///
/// let empty = ManagedObjectManifest::new(
///     QualifiedName::new("app", "managed_objects")?,
///     PublicExecute::Revoke,
///     [],
///     [],
/// );
/// assert_eq!(empty, Err(ManagedObjectError::EmptyManifest));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// The policy has no default, so a manifest cannot silently leave new
/// routines executable by `PUBLIC`:
///
/// ```compile_fail,E0061
/// use batter_sqlx::managed::ManagedObjectManifest;
/// use batter_sqlx::verification::QualifiedName;
/// let ledger = QualifiedName::new("app", "managed_objects").unwrap();
/// let _ = ManagedObjectManifest::new(ledger, [], []);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedObjectManifest {
    ledger: QualifiedName,
    public_execute: PublicExecute,
    objects: Vec<ManagedObject>,
    retirements: Vec<Retirement>,
    digest: ManagedObjectDigest,
}

impl ManagedObjectManifest {
    /// Validate a complete managed object set.
    ///
    /// Inputs are consumed only up to the combined capacity of 10,000 objects
    /// plus retirements. Sources may total at most 64 MiB.
    ///
    /// # Errors
    /// Returns [`ManagedObjectError`] for an empty set, a SQLx-reserved or
    /// temporary ledger, a temporary retirement, duplicate entries or
    /// retirements, a retirement of a declared identity, or exceeded capacity.
    pub fn new(
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
        let mut objects = collect_bounded(objects, MAX_MANAGED_OBJECTS)?;
        let mut retirements = collect_bounded(
            retirements,
            MAX_MANAGED_OBJECTS.saturating_sub(objects.len()),
        )?;
        if objects.is_empty() && retirements.is_empty() {
            return Err(ManagedObjectError::EmptyManifest);
        }
        let source_bytes = objects.iter().try_fold(0usize, |total, object| {
            total.checked_add(object.source.len())
        });
        if source_bytes.is_none_or(|bytes| bytes > MAX_MANIFEST_SOURCE_BYTES) {
            return Err(ManagedObjectError::Capacity);
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
            public_execute,
            objects,
            retirements,
            digest,
        })
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

fn collect_bounded<T>(
    values: impl IntoIterator<Item = T>,
    limit: usize,
) -> Result<Vec<T>, ManagedObjectError> {
    let mut collected = Vec::new();
    for value in values {
        if collected.len() == limit {
            return Err(ManagedObjectError::Capacity);
        }
        collected.push(value);
    }
    Ok(collected)
}

fn set_digest(
    public_execute: PublicExecute,
    objects: &[ManagedObject],
    retirements: &[Retirement],
) -> ManagedObjectDigest {
    fn field(hasher: &mut Sha256, bytes: &[u8]) {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    let mut hasher = Sha256::new();
    field(&mut hasher, b"batter.managed-objects.v1");
    field(
        &mut hasher,
        match public_execute {
            PublicExecute::Revoke => b"public-execute:revoke",
            PublicExecute::Unmanaged => b"public-execute:unmanaged",
        },
    );
    hasher.update((objects.len() as u64).to_be_bytes());
    for object in objects {
        field(&mut hasher, object.kind.as_str().as_bytes());
        field(&mut hasher, object.rendered.as_bytes());
        hasher.update(object.checksum);
    }
    hasher.update((retirements.len() as u64).to_be_bytes());
    for retirement in retirements {
        field(&mut hasher, retirement.kind.as_str().as_bytes());
        field(&mut hasher, retirement.rendered.as_bytes());
    }
    ManagedObjectDigest(hasher.finalize().into())
}

/// Invalid managed object input, rejected before any connection work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedObjectError {
    /// The source is not a `CREATE OR REPLACE` statement of the declared kind
    /// with a recognizable header. `CREATE CONSTRAINT TRIGGER`, which
    /// PostgreSQL cannot replace in place, is rejected here.
    SourceHeader,
    /// The header names another object than declared, or does not
    /// schema-qualify the routine name or trigger table.
    SourceIdentity,
    /// The source has an unterminated quote, comment or dollar quote, contains
    /// a NUL byte, or continues after its one statement.
    SourceStatement,
    /// One source exceeds 1 MiB.
    SourceTooLarge,
    /// An entry, retirement or the ledger lives in a session-local temporary
    /// namespace (`pg_temp`, `pg_temp_*` or `pg_toast_temp_*`), including a
    /// routine argument type.
    TemporaryNamespace,
    /// The ledger uses SQLx's reserved `_sqlx_migrations` relation name.
    ReservedLedger,
    /// Two entries share one identity. Functions and procedures share
    /// PostgreSQL's routine identities.
    DuplicateObject {
        /// The canonical identity declared twice.
        identity: String,
    },
    /// Two retirements share one identity.
    DuplicateRetirement {
        /// The canonical identity retired twice.
        identity: String,
    },
    /// A retirement names an identity the same manifest declares.
    RetiredObjectDeclared {
        /// The canonical identity both declared and retired.
        identity: String,
    },
    /// More than 10,000 entries plus retirements, or more than 64 MiB of source.
    Capacity,
    /// The manifest declares no object and no retirement.
    EmptyManifest,
}

impl fmt::Display for ManagedObjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceHeader => {
                formatter.write_str("managed object source is not CREATE OR REPLACE of its kind")
            }
            Self::SourceIdentity => formatter.write_str(
                "managed object source names another or unqualified object than declared",
            ),
            Self::SourceStatement => {
                formatter.write_str("managed object source is not exactly one complete statement")
            }
            Self::SourceTooLarge => formatter.write_str("managed object source exceeds 1 MiB"),
            Self::TemporaryNamespace => {
                formatter.write_str("managed object names a temporary namespace")
            }
            Self::ReservedLedger => {
                formatter.write_str("managed object ledger uses the SQLx migration ledger name")
            }
            Self::DuplicateObject { identity } => {
                write!(formatter, "managed object is declared twice: {identity}")
            }
            Self::DuplicateRetirement { identity } => {
                write!(formatter, "managed object is retired twice: {identity}")
            }
            Self::RetiredObjectDeclared { identity } => {
                write!(
                    formatter,
                    "managed object is both declared and retired: {identity}"
                )
            }
            Self::Capacity => formatter.write_str("managed object manifest capacity exceeded"),
            Self::EmptyManifest => {
                formatter.write_str("managed object manifest declares no object or retirement")
            }
        }
    }
}

impl std::error::Error for ManagedObjectError {}
