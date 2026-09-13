use super::*;
use crate::verification::{AllowedPrivilege, ObjectDefaults, PublicObject, RoutineType};

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).unwrap()
}

fn name(value: &str) -> QualifiedName {
    QualifiedName::new("service", value).unwrap()
}

fn exact_manifest() -> ExactRoleManifest {
    ExactRoleManifest::new(identifier("service"), DiscoveryScope::Declared).unwrap()
}

fn permissive_public_defaults() -> DiscoveryDefaults {
    let public = |privilege| vec![AllowedPrivilege::new(privilege, false)];
    DiscoveryDefaults {
        schemas: ObjectDefaults {
            public_privileges: public(ObjectPrivilege::Usage),
            ..ObjectDefaults::default()
        },
        relations: ObjectDefaults {
            public_privileges: public(ObjectPrivilege::Select),
            ..ObjectDefaults::default()
        },
        columns: ObjectDefaults {
            public_privileges: public(ObjectPrivilege::Select),
            ..ObjectDefaults::default()
        },
        types: ObjectDefaults {
            public_privileges: public(ObjectPrivilege::Usage),
            ..ObjectDefaults::default()
        },
        invoker_routines: ObjectDefaults {
            public_privileges: public(ObjectPrivilege::Execute),
            ..ObjectDefaults::default()
        },
        allow_row_type_public_usage: true,
        ..DiscoveryDefaults::default()
    }
}

#[test]
fn routine_rendering_preserves_scalar_array_zero_arg_and_quoted_identity() {
    let routines = [
        RoutineSignature::new("odd\"schema", "zero\"arg", []).unwrap(),
        RoutineSignature::new(
            "odd\"schema",
            "apply",
            [
                RoutineType::new("pg_catalog", "int4").unwrap(),
                RoutineType::array("custom\"types", "value").unwrap(),
            ],
        )
        .unwrap(),
    ];
    let mut manifest = exact_manifest();
    for routine in routines {
        manifest
            .add_routine(
                RoutineGrantSpec::new(
                    routine,
                    [ObjectPrivilege::Execute],
                    DeclarationPurpose::RequiredAndProvisioned,
                )
                .unwrap(),
            )
            .unwrap();
    }
    let sql = manifest
        .compile()
        .unwrap()
        .grant_plan()
        .render(&identifier("role\"name"), None)
        .unwrap();
    assert_eq!(
        sql,
        concat!(
            "GRANT EXECUTE ON ROUTINE \"odd\"\"schema\".\"apply\"(\"pg_catalog\".\"int4\", \"custom\"\"types\".\"value\"[]) TO \"role\"\"name\";\n",
            "GRANT EXECUTE ON ROUTINE \"odd\"\"schema\".\"zero\"\"arg\"() TO \"role\"\"name\";\n",
        )
    );
}

#[test]
fn renderer_is_deterministic_quoted_and_limited_to_normalized_grants() {
    let mut manifest = exact_manifest();
    manifest
        .add_schema(
            SchemaGrantSpec::new(
                identifier("odd\"schema"),
                [ObjectPrivilege::Create, ObjectPrivilege::Usage],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [QualifiedName::new("odd\"schema", "records\"table").unwrap()],
                [ObjectPrivilege::Update, ObjectPrivilege::Select],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap()
            .allow_grant_option(true),
        )
        .unwrap();
    let compiled = manifest.compile().unwrap();
    let first = compiled
        .grant_plan()
        .render(&identifier("reader\"role"), None)
        .unwrap();
    let second = compiled
        .grant_plan()
        .render(&identifier("reader\"role"), None)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.lines().count(), 2);
    assert!(first.contains("TO \"reader\"\"role\";"));
    for forbidden in [
        "CREATE ROLE",
        "ALTER ROLE",
        "REVOKE",
        "BEGIN",
        "COMMIT",
        "PASSWORD",
        "postgres://",
        "WITH GRANT OPTION",
    ] {
        assert!(!first.contains(forbidden));
    }
}

#[test]
fn database_rendering_context_is_required_exactly_for_provisioned_database_grants() {
    let mut manifest = exact_manifest();
    manifest
        .add_database(
            DatabaseGrantSpec::new(
                [ObjectPrivilege::Connect, ObjectPrivilege::Temporary],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = manifest.compile().unwrap().grant_plan().clone();
    assert_eq!(
        plan.render(&identifier("reader"), None),
        Err(GrantRenderError::MissingDatabaseContext)
    );
    assert_eq!(
        plan.render(&identifier("reader"), Some(&identifier("service\"db")))
            .unwrap(),
        "GRANT CONNECT, TEMPORARY ON DATABASE \"service\"\"db\" TO \"reader\";\n"
    );

    let empty = exact_manifest().compile().unwrap().grant_plan().clone();
    assert_eq!(
        empty.render(&identifier("reader"), Some(&identifier("service"))),
        Err(GrantRenderError::UnexpectedDatabaseContext)
    );
}

#[test]
fn renderer_rejects_special_and_reserved_role_targets_before_output() {
    let mut manifest = exact_manifest();
    manifest
        .add_schema(
            SchemaGrantSpec::new(
                identifier("service"),
                [ObjectPrivilege::Usage],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap(),
        )
        .unwrap();
    let plan = manifest.compile().unwrap().grant_plan().clone();
    let empty = exact_manifest().compile().unwrap().grant_plan().clone();

    for role in ["public", "NONE", "pg_read_all_data"] {
        assert_eq!(
            plan.render(&identifier(role), None),
            Err(GrantRenderError::InvalidRoleTarget)
        );
        assert_eq!(
            empty.render(&identifier(role), None),
            Err(GrantRenderError::InvalidRoleTarget)
        );
    }
}

#[test]
fn provisioned_columns_render_exact_role_only_and_require_parent_policy() {
    let records = name("records");
    let column_group = || {
        ColumnGrantGroup::new(
            records.clone(),
            [identifier("state"), identifier("note")],
            [ObjectPrivilege::Update, ObjectPrivilege::Select],
            DeclarationPurpose::RequiredAndProvisioned,
        )
        .unwrap()
        .public_delivery(PublicDelivery::AllowDeclared)
    };
    let mut missing_parent = ExactRoleManifest::new(
        identifier("service"),
        DiscoveryScope::Schemas(vec![identifier("service")]),
    )
    .unwrap();
    missing_parent
        .set_discovery_defaults(permissive_public_defaults())
        .unwrap();
    missing_parent.add_columns(column_group()).unwrap();
    assert_eq!(
        missing_parent.compile(),
        Err(ManifestError::MissingRelationDeclaration)
    );

    let mut manifest = exact_manifest();
    manifest.add_columns(column_group()).unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new([records.clone()], [], DeclarationPurpose::AllowedOnly)
                .unwrap(),
        )
        .unwrap();
    let compiled = manifest.compile().unwrap();
    assert_eq!(
        compiled
            .grant_plan()
            .render(&identifier("reader"), None)
            .unwrap(),
        concat!(
            "GRANT SELECT (\"note\"), UPDATE (\"note\") ON TABLE \"service\".\"records\" TO \"reader\";\n",
            "GRANT SELECT (\"state\"), UPDATE (\"state\") ON TABLE \"service\".\"records\" TO \"reader\";\n",
        )
    );
    assert!(
        compiled
            .authority_policy()
            .public_overrides
            .iter()
            .any(
                |entry| entry.object == PublicObject::Relation(name("records"))
                    && entry.privileges.is_empty()
            )
    );
}
