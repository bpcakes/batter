use super::privileges::{
    actual_for_capability_source, inspect_acl_object_collect, routine_matches,
};
use super::requests::inspect_roles;
use super::{
    AclEntry, Membership, RoleGraph, RoleInfo, RoutineObject, TypeName, supports_postgres_18,
};
use crate::verification::policy::{
    AllowedPrivilege, AuthorityPolicy, ObjectPrivilege, PublicObject, RolePolicy, RoutinePolicy,
    RoutineSignature, RoutineType,
};
use crate::verification::{FindingKind, RoleAttribute};
use std::collections::HashMap;

fn role(oid: i64, name: &str) -> RoleInfo {
    RoleInfo {
        oid,
        name: name.to_owned(),
        superuser: false,
        createdb: false,
        createrole: false,
        replication: false,
        bypassrls: false,
    }
}

#[test]
fn selected_server_version_boundary_is_explicit() {
    assert!(!supports_postgres_18(179_999));
    assert!(supports_postgres_18(180_000));
    assert!(supports_postgres_18(180_006));
    assert!(!supports_postgres_18(190_000));
}

#[test]
fn superusers_are_the_single_authority_representative_and_have_admin_reachability() {
    let mut root = role(1, "root");
    root.superuser = true;
    let target = role(2, "target");
    let roles = vec![root, target];
    let graph = RoleGraph::new(&roles, &[], Some(1), 1);
    assert_eq!(
        graph
            .active_roles()
            .iter()
            .map(|role| role.oid)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(graph.can_admin_any_role());
}

#[test]
fn database_owner_implicitly_inherits_predefined_role() {
    let root = role(1, "owner");
    let predefined = role(2, "pg_database_owner");
    let roles = vec![root, predefined];
    let graph = RoleGraph::new(&roles, &[], Some(1), 1);
    assert!(graph.capability_roles().iter().any(|role| role.oid == 2));
}

#[test]
fn inherited_admin_option_is_reachable_without_set_role() {
    let root = role(1, "root");
    let group = role(2, "group");
    let target = role(3, "target");
    let memberships = vec![
        Membership {
            role: 2,
            member: 1,
            admin: false,
            inherit: true,
            set: false,
        },
        Membership {
            role: 3,
            member: 2,
            admin: true,
            inherit: false,
            set: false,
        },
    ];
    let roles = vec![root, group, target];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);
    assert!(graph.can_admin_any_role());
    assert_eq!(
        graph
            .admin_target_roles()
            .iter()
            .map(|role| role.name.as_str())
            .collect::<Vec<_>>(),
        vec!["target"]
    );
    assert!(graph.capability_roles().iter().any(|role| role.oid == 3));
}

#[test]
fn admin_option_does_not_cross_an_unusable_intermediate_membership() {
    let roles = vec![role(1, "root"), role(2, "group"), role(3, "target")];
    let memberships = vec![
        Membership {
            role: 2,
            member: 1,
            admin: false,
            inherit: false,
            set: false,
        },
        Membership {
            role: 3,
            member: 2,
            admin: true,
            inherit: false,
            set: false,
        },
    ];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);
    assert!(!graph.can_admin_any_role());
}

#[test]
fn active_createrole_reaches_admin_through_all_membership_edges() {
    let mut root = role(1, "root");
    root.createrole = true;
    let roles = vec![root, role(2, "group"), role(3, "target")];
    let memberships = vec![
        Membership {
            role: 2,
            member: 1,
            admin: false,
            inherit: false,
            set: false,
        },
        Membership {
            role: 3,
            member: 2,
            admin: true,
            inherit: false,
            set: false,
        },
    ];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);
    assert!(graph.can_admin_any_role());
}

#[test]
fn membership_edge_inherit_controls_nested_subjects_not_role_attribute() {
    let root = role(1, "root");
    // PostgreSQL's role-level INHERIT flag is not the membership edge
    // policy used by the PG18 role graph. The edge remains effective.
    let inherited_login = role(2, "login");
    let nested = role(3, "nested");
    let predefined = role(4, "pg_read_all_data");
    let roles = vec![root, inherited_login, nested, predefined];
    let memberships = vec![
        Membership {
            role: 3,
            member: 2,
            admin: false,
            inherit: true,
            set: false,
        },
        Membership {
            role: 4,
            member: 3,
            admin: false,
            inherit: true,
            set: false,
        },
    ];
    // The test helper cannot currently model rolinherit (it is deliberately
    // not part of RoleInfo), so the important oracle is the nested edge
    // closure: an active login reaches both inherited roles.
    let graph = RoleGraph::new(&roles, &memberships, Some(2), 99);
    let capabilities = graph
        .capability_roles()
        .iter()
        .map(|role| role.oid)
        .collect::<Vec<_>>();
    assert!(capabilities.contains(&3));
    assert!(capabilities.contains(&4));
    assert!(graph.predefined_roles().any(|role| role.oid == 4));
}

