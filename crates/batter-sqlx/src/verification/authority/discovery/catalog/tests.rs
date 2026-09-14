use super::super::expand;
use crate::verification::authority::required::tests::snapshot;
use crate::verification::authority::{
    CatalogSnapshot, ColumnObject, RoutineObject, SchemaObject, TypeObject,
};
use crate::verification::report::VerificationError;
use crate::verification::{AuthorityPolicyBuilder, DiscoveryScope, PolicyError};

fn expansion_error(snapshot: &CatalogSnapshot) -> VerificationError {
    let policy = AuthorityPolicyBuilder::new(DiscoveryScope::UserSchemas)
        .build()
        .unwrap();
    crate::verification::authority::evaluation::tests::run(expand(
        &mut crate::verification::authority::evaluation::Evaluation::new(),
        snapshot,
        &policy,
    ))
    .unwrap_err()
}

#[test]
fn every_catalog_identity_is_checked_during_checkpointed_expansion() {
    let mut relation = snapshot();
    relation.relations.push(relation.relations[0].clone());

    let mut column = snapshot();
    let duplicate_column = ColumnObject {
        name: "value".to_owned(),
        acl: Vec::new(),
    };
    column.relations[0].columns = vec![duplicate_column.clone(), duplicate_column];

    let mut schema = snapshot();
    let duplicate_schema = SchemaObject {
        name: "service".to_owned(),
        owner: 3,
        acl: Vec::new(),
    };
    schema.schemas = vec![duplicate_schema.clone(), duplicate_schema];

    let mut type_snapshot = snapshot();
    let duplicate_type = TypeObject {
        schema: "service".to_owned(),
        name: "kind".to_owned(),
        owner: 3,
        relation_oid: 0,
        acl: std::sync::Arc::from([]),
    };
    type_snapshot.types = vec![duplicate_type.clone(), duplicate_type];

    let mut routine = snapshot();
    let duplicate_routine = RoutineObject {
        schema: "service".to_owned(),
        name: "perform".to_owned(),
        owner: 3,
        security_definer: false,
        argument_types: Vec::new(),
        acl: Vec::new(),
    };
    routine.routines = vec![duplicate_routine.clone(), duplicate_routine];

    for malformed in [relation, column, schema, type_snapshot, routine] {
        assert!(matches!(
            expansion_error(&malformed),
            VerificationError::CatalogPolicyExpansion(PolicyError::DuplicateAuthorityObject)
        ));
    }
}
