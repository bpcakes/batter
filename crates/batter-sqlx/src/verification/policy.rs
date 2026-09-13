pub(crate) mod coverage;
mod discovery;
mod limits;
mod public;
pub use discovery::{
    DiscoveryDefaults, DiscoveryScope, ObjectDefaults, PublicAllowance, RequiredPrivilege,
};
pub(crate) use public::PublicDeclarations;
use std::fmt;

pub(crate) const MAX_MIGRATION_CHECKSUM_BYTES: usize = 1_024;
pub(crate) const MAX_MIGRATION_LEDGER_ROWS: usize = 10_000;

/// A validated PostgreSQL identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Identifier(String);

impl Identifier {
    /// Validate one PostgreSQL identifier without interpolating it into SQL.
    pub fn new(value: impl Into<String>) -> Result<Self, PolicyError> {
        let value = value.into();
        validate_identifier(&value)?;
        Ok(Self(value))
    }

    /// Return the validated identifier without quoting.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn quoted(&self) -> String {
        quote_identifier(&self.0)
    }
}

/// A validated PostgreSQL configuration-parameter name.
///
/// Unlike a catalog identifier, a custom parameter name can contain multiple
/// dot-separated identifier components and its complete spelling is not
/// limited to 63 bytes; the verifier accepts at most 1,024 bytes. ASCII letters
/// are normalized to lowercase comparison keys, including mixed-case built-ins. The value is used only as a bound catalog value; it is
/// never interpolated into SQL.
///
/// ```
/// use batter_sqlx::verification::ParameterName;
///
/// let name = ParameterName::new(format!("service.{}", "x".repeat(63)))?;
/// assert!(name.is_custom());
/// assert_eq!(name.as_str().len(), 71);
/// # Ok::<(), batter_sqlx::verification::PolicyError>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParameterName(String);

impl ParameterName {
    /// Validate one built-in-style or custom PostgreSQL parameter name.
    pub fn new(value: impl Into<String>) -> Result<Self, PolicyError> {
        let value = value.into().to_ascii_lowercase();
        if value.len() > 1_024 {
            return Err(PolicyError::ParameterNameTooLarge);
        }
        if !is_valid_parameter_name(&value) {
            return Err(PolicyError::InvalidParameterName);
        }
        Ok(Self(value))
    }

    /// Return the validated parameter name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is a dot-qualified custom parameter name.
    pub fn is_custom(&self) -> bool {
        self.0.contains('.')
    }
}

/// A validated schema-qualified PostgreSQL name.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct QualifiedName {
    schema: Identifier,
    name: Identifier,
}

impl QualifiedName {
    /// Construct a schema-qualified name from two identifier components.
    pub fn new(schema: impl Into<String>, name: impl Into<String>) -> Result<Self, PolicyError> {
        Ok(Self {
            schema: Identifier::new(schema)?,
            name: Identifier::new(name)?,
        })
    }

    /// Return the schema component without quoting.
    pub fn schema(&self) -> &str {
        self.schema.as_str()
    }

    /// Return the object-name component without quoting.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub(crate) fn quoted(&self) -> String {
        format!("{}.{}", self.schema.quoted(), self.name.quoted())
    }

    pub(crate) fn matches(&self, schema: &str, name: &str) -> bool {
        self.schema() == schema && self.name() == name
    }
}

/// One canonical PostgreSQL input type used by a [`RoutineSignature`].
///
/// The schema and type name must be the exact `pg_type` identity. In
/// particular, this type does not parse aliases, typmods, argument names or
/// arbitrary SQL type expressions. PostgreSQL stores every dimensionality of a
/// true array under the same routine argument type OID, so this representation
/// distinguishes only scalar from true array identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RoutineType {
    name: QualifiedName,
    array: bool,
}

impl RoutineType {
    /// Construct a scalar type from its exact PostgreSQL catalog schema/name.
    pub fn new(schema: impl Into<String>, name: impl Into<String>) -> Result<Self, PolicyError> {
        Ok(Self {
            name: QualifiedName::new(schema, name)?,
            array: false,
        })
    }

