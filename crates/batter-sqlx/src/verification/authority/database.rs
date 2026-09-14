mod parameters;
pub(in crate::verification::authority) use parameters::ParameterCatalog;
use parameters::{load_parameters, settings_visible};

use super::super::PgTransaction;
use super::super::policy::{AuthorityPolicy, ObjectPrivilege};
use super::super::report::VerificationError;
use super::selection::CatalogSelection;
use super::{
    AclEntry, ColumnObject, DatabaseObject, Membership, ParameterObject, RelationObject, RoleInfo,
    RoutineObject, SchemaObject, TypeName, TypeObject,
};
use std::collections::{HashMap, HashSet};

impl super::CatalogSnapshot {
    pub(super) async fn load(
        transaction: &mut PgTransaction<'_>,
        policy: &AuthorityPolicy,
        session_user: String,
        current_user: String,
    ) -> Result<Self, VerificationError> {
        let mut selection = CatalogSelection::from_policy(policy);
        super::discovery::select(transaction, policy, &mut selection).await?;
        let database = load_database(transaction).await?;
        let (roles, memberships, root) = load_roles(transaction, &session_user).await?;

        let schemas = load_schemas(transaction, &selection.schemas).await?;
        let mut relations = load_relations(transaction, &selection.relations).await?;
        let relation_oids = relations
            .iter()
            .filter(|relation| relation.kind != "S")
            .map(|relation| relation.oid)
            .collect::<Vec<_>>();
        let mut columns_by_relation: HashMap<i64, Vec<ColumnObject>> = HashMap::new();
        for (relation, column, acl) in load_columns(transaction, &relation_oids).await? {
            columns_by_relation
                .entry(relation)
                .or_default()
                .push(ColumnObject { name: column, acl });
        }
        for relation in &mut relations {
            relation.columns = columns_by_relation
                .remove(&relation.oid)
                .unwrap_or_default();
        }
        let types = load_types(transaction, &selection.types, &relation_oids).await?;
        let routines = load_routines(transaction, &selection.routines).await?;
        let type_names = load_type_names(transaction, &routines).await?;
        let parameters = load_parameters(
            transaction,
            &selection.parameters,
            settings_visible(&roles, &memberships, &current_user),
        )
        .await?;
        Ok(Self {
            session_user,
            current_user,
            roles,
            memberships,
            root,
            schemas,
            relations,
            types,
            type_names,
            routines,
            database,
            parameters,
        })
    }
}

