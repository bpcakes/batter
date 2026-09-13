use super::super::policy::*;
use super::super::report::VerificationError;
use super::database::bounded_rows;
use super::evaluation::Evaluation;
use super::privileges::routine_signature_from_catalog;
use super::selection::CatalogSelection;
use super::{CatalogSnapshot, PgTransaction};
use std::collections::HashSet;

struct DeclaredPolicies<'a> {
    schemas: HashSet<&'a Identifier>,
    relations: HashSet<&'a QualifiedName>,
    sequences: HashSet<&'a QualifiedName>,
    types: HashSet<&'a QualifiedName>,
    routines: HashSet<&'a RoutineSignature>,
    public: PublicDeclarations<'a>,
}

impl<'a> DeclaredPolicies<'a> {
    fn new(policy: &'a AuthorityPolicy) -> Self {
        Self {
            schemas: policy.schemas.iter().map(|entry| &entry.schema).collect(),
            relations: policy
                .relations
                .iter()
                .map(|entry| &entry.relation)
                .collect(),
            sequences: policy
                .sequences
                .iter()
                .map(|entry| &entry.sequence)
                .collect(),
            types: policy.types.iter().map(|entry| &entry.type_name).collect(),
            routines: policy.routines.iter().map(|entry| &entry.routine).collect(),
            public: PublicDeclarations::new(policy),
        }
    }
}

pub(super) async fn select(
    transaction: &mut PgTransaction<'_>,
    policy: &AuthorityPolicy,
    selected: &mut CatalogSelection,
) -> Result<(), VerificationError> {
    let (schemas, all) = match &policy.discovery {
        DiscoveryScope::Declared => return Ok(()),
        DiscoveryScope::Schemas(names) => (
            names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
            false,
        ),
        DiscoveryScope::UserSchemas => (Vec::new(), true),
    };
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "WITH selected AS (
            SELECT oid, nspname FROM pg_catalog.pg_namespace
            WHERE nspname = ANY($1::text[])
               OR ($2 AND nspname <> 'information_schema' AND left(nspname, 3) <> 'pg_')
         )
         SELECT 'schema'::text, nspname::text, nspname::text FROM selected
         UNION ALL
         SELECT 'relation', n.nspname::text, c.relname::text
         FROM selected n JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid
         WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
         UNION ALL
         SELECT 'routine', n.nspname::text, p.proname::text
         FROM selected n JOIN pg_catalog.pg_proc p ON p.pronamespace = n.oid
         UNION ALL
         SELECT 'type', n.nspname::text, t.typname::text
         FROM selected n JOIN pg_catalog.pg_type t ON t.typnamespace = n.oid
         LIMIT 10001",
    )
    .bind(schemas)
    .bind(all)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| VerificationError::Native(error.into()))
    .and_then(bounded_rows)?;
    for (kind, schema, name) in rows {
        match kind.as_str() {
            "schema" => selected.schemas.push(name),
            "relation" => selected.relations.push((schema, name)),
            "routine" => selected.routines.push((schema, name)),
            "type" => selected.types.push((schema, name)),
            _ => unreachable!("fixed discovery discriminator"),
        }
    }
    selected.schemas.sort();
    selected.schemas.dedup();
    selected.relations.sort();
    selected.relations.dedup();
    selected.routines.sort();
    selected.routines.dedup();
    selected.types.sort();
    selected.types.dedup();
    Ok(())
}

fn add_public(
    policy: &mut AuthorityPolicy,
    declared: &DeclaredPolicies<'_>,
    object: PublicObject,
    defaults: &ObjectDefaults,
) {
    if !declared.public.defaults_apply(&object) {
        return;
    }
    policy.public_grants.extend(
        defaults
            .public_privileges
            .iter()
            .map(|privilege| PublicGrant {
                object: object.clone(),
                privilege: *privilege,
            }),
    );
}

