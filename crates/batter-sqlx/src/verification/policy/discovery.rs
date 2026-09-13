use super::super::work::{Work, WorkMap, WorkSet};
use super::*;
use std::collections::HashSet;

pub(super) trait ValidationWork: Copy + Work {}

#[derive(Clone, Copy)]
pub(super) struct UncountedWork;

impl ValidationWork for UncountedWork {}

impl Work for UncountedWork {
    fn record(&self) {}
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) struct CountingWork<'a>(&'a std::sync::atomic::AtomicUsize);

#[cfg(test)]
impl<'a> CountingWork<'a> {
    pub(super) fn new(operations: &'a std::sync::atomic::AtomicUsize) -> Self {
        Self(operations)
    }
}

#[cfg(test)]
impl ValidationWork for CountingWork<'_> {}

#[cfg(test)]
impl Work for CountingWork<'_> {
    fn record(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

enum DiscoveryIndex<'a, W> {
    Declared,
    Schemas(WorkSet<&'a str, W>),
    UserSchemas,
}

fn index_discovery<W: ValidationWork>(policy: &AuthorityPolicy, work: W) -> DiscoveryIndex<'_, W> {
    match &policy.discovery {
        DiscoveryScope::Declared => DiscoveryIndex::Declared,
        DiscoveryScope::Schemas(names) => {
            let mut indexed = WorkSet::with_capacity(names.len(), work);
            for name in names {
                work.record();
                indexed.insert(name.as_str());
            }
            DiscoveryIndex::Schemas(indexed)
        }
        DiscoveryScope::UserSchemas => DiscoveryIndex::UserSchemas,
    }
}

struct PolicyIndex<'a, W> {
    allow_superuser: bool,
    defaults: &'a DiscoveryDefaults,
    database: &'a DatabasePolicy,
    relations: WorkMap<(&'a str, &'a str), &'a RelationPolicy, W>,
    columns: WorkMap<(&'a str, &'a str, &'a str), &'a ColumnPolicy, W>,
    sequences: WorkMap<(&'a str, &'a str), &'a SequencePolicy, W>,
    schemas: WorkMap<&'a str, &'a SchemaPolicy, W>,
    routines: WorkMap<&'a RoutineSignature, &'a RoutinePolicy, W>,
    types: WorkMap<(&'a str, &'a str), &'a TypePolicy, W>,
    parameters: WorkMap<&'a str, &'a ParameterPolicy, W>,
    public_grants: WorkSet<&'a PublicObject, W>,
    public: PublicDeclarations<'a>,
    discovery: DiscoveryIndex<'a, W>,
    work: W,
}

