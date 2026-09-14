use super::super::super::policy::*;
use super::super::super::report::VerificationError;
use super::super::evaluation::Evaluation;
use super::super::{CatalogSnapshot, RelationObject};
use super::{DeclaredPolicies, add_public};
use std::collections::{HashMap, HashSet};

struct CatalogRelation<'a> {
    object: &'a RelationObject,
    name: QualifiedName,
    kind: PgClassKind,
}

struct CatalogRelations<'a> {
    selected: Vec<CatalogRelation<'a>>,
    kinds: HashMap<QualifiedName, PgClassKind>,
}

/// Reconcile expected relation kinds with the one kind observed in `pg_class`
/// before expanding discovery defaults for ACL evaluation.
pub(super) async fn reconcile_relations(
    evaluation: &mut Evaluation,
    snapshot: &CatalogSnapshot,
    original: &AuthorityPolicy,
    declared: &DeclaredPolicies<'_>,
    policy: &mut AuthorityPolicyBuilder,
) -> Result<(), VerificationError> {
    let catalog_relations = catalog_relations(evaluation, snapshot, original).await?;
    retain_catalog_compatible_declarations(policy, &catalog_relations.kinds);
    expand_relations(
        evaluation,
        &catalog_relations.selected,
        original,
        declared,
        policy,
    )
    .await
}

async fn catalog_relations<'a>(
    evaluation: &mut Evaluation,
    snapshot: &'a CatalogSnapshot,
    original: &AuthorityPolicy,
) -> Result<CatalogRelations<'a>, VerificationError> {
    let mut selected = Vec::new();
    let mut kinds = HashMap::new();
    for relation in &snapshot.relations {
        evaluation.checkpoint(&[]).await?;
        if !original.discovery.includes(&relation.schema) {
            continue;
        }
        let name = QualifiedName::new(relation.schema.clone(), relation.name.clone())
            .map_err(VerificationError::CatalogIdentity)?;
        let kind = if relation.kind == "S" {
            PgClassKind::Sequence
        } else {
            PgClassKind::Relation
        };
        if let Some(previous) = kinds.insert(name.clone(), kind) {
            let error = if previous == kind {
                PolicyError::DuplicateAuthorityObject
            } else {
                PolicyError::ConflictingRelationKind
            };
            return Err(VerificationError::CatalogPolicyExpansion(error));
        }
        selected.push(CatalogRelation {
            object: relation,
            name,
            kind,
        });
    }
    Ok(CatalogRelations { selected, kinds })
}

fn conflicts_with_catalog_kind(
    object: &PublicObject,
    catalog_kinds: &HashMap<QualifiedName, PgClassKind>,
) -> bool {
    object
        .pg_class_identity()
        .is_some_and(|(name, declared_kind)| {
            catalog_kinds
                .get(name)
                .is_some_and(|catalog_kind| *catalog_kind != declared_kind)
        })
}

fn retain_catalog_compatible_declarations(
    policy: &mut AuthorityPolicyBuilder,
    catalog_kinds: &HashMap<QualifiedName, PgClassKind>,
) {
    policy
        .relations
        .retain(|entry| catalog_kinds.get(&entry.relation) != Some(&PgClassKind::Sequence));
    policy
        .sequences
        .retain(|entry| catalog_kinds.get(&entry.sequence) != Some(&PgClassKind::Relation));
    policy
        .public_grants
        .retain(|entry| !conflicts_with_catalog_kind(&entry.object, catalog_kinds));
    policy
        .public_overrides
        .retain(|entry| !conflicts_with_catalog_kind(&entry.object, catalog_kinds));
    policy
        .required_privileges
        .retain(|entry| !conflicts_with_catalog_kind(&entry.object, catalog_kinds));
}

async fn expand_relations(
    evaluation: &mut Evaluation,
    relations: &[CatalogRelation<'_>],
    original: &AuthorityPolicy,
    declared: &DeclaredPolicies<'_>,
    policy: &mut AuthorityPolicyBuilder,
) -> Result<(), VerificationError> {
    let defaults = &original.defaults;
    for selected in relations {
        let relation = selected.object;
        let name = selected.name.clone();
        if selected.kind == PgClassKind::Sequence {
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
            let add_relation = !declared.relations.contains(&name);
            let mut columns = Vec::with_capacity(relation.columns.len());
            let mut catalog_columns = HashSet::with_capacity(relation.columns.len());
            for column in &relation.columns {
                evaluation.checkpoint(&[]).await?;
                let column_name = Identifier::new(column.name.clone())
                    .map_err(VerificationError::CatalogIdentity)?;
                if !catalog_columns.insert(column_name.clone()) {
                    return Err(VerificationError::CatalogPolicyExpansion(
                        PolicyError::DuplicateAuthorityObject,
                    ));
                }
                if add_relation {
                    columns.push(ColumnPolicy {
                        column: column_name.clone(),
                        privileges: defaults.columns.privileges.clone(),
                    });
                }
                add_public(
                    policy,
                    declared,
                    PublicObject::Column(name.clone(), column_name),
                    &defaults.columns,
                );
            }
            if add_relation {
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
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