    /// Construct a true PostgreSQL array type from its exact element type.
    pub fn array(schema: impl Into<String>, name: impl Into<String>) -> Result<Self, PolicyError> {
        Ok(Self {
            name: QualifiedName::new(schema, name)?,
            array: true,
        })
    }

    /// Return the exact catalog schema component.
    pub fn schema(&self) -> &str {
        self.name.schema()
    }

    /// Return the exact catalog type-name component.
    pub fn name(&self) -> &str {
        self.name.name()
    }

    /// Whether this is a true PostgreSQL array type.
    pub const fn is_array(&self) -> bool {
        self.array
    }

    pub(crate) fn display(&self) -> String {
        let base = self.name.quoted();
        if self.array {
            format!("{base}[]")
        } else {
            base
        }
    }
}

/// A structural `schema.name(catalog_type, ...)` routine identity.
///
/// Callers provide canonical [`RoutineType`] values rather than a SQL
/// signature string. This prevents named arguments, aliases, typmods and
/// case-folding from silently selecting a different overload. The rendered
/// string returned by [`Self::as_str`] is for diagnostics only.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RoutineSignature {
    schema: Identifier,
    name: Identifier,
    arguments: Vec<RoutineType>,
    display: String,
}

impl RoutineSignature {
    /// Construct a routine identity from exact catalog type identities.
    pub fn new(
        schema: impl Into<String>,
        name: impl Into<String>,
        arguments: impl IntoIterator<Item = RoutineType>,
    ) -> Result<Self, PolicyError> {
        let schema = Identifier::new(schema)?;
        let name = Identifier::new(name)?;
        let arguments = arguments.into_iter().collect::<Vec<_>>();
        let display = format!(
            "{}.{}({})",
            schema.quoted(),
            name.quoted(),
            arguments
                .iter()
                .map(RoutineType::display)
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(Self {
            schema,
            name,
            arguments,
            display,
        })
    }

    /// Return the rendered identity for diagnostics and findings.
    pub fn as_str(&self) -> &str {
        &self.display
    }

    /// Return the structurally constructed, identifier-quoted SQL identity.
    pub(crate) fn quoted(&self) -> &str {
        &self.display
    }

    pub(crate) fn schema(&self) -> &str {
        self.schema.as_str()
    }

    pub(crate) fn name(&self) -> &str {
        self.name.as_str()
    }

    pub(crate) fn arguments(&self) -> &[RoutineType] {
        &self.arguments
    }
}

/// How migration rows after the required set are handled.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AdditionalMigrations {
    /// Reject every migration version not in `MigrationPolicy::required`.
    #[default]
    Reject,
    /// Permit successful rows whose version is later than the required
    /// boundary. Earlier or equal unlisted rows remain unexpected. With an
    /// empty required set there is no boundary, so every ledger row is
    /// unexpected under this deliberately conservative policy.
    AllowSuccessful,
    /// Permit only the explicitly listed later rows and successful checksums.
    /// An empty required set likewise has no lower boundary; listed rows are
    /// therefore not considered later and are rejected.
    AllowListed(Vec<MigrationExpectation>),
}

/// One expected successful migration ledger row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationExpectation {
    /// Migration version recorded by the ledger.
    pub version: i64,
    /// Expected migration checksum.
    pub checksum: Vec<u8>,
}

impl MigrationExpectation {
    /// Construct one expected migration row.
    pub fn new(version: i64, checksum: impl Into<Vec<u8>>) -> Self {
        Self {
            version,
            checksum: checksum.into(),
        }
    }
}

/// Generic SQLx-compatible migration policy.
///
/// Verification accepts at most 10,000 ledger rows and expected checksums of
/// at most 1,024 bytes. These fixed bounds keep the startup check from
/// materializing an attacker-sized ledger or checksum value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationPolicy {
    /// Qualified migration ledger relation.
    pub ledger: QualifiedName,
    /// Rows that must exist, be successful and match their checksums.
    pub required: Vec<MigrationExpectation>,
    /// Compatibility policy for rows after the required set.
    pub additional: AdditionalMigrations,
}