#[test]
fn admin_option_is_separate_from_unrelated_createrole() {
    let mut unrelated_creator = role(3, "unrelated_creator");
    unrelated_creator.createrole = true;
    let roles = vec![role(1, "root"), role(2, "target"), unrelated_creator];
    let graph = RoleGraph::new(&roles, &[], Some(1), 99);
    assert!(!graph.can_admin_any_role());

    let memberships = [Membership {
        role: 2,
        member: 1,
        admin: true,
        inherit: false,
        set: false,
    }];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);
    assert!(graph.can_admin_any_role());
}

#[test]
fn admin_option_on_superuser_target_is_not_usable_authority() {
    let root = role(1, "root");
    let mut target = role(2, "superuser_target");
    target.superuser = true;
    let roles = vec![root, target];
    let memberships = [Membership {
        role: 2,
        member: 1,
        admin: true,
        inherit: false,
        set: false,
    }];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);

    assert!(!graph.can_admin_any_role());
    assert!(graph.admin_target_roles().is_empty());
    assert!(!graph.has_active_superuser());
    assert_eq!(
        graph
            .active_roles()
            .iter()
            .map(|role| role.name.as_str())
            .collect::<Vec<_>>(),
        vec!["root"]
    );
}

#[test]
fn set_reachable_superuser_target_remains_active_authority() {
    let root = role(1, "root");
    let mut target = role(2, "superuser_target");
    target.superuser = true;
    let roles = vec![root, target];
    let memberships = [Membership {
        role: 2,
        member: 1,
        admin: false,
        inherit: false,
        set: true,
    }];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);

    assert!(graph.has_active_superuser());
    assert_eq!(
        graph
            .active_roles()
            .iter()
            .map(|role| role.name.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "superuser_target"]
    );
    let mut findings = Vec::new();
    crate::verification::authority::evaluation::tests::run(inspect_roles(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        &graph,
        &AuthorityPolicy::default(),
        &mut findings,
    ))
    .unwrap();
    assert!(findings.iter().any(|finding| {
        finding.kind == FindingKind::RoleAttribute
            && finding.object.as_deref() == Some("superuser_target")
            && finding.role_attribute == Some(RoleAttribute::Superuser)
    }));
}

#[test]
fn large_reachable_role_chain_uses_shared_closures() {
    const ROLE_COUNT: i64 = 2_048;
    let roles = (0..ROLE_COUNT)
        .map(|oid| role(oid, &format!("role_{oid}")))
        .collect::<Vec<_>>();
    let memberships = (0..ROLE_COUNT - 1)
        .map(|member| Membership {
            role: member + 1,
            member,
            admin: member == ROLE_COUNT - 2,
            inherit: true,
            set: true,
        })
        .collect::<Vec<_>>();
    let graph = RoleGraph::new(&roles, &memberships, Some(0), ROLE_COUNT + 1);

    assert_eq!(graph.active_roles().len(), ROLE_COUNT as usize);
    assert_eq!(graph.capability_roles().len(), ROLE_COUNT as usize);
    assert!(graph.can_admin_any_role());
}

