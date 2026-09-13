use super::{AuthorityPolicy, DiscoveryScope, PublicObject, RoutineSignature};

/// PostgreSQL reserves the pg_ prefix. Session namespaces and the pg_temp
/// lookup alias have a different authority model from ordinary namespace ACLs.
/// Reject the whole requested inspection rather than silently dropping them.
pub(crate) fn temporary_namespace(schema: &str) -> bool {
    schema == "pg_temp" || schema.starts_with("pg_temp_") || schema.starts_with("pg_toast_temp_")
}

fn temporary_routine(routine: &RoutineSignature) -> bool {
    temporary_namespace(routine.schema())
        || routine
            .arguments()
            .iter()
            .any(|ty| temporary_namespace(ty.schema()))
}

fn temporary_object(object: &PublicObject) -> bool {
    match object {
        PublicObject::Schema(name) => temporary_namespace(name.as_str()),
        PublicObject::Relation(name)
        | PublicObject::Column(name, _)
        | PublicObject::Sequence(name)
        | PublicObject::Type(name) => temporary_namespace(name.schema()),
        PublicObject::Routine(routine) => temporary_routine(routine),
        PublicObject::Database | PublicObject::Parameter(_) => false,
    }
}

impl AuthorityPolicy {
    pub(crate) fn requests_temporary_namespace(&self) -> bool {
        matches!(&self.discovery, DiscoveryScope::Schemas(names) if names.iter().any(|name| temporary_namespace(name.as_str())))
            || self
                .schemas
                .iter()
                .any(|entry| temporary_namespace(entry.schema.as_str()))
            || self
                .relations
                .iter()
                .any(|entry| temporary_namespace(entry.relation.schema()))
            || self
                .sequences
                .iter()
                .any(|entry| temporary_namespace(entry.sequence.schema()))
            || self
                .types
                .iter()
                .any(|entry| temporary_namespace(entry.type_name.schema()))
            || self
                .routines
                .iter()
                .any(|entry| temporary_routine(&entry.routine))
            || self
                .public_grants
                .iter()
                .any(|entry| temporary_object(&entry.object))
            || self
                .public_overrides
                .iter()
                .any(|entry| temporary_object(&entry.object))
            || self
                .required_privileges
                .iter()
                .any(|entry| temporary_object(&entry.object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verification::{Identifier, PublicAllowance, QualifiedName, RoutineType};

    #[test]
    fn temporary_namespace_boundary_covers_aliases_objects_and_routine_dependencies() {
        for schema in ["pg_temp", "pg_temp_3", "pg_toast_temp_3"] {
            assert!(temporary_namespace(schema));
            let mut policy = AuthorityPolicy {
                discovery: DiscoveryScope::Schemas(vec![Identifier::new(schema).unwrap()]),
                ..AuthorityPolicy::default()
            };
            assert!(policy.requests_temporary_namespace());
            policy.discovery = DiscoveryScope::Declared;
            for object in [
                PublicObject::Schema(Identifier::new(schema).unwrap()),
                PublicObject::Relation(QualifiedName::new(schema, "records").unwrap()),
                PublicObject::Column(
                    QualifiedName::new(schema, "records").unwrap(),
                    Identifier::new("id").unwrap(),
                ),
                PublicObject::Sequence(QualifiedName::new(schema, "counter").unwrap()),
                PublicObject::Type(QualifiedName::new(schema, "value").unwrap()),
                PublicObject::Routine(
                    RoutineSignature::new(
                        "service",
                        "accept",
                        [RoutineType::new(schema, "value").unwrap()],
                    )
                    .unwrap(),
                ),
            ] {
                policy.public_overrides = vec![PublicAllowance {
                    object,
                    privileges: Vec::new(),
                }];
                assert!(policy.requests_temporary_namespace());
            }
        }
        for schema in [
            "public",
            "pg_catalog",
            "information_schema",
            "PG_TEMP",
            "service.pg_temp",
        ] {
            assert!(!temporary_namespace(schema));
        }
        assert!(
            !AuthorityPolicy {
                discovery: DiscoveryScope::UserSchemas,
                ..AuthorityPolicy::default()
            }
            .requests_temporary_namespace()
        );
    }
}
