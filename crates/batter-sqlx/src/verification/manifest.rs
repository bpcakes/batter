//! Pure construction of exact-role policies and PostgreSQL grant plans.
//!
//! [`ExactRoleManifest`] is a high-level, application-owned input. Compilation
//! validates and normalizes it into an immutable [`AuthorityPolicy`] plus a
//! deterministic [`GrantPlan`]. Neither compilation nor rendering connects to a
//! database or executes SQL.

mod compile;
mod render;

use super::{
    AuthorityPolicy, AuthorityPolicyBuilder, DiscoveryDefaults, DiscoveryScope, Identifier,
    ObjectPrivilege, PolicyError, QualifiedName, RolePolicy, RoutineSignature,
};
pub use render::GrantRenderError;
use std::fmt;

const MAX_MANIFEST_INPUTS: usize = 10_000;

/// Invalid exact-role manifest input.
///
/// This error is separate from [`PolicyError`] so adding high-level manifest
/// validation does not change the established low-level policy error contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    /// The normalized low-level authority policy is invalid.
    Policy(PolicyError),
    /// One grouped declaration has no target object.
    EmptyTarget,
    /// A privilege cannot apply to the declared PostgreSQL object kind.
    InvalidObjectPrivilege,
    /// Repeated declarations disagree about one exact semantic option.
    ContradictoryDeclaration,
    /// A column group has no explicit parent relation declaration.
    MissingRelationDeclaration,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Policy(_) => "compiled exact-role policy is invalid",
            Self::EmptyTarget => "exact-role manifest group has no target",
            Self::InvalidObjectPrivilege => {
                "privilege is invalid for the exact-role manifest object"
            }
            Self::ContradictoryDeclaration => {
                "exact-role manifest declarations contradict one another"
            }
            Self::MissingRelationDeclaration => {
                "column group requires an explicit parent relation declaration"
            }
        })
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Policy(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PolicyError> for ManifestError {
    fn from(error: PolicyError) -> Self {
        Self::Policy(error)
    }
}

/// Whether a declared privilege is required or merely an allowed ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationPurpose {
    /// Require the privilege, allow it during verification, and include it in
    /// the role-targeted grant plan.
    RequiredAndProvisioned,
    /// Permit the privilege during verification without provisioning it.
    AllowedOnly,
}

/// Whether PUBLIC may deliver one declared privilege.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PublicDelivery {
    /// Reject PUBLIC delivery of the privilege, including when discovery
    /// defaults would otherwise permit it. A column deny conflicts with a
    /// PUBLIC allowance for the same privilege on its parent relation because
    /// PostgreSQL relation privileges also apply to columns.
    #[default]
    Deny,
    /// Permit PUBLIC to deliver this exact declared privilege.
    AllowDeclared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeclarationOptions {
    purpose: DeclarationPurpose,
    public_delivery: PublicDelivery,
    allow_grant_option: bool,
}

impl DeclarationOptions {
    const fn new(purpose: DeclarationPurpose) -> Self {
        Self {
            purpose,
            public_delivery: PublicDelivery::Deny,
            allow_grant_option: false,
        }
    }
}

/// Current-database declaration for an exact role.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseGrantSpec {
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
    allow_owner: bool,
}

impl DatabaseGrantSpec {
    /// Construct a database declaration. An empty privilege list is allowed so
    /// ownership and an exact empty PUBLIC allowance can be expressed.
    pub fn new(
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
        purpose: DeclarationPurpose,
    ) -> Result<Self, ManifestError> {
        Ok(Self {
            privileges: collect_bounded(privileges)?,
            options: DeclarationOptions::new(purpose),
            allow_owner: false,
        })
    }

    /// Permit PUBLIC delivery for every privilege in this declaration.
    pub const fn public_delivery(mut self, delivery: PublicDelivery) -> Self {
        self.options.public_delivery = delivery;
        self
    }

    /// Permit the authenticated login to hold grant options for these
    /// privileges. The renderer still never provisions grant options.
    pub const fn allow_grant_option(mut self, allow: bool) -> Self {
        self.options.allow_grant_option = allow;
        self
    }