#[test]
fn allowed_superuser_does_not_duplicate_superuser_attributes() {
    let mut root = role(1, "root");
    root.superuser = true;
    root.createdb = true;
    root.createrole = true;
    let roles = vec![root];
    let graph = RoleGraph::new(&roles, &[], Some(1), 1);
    let mut findings = Vec::new();
    crate::verification::authority::evaluation::tests::run(inspect_roles(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        &graph,
        &AuthorityPolicy {
            roles: RolePolicy {
                allow_superuser: true,
                allow_create_database: false,
                allow_create_role: false,
                ..RolePolicy::default()
            },
            ..AuthorityPolicy::default()
        },
        &mut findings,
    ))
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn allowed_owner_authority_remains_available_to_follow_on_checks() {
    let roles = vec![role(1, "root")];
    let graph = RoleGraph::new(&roles, &[], Some(1), 99);
    let mut collected = Vec::new();
    let mut findings = Vec::new();
    crate::verification::authority::evaluation::tests::run(inspect_acl_object_collect(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        &graph,
        &mut collected,
        &mut findings,
        "object",
        1,
        &[],
        &[],
        true,
        false,
        &super::privileges::PublicPolicy::from_grants(&[]),
        PublicObject::Database,
        &[ObjectPrivilege::Execute],
    ))
    .unwrap();
    assert_eq!(collected, vec![(1, ObjectPrivilege::Execute, true, true)]);
    assert!(findings.is_empty());
}

#[test]
fn inherited_only_superuser_role_does_not_confer_superuser_authority() {
    let root = role(1, "root");
    let mut inherited = role(2, "inherited_superuser");
    inherited.superuser = true;
    let roles = vec![root, inherited];
    let memberships = [Membership {
        role: 2,
        member: 1,
        admin: false,
        inherit: true,
        set: false,
    }];
    let graph = RoleGraph::new(&roles, &memberships, Some(1), 99);

    assert_eq!(
        graph
            .active_roles()
            .iter()
            .map(|role| role.oid)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        actual_for_capability_source(&[], 1, None, &graph, &[ObjectPrivilege::Select]),
        vec![(ObjectPrivilege::Select, false, false)]
    );
    assert_eq!(
        actual_for_capability_source(
            &[AclEntry {
                grantee: 2,
                privilege: ObjectPrivilege::Select,
                grant_option: false,
            }],
            2,
            None,
            &graph,
            &[ObjectPrivilege::Select],
        ),
        vec![(ObjectPrivilege::Select, true, false)]
    );
    let mut findings = Vec::new();
    crate::verification::authority::evaluation::tests::run(inspect_roles(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        &graph,
        &AuthorityPolicy::default(),
        &mut findings,
    ))
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn routine_matching_uses_quoted_structural_identity_and_canonical_types() {
    let mut types = HashMap::new();
    types.insert(
        23,
        TypeName {
            schema: "pg_catalog".to_owned(),
            name: "int4".to_owned(),
            array_element: None,
        },
    );
    let routine = RoutineObject {
        schema: "a.b".to_owned(),
        name: "f".to_owned(),
        owner: 1,
        security_definer: false,

        argument_types: vec![23],
        acl: Vec::new(),
    };
    let policy = RoutinePolicy {
        routine: RoutineSignature::new(
            "a.b",
            "f",
            [RoutineType::new("pg_catalog", "int4").unwrap()],
        )
        .unwrap(),
        privileges: vec![AllowedPrivilege::new(
            crate::verification::policy::ObjectPrivilege::Execute,
            false,
        )],
        allow_owner: false,
        allow_security_definer: false,
    };
    assert!(routine_matches(&policy, &routine, &types));
}

#[test]
fn routine_matching_preserves_catalog_case_and_true_array_identity() {
    let mut types = HashMap::new();
    types.insert(
        10,
        TypeName {
            schema: "custom".to_owned(),
            name: "Foo".to_owned(),
            array_element: None,
        },
    );
    types.insert(
        11,
        TypeName {
            schema: "pg_catalog".to_owned(),
            name: "_Foo".to_owned(),
            array_element: Some(10),
        },
    );
    let routine = RoutineObject {
        schema: "quoted.schema".to_owned(),
        name: "quoted.function".to_owned(),
        owner: 1,
        security_definer: false,

        argument_types: vec![11],
        acl: Vec::new(),
    };
    let policy = |type_name: RoutineType| RoutinePolicy {
        routine: RoutineSignature::new("quoted.schema", "quoted.function", [type_name]).unwrap(),
        privileges: vec![AllowedPrivilege::new(
            crate::verification::policy::ObjectPrivilege::Execute,
            false,
        )],
        allow_owner: false,
        allow_security_definer: false,
    };
    assert!(routine_matches(
        &policy(RoutineType::array("custom", "Foo").unwrap()),
        &routine,
        &types,
    ));
    assert!(!routine_matches(
        &policy(RoutineType::array("custom", "foo").unwrap()),
        &routine,
        &types,
    ));
    assert!(!routine_matches(
        &policy(RoutineType::new("custom", "Foo").unwrap()),
        &routine,
        &types,
    ));

    types.insert(
        12,
        TypeName {
            schema: "custom".to_owned(),
            name: "vectorish".to_owned(),
            array_element: None,
        },
    );
    let non_array = RoutineObject {
        argument_types: vec![12],
        ..routine
    };
    assert!(!routine_matches(
        &policy(RoutineType::array("custom", "Foo").unwrap()),
        &non_array,
        &types,
    ));
}