impl MigrationPolicy {
    pub fn new(ledger: QualifiedName, required: Vec<MigrationExpectation>) -> Self {
        Self {
            ledger,
            required,
            additional: AdditionalMigrations::Reject,
        }
    }
}

/// Privileges used by the generic object policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObjectPrivilege {
    Select,
    Insert,
    Update,
    Delete,
    Truncate,
    References,
    Trigger,
    Maintain,
    Usage,
    Create,
    Connect,
    Temporary,
    Execute,
    Set,
    AlterSystem,
}

/// One allowed privilege, optionally including the ability to grant it onward.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AllowedPrivilege {
    pub privilege: ObjectPrivilege,
    pub grant_option: bool,
}

impl AllowedPrivilege {
    /// Construct one allowed privilege and its optional grant authority.
    pub const fn new(privilege: ObjectPrivilege, grant_option: bool) -> Self {
        Self {
            privilege,
            grant_option,
        }
    }
}

/// Column-level allowances nested inside a relation policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnPolicy {
    pub column: Identifier,
    pub privileges: Vec<AllowedPrivilege>,
}

/// Table, view, materialized-view or foreign-table authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationPolicy {
    pub relation: QualifiedName,
    pub privileges: Vec<AllowedPrivilege>,
    pub columns: Vec<ColumnPolicy>,
    pub allow_owner: bool,
    /// Explicitly allow the default PUBLIC USAGE on this relation's
    /// composite row type. PostgreSQL creates that type with PUBLIC USAGE;
    /// the allowance is kept on the relation policy so ordinary table policy
    /// does not require a duplicate `TypePolicy` entry.
    pub allow_row_type_public_usage: bool,
}

/// Sequence authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequencePolicy {
    pub sequence: QualifiedName,
    pub privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
}

/// Schema authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaPolicy {
    pub schema: Identifier,
    pub privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
}

/// Routine authority policy. Security-definer bodies and search paths remain
/// outside this generic ACL checker and are reported as unsupported coverage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutinePolicy {
    pub routine: RoutineSignature,
    pub privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
    pub allow_security_definer: bool,
}

/// User-defined type authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypePolicy {
    pub type_name: QualifiedName,
    pub privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
}

/// Parameter authority policy (`SET` and `ALTER SYSTEM`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterPolicy {
    pub parameter: ParameterName,
    pub privileges: Vec<AllowedPrivilege>,
}

/// Current-database authority policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DatabasePolicy {
    pub privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
}

/// Role attributes and predefined capabilities that may be present on roles
/// reachable by the login.
///
/// ```
/// use batter_sqlx::verification::{Identifier, RolePolicy};
///
/// let policy = RolePolicy {
///     allowed_admin_roles: vec![Identifier::new("tenant_operator")?],
///     ..RolePolicy::default()
/// };
/// assert_eq!(policy.allowed_admin_roles[0].as_str(), "tenant_operator");
/// # Ok::<(), batter_sqlx::verification::PolicyError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RolePolicy {
    pub allow_superuser: bool,
    pub allow_create_database: bool,
    pub allow_create_role: bool,
    pub allow_replication: bool,
    pub allow_bypass_rls: bool,
    /// Roles for which reachable `ADMIN` authority is explicitly permitted.
    ///
    /// An allowed target is still treated as a role the login can grant back
    /// to itself. Its attributes, predefined capabilities, ownership and ACL
    /// authority therefore remain subject to the rest of this policy.
    pub allowed_admin_roles: Vec<Identifier>,
    /// Explicitly permitted PostgreSQL predefined roles. Role names with a
    /// `pg_` prefix are not treated as predefined without this catalog-known
    /// allowlist entry.
    pub allowed_predefined_roles: Vec<Identifier>,
}

