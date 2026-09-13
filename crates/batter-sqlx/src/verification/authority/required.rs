use super::super::policy::{AuthorityPolicy, ObjectPrivilege, PublicObject, RequiredPrivilege};
use super::super::report::{Finding, FindingKind, VerificationError};
use super::evaluation::Evaluation;
use super::privileges::{public_object_name, routine_signature_matches};
use super::{AclEntry, CatalogSnapshot};
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) async fn inspect(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    policy: &AuthorityPolicy,
    findings: &mut Vec<Finding>,
) -> Result<(), VerificationError> {
    let current = snapshot
        .roles
        .iter()
        .find(|role| role.name == snapshot.current_user)
        .ok_or(VerificationError::MissingSessionUser)?;
    let inherited = inherited_roles(snapshot, current.oid);
    for required in &policy.required_privileges {
        evaluation.checkpoint(findings).await?;
        if matches!(&required.object, PublicObject::Parameter(name) if snapshot.parameters.iter().any(|parameter| parameter.name == name.as_str() && (!parameter.observable || parameter.custom)))
        {
            continue;
        }
        if !has_privilege(snapshot, &inherited, current.superuser, required) {
            findings.push(Finding::new(
                FindingKind::MissingPrivilege,
                Some(public_object_name(&required.object)),
                Some(snapshot.current_user.clone()),
                Some(required.privilege),
            ));
        }
    }
    Ok(())
}

fn inherited_roles(snapshot: &CatalogSnapshot, current: i64) -> HashSet<i64> {
    let mut by_member: HashMap<i64, Vec<i64>> = HashMap::new();
    for membership in &snapshot.memberships {
        if membership.inherit {
            by_member
                .entry(membership.member)
                .or_default()
                .push(membership.role);
        }
    }
    let mut inherited = HashSet::new();
    let mut queue = VecDeque::from([current]);
    while let Some(member) = queue.pop_front() {
        if !inherited.insert(member) {
            continue;
        }
        queue.extend(by_member.get(&member).into_iter().flatten().copied());
        if member == snapshot.database.owner
            && let Some(role) = snapshot
                .roles
                .iter()
                .find(|role| role.name == "pg_database_owner")
        {
            queue.push_back(role.oid);
        }
    }
    inherited
}

fn acl_grants(acl: &[AclEntry], inherited: &HashSet<i64>, privilege: ObjectPrivilege) -> bool {
    acl.iter().any(|entry| {
        entry.privilege == privilege && (entry.grantee == 0 || inherited.contains(&entry.grantee))
    })
}

fn has_privilege(
    snapshot: &CatalogSnapshot,
    inherited: &HashSet<i64>,
    superuser: bool,
    required: &RequiredPrivilege,
) -> bool {
    let privilege = required.privilege;
    let granted = |acl: &[AclEntry]| superuser || acl_grants(acl, inherited, privilege);
    let predefined = |name: &str| {
        snapshot
            .roles
            .iter()
            .any(|role| role.name == name && inherited.contains(&role.oid))
    };
    let data_access = match privilege {
        ObjectPrivilege::Select => predefined("pg_read_all_data"),
        ObjectPrivilege::Insert | ObjectPrivilege::Update | ObjectPrivilege::Delete => {
            predefined("pg_write_all_data")
        }
        ObjectPrivilege::Maintain => predefined("pg_maintain"),
        _ => false,
    };
    match &required.object {
        PublicObject::Relation(name) => snapshot.relations.iter().any(|object| {
            object.kind != "S"
                && name.matches(&object.schema, &object.name)
                && (granted(&object.acl) || data_access)
        }),
        PublicObject::Column(name, column) => snapshot.relations.iter().any(|object| {
            object.kind != "S"
                && name.matches(&object.schema, &object.name)
                && object.columns.iter().any(|entry| {
                    entry.name == column.as_str()
                        && (granted(&entry.acl) || granted(&object.acl) || data_access)
                })
        }),
        PublicObject::Sequence(name) => snapshot.relations.iter().any(|object| {
            object.kind == "S"
                && name.matches(&object.schema, &object.name)
                && (granted(&object.acl) || data_access)
        }),
        PublicObject::Schema(name) => snapshot.schemas.iter().any(|object| {
            object.name == name.as_str()
                && (granted(&object.acl)
                    || (privilege == ObjectPrivilege::Usage
                        && (predefined("pg_read_all_data") || predefined("pg_write_all_data"))))
        }),
        PublicObject::Routine(name) => snapshot.routines.iter().any(|object| {
            routine_signature_matches(name, object, &snapshot.type_names) && granted(&object.acl)
        }),
        PublicObject::Type(name) => snapshot
            .types
            .iter()
            .any(|object| name.matches(&object.schema, &object.name) && granted(&object.acl)),
        PublicObject::Database => granted(&snapshot.database.acl),
        PublicObject::Parameter(name) => snapshot.parameters.iter().any(|object| {
            object.name == name.as_str() && object.exists && {
                // This is parameter ACL/context authority, not proof that a value
                // can be changed in an already-running session or accepted by a hook.
                let default_set = privilege == ObjectPrivilege::Set
                    && matches!(object.context.as_deref(), Some("user" | "backend"));
                granted(&object.acl) || default_set
            }
        }),
    }
}

#[cfg(test)]
pub(super) mod tests;