/// Resolve defaults into ordinary exact policies once. The existing ACL
/// evaluator then visits every discovered object, including unlisted objects.
pub(super) async fn expand(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    original: &AuthorityPolicy,
) -> Result<AuthorityPolicy, VerificationError> {
    let declared = DeclaredPolicies::new(original);
    let mut policy = original.clone();
    for entry in &original.public_overrides {
        evaluation.checkpoint(&[]).await?;
        policy
            .public_grants
            .extend(entry.privileges.iter().map(|privilege| PublicGrant {
                object: entry.object.clone(),
                privilege: *privilege,
            }));
    }
    let defaults = &original.defaults;
    for schema in &snapshot.schemas {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&schema.name) {
            continue;
        }
        let name =
            Identifier::new(schema.name.clone()).map_err(VerificationError::InvalidPolicy)?;
        if !declared.schemas.contains(&name) {
            policy.schemas.push(SchemaPolicy {
                schema: name.clone(),
                privileges: defaults.schemas.privileges.clone(),
                allow_owner: defaults.schemas.allow_owner,
            });
        }
        add_public(
            &mut policy,
            &declared,
            PublicObject::Schema(name),
            &defaults.schemas,
        );
    }
    expand_relations(evaluation, snapshot, original, &declared, &mut policy).await?;
    for object in &snapshot.types {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&object.schema) {
            continue;
        }
        let name = QualifiedName::new(object.schema.clone(), object.name.clone())
            .map_err(VerificationError::InvalidPolicy)?;
        if !declared.types.contains(&name) {
            policy.types.push(TypePolicy {
                type_name: name.clone(),
                privileges: defaults.types.privileges.clone(),
                allow_owner: defaults.types.allow_owner,
            });
        }
        add_public(
            &mut policy,
            &declared,
            PublicObject::Type(name),
            &defaults.types,
        );
    }
    for object in &snapshot.routines {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&object.schema) {
            continue;
        }
        let name = routine_signature_from_catalog(object, &snapshot.type_names)
            .map_err(VerificationError::InvalidPolicy)?;
        let defaults = if object.security_definer {
            &defaults.definer_routines
        } else {
            &defaults.invoker_routines
        };
        if !declared.routines.contains(&name) {
            policy.routines.push(RoutinePolicy {
                routine: name.clone(),
                privileges: defaults.privileges.clone(),
                allow_owner: defaults.allow_owner,
                // Default definer authority remains limited by the explicit
                // definer EXECUTE/owner/PUBLIC allowances above.
                allow_security_definer: true,
            });
        }
        add_public(
            &mut policy,
            &declared,
            PublicObject::Routine(name),
            defaults,
        );
    }
    Ok(policy)
}