/// A PUBLIC grant target. PUBLIC is checked independently from each reachable
/// login role; a role's inherited privilege does not silently authorize it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PublicObject {
    /// A table, view, materialized view, partitioned table or foreign table.
    Relation(QualifiedName),
    /// One column of a relation.
    Column(QualifiedName, Identifier),
    /// A sequence.
    Sequence(QualifiedName),
    /// A schema.
    Schema(Identifier),
    /// One exact routine overload.
    Routine(RoutineSignature),
    /// A standalone or relation-owned type.
    Type(QualifiedName),
    /// A configuration parameter with an explicit PUBLIC ACL entry.
    Parameter(ParameterName),
    /// The currently connected database.
    Database,
}

/// One explicitly permitted PUBLIC grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicGrant {
    /// The object whose PUBLIC entry is allowed.
    pub object: PublicObject,
    /// The allowed privilege and optional grant option.
    pub privilege: AllowedPrivilege,
}

/// Complete generic authority policy.
///
/// Required privileges describe current-role needs. Allowances constrain the
/// authenticated login's potential authority, including reachable SET/ADMIN
/// roles. Discovery defaults apply only inside the explicit schema scope.
///
/// ```
/// use batter_sqlx::verification::{AuthorityPolicy, DiscoveryScope,
///     AllowedPrivilege, ObjectPrivilege, PublicObject, QualifiedName,
///     RequiredPrivilege};
/// let mut policy = AuthorityPolicy {
///     discovery: DiscoveryScope::UserSchemas,
///     ..AuthorityPolicy::default()
/// };
/// policy.defaults.relations.privileges.push(
///     AllowedPrivilege::new(ObjectPrivilege::Select, false));
/// policy.required_privileges.push(RequiredPrivilege {
///     object: PublicObject::Relation(QualifiedName::new("service", "records")?),
///     privilege: ObjectPrivilege::Select,
/// });
/// # Ok::<(), batter_sqlx::verification::PolicyError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthorityPolicy {
    /// Privileges required without SET ROLE or administrative changes.
    pub required_privileges: Vec<RequiredPrivilege>,
    /// Schema discovery boundary, in addition to explicitly named objects.
    pub discovery: DiscoveryScope,
    /// Allowances for discovered objects without an exact policy entry.
    pub defaults: DiscoveryDefaults,
    pub roles: RolePolicy,
    pub relations: Vec<RelationPolicy>,
    pub sequences: Vec<SequencePolicy>,
    pub schemas: Vec<SchemaPolicy>,
    pub routines: Vec<RoutinePolicy>,
    pub types: Vec<TypePolicy>,
    pub parameters: Vec<ParameterPolicy>,
    pub database: DatabasePolicy,
    pub public_grants: Vec<PublicGrant>,
    /// Exact PUBLIC allowances, including an empty list to reject every grant.
    pub public_overrides: Vec<PublicAllowance>,
    /// Unsupported coverage that the caller explicitly requires. Merely
    /// allowing a routine ACL does not request proof of its body or
    /// `search_path`; those are required only when listed here.
    pub required_surfaces: Vec<RequiredSurface>,
}

/// An unsupported surface that must be inspected for a complete verdict.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequiredSurface {
    /// SECURITY DEFINER body, search-path and trigger-context semantics.
    SecurityDefinerBody,
    /// Extension membership, ownership and upgrade behavior.
    ExtensionSemantics,
    /// Role-level defaults outside explicit role attributes and ACLs.
    RoleDefaults,
    /// Foreign-server definitions and their ownership/ACL semantics.
    ForeignServers,
    /// Foreign-data-wrapper definitions and their ownership/ACL semantics.
    ForeignDataWrappers,
    /// Tablespace ownership and access semantics.
    Tablespaces,
    /// Large-object ownership and ACL semantics.
    LargeObjects,
    /// Other databases in the cluster.
    OtherDatabases,
    /// Publication definitions and membership semantics.
    Publications,
    /// Event-trigger definitions and execution semantics.
    EventTriggers,
    /// Procedural-language installation and trust semantics.
    Languages,
}

/// All inputs for one verification snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationPolicy {
    pub migration: MigrationPolicy,
    pub authority: AuthorityPolicy,
}

