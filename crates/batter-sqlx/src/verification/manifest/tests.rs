use super::*;
use crate::verification::{
    AllowedPrivilege, ColumnPolicy, ObjectDefaults, PublicAllowance, PublicObject,
    RequiredPrivilege, RoutineType,
};

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
fn grouping_input_order_and_exact_duplicates_normalize_identically() {
    let records = name("records");
    let events = name("events");
    let mut grouped = exact_manifest();
    grouped
        .add_relations(
            RelationGrantGroup::new(
                [records.clone(), events.clone(), records.clone()],
                [ObjectPrivilege::Update, ObjectPrivilege::Select],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap(),
        )
        .unwrap();

    let mut split = exact_manifest();
    for relation in [events, records] {
        for privilege in [ObjectPrivilege::Select, ObjectPrivilege::Update] {
            let group = RelationGrantGroup::new(
                [relation.clone()],
                [privilege],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap();
            split.add_relations(group.clone()).unwrap();
            split.add_relations(group).unwrap();
        }
    }

    assert_eq!(grouped.compile().unwrap(), split.compile().unwrap());
}

#[test]
fn mixed_purpose_on_one_target_provisions_only_required_privileges() {
    let records = name("records");
    let mut manifest = exact_manifest();
    for (privilege, purpose) in [
        (ObjectPrivilege::Update, DeclarationPurpose::AllowedOnly),
        (
            ObjectPrivilege::Select,
            DeclarationPurpose::RequiredAndProvisioned,
        ),
    ] {
        manifest
            .add_relations(
                RelationGrantGroup::new([records.clone()], [privilege], purpose).unwrap(),
            )
            .unwrap();
    }
    let compiled = manifest.compile().unwrap();
    assert_eq!(
        compiled.authority_policy().relations[0].privileges,
        vec![
            AllowedPrivilege::new(ObjectPrivilege::Select, false),
            AllowedPrivilege::new(ObjectPrivilege::Update, false),
        ]
    );
    assert_eq!(
        compiled.authority_policy().required_privileges,
        vec![RequiredPrivilege {
            object: PublicObject::Relation(records),
            privilege: ObjectPrivilege::Select,
        }]
    );
    assert_eq!(
        compiled
            .grant_plan()
            .render(&identifier("reader"), None)
            .unwrap(),
        "GRANT SELECT ON TABLE \"service\".\"records\" TO \"reader\";\n"
    );
}

#[test]
fn contradictory_atom_and_object_options_are_rejected() {
    let cases = [
        RelationGrantGroup::new(
            [name("records")],
            [ObjectPrivilege::Select],
            DeclarationPurpose::AllowedOnly,
        )
        .unwrap()
        .public_delivery(PublicDelivery::AllowDeclared),
        RelationGrantGroup::new(
            [name("records")],
            [ObjectPrivilege::Select],
            DeclarationPurpose::RequiredAndProvisioned,
        )
        .unwrap(),
        RelationGrantGroup::new(
            [name("records")],
            [ObjectPrivilege::Select],
            DeclarationPurpose::AllowedOnly,
        )
        .unwrap()
        .allow_grant_option(true),
        RelationGrantGroup::new(
            [name("records")],
            [ObjectPrivilege::Update],
            DeclarationPurpose::AllowedOnly,
        )
        .unwrap()
        .allow_owner(true),
    ];
    for conflicting in cases {
        let mut manifest = exact_manifest();
        manifest
            .add_relations(
                RelationGrantGroup::new(
                    [name("records")],
                    [ObjectPrivilege::Select],
                    DeclarationPurpose::AllowedOnly,
                )
                .unwrap(),
            )
            .unwrap();
        manifest.add_relations(conflicting).unwrap();
        assert_eq!(
            manifest.compile(),
            Err(ManifestError::ContradictoryDeclaration)
        );
    }
}

#[test]
fn compiled_policy_preserves_required_allowed_public_and_object_options() {
    let records = name("records");
    let routine = RoutineSignature::new(
        "service",
        "apply",
        [RoutineType::new("pg_catalog", "int8").unwrap()],
    )
    .unwrap();
    let mut manifest = exact_manifest();
    manifest
        .add_database(
            DatabaseGrantSpec::new([ObjectPrivilege::Connect], DeclarationPurpose::AllowedOnly)
                .unwrap()
                .public_delivery(PublicDelivery::AllowDeclared),
        )
        .unwrap();
    manifest
        .add_schema(
            SchemaGrantSpec::new(
                identifier("service"),
                [ObjectPrivilege::Usage],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap()
            .allow_grant_option(true)
            .allow_owner(true),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [records.clone()],
                [ObjectPrivilege::Select],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap()
            .allow_row_type_public_usage(true),
        )
        .unwrap();
    manifest
        .add_columns(
            ColumnGrantGroup::new(
                records.clone(),
                [identifier("state")],
                [ObjectPrivilege::Update],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_routine(
            RoutineGrantSpec::new(
                routine.clone(),
                [ObjectPrivilege::Execute],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap()
            .allow_security_definer(true),
        )
        .unwrap();

    let compiled = manifest.compile().unwrap();
    let policy = compiled.authority_policy();
    assert_eq!(
        policy.required_privileges,
        vec![
            RequiredPrivilege {
                object: PublicObject::Schema(identifier("service")),
                privilege: ObjectPrivilege::Usage,
            },
            RequiredPrivilege {
                object: PublicObject::Relation(records.clone()),
                privilege: ObjectPrivilege::Select,
            },
        ]
    );
    assert_eq!(
        policy.database.privileges,
        vec![AllowedPrivilege::new(ObjectPrivilege::Connect, false)]
    );
    assert_eq!(
        policy.schemas[0].privileges,
        vec![AllowedPrivilege::new(ObjectPrivilege::Usage, true)]
    );
    assert!(policy.schemas[0].allow_owner);
    assert!(policy.relations[0].allow_row_type_public_usage);
    assert_eq!(
        policy.relations[0].columns,
        vec![ColumnPolicy {
            column: identifier("state"),
            privileges: vec![AllowedPrivilege::new(ObjectPrivilege::Update, false)],
        }]
    );
    assert!(policy.routines[0].allow_security_definer);
    assert_eq!(policy.routines[0].routine, routine);
    assert!(policy.public_overrides.iter().any(|entry| {
        entry.object == PublicObject::Database
            && entry.privileges == vec![AllowedPrivilege::new(ObjectPrivilege::Connect, false)]
    }));
    assert_eq!(compiled.grant_plan().len(), 2);
}

#[test]
fn public_denies_override_defaults_without_conflating_each_privilege() {
    let records = name("records");
    let routine =
        RoutineSignature::new("service", "apply", std::iter::empty::<RoutineType>()).unwrap();
    let mut manifest = ExactRoleManifest::new(
        identifier("service"),
        DiscoveryScope::Schemas(vec![identifier("service")]),
    )
    .unwrap();
    manifest
        .set_discovery_defaults(permissive_public_defaults())
        .unwrap();
    manifest
        .add_schema(
            SchemaGrantSpec::new(
                identifier("service"),
                [ObjectPrivilege::Usage],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [records.clone()],
                [ObjectPrivilege::Select],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap()
            .public_delivery(PublicDelivery::AllowDeclared),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [records.clone()],
                [ObjectPrivilege::Update],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_columns(
            ColumnGrantGroup::new(
                records,
                [identifier("private")],
                [ObjectPrivilege::Update],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_routine(
            RoutineGrantSpec::new(
                routine,
                [ObjectPrivilege::Execute],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();

    let policy = manifest.compile().unwrap().authority_policy().clone();
    let relation = policy
        .public_overrides
        .iter()
        .find(|entry| matches!(entry.object, PublicObject::Relation(_)))
        .unwrap();
    assert_eq!(
        relation.privileges,
        vec![AllowedPrivilege::new(ObjectPrivilege::Select, false)]
    );
    assert!(
        policy
            .public_overrides
            .iter()
            .filter(|entry| !matches!(entry.object, PublicObject::Relation(_)))
            .all(|entry| entry.privileges.is_empty())
    );
    assert!(!policy.relations[0].allow_row_type_public_usage);
    assert!(!policy.routines[0].allow_security_definer);
    assert!(policy.validate().is_ok());
}

#[test]
fn column_public_deny_conflicts_with_parent_public_allowance() {
    let records = name("records");
    let mut manifest = exact_manifest();
    manifest
        .add_columns(
            ColumnGrantGroup::new(
                records.clone(),
                [identifier("private")],
                [ObjectPrivilege::Select],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [records],
                [ObjectPrivilege::Select],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap()
            .public_delivery(PublicDelivery::AllowDeclared),
        )
        .unwrap();

    assert_eq!(
        manifest.compile(),
        Err(ManifestError::ContradictoryDeclaration)
    );
}

#[test]
fn allowed_only_and_verification_ceilings_never_render_sql() {
    let mut manifest = exact_manifest();
    manifest
        .add_database(
            DatabaseGrantSpec::new([ObjectPrivilege::Connect], DeclarationPurpose::AllowedOnly)
                .unwrap()
                .allow_grant_option(true)
                .allow_owner(true),
        )
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [name("records")],
                [ObjectPrivilege::Select],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap()
            .allow_owner(true),
        )
        .unwrap();
    let compiled = manifest.compile().unwrap();
    assert!(compiled.grant_plan().is_empty());
    assert!(!compiled.grant_plan().requires_database_context());
    assert_eq!(
        compiled.grant_plan().render(&identifier("reader"), None),
        Ok(String::new())
    );
}

#[test]
fn allowed_only_invalid_privileges_and_invalid_defaults_fail() {
    let mut manifest = exact_manifest();
    manifest
        .add_schema(
            SchemaGrantSpec::new(
                identifier("service"),
                [ObjectPrivilege::Execute],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        manifest.compile(),
        Err(ManifestError::InvalidObjectPrivilege)
    );

    let mut defaults = DiscoveryDefaults::default();
    defaults.columns.privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Delete, false)];
    let mut manifest = exact_manifest();
    manifest.set_discovery_defaults(defaults).unwrap();
    assert_eq!(
        manifest.compile(),
        Err(ManifestError::InvalidObjectPrivilege)
    );
}

#[test]
fn every_declared_target_kind_rejects_an_invalid_allowed_only_privilege() {
    let mut database = exact_manifest();
    database
        .add_database(
            DatabaseGrantSpec::new([ObjectPrivilege::Execute], DeclarationPurpose::AllowedOnly)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        database.compile(),
        Err(ManifestError::InvalidObjectPrivilege)
    );

    let mut relation = exact_manifest();
    relation
        .add_relations(
            RelationGrantGroup::new(
                [name("records")],
                [ObjectPrivilege::Connect],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        relation.compile(),
        Err(ManifestError::InvalidObjectPrivilege)
    );

    let records = name("records");
    let mut column = exact_manifest();
    column
        .add_relations(
            RelationGrantGroup::new([records.clone()], [], DeclarationPurpose::AllowedOnly)
                .unwrap(),
        )
        .unwrap();
    column
        .add_columns(
            ColumnGrantGroup::new(
                records,
                [identifier("state")],
                [ObjectPrivilege::Delete],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(column.compile(), Err(ManifestError::InvalidObjectPrivilege));

    let mut routine = exact_manifest();
    routine
        .add_routine(
            RoutineGrantSpec::new(
                RoutineSignature::new("service", "apply", []).unwrap(),
                [ObjectPrivilege::Select],
                DeclarationPurpose::AllowedOnly,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        routine.compile(),
        Err(ManifestError::InvalidObjectPrivilege)
    );
}

#[test]
fn grouped_expansion_and_retained_input_have_hard_capacity_bounds() {
    let relations = (0..5_001)
        .map(|index| QualifiedName::new("service", format!("r{index}")).unwrap())
        .collect::<Vec<_>>();
    let group = RelationGrantGroup::new(
        relations,
        [ObjectPrivilege::Select, ObjectPrivilege::Update],
        DeclarationPurpose::RequiredAndProvisioned,
    )
    .unwrap();
    let mut manifest = exact_manifest();
    manifest.add_relations(group).unwrap();
    assert_eq!(
        manifest.compile(),
        Err(ManifestError::Policy(PolicyError::AuthorityCapacity))
    );

    assert_eq!(
        RelationGrantGroup::new(
            (0..=MAX_MANIFEST_INPUTS).map(|index| QualifiedName::new(
                "service",
                format!("r{index}")
            )
            .unwrap()),
            [],
            DeclarationPurpose::AllowedOnly,
        ),
        Err(ManifestError::Policy(PolicyError::AuthorityCapacity))
    );
}

#[test]
fn generated_policy_entries_have_an_exact_capacity_preflight() {
    let compile_relations = |count| {
        let relations = (0..count)
            .map(|index| QualifiedName::new("service", format!("r{index}")).unwrap())
            .collect::<Vec<_>>();
        let mut manifest = exact_manifest();
        manifest
            .add_relations(
                RelationGrantGroup::new(
                    relations,
                    [ObjectPrivilege::Select],
                    DeclarationPurpose::RequiredAndProvisioned,
                )
                .unwrap(),
            )
            .unwrap();
        manifest.compile()
    };

    assert!(compile_relations(2_000).is_ok());
    assert_eq!(
        compile_relations(2_001),
        Err(ManifestError::Policy(PolicyError::AuthorityCapacity))
    );
}

#[test]
fn empty_group_targets_and_overlong_identifiers_fail_before_compilation() {
    assert_eq!(
        RelationGrantGroup::new(
            [],
            [ObjectPrivilege::Select],
            DeclarationPurpose::AllowedOnly,
        ),
        Err(ManifestError::EmptyTarget)
    );
    assert_eq!(
        ColumnGrantGroup::new(
            name("records"),
            [],
            [ObjectPrivilege::Select],
            DeclarationPurpose::AllowedOnly,
        ),
        Err(ManifestError::EmptyTarget)
    );
    assert_eq!(
        Identifier::new("x".repeat(64)),
        Err(PolicyError::InvalidIdentifier)
    );
}

#[test]
fn duplicate_default_privileges_are_idempotent_but_grant_option_conflicts_fail() {
    let select = AllowedPrivilege::new(ObjectPrivilege::Select, false);
    let mut defaults = DiscoveryDefaults::default();
    defaults.relations.privileges = vec![select, select];
    let mut manifest = exact_manifest();
    manifest.set_discovery_defaults(defaults).unwrap();
    assert_eq!(
        manifest
            .compile()
            .unwrap()
            .authority_policy()
            .defaults
            .relations
            .privileges,
        vec![select]
    );

    let mut defaults = DiscoveryDefaults::default();
    defaults.relations.privileges =
        vec![select, AllowedPrivilege::new(ObjectPrivilege::Select, true)];
    let mut manifest = exact_manifest();
    manifest.set_discovery_defaults(defaults).unwrap();
    assert_eq!(
        manifest.compile(),
        Err(ManifestError::ContradictoryDeclaration)
    );
}

#[test]
fn normalized_public_overrides_include_one_exact_row_type_target() {
    let mut manifest = exact_manifest();
    for privilege in [ObjectPrivilege::Select, ObjectPrivilege::Update] {
        manifest
            .add_relations(
                RelationGrantGroup::new(
                    [name("records")],
                    [privilege],
                    DeclarationPurpose::AllowedOnly,
                )
                .unwrap()
                .public_delivery(PublicDelivery::AllowDeclared),
            )
            .unwrap();
    }
    let policy = manifest.compile().unwrap().authority_policy().clone();
    assert_eq!(
        policy.public_overrides,
        vec![
            PublicAllowance {
                object: PublicObject::Relation(name("records")),
                privileges: vec![
                    AllowedPrivilege::new(ObjectPrivilege::Select, false),
                    AllowedPrivilege::new(ObjectPrivilege::Update, false),
                ],
            },
            PublicAllowance {
                object: PublicObject::Type(name("records")),
                privileges: Vec::new(),
            },
        ]
    );
}

#[test]
fn ownership_safeguard_is_retained_outside_the_low_level_policy() {
    let mut manifest = exact_manifest();
    manifest.deny_current_database_ownership(true);
    let compiled = manifest.compile().unwrap();
    assert!(compiled.denies_current_database_ownership());
    assert!(!compiled.authority_policy().database.allow_owner);
}