pub(super) async fn load_roles(
    transaction: &mut PgTransaction<'_>,
    session_user: &str,
) -> Result<(Vec<RoleInfo>, Vec<Membership>, Option<i64>), VerificationError> {
    let role_rows = sqlx::query_as::<_, (i64, String, bool, bool, bool, bool, bool)>(
        "SELECT oid::bigint, rolname::text, rolsuper,
                rolcreatedb, rolcreaterole, rolreplication, rolbypassrls
         FROM pg_catalog.pg_roles LIMIT 10001",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let roles: Vec<RoleInfo> = role_rows
        .into_iter()
        .map(
            |(oid, name, superuser, createdb, createrole, replication, bypassrls)| RoleInfo {
                oid,
                name,
                superuser,
                createdb,
                createrole,
                replication,
                bypassrls,
            },
        )
        .collect();
    let memberships = sqlx::query_as::<_, (i64, i64, bool, bool, bool)>(
        "SELECT roleid::bigint, member::bigint, admin_option,
                inherit_option, set_option
         FROM pg_catalog.pg_auth_members LIMIT 10001",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?
    .into_iter()
    .map(|(role, member, admin, inherit, set)| Membership {
        role,
        member,
        admin,
        inherit,
        set,
    })
    .collect::<Vec<_>>();
    let root = roles
        .iter()
        .find(|role| role.name == session_user)
        .map(|role| role.oid);
    Ok((roles, memberships, root))
}

async fn load_schemas(
    transaction: &mut PgTransaction<'_>,
    names: &[String],
) -> Result<Vec<SchemaObject>, VerificationError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let objects = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT n.oid::bigint, n.nspname::text, n.nspowner::bigint
         FROM pg_catalog.pg_namespace AS n
         WHERE n.nspname = ANY($1::text[]) LIMIT 10001",
    )
    .bind(names)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let object_oids = objects.iter().map(|(oid, _, _)| *oid).collect::<Vec<_>>();
    let acl_rows = sqlx::query_as::<_, (i64, i64, String, bool)>(
        "SELECT n.oid::bigint, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_namespace AS n
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(n.nspacl, pg_catalog.acldefault('n'::\"char\", n.nspowner))) AS acl
         WHERE n.oid = ANY($1::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(&object_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let mut acls = group_acl_rows(acl_rows, AclKind::Schema);
    Ok(objects
        .into_iter()
        .map(|(oid, name, owner)| SchemaObject {
            name,
            owner,
            acl: acls.remove(&oid).unwrap_or_default(),
        })
        .collect())
}

async fn load_relations(
    transaction: &mut PgTransaction<'_>,
    names: &[(String, String)],
) -> Result<Vec<RelationObject>, VerificationError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let schemas = names
        .iter()
        .map(|(schema, _)| schema.clone())
        .collect::<Vec<_>>();
    let objects_names = names
        .iter()
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    let objects = sqlx::query_as::<_, (i64, String, String, String, i64)>(
        "SELECT c.oid::bigint, n.nspname::text, c.relname::text,
                c.relkind::text, c.relowner::bigint
         FROM unnest($1::text[], $2::text[]) AS requested(schema_name, object_name)
         JOIN pg_catalog.pg_namespace AS n ON n.nspname = requested.schema_name
         JOIN pg_catalog.pg_class AS c
           ON c.relnamespace = n.oid AND c.relname = requested.object_name
         WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') LIMIT 10001",
    )
    .bind(&schemas)
    .bind(&objects_names)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let object_oids = objects
        .iter()
        .map(|(oid, _, _, _, _)| *oid)
        .collect::<Vec<_>>();
    let acl_rows = sqlx::query_as::<_, (i64, i64, String, bool)>(
        "SELECT c.oid::bigint, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_class AS c
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(
                 c.relacl,
                 pg_catalog.acldefault(
                     CASE WHEN c.relkind = 'S'::\"char\"
                          THEN 's'::\"char\" ELSE 'r'::\"char\" END,
                     c.relowner))) AS acl
         WHERE c.oid = ANY($1::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(&object_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let mut acls = group_acl_rows(acl_rows, AclKind::Relation);
    Ok(objects
        .into_iter()
        .map(|(oid, schema, name, kind, owner)| RelationObject {
            oid,
            schema,
            name,
            kind,
            owner,
            acl: acls.remove(&oid).unwrap_or_default(),
            columns: Vec::new(),
        })
        .collect())
}

async fn load_columns(
    transaction: &mut PgTransaction<'_>,
    relation_oids: &[i64],
) -> Result<Vec<(i64, String, Vec<AclEntry>)>, VerificationError> {
    if relation_oids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, (i64, String, Option<i64>, Option<String>, Option<bool>)>(
        "SELECT a.attrelid::bigint, a.attname::text, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_attribute AS a
         LEFT JOIN LATERAL pg_catalog.aclexplode(a.attacl) AS acl ON true
         WHERE a.attnum > 0 AND NOT a.attisdropped
           AND a.attrelid = ANY($1::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(relation_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let mut grouped: HashMap<(i64, String), Vec<AclEntry>> = HashMap::new();
    for (relation, column, grantee, privilege, grant_option) in rows {
        let acl = grouped.entry((relation, column)).or_default();
        if let (Some(grantee), Some(privilege), Some(grant_option)) =
            (grantee, privilege, grant_option)
            && let Some(privilege) = map_privilege(AclKind::Column, &privilege)
        {
            acl.push(AclEntry {
                grantee,
                privilege,
                grant_option,
            });
        }
    }
    Ok(grouped
        .into_iter()
        .map(|((relation, column), acl)| (relation, column, acl))
        .collect())
}

async fn load_type_names(
    transaction: &mut PgTransaction<'_>,
    routines: &[RoutineObject],
) -> Result<HashMap<i64, TypeName>, VerificationError> {
    let mut argument_oids = routines
        .iter()
        .flat_map(|routine| routine.argument_types.iter().copied())
        .collect::<Vec<_>>();
    argument_oids.sort_unstable();
    argument_oids.dedup();
    if argument_oids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (i64, String, String, Option<i64>)>(
        "WITH RECURSIVE selected(oid) AS (
             SELECT requested.oid::oid
             FROM unnest($1::bigint[]) AS requested(oid)
             UNION
             SELECT t.typelem
             FROM pg_catalog.pg_type AS t
             JOIN selected ON selected.oid = t.oid
             JOIN pg_catalog.pg_type AS element ON element.oid = t.typelem
             WHERE t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
               AND t.typelem <> 0
               AND element.typarray = t.oid
         )
         SELECT t.oid::bigint, n.nspname::text, t.typname::text,
                CASE
                    WHEN t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
                     AND element.typarray = t.oid
                    THEN NULLIF(t.typelem, 0)::bigint
                END
         FROM selected
         JOIN pg_catalog.pg_type AS t ON t.oid = selected.oid
         JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
         LEFT JOIN pg_catalog.pg_type AS element ON element.oid = t.typelem LIMIT 10001",
    )
    .bind(&argument_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    Ok(rows
        .into_iter()
        .map(|(oid, schema, name, array_element)| {
            (
                oid,
                TypeName {
                    schema,
                    name,
                    array_element,
                },
            )
        })
        .collect())
}

async fn load_types(
    transaction: &mut PgTransaction<'_>,
    names: &[(String, String)],
    relation_oids: &[i64],
) -> Result<Vec<TypeObject>, VerificationError> {
    if names.is_empty() && relation_oids.is_empty() {
        return Ok(Vec::new());
    }
    let schemas = names
        .iter()
        .map(|(schema, _)| schema.clone())
        .collect::<Vec<_>>();
    let type_names = names
        .iter()
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    // PostgreSQL ACL resolution follows IsTrueArrayType (element + handler),
    // then multirange -> range. Unlike routine identity/discovery alias
    // detection this deliberately includes oidvector and int2vector.
    let objects = sqlx::query_as::<_, (i64, String, String, i64, i64, i64)>(
        "SELECT t.oid::bigint, n.nspname::text, t.typname::text,
                effective.oid::bigint, effective.typowner::bigint, effective.typrelid::bigint
         FROM pg_catalog.pg_type AS t
         JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
         LEFT JOIN pg_catalog.pg_type element
           ON element.oid = t.typelem
          AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
         LEFT JOIN pg_catalog.pg_range range_type
           ON range_type.rngmultitypid = COALESCE(element.oid, t.oid)
         JOIN pg_catalog.pg_type effective
           ON effective.oid = COALESCE(range_type.rngtypid, element.oid, t.oid)
         WHERE EXISTS (
                   SELECT 1
                   FROM unnest($1::text[], $2::text[])
                        AS requested(schema_name, type_name)
                   WHERE requested.schema_name = n.nspname
                     AND requested.type_name = t.typname
               )
            OR t.typrelid = ANY($3::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(&schemas)
    .bind(&type_names)
    .bind(relation_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    // Read each authority source once, preserving every selected type identity.
    // Arrays and multiranges may share authority without sharing policy scope.
    let source_oids = objects
        .iter()
        .map(|(_, _, _, source, _, _)| *source)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let acl_rows = sqlx::query_as::<_, (i64, i64, String, bool)>(
        "SELECT effective.oid::bigint, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_type effective
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(effective.typacl, pg_catalog.acldefault('T'::\"char\", effective.typowner))) AS acl
         WHERE effective.oid = ANY($1::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(&source_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let acls: HashMap<_, std::sync::Arc<[AclEntry]>> = group_acl_rows(acl_rows, AclKind::Type)
        .into_iter()
        .map(|(oid, acl)| (oid, acl.into()))
        .collect();
    Ok(objects
        .into_iter()
        .map(
            |(_, schema, name, source, owner, relation_oid)| TypeObject {
                schema,
                name,
                owner,
                relation_oid,
                acl: acls.get(&source).cloned().unwrap_or_default(),
            },
        )
        .collect())
}

async fn load_routines(
    transaction: &mut PgTransaction<'_>,
    names: &[(String, String)],
) -> Result<Vec<RoutineObject>, VerificationError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let schemas = names
        .iter()
        .map(|(schema, _)| schema.clone())
        .collect::<Vec<_>>();
    let routine_names = names
        .iter()
        .map(|(_, name)| name.clone())
        .collect::<Vec<_>>();
    let objects = sqlx::query_as::<_, (i64, String, String, i64, bool, Vec<i64>)>(
        "SELECT p.oid::bigint, n.nspname::text, p.proname::text,
                p.proowner::bigint, p.prosecdef,
                ARRAY(SELECT argument::bigint FROM unnest(p.proargtypes) AS argument)
         FROM pg_catalog.pg_proc AS p
         JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE EXISTS (
                   SELECT 1
                   FROM unnest($1::text[], $2::text[])
                        AS requested(schema_name, routine_name)
                   WHERE requested.schema_name = n.nspname
                     AND requested.routine_name = p.proname
               ) LIMIT 10001",
    )
    .bind(&schemas)
    .bind(&routine_names)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let object_oids = objects
        .iter()
        .map(|(oid, _, _, _, _, _)| *oid)
        .collect::<Vec<_>>();
    let acl_rows = sqlx::query_as::<_, (i64, i64, String, bool)>(
        "SELECT p.oid::bigint, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_proc AS p
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(p.proacl, pg_catalog.acldefault('f'::\"char\", p.proowner))) AS acl
         WHERE p.oid = ANY($1::bigint[]::oid[]) LIMIT 10001",
    )
    .bind(&object_oids)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    let mut acls = group_acl_rows(acl_rows, AclKind::Routine);
    Ok(objects
        .into_iter()
        .map(
            |(oid, schema, name, owner, security_definer, argument_types)| RoutineObject {
                schema,
                name,
                owner,
                security_definer,
                argument_types,
                acl: acls.remove(&oid).unwrap_or_default(),
            },
        )
        .collect())
}

async fn load_database(
    transaction: &mut PgTransaction<'_>,
) -> Result<DatabaseObject, VerificationError> {
    let (oid, name, owner) = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT d.oid::bigint, d.datname::text, d.datdba::bigint
         FROM pg_catalog.pg_database AS d
         WHERE d.datname = current_database()",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))?;
    let acl_rows = sqlx::query_as::<_, (i64, i64, String, bool)>(
        "SELECT d.oid::bigint, acl.grantee::bigint,
                acl.privilege_type::text, acl.is_grantable
         FROM pg_catalog.pg_database AS d
         CROSS JOIN LATERAL pg_catalog.aclexplode(
             COALESCE(d.datacl, pg_catalog.acldefault('d'::\"char\", d.datdba))) AS acl
         WHERE d.oid = $1::oid LIMIT 10001",
    )
    .bind(oid)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    Ok(DatabaseObject {
        name,
        owner,
        acl: group_acl_rows(acl_rows, AclKind::Database)
            .remove(&oid)
            .unwrap_or_default(),
    })
}

#[derive(Clone, Copy)]
enum AclKind {
    Schema,
    Relation,
    Column,
    Type,
    Routine,
    Database,
    Parameter,
}

fn group_acl_rows(
    rows: Vec<(i64, i64, String, bool)>,
    kind: AclKind,
) -> HashMap<i64, Vec<AclEntry>> {
    let mut grouped = HashMap::new();
    for (oid, grantee, privilege, grant_option) in rows {
        if let Some(privilege) = map_privilege(kind, &privilege) {
            grouped.entry(oid).or_insert_with(Vec::new).push(AclEntry {
                grantee,
                privilege,
                grant_option,
            });
        }
    }
    grouped
}

fn map_privilege(kind: AclKind, privilege: &str) -> Option<ObjectPrivilege> {
    match (kind, privilege) {
        (AclKind::Schema, "USAGE") => Some(ObjectPrivilege::Usage),
        (AclKind::Schema, "CREATE") => Some(ObjectPrivilege::Create),
        (AclKind::Relation | AclKind::Column, "SELECT") => Some(ObjectPrivilege::Select),
        (AclKind::Relation | AclKind::Column, "INSERT") => Some(ObjectPrivilege::Insert),
        (AclKind::Relation | AclKind::Column, "UPDATE") => Some(ObjectPrivilege::Update),
        (AclKind::Relation, "DELETE") => Some(ObjectPrivilege::Delete),
        (AclKind::Relation, "TRUNCATE") => Some(ObjectPrivilege::Truncate),
        (AclKind::Relation | AclKind::Column, "REFERENCES") => Some(ObjectPrivilege::References),
        (AclKind::Relation, "TRIGGER") => Some(ObjectPrivilege::Trigger),
        (AclKind::Relation, "MAINTAIN") => Some(ObjectPrivilege::Maintain),
        (AclKind::Relation, "USAGE") => Some(ObjectPrivilege::Usage),
        (AclKind::Type, "USAGE") => Some(ObjectPrivilege::Usage),
        (AclKind::Routine, "EXECUTE") => Some(ObjectPrivilege::Execute),
        (AclKind::Database, "CONNECT") => Some(ObjectPrivilege::Connect),
        (AclKind::Database, "CREATE") => Some(ObjectPrivilege::Create),
        (AclKind::Database, "TEMPORARY") => Some(ObjectPrivilege::Temporary),
        (AclKind::Parameter, "SET") => Some(ObjectPrivilege::Set),
        (AclKind::Parameter, "ALTER SYSTEM") => Some(ObjectPrivilege::AlterSystem),
        _ => None,
    }
}

/// Reject the sentinel before any truncated result is evaluated.
pub(super) fn bounded_rows<T>(rows: Vec<T>) -> Result<Vec<T>, VerificationError> {
    if rows.len() > 10_000 {
        Err(VerificationError::CatalogCapacity)
    } else {
        Ok(rows)
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn overflow_sentinel_never_becomes_a_partial_catalog() {
        assert_eq!(bounded_rows(vec![0u8; 10_000]).unwrap().len(), 10_000);
        assert!(matches!(
            bounded_rows(vec![0u8; 10_001]),
            Err(VerificationError::CatalogCapacity)
        ));
    }
}