async fn expand_relations(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    original: &AuthorityPolicy,
    declared: &DeclaredPolicies<'_>,
    policy: &mut AuthorityPolicy,
) -> Result<(), VerificationError> {
    let defaults = &original.defaults;
    for relation in &snapshot.relations {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&relation.schema) {
            continue;
        }
        let name = QualifiedName::new(relation.schema.clone(), relation.name.clone())
            .map_err(VerificationError::InvalidPolicy)?;
        if relation.kind == "S" {
            if !declared.sequences.contains(&name) {
                policy.sequences.push(SequencePolicy {
                    sequence: name.clone(),
                    privileges: defaults.sequences.privileges.clone(),
                    allow_owner: defaults.sequences.allow_owner,
                });
            }
            add_public(
                policy,
                declared,
                PublicObject::Sequence(name),
                &defaults.sequences,
            );
        } else {
            if !declared.relations.contains(&name) {
                let columns = relation
                    .columns
                    .iter()
                    .map(|column| {
                        Ok(ColumnPolicy {
                            column: Identifier::new(column.name.clone())
                                .map_err(VerificationError::InvalidPolicy)?,
                            privileges: defaults.columns.privileges.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, VerificationError>>()?;
                policy.relations.push(RelationPolicy {
                    relation: name.clone(),
                    privileges: defaults.relations.privileges.clone(),
                    columns,
                    allow_owner: defaults.relations.allow_owner,
                    allow_row_type_public_usage: defaults.allow_row_type_public_usage,
                });
            }
            add_public(
                policy,
                declared,
                PublicObject::Relation(name.clone()),
                &defaults.relations,
            );
            for column in &relation.columns {
                evaluation.checkpoint(&[]).await?;
                add_public(
                    policy,
                    declared,
                    PublicObject::Column(
                        name.clone(),
                        Identifier::new(column.name.clone())
                            .map_err(VerificationError::InvalidPolicy)?,
                    ),
                    &defaults.columns,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::requests::inspect_relations;
    use super::super::required::tests::snapshot;
    use super::super::{AclEntry, RoleGraph};
    use super::*;
    use crate::verification::report::FindingKind;

    #[test]
    fn discovered_undeclared_grant_is_rejected_and_default_or_exact_allowance_is_applied() {
        let mut snapshot = snapshot();
        snapshot.relations[0].acl.push(AclEntry {
            grantee: 1,
            privilege: ObjectPrivilege::Select,
            grant_option: false,
        });
        let graph = RoleGraph::new(
            &snapshot.roles,
            &snapshot.memberships,
            snapshot.root,
            snapshot.database.owner,
        );
        let mut policy = AuthorityPolicy {
            discovery: DiscoveryScope::UserSchemas,
            ..AuthorityPolicy::default()
        };
        let check = |policy: &AuthorityPolicy| {
            let expanded = crate::verification::authority::evaluation::tests::run(expand(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                policy,
            ))
            .unwrap();
            let mut findings = Vec::new();
            crate::verification::authority::evaluation::tests::run(inspect_relations(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                &graph,
                &expanded,
                &mut findings,
            ))
            .unwrap();
            findings
        };
        let rejected = check(&policy);
        assert!(
            rejected
                .iter()
                .any(|finding| finding.kind == FindingKind::Privilege
                    && finding.privilege == Some(ObjectPrivilege::Select))
        );
        policy
            .defaults
            .relations
            .privileges
            .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
        assert!(check(&policy).is_empty());
        policy.relations.push(RelationPolicy {
            relation: QualifiedName::new("service", "records").unwrap(),
            privileges: Vec::new(),
            columns: Vec::new(),
            allow_owner: false,
            allow_row_type_public_usage: false,
        });
        assert!(
            !check(&policy).is_empty(),
            "exact empty policy must override permissive scope default"
        );
    }

    #[test]
    fn exact_empty_public_override_rejects_grants_allowed_by_discovery_default() {
        let mut snapshot = snapshot();
        snapshot.relations[0].acl.push(AclEntry {
            grantee: 0,
            privilege: ObjectPrivilege::Select,
            grant_option: false,
        });
        let graph = RoleGraph::new(
            &snapshot.roles,
            &snapshot.memberships,
            snapshot.root,
            snapshot.database.owner,
        );
        let mut policy = AuthorityPolicy {
            discovery: DiscoveryScope::UserSchemas,
            ..AuthorityPolicy::default()
        };
        policy
            .defaults
            .relations
            .public_privileges
            .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
        let mut findings = Vec::new();
        crate::verification::authority::evaluation::tests::run(inspect_relations(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &graph,
            &crate::verification::authority::evaluation::tests::run(expand(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                &policy,
            ))
            .unwrap(),
            &mut findings,
        ))
        .unwrap();
        assert!(findings.is_empty());
        policy.public_overrides.push(PublicAllowance {
            object: PublicObject::Relation(QualifiedName::new("service", "records").unwrap()),
            privileges: Vec::new(),
        });
        crate::verification::authority::evaluation::tests::run(inspect_relations(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &graph,
            &crate::verification::authority::evaluation::tests::run(expand(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                &policy,
            ))
            .unwrap(),
            &mut findings,
        ))
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject.as_deref(), Some("public"));
    }
}

#[cfg(test)]
mod scale_tests;
