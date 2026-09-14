use super::*;

impl AuthorityPolicyBuilder {
    /// Start a policy draft with an explicit discovery scope.
    pub fn new(discovery: DiscoveryScope) -> Self {
        Self {
            discovery,
            ..Self::default()
        }
    }

    /// Validate and normalize this draft into an immutable executable policy.
    ///
    /// Set-like names and identical privilege atoms are canonicalized as
    /// documented on [`AuthorityPolicyBuilder`]. Duplicate keyed declarations,
    /// conflicting grant options, incompatible privileges, relation/sequence
    /// identity conflicts, contradictions, and capacity excess return a
    /// [`PolicyError`] without acquiring a database connection.
    pub fn build(self) -> Result<AuthorityPolicy, PolicyError> {
        self.try_into()
    }

    // Catalog expansion starts from a caller-validated, normalized policy. Its
    // private constructor checks new catalog identities during the already
    // checkpointed traversal and only appends normalized discovery defaults.
    // Re-running the external compiler here would synchronously sort and index
    // a much larger generated policy inside one executor poll.
    pub(crate) fn finish_catalog_expansion(self) -> AuthorityPolicy {
        AuthorityPolicy::from_builder(self)
    }
}

impl TryFrom<AuthorityPolicyBuilder> for AuthorityPolicy {
    type Error = PolicyError;

    fn try_from(mut builder: AuthorityPolicyBuilder) -> Result<Self, Self::Error> {
        limits::validate_builder(&builder)?;
        normalize_authority(&mut builder)?;
        let policy = Self::from_builder(builder);
        policy.validate_with_work(discovery::UncountedWork)?;
        Ok(policy)
    }
}

impl AuthorityPolicy {
    fn from_builder(builder: AuthorityPolicyBuilder) -> Self {
        Self {
            required_privileges: builder.required_privileges,
            discovery: builder.discovery,
            defaults: builder.defaults,
            roles: builder.roles,
            relations: builder.relations,
            sequences: builder.sequences,
            schemas: builder.schemas,
            routines: builder.routines,
            types: builder.types,
            parameters: builder.parameters,
            database: builder.database,
            public_grants: builder.public_grants,
            public_overrides: builder.public_overrides,
            required_surfaces: builder.required_surfaces,
        }
    }

    /// Start a mutable policy draft with an explicit discovery scope.
    pub fn builder(discovery: DiscoveryScope) -> AuthorityPolicyBuilder {
        AuthorityPolicyBuilder::new(discovery)
    }

    /// Return the selected discovery boundary.
    pub const fn discovery(&self) -> &DiscoveryScope {
        &self.discovery
    }

    /// Return required current-role privileges.
    pub fn required_privileges(&self) -> &[RequiredPrivilege] {
        &self.required_privileges
    }

    /// Return discovery defaults.
    pub const fn defaults(&self) -> &DiscoveryDefaults {
        &self.defaults
    }

    /// Return role-attribute and reachable-role allowances.
    pub const fn roles(&self) -> &RolePolicy {
        &self.roles
    }

    /// Return exact relation policies.
    pub fn relations(&self) -> &[RelationPolicy] {
        &self.relations
    }

    /// Return exact sequence policies.
    pub fn sequences(&self) -> &[SequencePolicy] {
        &self.sequences
    }

    /// Return exact schema policies.
    pub fn schemas(&self) -> &[SchemaPolicy] {
        &self.schemas
    }

    /// Return exact routine policies.
    pub fn routines(&self) -> &[RoutinePolicy] {
        &self.routines
    }

    /// Return exact type policies.
    pub fn types(&self) -> &[TypePolicy] {
        &self.types
    }

    /// Return exact parameter policies.
    pub fn parameters(&self) -> &[ParameterPolicy] {
        &self.parameters
    }

    /// Return current-database allowances.
    pub const fn database(&self) -> &DatabasePolicy {
        &self.database
    }

    /// Return legacy exact PUBLIC grants after normalization.
    pub fn public_grants(&self) -> &[PublicGrant] {
        &self.public_grants
    }

    /// Return exact PUBLIC allowances, including explicit empty denials.
    pub fn public_overrides(&self) -> &[PublicAllowance] {
        &self.public_overrides
    }

    /// Return unsupported surfaces required for a complete verdict.
    pub fn required_surfaces(&self) -> &[RequiredSurface] {
        &self.required_surfaces
    }