    /// Permit current-database ownership in the compiled authority policy.
    pub const fn allow_owner(mut self, allow: bool) -> Self {
        self.allow_owner = allow;
        self
    }
}

/// One schema declaration for an exact role.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaGrantSpec {
    schema: Identifier,
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
    allow_owner: bool,
}

impl SchemaGrantSpec {
    /// Construct one schema declaration.
    pub fn new(
        schema: Identifier,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
        purpose: DeclarationPurpose,
    ) -> Result<Self, ManifestError> {
        Ok(Self {
            schema,
            privileges: collect_bounded(privileges)?,
            options: DeclarationOptions::new(purpose),
            allow_owner: false,
        })
    }

    /// Permit PUBLIC delivery for every privilege in this declaration.
    pub const fn public_delivery(mut self, delivery: PublicDelivery) -> Self {
        self.options.public_delivery = delivery;
        self
    }

    /// Permit grant options for these privileges during verification only.
    pub const fn allow_grant_option(mut self, allow: bool) -> Self {
        self.options.allow_grant_option = allow;
        self
    }

    /// Permit ownership of this schema.
    pub const fn allow_owner(mut self, allow: bool) -> Self {
        self.allow_owner = allow;
        self
    }
}

/// One privilege declaration expanded over one or more exact relations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationGrantGroup {
    relations: Vec<QualifiedName>,
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
    allow_owner: bool,
    allow_row_type_public_usage: bool,
}

impl RelationGrantGroup {
    /// Construct a relation group. At least one relation is required; an empty
    /// privilege list still declares the relation and contributes no allowed
    /// privilege. Later compatible groups may add privileges to that relation.
    /// PostgreSQL ordinarily gives PUBLIC USAGE on table row types, so callers
    /// that retain that default must opt in with
    /// [`Self::allow_row_type_public_usage`].
    pub fn new(
        relations: impl IntoIterator<Item = QualifiedName>,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
        purpose: DeclarationPurpose,
    ) -> Result<Self, ManifestError> {
        let relations = collect_bounded(relations)?;
        if relations.is_empty() {
            return Err(ManifestError::EmptyTarget);
        }
        Ok(Self {
            relations,
            privileges: collect_bounded(privileges)?,
            options: DeclarationOptions::new(purpose),
            allow_owner: false,
            allow_row_type_public_usage: false,
        })
    }

    /// Permit PUBLIC delivery for every privilege in this declaration.
    pub const fn public_delivery(mut self, delivery: PublicDelivery) -> Self {
        self.options.public_delivery = delivery;
        self
    }

    /// Permit grant options for these privileges during verification only.
    pub const fn allow_grant_option(mut self, allow: bool) -> Self {
        self.options.allow_grant_option = allow;
        self
    }

    /// Permit ownership of every relation in the group.
    pub const fn allow_owner(mut self, allow: bool) -> Self {
        self.allow_owner = allow;
        self
    }

    /// Permit default PUBLIC USAGE on every relation's composite row type.
    pub const fn allow_row_type_public_usage(mut self, allow: bool) -> Self {
        self.allow_row_type_public_usage = allow;
        self
    }
}

/// One privilege declaration expanded over columns of one exact relation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnGrantGroup {
    relation: QualifiedName,
    columns: Vec<Identifier>,
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
}

impl ColumnGrantGroup {
    /// Construct a column group. At least one column is required; an empty
    /// privilege list contributes no allowed column privilege. Compilation
    /// requires the parent relation to be declared by a relation group so its
    /// ownership, row-type and PUBLIC behavior are never implicit.
    pub fn new(
        relation: QualifiedName,
        columns: impl IntoIterator<Item = Identifier>,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
        purpose: DeclarationPurpose,
    ) -> Result<Self, ManifestError> {
        let columns = collect_bounded(columns)?;
        if columns.is_empty() {
            return Err(ManifestError::EmptyTarget);
        }
        Ok(Self {
            relation,
            columns,
            privileges: collect_bounded(privileges)?,
            options: DeclarationOptions::new(purpose),
        })
    }

