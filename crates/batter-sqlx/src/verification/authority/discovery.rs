use super::super::policy::*;
use super::super::report::VerificationError;
use super::database::bounded_rows;
use super::evaluation::Evaluation;
use super::privileges::routine_signature_from_catalog;
use super::selection::CatalogSelection;
use super::{CatalogSnapshot, PgTransaction};
use std::collections::HashSet;
use std::ops::Deref;

mod catalog;

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

/// Catalog-resolved policy used only for ACL evaluation. The original compiled
/// policy remains the source of requested-object and required-privilege findings.
#[derive(Debug)]
pub(super) struct CatalogAuthorityPolicy(AuthorityPolicy);

impl Deref for CatalogAuthorityPolicy {
    type Target = AuthorityPolicy;

    fn deref(&self) -> &Self::Target {
        &self.0
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
    policy: &mut AuthorityPolicyBuilder,
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
) -> Result<CatalogAuthorityPolicy, VerificationError> {
    let declared = DeclaredPolicies::new(original);
    let mut policy = original.to_builder();
    let defaults = &original.defaults;
    let mut catalog_schemas = HashSet::with_capacity(snapshot.schemas.len());
    for schema in &snapshot.schemas {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&schema.name) {
            continue;
        }
        let name =
            Identifier::new(schema.name.clone()).map_err(VerificationError::CatalogIdentity)?;
        if !catalog_schemas.insert(name.clone()) {
            return Err(VerificationError::CatalogPolicyExpansion(
                PolicyError::DuplicateAuthorityObject,
            ));
        }
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
    catalog::reconcile_relations(evaluation, snapshot, original, &declared, &mut policy).await?;
    let mut catalog_types = HashSet::with_capacity(snapshot.types.len());
    for object in &snapshot.types {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&object.schema) {
            continue;
        }
        let name = QualifiedName::new(object.schema.clone(), object.name.clone())
            .map_err(VerificationError::CatalogIdentity)?;
        if !catalog_types.insert(name.clone()) {
            return Err(VerificationError::CatalogPolicyExpansion(
                PolicyError::DuplicateAuthorityObject,
            ));
        }
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
    let mut catalog_routines = HashSet::with_capacity(snapshot.routines.len());
    for object in &snapshot.routines {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&object.schema) {
            continue;
        }
        let name = routine_signature_from_catalog(object, &snapshot.type_names)
            .map_err(VerificationError::CatalogIdentity)?;
        if !catalog_routines.insert(name.clone()) {
            return Err(VerificationError::CatalogPolicyExpansion(
                PolicyError::DuplicateAuthorityObject,
            ));
        }
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
    Ok(CatalogAuthorityPolicy(policy.finish_catalog_expansion()))
}

#[cfg(test)]
mod tests {
    use super::super::requests::{inspect_relations, inspect_types};
    use super::super::required::tests::snapshot;
    use super::super::{AclEntry, ColumnObject, RoleGraph, RoutineObject, TypeObject};
    use super::*;
    use crate::verification::report::FindingKind;
    use crate::verification::{
        DeclarationPurpose, ExactRoleManifest, PublicDelivery, RelationGrantGroup,
    };

    fn required_findings(
        snapshot: &CatalogSnapshot,
        policy: &AuthorityPolicy,
    ) -> Vec<crate::verification::Finding> {
        let expanded = crate::verification::authority::evaluation::tests::run(expand(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            snapshot,
            policy,
        ))
        .unwrap();
        let parameters = super::super::index_parameters(&snapshot.parameters);
        let mut findings = Vec::new();
        crate::verification::authority::evaluation::tests::run(super::super::required::inspect(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            snapshot,
            &parameters,
            &expanded,
            &mut findings,
        ))
        .unwrap();
        findings
    }

    #[test]
    fn catalog_dependent_requirements_remain_report_findings_after_expansion() {
        let mut snapshot = snapshot();
        snapshot.relations[0].columns.push(ColumnObject {
            name: "present".to_owned(),
            acl: Vec::new(),
        });

        let relation = QualifiedName::new("service", "records").unwrap();
        let column =
            |name: &str| PublicObject::Column(relation.clone(), Identifier::new(name).unwrap());
        let mut owner_draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
        owner_draft.defaults.columns.allow_owner = true;
        owner_draft.required_privileges.push(RequiredPrivilege {
            object: column("present"),
            privilege: ObjectPrivilege::Select,
        });
        let owner_findings = required_findings(&snapshot, &owner_draft.build().unwrap());
        assert_eq!(owner_findings.len(), 1);
        assert_eq!(owner_findings[0].kind, FindingKind::MissingPrivilege);

        let mut missing_draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
        missing_draft.defaults.columns.privileges =
            vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
        missing_draft.required_privileges.push(RequiredPrivilege {
            object: column("missing"),
            privilege: ObjectPrivilege::Select,
        });
        let missing_findings = required_findings(&snapshot, &missing_draft.build().unwrap());
        assert_eq!(missing_findings.len(), 1);
        assert_eq!(missing_findings[0].kind, FindingKind::MissingPrivilege);

        snapshot.routines.push(RoutineObject {
            schema: "service".to_owned(),
            name: "perform".to_owned(),
            owner: 3,
            security_definer: true,
            argument_types: Vec::new(),
            acl: Vec::new(),
        });
        let routine =
            RoutineSignature::new("service", "perform", std::iter::empty::<RoutineType>()).unwrap();
        let mut routine_draft = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas);
        routine_draft.defaults.invoker_routines.privileges =
            vec![AllowedPrivilege::new(ObjectPrivilege::Execute, false)];
        routine_draft.required_privileges.push(RequiredPrivilege {
            object: PublicObject::Routine(routine),
            privilege: ObjectPrivilege::Execute,
        });
        let routine_findings = required_findings(&snapshot, &routine_draft.build().unwrap());
        assert_eq!(routine_findings.len(), 1);
        assert_eq!(routine_findings[0].kind, FindingKind::MissingPrivilege);
    }

