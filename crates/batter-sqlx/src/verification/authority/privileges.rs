use super::evaluation::Evaluation;
mod public;
use super::super::policy::{
    AllowedPrivilege, ObjectPrivilege, PolicyError, PublicObject, RoutinePolicy, RoutineSignature,
    RoutineType,
};
use super::super::report::{Finding, FindingKind, VerificationError};
use super::{AclEntry, CatalogSnapshot, ParameterIndex, RoleGraph, RoutineObject, TypeName};
pub(super) use public::PublicPolicy;
use std::collections::HashMap;

pub(super) struct IndexedAcl {
    grants: HashMap<(i64, ObjectPrivilege), bool>,
}

impl IndexedAcl {
    pub(super) fn new(parts: &[&[AclEntry]]) -> Self {
        let mut grants = HashMap::new();
        for entry in parts.iter().flat_map(|entries| entries.iter()) {
            grants
                .entry((entry.grantee, entry.privilege))
                .and_modify(|grant_option| *grant_option |= entry.grant_option)
                .or_insert(entry.grant_option);
        }
        Self { grants }
    }

    pub(super) fn actual_for(
        &self,
        subject: i64,
        owner: Option<i64>,
        graph: &RoleGraph<'_>,
        privileges: &[ObjectPrivilege],
    ) -> Vec<(ObjectPrivilege, bool, bool)> {
        let owner_authority = owner == Some(subject)
            || (graph.is_active_role(subject)
                && graph.role(subject).is_some_and(|role| role.superuser));
        privileges
            .iter()
            .copied()
            .map(|privilege| {
                let explicit = self.grants.get(&(subject, privilege));
                (
                    privilege,
                    owner_authority || explicit.is_some(),
                    owner_authority || explicit.is_some_and(|grant_option| *grant_option),
                )
            })
            .collect()
    }

    pub(super) fn public(
        &self,
        privileges: &[ObjectPrivilege],
    ) -> Vec<(ObjectPrivilege, bool, bool)> {
        privileges
            .iter()
            .copied()
            .map(|privilege| {
                let explicit = self.grants.get(&(0, privilege));
                (
                    privilege,
                    explicit.is_some(),
                    explicit.is_some_and(|grant_option| *grant_option),
                )
            })
            .collect()
    }
}