    /// Permit PUBLIC delivery for every privilege in this declaration.
    pub const fn public_delivery(mut self, delivery: PublicDelivery) -> Self {
        self.options.public_delivery = delivery;
        self
    }

    /// Permit grant options for these privileges during verification only.
    pub const fn allow_grant_option(mut self, allow: bool) -> Self {
        self.options.allow_grant_option = allow;
        self
    }
}

/// One exact routine-overload declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutineGrantSpec {
    routine: RoutineSignature,
    privileges: Vec<ObjectPrivilege>,
    options: DeclarationOptions,
    allow_owner: bool,
    allow_security_definer: bool,
}

impl RoutineGrantSpec {
    /// Construct an exact routine declaration. The existing structural
    /// [`RoutineSignature`] is retained without parsing a SQL type expression.
    pub fn new(
        routine: RoutineSignature,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
        purpose: DeclarationPurpose,
    ) -> Result<Self, ManifestError> {
        Ok(Self {
            routine,
            privileges: collect_bounded(privileges)?,
            options: DeclarationOptions::new(purpose),
            allow_owner: false,
            allow_security_definer: false,
        })
    }

    /// Permit PUBLIC delivery for every privilege in this declaration.
    pub const fn public_delivery(mut self, delivery: PublicDelivery) -> Self {
        self.options.public_delivery = delivery;
        self
    }

    /// Permit grant options for these privileges during verification only.
    pub const fn allow_grant_option(mut self, allow: bool) -> Self {
        self.options.allow_grant_option = allow;
        self
    }

    /// Permit ownership of this routine.
    pub const fn allow_owner(mut self, allow: bool) -> Self {
        self.allow_owner = allow;
        self
    }

    /// Permit this exact routine to be SECURITY DEFINER.
    pub const fn allow_security_definer(mut self, allow: bool) -> Self {
        self.allow_security_definer = allow;
        self
    }
}

/// Application-owned exact-role input compiled without database I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactRoleManifest {
    primary_schema: Identifier,
    discovery: DiscoveryScope,
    defaults: DiscoveryDefaults,
    roles: RolePolicy,
    databases: Vec<DatabaseGrantSpec>,
    schemas: Vec<SchemaGrantSpec>,
    relations: Vec<RelationGrantGroup>,
    columns: Vec<ColumnGrantGroup>,
    routines: Vec<RoutineGrantSpec>,
    deny_current_database_ownership: bool,
}

impl ExactRoleManifest {
    /// Begin an exact-role manifest with an application schema and discovery
    /// boundary. The schema is retained for later protected schema requests; it
    /// does not implicitly grant authority.
    pub fn new(
        primary_schema: Identifier,
        discovery: DiscoveryScope,
    ) -> Result<Self, ManifestError> {
        let manifest = Self {
            primary_schema,
            discovery,
            defaults: DiscoveryDefaults::default(),
            roles: RolePolicy::default(),
            databases: Vec::new(),
            schemas: Vec::new(),
            relations: Vec::new(),
            columns: Vec::new(),
            routines: Vec::new(),
            deny_current_database_ownership: false,
        };
        manifest.ensure_input_capacity(0)?;
        Ok(manifest)
    }

