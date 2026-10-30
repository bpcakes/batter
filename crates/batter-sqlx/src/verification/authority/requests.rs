mod parameters;
pub(super) use parameters::inspect_parameters;

use crate::verification::authority::privileges::GrantAuthority;
use crate::verification::authority::privileges::OwnerAllowance;
use crate::verification::authority::privileges::SuperuserAllowance;

use super::super::policy::{
    AllowedPrivilege, AuthorityPolicy, ColumnPolicy, ObjectPrivilege, PublicObject, QualifiedName,
    RelationPolicy, SchemaPolicy, SequencePolicy,
};
use super::super::report::{Finding, FindingKind, RoleAttribute, VerificationError};
use super::evaluation::Evaluation;
use super::privileges::*;
use super::{
    CatalogSnapshot, PREDEFINED_ROLES, ParameterIndex, ParameterObject, RelationObject, RoleGraph,
    RoleInfo,
};
use std::collections::{HashMap, HashSet};

pub(super) async fn inspect_roles(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    for role in graph.active_roles() {
        evaluation.checkpoint(findings).await?;
        if role.superuser {
            if !policy.roles.allow_superuser {
                push_role_attribute(findings, role, RoleAttribute::Superuser);
            } else {
                // PostgreSQL superuser status subsumes the other role
                // attributes. Once explicitly allowed, do not report the
                // same reachable superuser again as CREATEDB/CREATEROLE/etc.
                continue;
            }
        }
        if role.createdb && !policy.roles.allow_create_database {
            push_role_attribute(findings, role, RoleAttribute::CreateDatabase);
        }
        if role.createrole && !policy.roles.allow_create_role {
            push_role_attribute(findings, role, RoleAttribute::CreateRole);
        }
        if role.replication && !policy.roles.allow_replication {
            push_role_attribute(findings, role, RoleAttribute::Replication);
        }
        if role.bypassrls && !policy.roles.allow_bypass_rls {
            push_role_attribute(findings, role, RoleAttribute::BypassRowLevelSecurity);
        }
    }
    for role in graph.predefined_roles() {
        evaluation.checkpoint(findings).await?;
        if PREDEFINED_ROLES.contains(&role.name.as_str())
            && !policy
                .roles
                .allowed_predefined_roles
                .iter()
                .any(|allowed| allowed.as_str() == role.name)
        {
            push_role_attribute(findings, role, RoleAttribute::PredefinedRole);
        }
    }

    Ok(())
}

fn push_role_attribute(findings: &mut Vec<Finding>, role: &RoleInfo, attribute: RoleAttribute) {
    findings.push(Finding::role_attribute(role.name.clone(), attribute));
}

pub(super) async fn inspect_admin_options(
    evaluation: &mut Evaluation,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    // Superuser authority is already an explicit role-attribute decision and
    // subsumes role administration. For other identities, retain each ADMIN
    // target so one legitimate administrative role cannot conceal another.
    if graph.has_active_superuser() {
        return Ok(());
    }
    for role in graph.admin_target_roles() {
        evaluation.checkpoint(findings).await?;
        if !policy
            .roles
            .allowed_admin_roles
            .iter()
            .any(|allowed| allowed.as_str() == role.name)
        {
            findings.push(Finding::new(
                FindingKind::RoleAdminOption,
                Some(role.name.clone()),
                None::<String>,
                None,
            ));
        }
    }

    Ok(())
}

pub(super) async fn inspect_requested_objects(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    parameters: &ParameterIndex<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let mut missing = HashSet::new();
    let relations: HashMap<(&str, &str), &RelationObject> = snapshot
        .relations
        .iter()
        .map(|object| ((object.schema.as_str(), object.name.as_str()), object))
        .collect();
    for relation in &policy.relations {
        evaluation.checkpoint(findings).await?;
        let object = relations
            .get(&(relation.relation.schema(), relation.relation.name()))
            .copied()
            .filter(|object| object.kind != "S");
        let Some(object) = object else {
            push_missing(findings, &mut missing, relation.relation.quoted());
            continue;
        };
        for column in &relation.columns {
            evaluation.checkpoint(findings).await?;
            if !object
                .columns
                .iter()
                .any(|candidate| candidate.name == column.column.as_str())
            {
                push_missing(
                    findings,
                    &mut missing,
                    public_object_name(&PublicObject::Column(
                        relation.relation.clone(),
                        column.column.clone(),
                    )),
                );
            }
        }
    }
    for sequence in &policy.sequences {
        evaluation.checkpoint(findings).await?;
        let found = relations
            .get(&(sequence.sequence.schema(), sequence.sequence.name()))
            .is_some_and(|object| object.kind == "S");
        if !found {
            push_missing(findings, &mut missing, sequence.sequence.quoted());
        }
    }
    inspect_other_requested(
        evaluation,
        snapshot,
        parameters,
        policy,
        findings,
        &mut missing,
    )
    .await
}

