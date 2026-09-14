use super::{RoleGraph, database::load_roles};
use crate::verification::{
    InspectionFragment, PgTransaction, VerificationError,
    report::{Finding, FindingKind, SupportedSurface, UnsupportedSurface},
};
use std::collections::HashSet;

#[derive(Clone, Debug)]
struct OwnershipRecord {
    class_id: i64,
    class_name: Option<String>,
    object_id: i64,
    subobject_id: i32,
    owner: i64,
    dependency_type: String,
}

const OWNERSHIP_ROWS_SQL: &str = "SELECT d.classid::bigint, catalog.relname::text,
            d.objid::bigint, d.objsubid, d.refobjid::bigint, d.deptype::text
     FROM pg_catalog.pg_shdepend AS d
     LEFT JOIN pg_catalog.pg_class AS catalog ON catalog.oid = d.classid
     LEFT JOIN pg_catalog.pg_roles AS referenced_role ON referenced_role.oid = d.refobjid
     WHERE d.dbid = $1::bigint::oid
       AND d.refclassid = 'pg_catalog.pg_authid'::regclass
       AND (
           d.classid = 0 OR d.objid = 0 OR d.objsubid < 0
           OR referenced_role.oid IS NULL
           OR d.deptype NOT IN ('a', 'i', 'o', 'r')
           OR (d.deptype = 'o' AND d.refobjid = ANY($2::bigint[]::oid[])))
     LIMIT 10001";

pub(crate) async fn inspect(
    transaction: &mut PgTransaction<'_>,
    session_user: &str,
) -> Result<InspectionFragment, VerificationError> {
    let (database_oid, database_name, database_owner) = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT oid::bigint, datname::text, datdba::bigint
             FROM pg_catalog.pg_database WHERE datname = current_database()",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    let (roles, memberships, root) = load_roles(transaction, session_user).await?;
    let Some(root) = root else {
        return Err(VerificationError::MissingSessionUser);
    };
    let graph = RoleGraph::new(&roles, &memberships, Some(root), database_owner);
    let reachable_owners = graph
        .capability_roles()
        .iter()
        .map(|role| role.oid)
        .collect::<Vec<_>>();
    let rows =
        sqlx::query_as::<_, (i64, Option<String>, i64, i32, i64, String)>(OWNERSHIP_ROWS_SQL)
            .bind(database_oid)
            .bind(&reachable_owners)
            .fetch_all(&mut **transaction)
            .await
            .map_err(|error| VerificationError::Native(error.into()))?;
    if rows.len() > 10_000 {
        return Ok(capacity_fragment(
            roles
                .len()
                .saturating_add(memberships.len())
                .saturating_add(rows.len())
                .saturating_add(1),
        ));
    }
    let records = rows
        .into_iter()
        .map(
            |(class_id, class_name, object_id, subobject_id, owner, dependency_type)| {
                OwnershipRecord {
                    class_id,
                    class_name,
                    object_id,
                    subobject_id,
                    owner,
                    dependency_type,
                }
            },
        )
        .collect::<Vec<_>>();
    let mut fragment = evaluate(&graph, &database_name, database_owner, &records);
    fragment.evaluated_items = roles
        .len()
        .saturating_add(memberships.len())
        .saturating_add(records.len())
        .saturating_add(1);
    Ok(fragment)
}

fn capacity_fragment(evaluated_items: usize) -> InspectionFragment {
    InspectionFragment {
        supported: vec![SupportedSurface::CurrentDatabaseOwnership],
        unsupported: vec![UnsupportedSurface::OwnershipCatalog],
        evaluated_items,
        ..InspectionFragment::default()
    }
}