    /// Replace discovery defaults. Defaults constrain verification and never
    /// produce grant statements.
    pub fn set_discovery_defaults(
        &mut self,
        defaults: DiscoveryDefaults,
    ) -> Result<(), ManifestError> {
        let previous = std::mem::replace(&mut self.defaults, defaults);
        if let Err(error) = self.ensure_input_capacity(0) {
            self.defaults = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Replace role-attribute and reachable-role ceilings.
    pub fn set_role_policy(&mut self, roles: RolePolicy) -> Result<(), ManifestError> {
        let previous = std::mem::replace(&mut self.roles, roles);
        if let Err(error) = self.ensure_input_capacity(0) {
            self.roles = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Add a current-database declaration.
    pub fn add_database(&mut self, spec: DatabaseGrantSpec) -> Result<(), ManifestError> {
        self.ensure_input_capacity(spec.input_size())?;
        self.databases.push(spec);
        Ok(())
    }

    /// Add one schema declaration.
    pub fn add_schema(&mut self, spec: SchemaGrantSpec) -> Result<(), ManifestError> {
        self.ensure_input_capacity(spec.input_size())?;
        self.schemas.push(spec);
        Ok(())
    }

    /// Add a grouped relation declaration.
    pub fn add_relations(&mut self, group: RelationGrantGroup) -> Result<(), ManifestError> {
        self.ensure_input_capacity(group.input_size())?;
        self.relations.push(group);
        Ok(())
    }

    /// Add a grouped column declaration.
    pub fn add_columns(&mut self, group: ColumnGrantGroup) -> Result<(), ManifestError> {
        self.ensure_input_capacity(group.input_size())?;
        self.columns.push(group);
        Ok(())
    }

    /// Add one exact routine declaration.
    pub fn add_routine(&mut self, spec: RoutineGrantSpec) -> Result<(), ManifestError> {
        self.ensure_input_capacity(spec.input_size())?;
        self.routines.push(spec);
        Ok(())
    }

    /// Reject ownership of the current database or any current-database object
    /// by a role reachable from the authenticated login.
    pub const fn deny_current_database_ownership(&mut self, deny: bool) {
        self.deny_current_database_ownership = deny;
    }

    /// Validate and normalize the manifest into one verifier policy and grant
    /// plan. Failure returns no partial compiled value or rendered SQL.
    pub fn compile(self) -> Result<CompiledExactRole, ManifestError> {
        compile::compile(self)
    }

    fn ensure_input_capacity(&self, additional: usize) -> Result<(), ManifestError> {
        self.input_size()?
            .checked_add(additional)
            .filter(|size| *size <= MAX_MANIFEST_INPUTS)
            .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))
            .map(|_| ())
    }

    fn input_size(&self) -> Result<usize, ManifestError> {
        let discovery = match &self.discovery {
            DiscoveryScope::Schemas(schemas) => schemas.len(),
            DiscoveryScope::Declared | DiscoveryScope::UserSchemas => 0,
        };
        let defaults = defaults_input_size(&self.defaults)?;
        let roles = self
            .roles
            .allowed_admin_roles
            .len()
            .checked_add(self.roles.allowed_predefined_roles.len())
            .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))?;
        self.databases
            .iter()
            .map(DatabaseGrantSpec::input_size)
            .chain(self.schemas.iter().map(SchemaGrantSpec::input_size))
            .chain(self.relations.iter().map(RelationGrantGroup::input_size))
            .chain(self.columns.iter().map(ColumnGrantGroup::input_size))
            .chain(self.routines.iter().map(RoutineGrantSpec::input_size))
            .try_fold(discovery + defaults + roles + 1, checked_add)
    }
}

/// Normalized exact-role output. Verification consumes this value so its policy
/// and ownership safeguard stay coupled. The grant plan remains inert data until
/// a caller explicitly renders it.
///
/// ```compile_fail
/// use batter_sqlx::verification::{DiscoveryScope, ExactRoleManifest, Identifier};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let role = ExactRoleManifest::new(
///     Identifier::new("service")?,
///     DiscoveryScope::Declared,
/// )?.compile()?;
/// // The authority policy cannot be detached from exact-role safeguards.
/// let _ = role.authority_policy();
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledExactRole {
    primary_schema: Identifier,
    authority_policy: AuthorityPolicy,
    grant_plan: GrantPlan,
    deny_current_database_ownership: bool,
}

impl CompiledExactRole {
    /// Return the primary application schema supplied by the manifest.
    pub fn primary_schema(&self) -> &Identifier {
        &self.primary_schema
    }

    pub(crate) fn authority_policy(&self) -> &AuthorityPolicy {
        &self.authority_policy
    }

    /// Borrow the deterministic, non-executing role grant plan.
    pub fn grant_plan(&self) -> &GrantPlan {
        &self.grant_plan
    }

    pub(crate) const fn denies_current_database_ownership(&self) -> bool {
        self.deny_current_database_ownership
    }
}