impl VerificationPolicy {
    /// Construct the migration and authority policy for one snapshot.
    pub fn new(migration: MigrationPolicy, authority: AuthorityPolicy) -> Self {
        Self {
            migration,
            authority,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        self.migration.validate()?;
        self.authority.validate()
    }
}

/// Invalid policy input, before any network work begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    /// Authority input exceeds 10,000 entries (including nested data), or one allowance list exceeds 32 entries.
    AuthorityCapacity,
    /// Duplicate required object/privilege pair.
    DuplicateRequiredPrivilege,
    /// Required privilege is invalid for the object or forbidden by its allowance.
    ContradictoryRequiredPrivilege,
    /// Parameter name exceeds the 1,024-byte input capacity.
    ParameterNameTooLarge,
    InvalidIdentifier,
    InvalidParameterName,
    InvalidRoutineType,
    DuplicateMigrationVersion,
    MigrationChecksumTooLarge,
    DuplicateAuthorityObject,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateRequiredPrivilege => "duplicate required privilege",
            Self::ContradictoryRequiredPrivilege => {
                "required privilege contradicts authority policy"
            }
            Self::ParameterNameTooLarge => "parameter name exceeds verification capacity",
            Self::InvalidIdentifier => "invalid PostgreSQL verification identifier",
            Self::InvalidParameterName => "invalid PostgreSQL configuration-parameter name",
            Self::InvalidRoutineType => "invalid canonical PostgreSQL routine type",
            Self::DuplicateMigrationVersion => "duplicate migration version in verification policy",
            Self::AuthorityCapacity => "authority policy capacity exceeded",
            Self::MigrationChecksumTooLarge => {
                "migration checksum exceeds the verification byte limit"
            }
            Self::DuplicateAuthorityObject => "duplicate authority object in verification policy",
        })
    }
}

impl std::error::Error for PolicyError {}

fn validate_identifier(value: &str) -> Result<(), PolicyError> {
    if value.is_empty() || value.len() > 63 || value.contains('\0') {
        return Err(PolicyError::InvalidIdentifier);
    }
    Ok(())
}

pub(crate) fn is_valid_custom_parameter_name(value: &str) -> bool {
    value.contains('.') && value.split('.').all(is_valid_parameter_component)
}

fn is_valid_parameter_name(value: &str) -> bool {
    !value.contains('\0')
        && if value.contains('.') {
            is_valid_custom_parameter_name(value)
        } else {
            is_valid_parameter_component(value)
        }
}

fn is_valid_parameter_component(component: &str) -> bool {
    let mut characters = component.chars();
    characters.next().is_some_and(|character| {
        character == '_' || character.is_ascii_alphabetic() || !character.is_ascii()
    }) && characters.all(|character| {
        character == '_'
            || character == '$'
            || character.is_ascii_alphanumeric()
            || !character.is_ascii()
    })
}

pub(crate) fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests;

impl MigrationPolicy {
    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        let mut versions = std::collections::HashSet::new();
        let listed = match &self.additional {
            AdditionalMigrations::AllowListed(rows) => rows.as_slice(),
            AdditionalMigrations::Reject | AdditionalMigrations::AllowSuccessful => &[],
        };
        for migration in self.required.iter().chain(listed.iter()) {
            if migration.checksum.len() > MAX_MIGRATION_CHECKSUM_BYTES {
                return Err(PolicyError::MigrationChecksumTooLarge);
            }
            if !versions.insert(migration.version) {
                return Err(PolicyError::DuplicateMigrationVersion);
            }
        }

        Ok(())
    }
}

impl AuthorityPolicy {
    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        self.validate_with_work(discovery::UncountedWork)
    }

    fn validate_with_work<W: discovery::ValidationWork>(&self, work: W) -> Result<(), PolicyError> {
        limits::validate(self)?;
        discovery::validate_authority(self, work)
    }

    #[cfg(test)]
    pub(crate) fn validate_counted(
        &self,
        operations: &std::sync::atomic::AtomicUsize,
    ) -> Result<(), PolicyError> {
        self.validate_with_work(discovery::CountingWork::new(operations))
    }
}