    /// Return a mutable draft initialized from this policy.
    ///
    /// The returned builder is not executable until [`AuthorityPolicyBuilder::build`]
    /// succeeds again.
    pub fn to_builder(&self) -> AuthorityPolicyBuilder {
        AuthorityPolicyBuilder {
            required_privileges: self.required_privileges.clone(),
            discovery: self.discovery.clone(),
            defaults: self.defaults.clone(),
            roles: self.roles.clone(),
            relations: self.relations.clone(),
            sequences: self.sequences.clone(),
            schemas: self.schemas.clone(),
            routines: self.routines.clone(),
            types: self.types.clone(),
            parameters: self.parameters.clone(),
            database: self.database.clone(),
            public_grants: self.public_grants.clone(),
            public_overrides: self.public_overrides.clone(),
            required_surfaces: self.required_surfaces.clone(),
        }
    }

    pub(crate) fn requires_sequence_identity(&self, name: &QualifiedName) -> bool {
        self.sequences.iter().any(|entry| entry.sequence == *name)
            || self
                .public_grants
                .iter()
                .any(|entry| sequence_object_matches(&entry.object, name))
            || self
                .public_overrides
                .iter()
                .any(|entry| sequence_object_matches(&entry.object, name))
            || self
                .required_privileges
                .iter()
                .any(|entry| sequence_object_matches(&entry.object, name))
    }

    #[cfg(test)]
    pub(crate) fn validate_semantics(&self) -> Result<(), PolicyError> {
        limits::validate_builder(&self.to_builder())?;
        self.validate_with_work(discovery::UncountedWork)
    }

    fn validate_with_work<W: discovery::ValidationWork>(&self, work: W) -> Result<(), PolicyError> {
        discovery::validate_authority(self, work)
    }

    #[cfg(test)]
    pub(crate) fn validate(&self) -> Result<(), PolicyError> {
        self.validate_semantics()
    }

    #[cfg(test)]
    pub(crate) fn validate_counted(
        &self,
        operations: &std::sync::atomic::AtomicUsize,
    ) -> Result<(), PolicyError> {
        self.validate_with_work(discovery::CountingWork::new(operations))
    }
}

fn sequence_object_matches(object: &PublicObject, name: &QualifiedName) -> bool {
    matches!(object, PublicObject::Sequence(candidate) if candidate == name)
}

impl Default for AuthorityPolicy {
    fn default() -> Self {
        AuthorityPolicyBuilder::default()
            .build()
            .expect("the empty deny-all authority policy is valid")
    }
}

#[derive(Clone, Copy)]
enum PrivilegeKind {
    Relation,
    Column,
    Sequence,
    Schema,
    Routine,
    Type,
    Parameter,
    Database,
}

fn privilege_kind(object: &PublicObject) -> PrivilegeKind {
    match object {
        PublicObject::Relation(_) => PrivilegeKind::Relation,
        PublicObject::Column(..) => PrivilegeKind::Column,
        PublicObject::Sequence(_) => PrivilegeKind::Sequence,
        PublicObject::Schema(_) => PrivilegeKind::Schema,
        PublicObject::Routine(_) => PrivilegeKind::Routine,
        PublicObject::Type(_) => PrivilegeKind::Type,
        PublicObject::Parameter(_) => PrivilegeKind::Parameter,
        PublicObject::Database => PrivilegeKind::Database,
    }
}

pub(crate) fn valid_privilege(object: &PublicObject, privilege: ObjectPrivilege) -> bool {
    valid_kind_privilege(privilege_kind(object), privilege)
}

fn valid_kind_privilege(kind: PrivilegeKind, privilege: ObjectPrivilege) -> bool {
    use ObjectPrivilege::*;
    match kind {
        PrivilegeKind::Relation => matches!(
            privilege,
            Select | Insert | Update | Delete | Truncate | References | Trigger | Maintain
        ),
        PrivilegeKind::Column => matches!(privilege, Select | Insert | Update | References),
        PrivilegeKind::Sequence => matches!(privilege, Usage | Select | Update),
        PrivilegeKind::Schema => matches!(privilege, Usage | Create),
        PrivilegeKind::Routine => privilege == Execute,
        PrivilegeKind::Type => privilege == Usage,
        PrivilegeKind::Parameter => matches!(privilege, Set | AlterSystem),
        PrivilegeKind::Database => matches!(privilege, Connect | Create | Temporary),
    }
}

fn normalize_allowances(
    kind: PrivilegeKind,
    privileges: &mut Vec<AllowedPrivilege>,
) -> Result<(), PolicyError> {
    privileges.sort_unstable();
    let mut normalized: Vec<AllowedPrivilege> = Vec::with_capacity(privileges.len());
    for entry in privileges.drain(..) {
        if !valid_kind_privilege(kind, entry.privilege) {
            return Err(PolicyError::InvalidObjectPrivilege);
        }
        if let Some(previous) = normalized.last()
            && previous.privilege == entry.privilege
        {
            if previous.grant_option != entry.grant_option {
                return Err(PolicyError::ContradictoryAuthorityPrivilege);
            }
            continue;
        }
        normalized.push(entry);
    }
    *privileges = normalized;
    Ok(())
}