async fn inspect_other_requested(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    parameters: &ParameterIndex<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
    missing: &mut HashSet<String>,
) -> Result<(), VerificationError> {
    for schema in &policy.schemas {
        evaluation.checkpoint(findings).await?;
        if !snapshot
            .schemas
            .iter()
            .any(|object| object.name == schema.schema.as_str())
        {
            push_missing(findings, missing, schema.schema.quoted());
        }
    }
    for routine in &policy.routines {
        evaluation.checkpoint(findings).await?;
        if !snapshot
            .routines
            .iter()
            .any(|object| routine_matches(routine, object, &snapshot.type_names))
        {
            push_missing(findings, missing, routine.routine.as_str().to_owned());
        }
    }
    for type_policy in &policy.types {
        evaluation.checkpoint(findings).await?;
        if !snapshot
            .types
            .iter()
            .any(|object| type_policy.type_name.matches(&object.schema, &object.name))
        {
            push_missing(findings, missing, type_policy.type_name.quoted());
        }
    }
    if let super::super::policy::DiscoveryScope::Schemas(names) = &policy.discovery {
        for name in names {
            evaluation.checkpoint(findings).await?;
            if !snapshot
                .schemas
                .iter()
                .any(|schema| schema.name == name.as_str())
            {
                push_missing(findings, missing, name.quoted());
            }
        }
    }
    for object in policy
        .public_grants
        .iter()
        .map(|entry| &entry.object)
        .chain(policy.public_overrides.iter().map(|entry| &entry.object))
    {
        evaluation.checkpoint(findings).await?;
        if matches!(object, PublicObject::Parameter(name) if parameters.get(name.as_str()).is_some_and(|parameter| !parameter.observable))
        {
            continue;
        }
        if !public_object_exists(snapshot, parameters, object) {
            push_missing(findings, missing, public_object_name(object));
        }
    }
    Ok(())
}

fn push_missing(findings: &mut Vec<Finding>, missing: &mut HashSet<String>, object: String) {
    if missing.insert(object.clone()) {
        findings.push(Finding::new(
            FindingKind::MissingObject,
            Some(object),
            None::<String>,
            None,
        ));
    }
}

pub(super) async fn inspect_database(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let public = PublicPolicy::new(policy);
    inspect_acl_object(
        evaluation,
        graph,
        findings,
        &snapshot.database.name,
        snapshot.database.owner,
        &snapshot.database.acl,
        &policy.database.privileges,
        if policy.database.allow_owner {
            OwnerAllowance::Allow
        } else {
            OwnerAllowance::Deny
        },
        if policy.roles.allow_superuser {
            SuperuserAllowance::Allow
        } else {
            SuperuserAllowance::Deny
        },
        &public,
        PublicObject::Database,
        &[
            ObjectPrivilege::Create,
            ObjectPrivilege::Connect,
            ObjectPrivilege::Temporary,
        ],
    )
    .await?;
    Ok(())
}