impl<'a, W: ValidationWork> PolicyIndex<'a, W> {
    fn new(policy: &'a AuthorityPolicy, work: W) -> Result<Self, PolicyError> {
        let mut relations = WorkMap::with_capacity(policy.relations.len(), work);
        let mut columns = WorkMap::with_capacity(0, work);
        for relation in &policy.relations {
            work.record();
            let relation_key = (relation.relation.schema(), relation.relation.name());
            if relations.insert(relation_key, relation).is_some() {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
            for column in &relation.columns {
                work.record();
                let column_key = (
                    relation.relation.schema(),
                    relation.relation.name(),
                    column.column.as_str(),
                );
                if columns.insert(column_key, column).is_some() {
                    return Err(PolicyError::DuplicateAuthorityObject);
                }
            }
        }

        let mut sequences = WorkMap::with_capacity(policy.sequences.len(), work);
        for sequence in &policy.sequences {
            work.record();
            if sequences
                .insert(
                    (sequence.sequence.schema(), sequence.sequence.name()),
                    sequence,
                )
                .is_some()
            {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
        }

        let mut schemas = WorkMap::with_capacity(policy.schemas.len(), work);
        for schema in &policy.schemas {
            work.record();
            if schemas.insert(schema.schema.as_str(), schema).is_some() {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
        }

        let mut routines = WorkMap::with_capacity(policy.routines.len(), work);
        for routine in &policy.routines {
            work.record();
            if routines.insert(&routine.routine, routine).is_some() {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
        }

        let mut types = WorkMap::with_capacity(policy.types.len(), work);
        for type_policy in &policy.types {
            work.record();
            if types
                .insert(
                    (type_policy.type_name.schema(), type_policy.type_name.name()),
                    type_policy,
                )
                .is_some()
            {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
        }

        let mut parameters = WorkMap::with_capacity(policy.parameters.len(), work);
        for parameter in &policy.parameters {
            work.record();
            if parameters
                .insert(parameter.parameter.as_str(), parameter)
                .is_some()
            {
                return Err(PolicyError::DuplicateAuthorityObject);
            }
        }

        let mut public_grants = WorkSet::with_capacity(policy.public_grants.len(), work);
        for grant in &policy.public_grants {
            work.record();
            public_grants.insert(&grant.object);
        }
        let discovery = index_discovery(policy, work);

        Ok(Self {
            allow_superuser: policy.roles.allow_superuser,
            defaults: &policy.defaults,
            database: &policy.database,
            relations,
            columns,
            sequences,
            schemas,
            routines,
            types,
            parameters,
            public_grants,
            public: PublicDeclarations::new(policy),
            discovery,
            work,
        })
    }

    fn discovery_includes(&self, schema: &str) -> bool {
        match &self.discovery {
            DiscoveryIndex::Declared => {
                self.work.record();
                false
            }
            DiscoveryIndex::Schemas(names) => names.contains(schema),
            DiscoveryIndex::UserSchemas => {
                self.work.record();
                schema != "information_schema" && !schema.starts_with("pg_")
            }
        }
    }

    fn relation(&self, name: &QualifiedName) -> Option<&'a RelationPolicy> {
        self.relations.get(&(name.schema(), name.name())).copied()
    }

    fn column(&self, name: &QualifiedName, column: &Identifier) -> Option<&'a ColumnPolicy> {
        self.columns
            .get(&(name.schema(), name.name(), column.as_str()))
            .copied()
    }

    fn sequence(&self, name: &QualifiedName) -> Option<&'a SequencePolicy> {
        self.sequences.get(&(name.schema(), name.name())).copied()
    }

    fn schema(&self, name: &Identifier) -> Option<&'a SchemaPolicy> {
        self.schemas.get(name.as_str()).copied()
    }

    fn routine(&self, name: &RoutineSignature) -> Option<&'a RoutinePolicy> {
        self.routines.get(name).copied()
    }

    fn type_policy(&self, name: &QualifiedName) -> Option<&'a TypePolicy> {
        self.types.get(&(name.schema(), name.name())).copied()
    }

    fn parameter(&self, name: &ParameterName) -> Option<&'a ParameterPolicy> {
        self.parameters.get(name.as_str()).copied()
    }
}

/// Objects to discover in addition to explicitly declared policy targets.
/// Discovery is bounded by the verifier's catalog capacity and uses its snapshot.
///
/// ```
/// use batter_sqlx::verification::{AuthorityPolicy, DiscoveryScope, ObjectDefaults,
///     AllowedPrivilege, ObjectPrivilege, PublicAllowance, PublicObject, QualifiedName};
/// let mut policy = AuthorityPolicy {
///     discovery: DiscoveryScope::UserSchemas,
///     ..AuthorityPolicy::default()
/// };
/// policy.defaults.invoker_routines = ObjectDefaults {
///     privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
///     public_privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)],
///     allow_owner: false,
/// };
/// policy.public_overrides.push(PublicAllowance {
///     object: PublicObject::Relation(QualifiedName::new("service", "private_records")?),
///     privileges: Vec::new(),
/// });
/// # Ok::<(), batter_sqlx::verification::PolicyError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DiscoveryScope {
    /// Only declared, required, and explicit PUBLIC targets.
    #[default]
    Declared,
    /// Every supported object in these exact schemas.
    Schemas(Vec<Identifier>),
    /// Every schema except `information_schema` and names beginning `pg_`.
    UserSchemas,
}

/// Allowances applied to a discovered object without an exact object policy.
/// Exact object policies replace role allowances and ownership defaults. An
/// explicit PUBLIC entry for a target replaces that target's PUBLIC defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObjectDefaults {
    pub privileges: Vec<AllowedPrivilege>,
    pub public_privileges: Vec<AllowedPrivilege>,
    pub allow_owner: bool,
}