fn evaluate(
    graph: &RoleGraph<'_>,
    database_name: &str,
    database_owner: i64,
    records: &[OwnershipRecord],
) -> InspectionFragment {
    let mut fragment = InspectionFragment {
        supported: vec![SupportedSurface::CurrentDatabaseOwnership],
        ..InspectionFragment::default()
    };
    if graph.has_unrecorded_ownership_capability() {
        push_unsupported(&mut fragment);
    }
    let mut seen = HashSet::new();
    if graph.is_capability_role(database_owner) {
        fragment.findings.push(Finding::new(
            FindingKind::ReachableOwnership,
            Some(format!("database:{}", quote(database_name))),
            graph.role_name(database_owner).map(str::to_owned),
            None,
        ));
    }
    for record in records {
        if graph.role(record.owner).is_none() {
            push_unsupported(&mut fragment);
            continue;
        }
        if record.class_id == 0 || record.object_id == 0 || record.subobject_id < 0 {
            push_unsupported(&mut fragment);
            continue;
        }
        match record.dependency_type.as_str() {
            "a" | "i" | "r" => continue,
            "o" => {}
            _ => {
                push_unsupported(&mut fragment);
                continue;
            }
        }
        if !graph.is_capability_role(record.owner) {
            continue;
        }
        let identity = format!(
            "class:{};classid={};objid={};objsubid={}",
            record.class_name.as_deref().unwrap_or("<unknown>"),
            record.class_id,
            record.object_id,
            record.subobject_id
        );
        if seen.insert((record.owner, identity.clone())) {
            fragment.findings.push(Finding::new(
                FindingKind::ReachableOwnership,
                Some(identity),
                graph.role_name(record.owner).map(str::to_owned),
                None,
            ));
        }
    }
    fragment
}

fn push_unsupported(fragment: &mut InspectionFragment) {
    if !fragment
        .unsupported
        .contains(&UnsupportedSurface::OwnershipCatalog)
    {
        fragment
            .unsupported
            .push(UnsupportedSurface::OwnershipCatalog);
    }
}