// The arguments are the complete immutable inspection context for one ACL
// object; keeping them explicit avoids hiding policy or PUBLIC semantics in a
// second object abstraction.
#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_acl_object(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    findings: &mut Vec<Finding>,
    object: &str,
    owner: i64,
    acl: &[AclEntry],
    allowed: &[AllowedPrivilege],
    allow_owner: bool,
    allow_superuser: bool,
    public_grants: &PublicPolicy,
    public_object: PublicObject,
    privileges: &[ObjectPrivilege],
) -> Result<(), VerificationError> {
    let mut ignored = Vec::new();
    inspect_acl_object_parts_collect(
        evaluation,
        graph,
        &mut ignored,
        findings,
        object,
        owner,
        &[acl],
        &[allowed],
        allow_owner,
        allow_superuser,
        public_grants,
        public_object,
        privileges,
    )
    .await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_acl_object_collect(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    collected: &mut Vec<(i64, ObjectPrivilege, bool, bool)>,
    findings: &mut Vec<Finding>,
    object: &str,
    owner: i64,
    acl: &[AclEntry],
    allowed: &[AllowedPrivilege],
    allow_owner: bool,
    allow_superuser: bool,
    public_grants: &PublicPolicy,
    public_object: PublicObject,
    privileges: &[ObjectPrivilege],
) -> Result<(), VerificationError> {
    inspect_acl_object_parts_collect(
        evaluation,
        graph,
        collected,
        findings,
        object,
        owner,
        &[acl],
        &[allowed],
        allow_owner,
        allow_superuser,
        public_grants,
        public_object,
        privileges,
    )
    .await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_acl_object_parts(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    findings: &mut Vec<Finding>,
    object: &str,
    owner: i64,
    acl: &[&[AclEntry]],
    allowed: &[&[AllowedPrivilege]],
    allow_owner: bool,
    allow_superuser: bool,
    public_grants: &PublicPolicy,
    public_object: PublicObject,
    privileges: &[ObjectPrivilege],
) -> Result<(), VerificationError> {
    let mut ignored = Vec::new();
    inspect_acl_object_parts_collect(
        evaluation,
        graph,
        &mut ignored,
        findings,
        object,
        owner,
        acl,
        allowed,
        allow_owner,
        allow_superuser,
        public_grants,
        public_object,
        privileges,
    )
    .await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn inspect_acl_object_parts_collect(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    collected: &mut Vec<(i64, ObjectPrivilege, bool, bool)>,
    findings: &mut Vec<Finding>,
    object: &str,
    owner: i64,
    acl: &[&[AclEntry]],
    allowed: &[&[AllowedPrivilege]],
    allow_owner: bool,
    allow_superuser: bool,
    public_grants: &PublicPolicy,
    public_object: PublicObject,
    privileges: &[ObjectPrivilege],
) -> Result<(), VerificationError> {
    let indexed_acl = IndexedAcl::new(acl);
    for role in graph.acl_sources(
        indexed_acl.grants.keys().map(|(grantee, _)| *grantee),
        Some(owner),
    ) {
        evaluation.checkpoint(findings).await?;
        let active_superuser = role.superuser && graph.is_active_role(role.oid);
        let owner_applies = role.oid == owner;
        let actual = indexed_acl.actual_for(role.oid, Some(owner), graph, privileges);
        collected.extend(actual.iter().filter(|(_, granted, _)| *granted).map(
            |(privilege, granted, grant_option)| (role.oid, *privilege, *granted, *grant_option),
        ));
        let authority_allowed =
            (active_superuser && allow_superuser) || (owner_applies && allow_owner);
        if owner_applies && !allow_owner && !(active_superuser && allow_superuser) {
            findings.push(Finding::new(
                FindingKind::Ownership,
                Some(object.to_owned()),
                Some(role.name.clone()),
                None,
            ));
        }
        for (privilege, granted, grant_option) in actual {
            evaluation.checkpoint(findings).await?;
            if granted && !authority_allowed && !allows_parts(allowed, privilege, false) {
                findings.push(Finding::new(
                    FindingKind::Privilege,
                    Some(object.to_owned()),
                    Some(role.name.clone()),
                    Some(privilege),
                ));
            }
            if grant_option && !authority_allowed && !allows_parts(allowed, privilege, true) {
                findings.push(Finding::new(
                    FindingKind::GrantOption,
                    Some(object.to_owned()),
                    Some(role.name.clone()),
                    Some(privilege),
                ));
            }
        }
    }
    for (privilege, granted, grant_option) in indexed_acl.public(privileges) {
        evaluation.checkpoint(findings).await?;
        if granted && !public_grants.allows(&public_object, privilege, false) {
            findings.push(Finding::new(
                FindingKind::Privilege,
                Some(object.to_owned()),
                Some("public".to_owned()),
                Some(privilege),
            ));
        }
        if grant_option && !public_grants.allows(&public_object, privilege, true) {
            findings.push(Finding::new(
                FindingKind::GrantOption,
                Some(object.to_owned()),
                Some("public".to_owned()),
                Some(privilege),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
pub(super) fn actual_for_capability_source(
    acl: &[AclEntry],
    subject: i64,
    owner: Option<i64>,
    graph: &RoleGraph<'_>,
    privileges: &[ObjectPrivilege],
) -> Vec<(ObjectPrivilege, bool, bool)> {
    IndexedAcl::new(&[acl]).actual_for(subject, owner, graph, privileges)
}

pub(super) fn public_privileges(
    acl: &[AclEntry],
    privileges: &[ObjectPrivilege],
) -> Vec<(ObjectPrivilege, bool, bool)> {
    IndexedAcl::new(&[acl]).public(privileges)
}

pub(super) fn allows(
    allowed: &[AllowedPrivilege],
    privilege: ObjectPrivilege,
    grant_option: bool,
) -> bool {
    allows_parts(&[allowed], privilege, grant_option)
}

fn allows_parts(
    allowed: &[&[AllowedPrivilege]],
    privilege: ObjectPrivilege,
    grant_option: bool,
) -> bool {
    allowed
        .iter()
        .flat_map(|entries| entries.iter())
        .any(|entry| entry.privilege == privilege && (!grant_option || entry.grant_option))
}

pub(super) fn routine_matches(
    policy: &RoutinePolicy,
    routine: &RoutineObject,
    types: &HashMap<i64, TypeName>,
) -> bool {
    routine_signature_matches(&policy.routine, routine, types)
}

pub(super) fn routine_signature_matches(
    signature: &RoutineSignature,
    routine: &RoutineObject,
    types: &HashMap<i64, TypeName>,
) -> bool {
    signature.schema() == routine.schema
        && signature.name() == routine.name
        && signature.arguments().len() == routine.argument_types.len()
        && signature
            .arguments()
            .iter()
            .zip(&routine.argument_types)
            .all(|(policy_type, oid)| routine_type_matches(policy_type, *oid, types))
}

fn routine_type_matches(
    policy_type: &RoutineType,
    oid: i64,
    types: &HashMap<i64, TypeName>,
) -> bool {
    let catalog_type = match types.get(&oid) {
        Some(catalog_type) => catalog_type,
        None => return false,
    };
    let compared = if policy_type.is_array() {
        let Some(element) = catalog_type.array_element else {
            return false;
        };
        match types.get(&element) {
            Some(element) => element,
            None => return false,
        }
    } else {
        if catalog_type.array_element.is_some() {
            return false;
        }
        catalog_type
    };
    compared.schema == policy_type.schema() && compared.name == policy_type.name()
}

pub(super) fn routine_signature_from_catalog(
    routine: &RoutineObject,
    types: &HashMap<i64, TypeName>,
) -> Result<RoutineSignature, PolicyError> {
    let mut arguments = Vec::with_capacity(routine.argument_types.len());
    for oid in &routine.argument_types {
        let catalog_type = types.get(oid).ok_or(PolicyError::InvalidRoutineType)?;
        arguments.push(if let Some(element) = catalog_type.array_element {
            let element = types.get(&element).ok_or(PolicyError::InvalidRoutineType)?;
            RoutineType::array(element.schema.clone(), element.name.clone())?
        } else {
            RoutineType::new(catalog_type.schema.clone(), catalog_type.name.clone())?
        });
    }
    RoutineSignature::new(routine.schema.clone(), routine.name.clone(), arguments)
}

pub(super) fn public_object_exists(
    snapshot: &CatalogSnapshot,
    parameters: &ParameterIndex<'_>,
    object: &PublicObject,
) -> bool {
    match object {
        PublicObject::Relation(name) => snapshot.relations.iter().any(|candidate| {
            candidate.kind != "S" && name.matches(&candidate.schema, &candidate.name)
        }),
        PublicObject::Column(name, column) => snapshot.relations.iter().any(|candidate| {
            candidate.kind != "S"
                && name.matches(&candidate.schema, &candidate.name)
                && candidate
                    .columns
                    .iter()
                    .any(|candidate| candidate.name == column.as_str())
        }),
        PublicObject::Sequence(name) => snapshot.relations.iter().any(|candidate| {
            candidate.kind == "S" && name.matches(&candidate.schema, &candidate.name)
        }),
        PublicObject::Schema(name) => snapshot
            .schemas
            .iter()
            .any(|candidate| candidate.name == name.as_str()),
        PublicObject::Routine(signature) => snapshot
            .routines
            .iter()
            .any(|candidate| routine_signature_matches(signature, candidate, &snapshot.type_names)),
        PublicObject::Type(name) => snapshot
            .types
            .iter()
            .any(|candidate| name.matches(&candidate.schema, &candidate.name)),
        PublicObject::Parameter(name) => parameters
            .get(name.as_str())
            .is_some_and(|candidate| candidate.exists),
        PublicObject::Database => true,
    }
}

pub(super) fn public_object_name(object: &PublicObject) -> String {
    match object {
        PublicObject::Relation(name) | PublicObject::Sequence(name) | PublicObject::Type(name) => {
            name.quoted()
        }
        PublicObject::Column(name, column) => format!("{}.{}", name.quoted(), column.quoted()),
        PublicObject::Schema(name) => name.quoted(),
        PublicObject::Routine(signature) => signature.as_str().to_owned(),
        PublicObject::Parameter(name) => name.as_str().to_owned(),
        PublicObject::Database => "current_database".to_owned(),
    }
}