/// Defaults by catalog object kind. Empty allowances reject observed authority.
/// Invoker and definer routines are separate because EXECUTE on a definer can
/// cross the caller's authority boundary; routine body correctness is unproven.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryDefaults {
    pub schemas: ObjectDefaults,
    pub relations: ObjectDefaults,
    pub columns: ObjectDefaults,
    pub sequences: ObjectDefaults,
    pub types: ObjectDefaults,
    pub invoker_routines: ObjectDefaults,
    pub definer_routines: ObjectDefaults,
    pub allow_row_type_public_usage: bool,
}

/// An exact PUBLIC allowance replacing scope defaults for this target.
/// Empty privileges explicitly reject every PUBLIC grant on this target.
/// An exact relation declaration also suppresses PUBLIC column defaults;
/// explicitly declared columns remain exceptions. Whole-relation allowances
/// still supply column privileges, so a column deny cannot subtract them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicAllowance {
    pub object: PublicObject,
    pub privileges: Vec<AllowedPrivilege>,
}

/// A privilege that the effective current role must possess without changing
/// roles or membership. PUBLIC and INHERIT grants count; SET and ADMIN do not.
/// This tests catalog privilege authority, not RLS-visible rows or routine bodies.
/// A custom parameter without observable context cannot satisfy a requirement;
/// its conservative possible SET authority produces incomplete coverage here.
///
/// ```
/// use batter_sqlx::verification::{RequiredPrivilege, PublicObject, QualifiedName, ObjectPrivilege};
/// let required = RequiredPrivilege {
///     object: PublicObject::Relation(QualifiedName::new("service", "records")?),
///     privilege: ObjectPrivilege::Select,
/// };
/// # Ok::<(), batter_sqlx::verification::PolicyError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredPrivilege {
    pub object: PublicObject,
    pub privilege: ObjectPrivilege,
}

impl DiscoveryScope {
    pub(crate) fn includes(&self, schema: &str) -> bool {
        match self {
            Self::Declared => false,
            Self::Schemas(names) => names.iter().any(|name| name.as_str() == schema),
            Self::UserSchemas => schema != "information_schema" && !schema.starts_with("pg_"),
        }
    }
}

pub(super) fn validate_authority<W: ValidationWork>(
    policy: &AuthorityPolicy,
    work: W,
) -> Result<(), PolicyError> {
    let index = PolicyIndex::new(policy, work)?;
    let mut overrides = HashSet::new();
    for entry in &policy.public_overrides {
        if !overrides.insert(&entry.object) || index.public_grants.contains(&entry.object) {
            return Err(PolicyError::DuplicateAuthorityObject);
        }
    }
    let mut seen = HashSet::new();
    for required in &policy.required_privileges {
        if !seen.insert((&required.object, required.privilege)) {
            return Err(PolicyError::DuplicateRequiredPrivilege);
        }
        if !valid_privilege(&required.object, required.privilege)
            || !requirement_allowed(&index, required)
        {
            return Err(PolicyError::ContradictoryRequiredPrivilege);
        }
    }
    Ok(())
}

fn contains(allowed: &[AllowedPrivilege], privilege: ObjectPrivilege) -> bool {
    allowed.iter().any(|entry| entry.privilege == privilege)
}

fn requirement_allowed<W: ValidationWork>(
    index: &PolicyIndex<'_, W>,
    required: &RequiredPrivilege,
) -> bool {
    if index.allow_superuser
        || public_requirement_allowed(index, &required.object, required.privilege)
    {
        return true;
    }
    let mut choices: Vec<(&[AllowedPrivilege], bool)> = Vec::new();
    let defaults = collect_allowances(index, &required.object, &mut choices);
    if let Some(defaults) = defaults {
        choices.push((&defaults.privileges, defaults.allow_owner));
    }
    choices
        .iter()
        .any(|(privileges, owner)| *owner || contains(privileges, required.privilege))
}