fn quote(identifier: &str) -> String {
    crate::verification::policy::quote_identifier(identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::authority::{Membership, RoleInfo};

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

    fn complete_roles(mut roles: Vec<RoleInfo>, database_owner: i64) -> Vec<RoleInfo> {
        if !roles.iter().any(|role| role.oid == database_owner) {
            roles.push(role(database_owner, "database_owner"));
        }
        if !roles.iter().any(|role| role.name == "pg_database_owner") {
            roles.push(role(10_000, "pg_database_owner"));
        }
        roles
    }

    #[test]
    fn reachable_owners_are_findings_and_nonownership_rows_are_ignored() {
        let roles = complete_roles(vec![role(1, "login"), role(2, "owner")], 9);
        let memberships = [Membership {
            role: 2,
            member: 1,
            admin: false,
            inherit: false,
            set: true,
        }];
        let graph = RoleGraph::new(&roles, &memberships, Some(1), 9);
        let records = [
            OwnershipRecord {
                class_id: 1259,
                class_name: Some("pg_class".to_owned()),
                object_id: 42,
                subobject_id: 3,
                owner: 2,
                dependency_type: "o".to_owned(),
            },
            OwnershipRecord {
                dependency_type: "a".to_owned(),
                object_id: 43,
                ..OwnershipRecord {
                    class_id: 1259,
                    class_name: Some("pg_class".to_owned()),
                    object_id: 42,
                    subobject_id: 0,
                    owner: 2,
                    dependency_type: "o".to_owned(),
                }
            },
        ];
        let fragment = evaluate(&graph, "db", 9, &records);
        assert_eq!(fragment.findings.len(), 1);
        assert!(
            fragment.findings[0]
                .object
                .as_deref()
                .unwrap()
                .contains("objsubid=3")
        );
        assert!(fragment.unsupported.is_empty());
    }

    #[test]
    fn unknown_dependency_form_is_incomplete_but_unknown_class_is_coarse() {
        let roles = complete_roles(vec![role(1, "login")], 9);
        let graph = RoleGraph::new(&roles, &[], Some(1), 9);
        let records = [
            OwnershipRecord {
                class_id: 99,
                class_name: None,
                object_id: 42,
                subobject_id: 0,
                owner: 1,
                dependency_type: "o".to_owned(),
            },
            OwnershipRecord {
                class_id: 99,
                class_name: None,
                object_id: 43,
                subobject_id: 0,
                owner: 1,
                dependency_type: "?".to_owned(),
            },
        ];
        let fragment = evaluate(&graph, "db", 9, &records);
        assert_eq!(fragment.findings.len(), 1);
        assert_eq!(fragment.unsupported, [UnsupportedSurface::OwnershipCatalog]);
    }

    #[test]
    fn impossible_and_malformed_nonowner_rows_are_incomplete() {
        let roles = complete_roles(vec![role(1, "login")], 9);
        let graph = RoleGraph::new(&roles, &[], Some(1), 9);
        for record in [
            OwnershipRecord {
                class_id: 1259,
                class_name: Some("pg_class".to_owned()),
                object_id: 42,
                subobject_id: 0,
                owner: 1,
                dependency_type: "t".to_owned(),
            },
            OwnershipRecord {
                class_id: 0,
                class_name: None,
                object_id: 0,
                subobject_id: -1,
                owner: 1,
                dependency_type: "a".to_owned(),
            },
        ] {
            let fragment = evaluate(&graph, "db", 9, &[record]);
            assert!(fragment.findings.is_empty());
            assert_eq!(fragment.unsupported, [UnsupportedSurface::OwnershipCatalog]);
        }
    }

    #[test]
    fn unreachable_owners_are_clean_but_pinned_capabilities_are_incomplete() {
        let roles = complete_roles(vec![role(1, "login"), role(2, "owner")], 9);
        let graph = RoleGraph::new(&roles, &[], Some(1), 9);
        let records = [OwnershipRecord {
            class_id: 1259,
            class_name: Some("pg_class".to_owned()),
            object_id: 42,
            subobject_id: 0,
            owner: 2,
            dependency_type: "o".to_owned(),
        }];
        let fragment = evaluate(&graph, "db", 9, &records);
        assert!(fragment.findings.is_empty());
        assert!(fragment.unsupported.is_empty());

        let mut superuser = role(1, "login");
        superuser.superuser = true;
        let roles = complete_roles(vec![superuser], 9);
        let graph = RoleGraph::new(&roles, &[], Some(1), 9);
        assert_eq!(
            evaluate(&graph, "db", 9, &[]).unsupported,
            [UnsupportedSurface::OwnershipCatalog]
        );

        let roles = complete_roles(vec![role(1, "login"), role(2, "pg_read_all_data")], 9);
        let memberships = [Membership {
            role: 2,
            member: 1,
            admin: false,
            inherit: true,
            set: false,
        }];
        let graph = RoleGraph::new(&roles, &memberships, Some(1), 9);
        assert_eq!(
            evaluate(&graph, "db", 9, &[]).unsupported,
            [UnsupportedSurface::OwnershipCatalog]
        );
    }

    #[test]
    fn current_database_owner_is_a_finding_without_implicit_role_incompleteness() {
        let roles = [role(1, "login"), role(2, "pg_database_owner")];
        let graph = RoleGraph::new(&roles, &[], Some(1), 1);
        let fragment = evaluate(&graph, "db", 1, &[]);

        assert_eq!(fragment.findings.len(), 1);
        assert_eq!(fragment.findings[0].kind, FindingKind::ReachableOwnership);
        assert_eq!(fragment.findings[0].subject.as_deref(), Some("login"));
        assert!(fragment.unsupported.is_empty());
    }

    #[test]
    fn stored_database_owner_memberships_are_incomplete_catalog_evidence() {
        for membership in [
            Membership {
                role: 2,
                member: 1,
                admin: false,
                inherit: true,
                set: false,
            },
            Membership {
                role: 2,
                member: 1,
                admin: false,
                inherit: false,
                set: true,
            },
            Membership {
                role: 2,
                member: 1,
                admin: true,
                inherit: false,
                set: false,
            },
            Membership {
                role: 1,
                member: 2,
                admin: false,
                inherit: true,
                set: true,
            },
        ] {
            let roles = complete_roles(vec![role(1, "login"), role(2, "pg_database_owner")], 9);
            let graph = RoleGraph::new(&roles, &[membership], Some(1), 9);
            let fragment = evaluate(&graph, "db", 9, &[]);

            assert!(fragment.findings.is_empty());
            assert_eq!(fragment.unsupported, [UnsupportedSurface::OwnershipCatalog]);
        }
    }

    #[test]
    fn inherited_and_admin_reachable_owners_are_findings() {
        for membership in [
            Membership {
                role: 2,
                member: 1,
                admin: false,
                inherit: true,
                set: false,
            },
            Membership {
                role: 2,
                member: 1,
                admin: true,
                inherit: false,
                set: false,
            },
        ] {
            let roles = complete_roles(vec![role(1, "login"), role(2, "owner")], 9);
            let memberships = [membership];
            let graph = RoleGraph::new(&roles, &memberships, Some(1), 9);
            let records = [OwnershipRecord {
                class_id: 1259,
                class_name: Some("pg_class".to_owned()),
                object_id: 42,
                subobject_id: 0,
                owner: 2,
                dependency_type: "o".to_owned(),
            }];

            let fragment = evaluate(&graph, "db", 9, &records);
            assert_eq!(fragment.findings.len(), 1);
            assert_eq!(fragment.findings[0].subject.as_deref(), Some("owner"));
            assert!(fragment.unsupported.is_empty());
        }
    }

    #[test]
    fn ownership_capacity_is_reported_as_incomplete() {
        let fragment = capacity_fragment(10_042);
        assert_eq!(
            fragment.supported,
            [SupportedSurface::CurrentDatabaseOwnership]
        );
        assert_eq!(fragment.unsupported, [UnsupportedSurface::OwnershipCatalog]);
        assert_eq!(fragment.evaluated_items, 10_042);
    }

    #[test]
    fn ownership_role_references_must_exist_in_the_bounded_role_snapshot() {
        let complete = complete_roles(vec![role(1, "login")], 9);
        let missing_database_owner = [role(1, "login"), role(2, "pg_database_owner")];
        let missing_predefined_role = [role(1, "login"), role(9, "database_owner")];
        let malformed_membership = [Membership {
            role: 88,
            member: 1,
            admin: false,
            inherit: true,
            set: false,
        }];
        let missing_owner_record = OwnershipRecord {
            class_id: 1_259,
            class_name: Some("pg_class".to_owned()),
            object_id: 42,
            subobject_id: 0,
            owner: 88,
            dependency_type: "o".to_owned(),
        };

        for (graph, records) in [
            (
                RoleGraph::new(&missing_database_owner, &[], Some(1), 9),
                &[][..],
            ),
            (
                RoleGraph::new(&missing_predefined_role, &[], Some(1), 9),
                &[][..],
            ),
            (
                RoleGraph::new(&complete, &malformed_membership, Some(1), 9),
                &[][..],
            ),
            (
                RoleGraph::new(&complete, &[], Some(1), 9),
                std::slice::from_ref(&missing_owner_record),
            ),
        ] {
            assert_eq!(
                evaluate(&graph, "db", 9, records).unsupported,
                [UnsupportedSurface::OwnershipCatalog]
            );
        }
    }

    #[test]
    fn ownership_query_bounds_only_reachable_or_malformed_evidence() {
        assert!(OWNERSHIP_ROWS_SQL.contains("d.refobjid = ANY($2::bigint[]::oid[])"));
        assert!(OWNERSHIP_ROWS_SQL.contains("referenced_role.oid IS NULL"));
        assert!(OWNERSHIP_ROWS_SQL.contains("d.deptype NOT IN ('a', 'i', 'o', 'r')"));
        assert!(!OWNERSHIP_ROWS_SQL.contains("d.deptype = 'o' OR"));
    }
}
