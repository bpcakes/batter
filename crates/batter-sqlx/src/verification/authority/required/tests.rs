use super::super::database::ParameterCatalog;
use super::super::{DatabaseObject, Membership, RelationObject, RoleInfo};
use super::*;
use crate::verification::policy::QualifiedName;
use std::collections::HashMap;

pub(crate) fn snapshot() -> CatalogSnapshot {
    let roles = [
        (1, "login"),
        (2, "current"),
        (3, "inherited"),
        (4, "switchable"),
        (5, "administered"),
    ]
    .into_iter()
    .map(|(oid, name)| RoleInfo {
        oid,
        name: name.to_owned(),
        superuser: false,
        createdb: false,
        createrole: false,
        replication: false,
        bypassrls: false,
    })
    .collect();
    CatalogSnapshot {
        session_user: "login".to_owned(),
        current_user: "current".to_owned(),
        roles,
        memberships: vec![
            Membership {
                role: 3,
                member: 2,
                inherit: true,
                set: false,
                admin: false,
            },
            Membership {
                role: 4,
                member: 2,
                inherit: false,
                set: true,
                admin: false,
            },
            Membership {
                role: 5,
                member: 2,
                inherit: false,
                set: false,
                admin: true,
            },
        ],
        root: Some(1),
        schemas: Vec::new(),
        relations: vec![RelationObject {
            oid: 20,
            schema: "service".to_owned(),
            name: "records".to_owned(),
            kind: "r".to_owned(),
            owner: 3,
            acl: Vec::new(),
            columns: Vec::new(),
        }],
        types: Vec::new(),
        type_names: HashMap::new(),
        routines: Vec::new(),
        database: DatabaseObject {
            name: "database".to_owned(),
            owner: 99,
            acl: Vec::new(),
        },
        parameters: ParameterCatalog::default(),
    }
}

#[test]
fn required_privilege_uses_effective_current_role_inheritance_without_set_or_admin() {
    let mut snapshot = snapshot();
    let required = RequiredPrivilege {
        object: PublicObject::Relation(QualifiedName::new("service", "records").unwrap()),
        privilege: ObjectPrivilege::Select,
    };
    let inherited = inherited_roles(&snapshot, 2);
    let parameters = super::super::index_parameters(&snapshot.parameters);
    assert_eq!(inherited, HashSet::from([2, 3]));
    for grantee in [0, 2, 3, 1, 4, 5] {
        snapshot.relations[0].acl = vec![AclEntry {
            grantee,
            privilege: ObjectPrivilege::Select,
            grant_option: false,
        }];
        assert_eq!(
            has_privilege(&snapshot, &parameters, &inherited, false, &required),
            [0, 2, 3].contains(&grantee),
            "grantee {grantee}"
        );
    }
    snapshot.relations[0].acl.clear();
    assert!(
        !has_privilege(&snapshot, &parameters, &inherited, false, &required),
        "ownership must not restore a revoked ordinary privilege"
    );
    let policy = AuthorityPolicy {
        required_privileges: vec![required],
        ..AuthorityPolicy::default()
    };
    let mut findings = Vec::new();
    crate::verification::authority::evaluation::tests::run(inspect(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        &snapshot,
        &parameters,
        &policy,
        &mut findings,
    ))
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, FindingKind::MissingPrivilege);
    assert_eq!(findings[0].subject.as_deref(), Some("current"));
}

#[test]
fn unknown_custom_context_cannot_prove_required_authority_even_with_an_acl() {
    use super::super::{ParameterObject, RoleGraph, requests};
    use crate::verification::policy::{AllowedPrivilege, ParameterName, ParameterPolicy};
    use crate::verification::report::{VerificationReport, VerificationStatus};
    let mut snapshot = snapshot();
    let parameter = ParameterName::new("extension.hidden_setting").unwrap();
    let policy = AuthorityPolicy {
        parameters: vec![ParameterPolicy {
            parameter: parameter.clone(),
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Set, false)],
        }],
        required_privileges: vec![RequiredPrivilege {
            object: PublicObject::Parameter(parameter),
            privilege: ObjectPrivilege::Set,
        }],
        ..AuthorityPolicy::default()
    };
    for explicit_acl in [false, true] {
        snapshot.parameters = ParameterCatalog::try_from_objects(vec![ParameterObject {
            name: "extension.hidden_setting".to_owned(),
            context: None,
            observable: true,
            exists: true,
            custom: true,
            acl: if explicit_acl {
                vec![AclEntry {
                    grantee: 2,
                    privilege: ObjectPrivilege::Set,
                    grant_option: false,
                }]
            } else {
                Vec::new()
            },
        }])
        .unwrap();
        let graph = RoleGraph::new(
            &snapshot.roles,
            &snapshot.memberships,
            snapshot.root,
            snapshot.database.owner,
        );
        let parameters = super::super::index_parameters(&snapshot.parameters);
        let mut findings = Vec::new();
        crate::verification::authority::evaluation::tests::run(requests::inspect_parameters(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &parameters,
            &graph,
            &policy,
            false,
            &mut findings,
        ))
        .unwrap();
        crate::verification::authority::evaluation::tests::run(inspect(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &parameters,
            &policy,
            &mut findings,
        ))
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::ParameterUnobservable);
        assert_eq!(
            findings[0].object.as_deref(),
            Some("extension.hidden_setting")
        );
        let report = VerificationReport::new(
            findings,
            Vec::new(),
            Vec::new(),
            snapshot.session_user.clone(),
            snapshot.current_user.clone(),
            &[],
        );
        assert_eq!(report.status(), VerificationStatus::Incomplete);
    }
}