fn normalize_defaults(
    kind: PrivilegeKind,
    defaults: &mut ObjectDefaults,
) -> Result<(), PolicyError> {
    normalize_allowances(kind, &mut defaults.privileges)?;
    normalize_allowances(kind, &mut defaults.public_privileges)
}

fn normalize_authority(builder: &mut AuthorityPolicyBuilder) -> Result<(), PolicyError> {
    if let DiscoveryScope::Schemas(schemas) = &mut builder.discovery {
        schemas.sort_unstable();
        schemas.dedup();
    }
    builder.roles.allowed_admin_roles.sort_unstable();
    builder.roles.allowed_admin_roles.dedup();
    builder.roles.allowed_predefined_roles.sort_unstable();
    builder.roles.allowed_predefined_roles.dedup();
    builder.required_surfaces.sort_unstable();
    builder.required_surfaces.dedup();
    normalize_defaults(PrivilegeKind::Relation, &mut builder.defaults.relations)?;
    normalize_defaults(PrivilegeKind::Column, &mut builder.defaults.columns)?;
    normalize_defaults(PrivilegeKind::Sequence, &mut builder.defaults.sequences)?;
    normalize_defaults(PrivilegeKind::Schema, &mut builder.defaults.schemas)?;
    normalize_defaults(PrivilegeKind::Type, &mut builder.defaults.types)?;
    normalize_defaults(
        PrivilegeKind::Routine,
        &mut builder.defaults.invoker_routines,
    )?;
    normalize_defaults(
        PrivilegeKind::Routine,
        &mut builder.defaults.definer_routines,
    )?;

    for relation in &mut builder.relations {
        normalize_allowances(PrivilegeKind::Relation, &mut relation.privileges)?;
        relation
            .columns
            .sort_by(|left, right| left.column.cmp(&right.column));
        for column in &mut relation.columns {
            normalize_allowances(PrivilegeKind::Column, &mut column.privileges)?;
        }
    }
    builder
        .relations
        .sort_by(|left, right| left.relation.cmp(&right.relation));
    for sequence in &mut builder.sequences {
        normalize_allowances(PrivilegeKind::Sequence, &mut sequence.privileges)?;
    }
    builder
        .sequences
        .sort_by(|left, right| left.sequence.cmp(&right.sequence));
    for schema in &mut builder.schemas {
        normalize_allowances(PrivilegeKind::Schema, &mut schema.privileges)?;
    }
    builder
        .schemas
        .sort_by(|left, right| left.schema.cmp(&right.schema));
    for routine in &mut builder.routines {
        normalize_allowances(PrivilegeKind::Routine, &mut routine.privileges)?;
    }
    builder
        .routines
        .sort_by(|left, right| left.routine.cmp(&right.routine));
    for type_policy in &mut builder.types {
        normalize_allowances(PrivilegeKind::Type, &mut type_policy.privileges)?;
    }
    builder
        .types
        .sort_by(|left, right| left.type_name.cmp(&right.type_name));
    for parameter in &mut builder.parameters {
        normalize_allowances(PrivilegeKind::Parameter, &mut parameter.privileges)?;
    }
    builder
        .parameters
        .sort_by(|left, right| left.parameter.cmp(&right.parameter));
    normalize_allowances(PrivilegeKind::Database, &mut builder.database.privileges)?;

    builder.public_grants.sort_unstable();
    let mut normalized_grants: Vec<PublicGrant> = Vec::with_capacity(builder.public_grants.len());
    for grant in builder.public_grants.drain(..) {
        if !valid_privilege(&grant.object, grant.privilege.privilege) {
            return Err(PolicyError::InvalidObjectPrivilege);
        }
        if let Some(previous) = normalized_grants.last()
            && previous.object == grant.object
            && previous.privilege.privilege == grant.privilege.privilege
        {
            if previous.privilege.grant_option != grant.privilege.grant_option {
                return Err(PolicyError::ContradictoryAuthorityPrivilege);
            }
            continue;
        }
        normalized_grants.push(grant);
    }
    builder.public_grants = normalized_grants;

    for allowance in &mut builder.public_overrides {
        normalize_allowances(privilege_kind(&allowance.object), &mut allowance.privileges)?;
    }
    builder
        .public_overrides
        .sort_by(|left, right| left.object.cmp(&right.object));

    for required in &builder.required_privileges {
        if !valid_privilege(&required.object, required.privilege) {
            return Err(PolicyError::InvalidObjectPrivilege);
        }
    }
    Ok(())
}
