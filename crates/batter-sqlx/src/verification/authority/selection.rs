use super::super::policy::{AuthorityPolicy, PublicObject};
use std::collections::HashSet;

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct CatalogSelection {
    pub(super) schemas: Vec<String>,
    pub(super) relations: Vec<(String, String)>,
    pub(super) routines: Vec<(String, String)>,
    pub(super) types: Vec<(String, String)>,
    pub(super) parameters: Vec<String>,
}

impl CatalogSelection {
    pub(super) fn from_policy(policy: &AuthorityPolicy) -> Self {
        let mut schemas = policy
            .schemas
            .iter()
            .map(|entry| entry.schema.as_str().to_owned())
            .collect::<HashSet<_>>();
        let mut relations = policy
            .relations
            .iter()
            .map(|entry| {
                (
                    entry.relation.schema().to_owned(),
                    entry.relation.name().to_owned(),
                )
            })
            .chain(policy.sequences.iter().map(|entry| {
                (
                    entry.sequence.schema().to_owned(),
                    entry.sequence.name().to_owned(),
                )
            }))
            .collect::<HashSet<_>>();
        let mut routines = policy
            .routines
            .iter()
            .map(|entry| {
                (
                    entry.routine.schema().to_owned(),
                    entry.routine.name().to_owned(),
                )
            })
            .collect::<HashSet<_>>();
        let mut types = policy
            .types
            .iter()
            .map(|entry| {
                (
                    entry.type_name.schema().to_owned(),
                    entry.type_name.name().to_owned(),
                )
            })
            .collect::<HashSet<_>>();
        let mut parameters = policy
            .parameters
            .iter()
            .map(|entry| entry.parameter.as_str().to_owned())
            .collect::<HashSet<_>>();

        for object in policy
            .public_grants
            .iter()
            .map(|grant| &grant.object)
            .chain(policy.public_overrides.iter().map(|entry| &entry.object))
            .chain(
                policy
                    .required_privileges
                    .iter()
                    .map(|required| &required.object),
            )
        {
            match object {
                PublicObject::Relation(name)
                | PublicObject::Column(name, _)
                | PublicObject::Sequence(name) => {
                    relations.insert((name.schema().to_owned(), name.name().to_owned()));
                }
                PublicObject::Schema(name) => {
                    schemas.insert(name.as_str().to_owned());
                }
                PublicObject::Routine(signature) => {
                    routines.insert((signature.schema().to_owned(), signature.name().to_owned()));
                }
                PublicObject::Type(name) => {
                    types.insert((name.schema().to_owned(), name.name().to_owned()));
                }
                PublicObject::Parameter(name) => {
                    parameters.insert(name.as_str().to_owned());
                }
                PublicObject::Database => {}
            }
        }

        let mut selection = Self {
            schemas: schemas.into_iter().collect(),
            relations: relations.into_iter().collect(),
            routines: routines.into_iter().collect(),
            types: types.into_iter().collect(),
            parameters: parameters.into_iter().collect(),
        };
        selection.schemas.sort();
        selection.relations.sort();
        selection.routines.sort();
        selection.types.sort();
        selection.parameters.sort();
        selection
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::{
        AllowedPrivilege, ColumnPolicy, Identifier, ObjectPrivilege, ParameterName, PublicGrant,
        QualifiedName, RelationPolicy,
    };

    #[test]
    fn selection_is_limited_to_declared_and_public_targets() {
        let relation = QualifiedName::new("selected", "records").unwrap();
        let public_relation = QualifiedName::new("selected", "visible").unwrap();
        let parameter = ParameterName::new("work_mem").unwrap();
        let policy = AuthorityPolicy {
            relations: vec![RelationPolicy {
                relation: relation.clone(),
                privileges: Vec::new(),
                columns: vec![ColumnPolicy {
                    column: Identifier::new("value").unwrap(),
                    privileges: Vec::new(),
                }],
                allow_owner: false,
                allow_row_type_public_usage: false,
            }],
            public_grants: vec![
                PublicGrant {
                    object: PublicObject::Column(
                        public_relation.clone(),
                        Identifier::new("value").unwrap(),
                    ),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
                },
                PublicGrant {
                    object: PublicObject::Parameter(parameter.clone()),
                    privilege: AllowedPrivilege::new(ObjectPrivilege::Set, false),
                },
            ],
            ..AuthorityPolicy::default()
        };

        let selected = CatalogSelection::from_policy(&policy);
        assert_eq!(
            selected.relations,
            vec![
                ("selected".to_owned(), "records".to_owned()),
                ("selected".to_owned(), "visible".to_owned()),
            ]
        );
        assert_eq!(selected.parameters, vec!["work_mem"]);
        assert!(selected.schemas.is_empty());
        assert!(selected.routines.is_empty());
        assert!(selected.types.is_empty());
    }
}