    #[test]
    fn malformed_catalog_identity_retains_its_typed_cause() {
        let mut snapshot = snapshot();
        snapshot.routines.push(RoutineObject {
            schema: "service".to_owned(),
            name: "perform".to_owned(),
            owner: 3,
            security_definer: false,
            argument_types: vec![i64::MAX],
            acl: Vec::new(),
        });
        let policy = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas)
            .build()
            .unwrap();
        let error = crate::verification::authority::evaluation::tests::run(expand(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &policy,
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            VerificationError::CatalogIdentity(PolicyError::InvalidRoutineType)
        ));
    }

    #[test]
    fn catalog_expansion_rejects_one_pg_class_identity_with_two_kinds() {
        let mut snapshot = snapshot();
        let mut conflicting = snapshot.relations[0].clone();
        conflicting.kind = "S".to_owned();
        conflicting.columns.clear();
        snapshot.relations.push(conflicting);
        let policy = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas)
            .build()
            .unwrap();
        let error = crate::verification::authority::evaluation::tests::run(expand(
            &mut crate::verification::authority::evaluation::Evaluation::new(),
            &snapshot,
            &policy,
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            VerificationError::CatalogPolicyExpansion(PolicyError::ConflictingRelationKind)
        ));
    }

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

    #[test]
    fn compiled_manifest_public_delivery_controls_expanded_verifier_findings() {
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
        let check = |delivery| {
            let mut manifest = ExactRoleManifest::new(
                Identifier::new("service").unwrap(),
                DiscoveryScope::UserSchemas,
            )
            .unwrap();
            let mut defaults = DiscoveryDefaults::default();
            defaults.relations.public_privileges =
                vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)];
            manifest.set_discovery_defaults(defaults).unwrap();
            manifest
                .add_relations(
                    RelationGrantGroup::new(
                        [QualifiedName::new("service", "records").unwrap()],
                        [ObjectPrivilege::Select],
                        DeclarationPurpose::AllowedOnly,
                    )
                    .unwrap()
                    .public_delivery(delivery)
                    .allow_row_type_public_usage(true),
                )
                .unwrap();
            let compiled = manifest.compile().unwrap();
            let expanded = crate::verification::authority::evaluation::tests::run(expand(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                compiled.authority_policy(),
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

        let denied = check(PublicDelivery::Deny);
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].subject.as_deref(), Some("public"));
        assert!(check(PublicDelivery::AllowDeclared).is_empty());
    }

    #[test]
    fn compiled_manifest_row_type_choice_overrides_type_public_defaults() {
        let mut snapshot = snapshot();
        snapshot.types.push(TypeObject {
            schema: "service".to_owned(),
            name: "records".to_owned(),
            owner: 3,
            relation_oid: 20,
            acl: std::sync::Arc::from([AclEntry {
                grantee: 0,
                privilege: ObjectPrivilege::Usage,
                grant_option: false,
            }]),
        });
        let graph = RoleGraph::new(
            &snapshot.roles,
            &snapshot.memberships,
            snapshot.root,
            snapshot.database.owner,
        );
        let check = |allow_row_type_public_usage| {
            let mut manifest = ExactRoleManifest::new(
                Identifier::new("service").unwrap(),
                DiscoveryScope::UserSchemas,
            )
            .unwrap();
            let mut defaults = DiscoveryDefaults::default();
            defaults.types.public_privileges =
                vec![AllowedPrivilege::new(ObjectPrivilege::Usage, false)];
            manifest.set_discovery_defaults(defaults).unwrap();
            manifest
                .add_relations(
                    RelationGrantGroup::new(
                        [QualifiedName::new("service", "records").unwrap()],
                        [],
                        DeclarationPurpose::AllowedOnly,
                    )
                    .unwrap()
                    .allow_row_type_public_usage(allow_row_type_public_usage),
                )
                .unwrap();
            let compiled = manifest.compile().unwrap();
            let expanded = crate::verification::authority::evaluation::tests::run(expand(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                compiled.authority_policy(),
            ))
            .unwrap();
            let mut findings = Vec::new();
            crate::verification::authority::evaluation::tests::run(inspect_types(
                &mut crate::verification::authority::evaluation::Evaluation::new(),
                &snapshot,
                &graph,
                &expanded,
                &mut findings,
            ))
            .unwrap();
            findings
        };

        let denied = check(false);
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].subject.as_deref(), Some("public"));
        assert!(check(true).is_empty());
    }
}

#[cfg(test)]
mod scale_tests;

#[cfg(test)]
mod drift_tests;