pub(super) async fn inspect_schemas(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let public = PublicPolicy::new(policy);
    let schemas: HashMap<&str, &SchemaPolicy> = policy
        .schemas
        .iter()
        .map(|entry| (entry.schema.as_str(), entry))
        .collect();
    for schema in &snapshot.schemas {
        evaluation.checkpoint(findings).await?;
        let configured = schemas.get(schema.name.as_str()).copied();
        let public_object = PublicObject::Schema(
            super::super::policy::Identifier::new(schema.name.clone())
                .map_err(VerificationError::CatalogIdentity)?,
        );
        let public_selected = public.contains(&public_object);
        if configured.is_none() && !public_selected {
            continue;
        }
        inspect_acl_object(
            evaluation,
            graph,
            findings,
            &public_object_name(&public_object),
            schema.owner,
            &schema.acl,
            configured.map_or(&[], |entry| entry.privileges.as_slice()),
            if configured.is_some_and(|entry| entry.allow_owner) {
                OwnerAllowance::Allow
            } else {
                OwnerAllowance::Deny
            },
            if policy.roles.allow_superuser {
                SuperuserAllowance::Allow
            } else {
                SuperuserAllowance::Deny
            },
            &public,
            public_object,
            &[ObjectPrivilege::Usage, ObjectPrivilege::Create],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn inspect_relations(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let public = PublicPolicy::new(policy);
    let relation_policies: HashMap<(&str, &str), &RelationPolicy> = policy
        .relations
        .iter()
        .map(|entry| ((entry.relation.schema(), entry.relation.name()), entry))
        .collect();
    let sequence_policies: HashMap<(&str, &str), &SequencePolicy> = policy
        .sequences
        .iter()
        .map(|entry| ((entry.sequence.schema(), entry.sequence.name()), entry))
        .collect();
    for relation in &snapshot.relations {
        evaluation.checkpoint(findings).await?;
        let qualified = QualifiedName::new(relation.schema.clone(), relation.name.clone())
            .map_err(VerificationError::CatalogIdentity)?;
        let key = (relation.schema.as_str(), relation.name.as_str());
        let relation_policy = relation_policies.get(&key).copied();
        let sequence_policy = sequence_policies.get(&key).copied();
        let public_selected = public.selects_relation(&qualified);
        if relation_policy.is_none() && sequence_policy.is_none() && !public_selected {
            continue;
        }
        let (public_object, allowed, allow_owner, privileges) = if relation.kind == "S" {
            (
                PublicObject::Sequence(qualified.clone()),
                sequence_policy.map_or(&[][..], |entry| entry.privileges.as_slice()),
                sequence_policy.is_some_and(|entry| entry.allow_owner),
                &[
                    ObjectPrivilege::Usage,
                    ObjectPrivilege::Select,
                    ObjectPrivilege::Update,
                ][..],
            )
        } else {
            (
                PublicObject::Relation(qualified.clone()),
                relation_policy.map_or(&[][..], |entry| entry.privileges.as_slice()),
                relation_policy.is_some_and(|entry| entry.allow_owner),
                &[
                    ObjectPrivilege::Select,
                    ObjectPrivilege::Insert,
                    ObjectPrivilege::Update,
                    ObjectPrivilege::Delete,
                    ObjectPrivilege::Truncate,
                    ObjectPrivilege::References,
                    ObjectPrivilege::Trigger,
                    ObjectPrivilege::Maintain,
                ][..],
            )
        };
        let object = public_object_name(&public_object);
        inspect_acl_object(
            evaluation,
            graph,
            findings,
            &object,
            relation.owner,
            &relation.acl,
            allowed,
            if allow_owner {
                OwnerAllowance::Allow
            } else {
                OwnerAllowance::Deny
            },
            if policy.roles.allow_superuser {
                SuperuserAllowance::Allow
            } else {
                SuperuserAllowance::Deny
            },
            &public,
            public_object,
            privileges,
        )
        .await?;
        if relation.kind == "S" {
            continue;
        }
        inspect_relation_columns(
            evaluation,
            relation,
            relation_policy,
            &qualified,
            graph,
            policy,
            &public,
            findings,
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn inspect_relation_columns(
    evaluation: &mut Evaluation,
    relation: &RelationObject,
    relation_policy: Option<&super::super::policy::RelationPolicy>,
    qualified: &QualifiedName,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    public: &PublicPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let column_policies: HashMap<&str, &ColumnPolicy> = relation_policy
        .into_iter()
        .flat_map(|entry| {
            entry
                .columns
                .iter()
                .map(|column| (column.column.as_str(), column))
        })
        .collect();
    for column in &relation.columns {
        evaluation.checkpoint(findings).await?;
        let column_object = PublicObject::Column(
            qualified.clone(),
            super::super::policy::Identifier::new(column.name.clone())
                .map_err(VerificationError::CatalogIdentity)?,
        );
        let column_name = public_object_name(&column_object);
        let column_policy = column_policies.get(column.name.as_str()).copied();
        let relation_allowed = relation_policy.map_or(&[][..], |entry| entry.privileges.as_slice());
        let column_allowed = column_policy.map_or(&[][..], |entry| entry.privileges.as_slice());
        inspect_acl_object_parts(
            evaluation,
            graph,
            findings,
            &column_name,
            None,
            relation_policy
                .is_some_and(|entry| entry.allow_owner)
                .then_some(relation.owner),
            &[&column.acl],
            &[relation_allowed, column_allowed],
            OwnerAllowance::Deny,
            if policy.roles.allow_superuser {
                SuperuserAllowance::Allow
            } else {
                SuperuserAllowance::Deny
            },
            public,
            column_object,
            &[
                ObjectPrivilege::Select,
                ObjectPrivilege::Insert,
                ObjectPrivilege::Update,
                ObjectPrivilege::References,
            ],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn inspect_types(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let mut public = PublicPolicy::new(policy);
    let configured_types: HashMap<_, _> = policy
        .types
        .iter()
        .map(|entry| ((entry.type_name.schema(), entry.type_name.name()), entry))
        .collect();
    let overrides: HashSet<_> = policy
        .public_overrides
        .iter()
        .map(|entry| &entry.object)
        .collect();
    let relation_policies: HashMap<(&str, &str), &RelationPolicy> = policy
        .relations
        .iter()
        .map(|entry| ((entry.relation.schema(), entry.relation.name()), entry))
        .collect();
    let relation_policies_by_oid: HashMap<i64, &RelationPolicy> = snapshot
        .relations
        .iter()
        .filter_map(|relation| {
            relation_policies
                .get(&(relation.schema.as_str(), relation.name.as_str()))
                .map(|entry| (relation.oid, *entry))
        })
        .collect();

    for type_object in &snapshot.types {
        evaluation.checkpoint(findings).await?;
        let relation_policy = relation_policies_by_oid
            .get(&type_object.relation_oid)
            .copied();
        let object = PublicObject::Type(
            QualifiedName::new(type_object.schema.clone(), type_object.name.clone())
                .map_err(VerificationError::CatalogIdentity)?,
        );
        if relation_policy.is_some_and(|entry| entry.allow_row_type_public_usage)
            && !overrides.contains(&object)
        {
            public.add(object, AllowedPrivilege::privilege(ObjectPrivilege::Usage));
        }
    }
    for type_object in &snapshot.types {
        evaluation.checkpoint(findings).await?;
        let configured = configured_types
            .get(&(type_object.schema.as_str(), type_object.name.as_str()))
            .copied();
        let relation_policy = relation_policies_by_oid
            .get(&type_object.relation_oid)
            .copied();
        let public_object = PublicObject::Type(
            QualifiedName::new(type_object.schema.clone(), type_object.name.clone())
                .map_err(VerificationError::CatalogIdentity)?,
        );
        let public_selected = public.contains(&public_object);
        if configured.is_none() && relation_policy.is_none() && !public_selected {
            continue;
        }
        let allowed = configured.map_or(&[][..], |entry| entry.privileges.as_slice());
        let allow_owner = configured.is_some_and(|entry| entry.allow_owner)
            || relation_policy.is_some_and(|entry| entry.allow_owner);
        let relation_backed = relation_policy.is_some();
        inspect_acl_object_parts(
            evaluation,
            graph,
            findings,
            &public_object_name(&public_object),
            (!relation_backed).then_some(type_object.owner),
            (relation_backed && allow_owner).then_some(type_object.owner),
            &[&type_object.acl],
            &[allowed],
            if allow_owner {
                OwnerAllowance::Allow
            } else {
                OwnerAllowance::Deny
            },
            if policy.roles.allow_superuser {
                SuperuserAllowance::Allow
            } else {
                SuperuserAllowance::Deny
            },
            &public,
            public_object,
            &[ObjectPrivilege::Usage],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn inspect_routines(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    graph: &RoleGraph<'_>,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let public = PublicPolicy::new(policy);
    let configured_routines: HashMap<_, _> = policy
        .routines
        .iter()
        .map(|entry| (&entry.routine, entry))
        .collect();
    for routine in &snapshot.routines {
        evaluation.checkpoint(findings).await?;
        let signature = routine_signature_from_catalog(routine, &snapshot.type_names)
            .map_err(VerificationError::CatalogIdentity)?;
        let configured = configured_routines.get(&signature).copied();
        let public_object = PublicObject::Routine(signature);
        let public_selected = public.contains(&public_object);
        if configured.is_none() && !public_selected {
            continue;
        }
        let object = public_object_name(&public_object);
        let mut actual = Vec::new();
        inspect_acl_object_collect(
            evaluation,
            graph,
            &mut actual,
            findings,
            &object,
            routine.owner,
            &routine.acl,
            configured.map_or(&[][..], |entry| entry.privileges.as_slice()),
            if configured.is_some_and(|entry| entry.allow_owner) {
                OwnerAllowance::Allow
            } else {
                OwnerAllowance::Deny
            },
            if policy.roles.allow_superuser {
                SuperuserAllowance::Allow
            } else {
                SuperuserAllowance::Deny
            },
            &public,
            public_object,
            &[ObjectPrivilege::Execute],
        )
        .await?;
        let executable = actual
            .iter()
            .any(|(_, privilege, granted, _)| *privilege == ObjectPrivilege::Execute && *granted);
        let public_executable = public_privileges(&routine.acl, &[ObjectPrivilege::Execute])
            .into_iter()
            .any(|(privilege, granted, _)| privilege == ObjectPrivilege::Execute && granted);
        if routine.security_definer
            && (executable || public_executable)
            && !configured.is_some_and(|entry| entry.allow_security_definer)
        {
            findings.push(Finding::new(
                FindingKind::Privilege,
                Some(object),
                None::<String>,
                Some(ObjectPrivilege::Execute),
            ));
        }
    }
    Ok(())
}