/// A normalized pure plan of role-targeted PostgreSQL `GRANT` statements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantPlan {
    atoms: Vec<GrantAtom>,
}

impl GrantPlan {
    /// Return whether the plan contains no provisioned grants.
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Return the number of normalized object/privilege grants.
    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    /// Return whether rendering requires an explicit database identifier.
    pub fn requires_database_context(&self) -> bool {
        self.atoms
            .iter()
            .any(|atom| matches!(atom.target, GrantTarget::Database))
    }

    /// Render deterministic PostgreSQL statements without performing I/O.
    ///
    /// `database` is required exactly when [`Self::requires_database_context`]
    /// is true. The returned string ends each statement with a newline.
    ///
    /// ```
    /// use batter_sqlx::verification::{
    ///     DeclarationPurpose, DiscoveryScope, ExactRoleManifest, Identifier,
    ///     ObjectPrivilege, SchemaGrantSpec,
    /// };
    ///
    /// let schema = Identifier::new("service")?;
    /// let mut manifest = ExactRoleManifest::new(schema.clone(), DiscoveryScope::Declared)?;
    /// manifest.add_schema(SchemaGrantSpec::new(
    ///     schema,
    ///     [ObjectPrivilege::Usage],
    ///     DeclarationPurpose::RequiredAndProvisioned,
    /// )?)?;
    /// let compiled = manifest.compile()?;
    /// assert_eq!(
    ///     compiled
    ///         .grant_plan()
    ///         .render(&Identifier::new("service_reader")?, None)?,
    ///     "GRANT USAGE ON SCHEMA \"service\" TO \"service_reader\";\n",
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`GrantRenderError`] before constructing output when the role is
    /// a PostgreSQL special/reserved target, database context is missing, or
    /// database context is supplied for a plan that has no database grant.
    pub fn render(
        &self,
        role: &Identifier,
        database: Option<&Identifier>,
    ) -> Result<String, GrantRenderError> {
        render::render(self, role, database)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum GrantTarget {
    Database,
    Schema(Identifier),
    Relation(QualifiedName),
    Column(QualifiedName, Identifier),
    Routine(RoutineSignature),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GrantAtom {
    target: GrantTarget,
    privilege: ObjectPrivilege,
}

fn collect_bounded<T>(items: impl IntoIterator<Item = T>) -> Result<Vec<T>, ManifestError> {
    let mut collected = Vec::new();
    for item in items {
        if collected.len() == MAX_MANIFEST_INPUTS {
            return Err(ManifestError::Policy(PolicyError::AuthorityCapacity));
        }
        collected.push(item);
    }
    Ok(collected)
}

fn checked_add(total: usize, next: usize) -> Result<usize, ManifestError> {
    total
        .checked_add(next)
        .filter(|size| *size <= MAX_MANIFEST_INPUTS)
        .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))
}

fn defaults_input_size(defaults: &DiscoveryDefaults) -> Result<usize, ManifestError> {
    [
        &defaults.schemas,
        &defaults.relations,
        &defaults.columns,
        &defaults.sequences,
        &defaults.types,
        &defaults.invoker_routines,
        &defaults.definer_routines,
    ]
    .into_iter()
    .flat_map(|entry| [&entry.privileges, &entry.public_privileges])
    .map(Vec::len)
    .try_fold(0, checked_add)
}

impl DatabaseGrantSpec {
    fn input_size(&self) -> usize {
        self.privileges.len() + 1
    }
}

impl SchemaGrantSpec {
    fn input_size(&self) -> usize {
        self.privileges.len() + 1
    }
}

impl RelationGrantGroup {
    fn input_size(&self) -> usize {
        self.relations.len() + self.privileges.len() + 1
    }
}

impl ColumnGrantGroup {
    fn input_size(&self) -> usize {
        self.columns.len() + self.privileges.len() + 1
    }
}

impl RoutineGrantSpec {
    fn input_size(&self) -> usize {
        self.routine.arguments().len() + self.privileges.len() + 1
    }
}

#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod tests;
