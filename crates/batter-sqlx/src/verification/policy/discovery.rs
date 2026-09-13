use super::*;
use std::collections::HashSet;

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

pub(super) fn validate_authority(policy: &AuthorityPolicy) -> Result<(), PolicyError> {
    let mut overrides = HashSet::new();
    for entry in &policy.public_overrides {
        if !overrides.insert(&entry.object)
            || policy
                .public_grants
                .iter()
                .any(|grant| grant.object == entry.object)
        {
            return Err(PolicyError::DuplicateAuthorityObject);
        }
    }
    let public = PublicDeclarations::new(policy);
    let mut seen = HashSet::new();
    for required in &policy.required_privileges {
        if !seen.insert((&required.object, required.privilege)) {
            return Err(PolicyError::DuplicateRequiredPrivilege);
        }
        if !valid_privilege(&required.object, required.privilege)
            || !requirement_allowed(policy, required, &public)
        {
            return Err(PolicyError::ContradictoryRequiredPrivilege);
        }
    }
    Ok(())
}

fn contains(allowed: &[AllowedPrivilege], privilege: ObjectPrivilege) -> bool {
    allowed.iter().any(|entry| entry.privilege == privilege)
}

fn requirement_allowed(
    policy: &AuthorityPolicy,
    required: &RequiredPrivilege,
    public: &PublicDeclarations<'_>,
) -> bool {
    if policy.roles.allow_superuser
        || public_requirement_allowed(policy, public, &required.object, required.privilege)
    {
        return true;
    }
    let mut choices: Vec<(&[AllowedPrivilege], bool)> = Vec::new();
    let defaults = collect_allowances(policy, &required.object, &mut choices);
    if let Some(defaults) = defaults {
        choices.push((&defaults.privileges, defaults.allow_owner));
    }
    choices
        .iter()
        .any(|(privileges, owner)| *owner || contains(privileges, required.privilege))
}

fn public_requirement_allowed(
    policy: &AuthorityPolicy,
    public: &PublicDeclarations<'_>,
    object: &PublicObject,
    privilege: ObjectPrivilege,
) -> bool {
    if let PublicObject::Column(relation, _) = object
        && public_requirement_allowed(
            policy,
            public,
            &PublicObject::Relation(relation.clone()),
            privilege,
        )
    {
        return true;
    }
    if public.allows_exact(object, privilege) {
        return true;
    }
    if !public.defaults_apply(object) {
        return false;
    }
    let (schema, defaults) = match object {
        PublicObject::Relation(name) => (name.schema(), &policy.defaults.relations),
        PublicObject::Column(name, _) => (name.schema(), &policy.defaults.columns),
        PublicObject::Sequence(name) => (name.schema(), &policy.defaults.sequences),
        PublicObject::Schema(name) => (name.as_str(), &policy.defaults.schemas),
        PublicObject::Type(name) => (name.schema(), &policy.defaults.types),
        PublicObject::Routine(name) => {
            if policy.discovery.includes(name.schema())
                && contains(
                    &policy.defaults.definer_routines.public_privileges,
                    privilege,
                )
            {
                return true;
            }
            (name.schema(), &policy.defaults.invoker_routines)
        }
        PublicObject::Parameter(_) | PublicObject::Database => return false,
    };
    policy.discovery.includes(schema) && contains(&defaults.public_privileges, privilege)
}

fn collect_allowances<'a>(
    policy: &'a AuthorityPolicy,
    object: &PublicObject,
    choices: &mut Vec<(&'a [AllowedPrivilege], bool)>,
) -> Option<&'a ObjectDefaults> {
    let mut defaults = None;
    match object {
        PublicObject::Relation(name) => {
            if let Some(entry) = policy
                .relations
                .iter()
                .find(|entry| entry.relation == *name)
            {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if policy.discovery.includes(name.schema()) {
                defaults = Some(&policy.defaults.relations);
            }
        }
        PublicObject::Column(name, column) => {
            if let Some(entry) = policy
                .relations
                .iter()
                .find(|entry| entry.relation == *name)
            {
                choices.push((&entry.privileges, entry.allow_owner));
                if let Some(column) = entry.columns.iter().find(|entry| entry.column == *column) {
                    choices.push((&column.privileges, false));
                }
            } else if policy.discovery.includes(name.schema()) {
                choices.push((
                    &policy.defaults.relations.privileges,
                    policy.defaults.relations.allow_owner,
                ));
                defaults = Some(&policy.defaults.columns);
            }
        }
        PublicObject::Sequence(name) => {
            if let Some(entry) = policy
                .sequences
                .iter()
                .find(|entry| entry.sequence == *name)
            {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if policy.discovery.includes(name.schema()) {
                defaults = Some(&policy.defaults.sequences);
            }
        }
        PublicObject::Schema(name) => {
            if let Some(entry) = policy.schemas.iter().find(|entry| entry.schema == *name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if policy.discovery.includes(name.as_str()) {
                defaults = Some(&policy.defaults.schemas);
            }
        }
        PublicObject::Type(name) => {
            if let Some(entry) = policy.types.iter().find(|entry| entry.type_name == *name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if policy.discovery.includes(name.schema()) {
                defaults = Some(&policy.defaults.types);
            }
        }
        PublicObject::Routine(name) => {
            if let Some(entry) = policy.routines.iter().find(|entry| entry.routine == *name) {
                choices.push((&entry.privileges, entry.allow_owner));
            } else if policy.discovery.includes(name.schema()) {
                // The catalog determines which routine default applies; any
                // forbidden observed authority is still reported during inspection.
                choices.push((
                    &policy.defaults.definer_routines.privileges,
                    policy.defaults.definer_routines.allow_owner,
                ));
                defaults = Some(&policy.defaults.invoker_routines);
            }
        }
        PublicObject::Parameter(name) => {
            if let Some(entry) = policy
                .parameters
                .iter()
                .find(|entry| entry.parameter == *name)
            {
                choices.push((&entry.privileges, false));
            }
        }
        PublicObject::Database => {
            choices.push((&policy.database.privileges, policy.database.allow_owner))
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
