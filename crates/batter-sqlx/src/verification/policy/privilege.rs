use super::{Identifier, ParameterName, QualifiedName, RoutineSignature};

/// Privileges used by the generic object policy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ObjectPrivilege {
    /// PostgreSQL `SELECT`.
    Select,
    /// PostgreSQL `INSERT`.
    Insert,
    /// PostgreSQL `UPDATE`.
    Update,
    /// PostgreSQL `DELETE`.
    Delete,
    /// PostgreSQL `TRUNCATE`.
    Truncate,
    /// PostgreSQL `REFERENCES`.
    References,
    /// PostgreSQL `TRIGGER`.
    Trigger,
    /// PostgreSQL `MAINTAIN`.
    Maintain,
    /// PostgreSQL `USAGE`.
    Usage,
    /// PostgreSQL `CREATE`.
    Create,
    /// PostgreSQL `CONNECT`.
    Connect,
    /// PostgreSQL `TEMPORARY`.
    Temporary,
    /// PostgreSQL `EXECUTE`.
    Execute,
    /// PostgreSQL `SET` on a configuration parameter.
    Set,
    /// PostgreSQL `ALTER SYSTEM` on a configuration parameter.
    AlterSystem,
}

/// One allowed privilege, optionally including the ability to grant it onward.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AllowedPrivilege {
    /// The allowed privilege.
    pub privilege: ObjectPrivilege,
    /// Whether the privilege may also carry its grant option.
    pub grant_option: bool,
}

/// Column-level allowances nested inside a relation policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnPolicy {
    /// The column within the enclosing relation.
    pub column: Identifier,
    /// Privileges reachable roles may hold on this column.
    pub privileges: Vec<AllowedPrivilege>,
}

/// Table, view, materialized-view or foreign-table authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationPolicy {
    /// The schema-qualified relation.
    pub relation: QualifiedName,
    /// Privileges reachable roles may hold on the whole relation.
    pub privileges: Vec<AllowedPrivilege>,
    /// Additional column-level allowances.
    pub columns: Vec<ColumnPolicy>,
    /// Permit a role reachable by the login to own this relation.
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
    /// The schema-qualified sequence.
    pub sequence: QualifiedName,
    /// Privileges reachable roles may hold on the sequence.
    pub privileges: Vec<AllowedPrivilege>,
    /// Permit a role reachable by the login to own this sequence.
    pub allow_owner: bool,
}

/// Schema authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaPolicy {
    /// The schema.
    pub schema: Identifier,
    /// Privileges reachable roles may hold on the schema.
    pub privileges: Vec<AllowedPrivilege>,
    /// Permit a role reachable by the login to own this schema.
    pub allow_owner: bool,
}

/// Routine authority policy. Security-definer bodies and search paths remain
/// outside this generic ACL checker and are reported as unsupported coverage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutinePolicy {
    /// The exact routine overload.
    pub routine: RoutineSignature,
    /// Privileges reachable roles may hold on the routine.
    pub privileges: Vec<AllowedPrivilege>,
    /// Permit a role reachable by the login to own this routine.
    pub allow_owner: bool,
    /// Permit this exact routine to be `SECURITY DEFINER`.
    pub allow_security_definer: bool,
}

/// User-defined type authority policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypePolicy {
    /// The schema-qualified type.
    pub type_name: QualifiedName,
    /// Privileges reachable roles may hold on the type.
    pub privileges: Vec<AllowedPrivilege>,
    /// Permit a role reachable by the login to own this type.
    pub allow_owner: bool,
}

/// Parameter authority policy (`SET` and `ALTER SYSTEM`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterPolicy {
    /// The configuration parameter.
    pub parameter: ParameterName,
    /// `SET` or `ALTER SYSTEM` authority reachable roles may hold.
    pub privileges: Vec<AllowedPrivilege>,
}

/// Current-database authority policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DatabasePolicy {
    /// Privileges reachable roles may hold on the current database.
    pub privileges: Vec<AllowedPrivilege>,
    /// Permit a role reachable by the login to own this current database.
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
    /// Permit a reachable role with `SUPERUSER`.
    pub allow_superuser: bool,
    /// Permit a reachable role with `CREATEDB`.
    pub allow_create_database: bool,
    /// Permit a reachable role with `CREATEROLE`.
    pub allow_create_role: bool,
    /// Permit a reachable role with `REPLICATION`.
    pub allow_replication: bool,
    /// Permit a reachable role with `BYPASSRLS`.
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

impl AllowedPrivilege {
    /// A privilege without authority to grant it onward.
    ///
    /// Migration: replaces `AllowedPrivilege::new(privilege, false)`.
    ///
    /// ```
    /// use batter_sqlx::verification::{AllowedPrivilege, ObjectPrivilege};
    /// let usage = AllowedPrivilege::privilege(ObjectPrivilege::Usage);
    /// assert!(!usage.grant_option);
    /// ```
    pub const fn privilege(privilege: ObjectPrivilege) -> Self {
        Self {
            privilege,
            grant_option: false,
        }
    }

    /// A privilege with authority to grant it onward.
    ///
    /// Migration: replaces `AllowedPrivilege::new(privilege, true)`.
    ///
    /// ```
    /// use batter_sqlx::verification::{AllowedPrivilege, ObjectPrivilege};
    /// let usage = AllowedPrivilege::grantable(ObjectPrivilege::Usage);
    /// assert!(usage.grant_option);
    /// ```
    pub const fn grantable(privilege: ObjectPrivilege) -> Self {
        Self {
            privilege,
            grant_option: true,
        }
    }
}