fn public_requirement_allowed<W: ValidationWork>(
    index: &PolicyIndex<'_, W>,
    object: &PublicObject,
    privilege: ObjectPrivilege,
) -> bool {
    if let PublicObject::Column(relation, _) = object
        && public_requirement_allowed(index, &PublicObject::Relation(relation.clone()), privilege)
    {
        return true;
    }
    if index.public.allows_exact(object, privilege) {
        return true;
    }
    if !index.public.defaults_apply(object) {
        return false;
    }
    let (schema, defaults) = match object {
        PublicObject::Relation(name) => (name.schema(), &index.defaults.relations),
        PublicObject::Column(name, _) => (name.schema(), &index.defaults.columns),
        PublicObject::Sequence(name) => (name.schema(), &index.defaults.sequences),
        PublicObject::Schema(name) => (name.as_str(), &index.defaults.schemas),
        PublicObject::Type(name) => (name.schema(), &index.defaults.types),
        PublicObject::Routine(name) => {
            if index.discovery_includes(name.schema())
                && contains(
                    &index.defaults.definer_routines.public_privileges,
                    privilege,
                )
            {
                return true;
            }
            (name.schema(), &index.defaults.invoker_routines)
        }
        PublicObject::Parameter(_) | PublicObject::Database => return false,
    };
    index.discovery_includes(schema) && contains(&defaults.public_privileges, privilege)
}

fn collect_allowances<'a, W: ValidationWork>(
    index: &'a PolicyIndex<'a, W>,
    object: &PublicObject,
    choices: &mut Vec<(&'a [AllowedPrivilege], bool)>,
) -> Option<&'a ObjectDefaults> {
    let mut defaults = None;
    match object {
        PublicObject::Relation(name) => {
            if let Some(entry) = index.relation(name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if index.discovery_includes(name.schema()) {
                defaults = Some(&index.defaults.relations);
            }
        }
        PublicObject::Column(name, column) => {
            if let Some(entry) = index.relation(name) {
                choices.push((&entry.privileges, entry.allow_owner));
                if let Some(column) = index.column(name, column) {
                    choices.push((&column.privileges, false));
                }
            } else if index.discovery_includes(name.schema()) {
                choices.push((
                    &index.defaults.relations.privileges,
                    index.defaults.relations.allow_owner,
                ));
                defaults = Some(&index.defaults.columns);
            }
        }
        PublicObject::Sequence(name) => {
            if let Some(entry) = index.sequence(name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if index.discovery_includes(name.schema()) {
                defaults = Some(&index.defaults.sequences);
            }
        }
        PublicObject::Schema(name) => {
            if let Some(entry) = index.schema(name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if index.discovery_includes(name.as_str()) {
                defaults = Some(&index.defaults.schemas);
            }
        }
        PublicObject::Type(name) => {
            if let Some(entry) = index.type_policy(name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if index.discovery_includes(name.schema()) {
                defaults = Some(&index.defaults.types);
            }
        }
        PublicObject::Routine(name) => {
            if let Some(entry) = index.routine(name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if index.discovery_includes(name.schema()) {
                // The catalog determines which routine default applies; any
                // forbidden observed authority is still reported during inspection.
                choices.push((
                    &index.defaults.definer_routines.privileges,
                    index.defaults.definer_routines.allow_owner,
                ));
                defaults = Some(&index.defaults.invoker_routines);
            }
        }
        PublicObject::Parameter(name) => {
            if let Some(entry) = index.parameter(name) {
                choices.push((&entry.privileges, false));
            }
        }
        PublicObject::Database => {
            choices.push((&index.database.privileges, index.database.allow_owner))
        }
    }
    defaults
}

fn valid_privilege(object: &PublicObject, privilege: ObjectPrivilege) -> bool {
    use ObjectPrivilege::*;
    match object {
        PublicObject::Relation(_) => matches!(
            privilege,
            Select | Insert | Update | Delete | Truncate | References | Trigger | Maintain
        ),
        PublicObject::Column(..) => matches!(privilege, Select | Insert | Update | References),
        PublicObject::Sequence(_) => matches!(privilege, Usage | Select | Update),
        PublicObject::Schema(_) => matches!(privilege, Usage | Create),
        PublicObject::Routine(_) => privilege == Execute,
        PublicObject::Type(_) => privilege == Usage,
        PublicObject::Parameter(_) => matches!(privilege, Set | AlterSystem),
        PublicObject::Database => matches!(privilege, Connect | Create | Temporary),
    }
}
