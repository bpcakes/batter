use super::*;

#[test]
fn qualified_names_validate_and_quote_each_component() {
    let name = QualifiedName::new("tenant\"schema", "ledger").unwrap();
    assert_eq!(name.quoted(), "\"tenant\"\"schema\".\"ledger\"");
    assert!(Identifier::new("").is_err());
    assert!(Identifier::new("a\0b").is_err());
    assert!(Identifier::new("x".repeat(64)).is_err());
}

#[test]
fn parameter_names_normalize_postgres_ascii_case() {
    let name = ParameterName::new("Extension.Mixed_Name").unwrap();
    assert_eq!(name.as_str(), "extension.mixed_name");
    assert!(name.is_custom());
}

#[test]
fn parameter_names_follow_postgres_custom_name_rules_without_a_total_identifier_limit() {
    let long = ParameterName::new(format!("app.{}", "x".repeat(63))).unwrap();
    assert_eq!(long.as_str().len(), 67);
    assert!(long.is_custom());
    assert!(ParameterName::new("work_mem").is_ok());
    for invalid in ["", ".app", "app.", "app..value", "9app.value", "app.-value"] {
        assert_eq!(
            ParameterName::new(invalid),
            Err(PolicyError::InvalidParameterName)
        );
    }
}

#[test]
fn migration_policy_rejects_duplicate_required_or_allowlisted_versions() {
    let ledger = QualifiedName::new("public", "ledger").unwrap();
    assert_eq!(
        MigrationPolicy::new(ledger.clone(), [MigrationExpectation::new(1, [1])],)
            .unwrap()
            .with_additional(AdditionalMigrations::AllowListed(vec![
                MigrationExpectation::new(1, [2]),
            ])),
        Err(PolicyError::DuplicateMigrationVersion)
    );
    assert_eq!(ledger.name(), "ledger");
}

#[test]
fn migration_policy_bounds_required_and_allowlisted_rows_together() {
    let required = (0..MAX_MIGRATION_LEDGER_ROWS - 1)
        .map(|version| MigrationExpectation::new(i64::try_from(version).unwrap(), []));
    let policy =
        MigrationPolicy::new(QualifiedName::new("public", "ledger").unwrap(), required).unwrap();
    assert!(
        policy
            .clone()
            .with_additional(AdditionalMigrations::AllowListed(vec![
                MigrationExpectation::new(i64::try_from(MAX_MIGRATION_LEDGER_ROWS).unwrap(), []),
            ]))
            .is_ok()
    );
    assert_eq!(
        policy.with_additional(AdditionalMigrations::AllowListed(vec![
            MigrationExpectation::new(i64::try_from(MAX_MIGRATION_LEDGER_ROWS).unwrap(), []),
            MigrationExpectation::new(i64::try_from(MAX_MIGRATION_LEDGER_ROWS + 1).unwrap(), [],),
        ])),
        Err(PolicyError::AuthorityCapacity)
    );
}

#[test]
fn migration_policy_rejects_unbounded_expected_checksums() {
    assert_eq!(
        MigrationPolicy::new(
            QualifiedName::new("public", "ledger").unwrap(),
            vec![MigrationExpectation::new(
                1,
                vec![0; MAX_MIGRATION_CHECKSUM_BYTES + 1],
            )],
        ),
        Err(PolicyError::MigrationChecksumTooLarge)
    );
}

#[test]
fn authority_policy_rejects_duplicate_object_and_column_identities() {
    let relation = RelationPolicy {
        relation: QualifiedName::new("app", "records").unwrap(),
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    };
    let mut policy = AuthorityPolicyBuilder {
        relations: vec![relation.clone(), relation],
        ..AuthorityPolicyBuilder::default()
    };
    assert_eq!(
        policy.clone().build(),
        Err(PolicyError::DuplicateAuthorityObject)
    );

    policy.relations = vec![RelationPolicy {
        relation: QualifiedName::new("app", "records").unwrap(),
        privileges: Vec::new(),
        columns: vec![
            ColumnPolicy {
                column: Identifier::new("value").unwrap(),
                privileges: Vec::new(),
            },
            ColumnPolicy {
                column: Identifier::new("value").unwrap(),
                privileges: Vec::new(),
            },
        ],
        allow_owner: false,
        allow_row_type_public_usage: false,
    }];
    assert_eq!(policy.build(), Err(PolicyError::DuplicateAuthorityObject));
}

#[test]
fn authority_policy_rejects_cross_kind_pg_class_identities() {
    let name = QualifiedName::new("service", "records").unwrap();
    let relation = RelationPolicy {
        relation: name.clone(),
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    };
    let sequence = SequencePolicy {
        sequence: name.clone(),
        privileges: Vec::new(),
        allow_owner: false,
    };
    let policies = [
        AuthorityPolicyBuilder {
            relations: vec![relation],
            sequences: vec![sequence.clone()],
            ..AuthorityPolicyBuilder::default()
        },
        AuthorityPolicyBuilder {
            public_grants: vec![PublicGrant {
                object: PublicObject::Relation(name.clone()),
                privilege: AllowedPrivilege::new(ObjectPrivilege::Select, false),
            }],
            public_overrides: vec![PublicAllowance {
                object: PublicObject::Sequence(name.clone()),
                privileges: Vec::new(),
            }],
            ..AuthorityPolicyBuilder::default()
        },
        AuthorityPolicyBuilder {
            sequences: vec![sequence],
            required_privileges: vec![RequiredPrivilege {
                object: PublicObject::Column(name, Identifier::new("value").unwrap()),
                privilege: ObjectPrivilege::Select,
            }],
            ..AuthorityPolicyBuilder::default()
        },
    ];

    for policy in policies {
        assert_eq!(policy.build(), Err(PolicyError::ConflictingRelationKind));
    }
}

#[test]
fn routine_identity_requires_canonical_structural_types() {
    let signature = RoutineSignature::new(
        "a.b",
        "f",
        [RoutineType::new("pg_catalog", "int4").unwrap()],
    )
    .unwrap();
    assert_eq!(signature.schema(), "a.b");
    assert_eq!(signature.name(), "f");
    assert_eq!(signature.arguments()[0].name(), "int4");
    assert!(RoutineType::array("pg_catalog", "int4").unwrap().is_array());
    assert_eq!(signature.as_str(), "\"a.b\".\"f\"(\"pg_catalog\".\"int4\")");
}

#[test]
fn requirements_reject_duplicates_contradictions_and_invalid_object_privileges() {
    let object = PublicObject::Relation(QualifiedName::new("service", "records").unwrap());
    let required = RequiredPrivilege {
        object: object.clone(),
        privilege: ObjectPrivilege::Select,
    };
    let mut authority = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::UserSchemas,
        required_privileges: vec![required.clone()],
        ..AuthorityPolicyBuilder::default()
    };
    assert_eq!(
        authority.clone().build(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
    authority
        .defaults
        .relations
        .privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    assert!(authority.clone().build().is_ok());
    authority.required_privileges.push(required);
    assert_eq!(
        authority.clone().build(),
        Err(PolicyError::DuplicateRequiredPrivilege)
    );
    authority.required_privileges.pop();
    authority.required_privileges[0].privilege = ObjectPrivilege::Execute;
    authority.roles.allow_superuser = true;
    assert_eq!(authority.build(), Err(PolicyError::InvalidObjectPrivilege));
}

#[test]
fn every_authority_object_accepts_only_its_postgres_privilege_set() {
    use ObjectPrivilege::*;

    let name = QualifiedName::new("service", "records").unwrap();
    let routine = RoutineSignature::new("service", "perform", []).unwrap();
    let cases: Vec<(PublicObject, &[ObjectPrivilege])> = vec![
        (
            PublicObject::Relation(name.clone()),
            &[
                Select, Insert, Update, Delete, Truncate, References, Trigger, Maintain,
            ],
        ),
        (
            PublicObject::Column(name.clone(), Identifier::new("value").unwrap()),
            &[Select, Insert, Update, References],
        ),
        (
            PublicObject::Sequence(name.clone()),
            &[Usage, Select, Update],
        ),
        (
            PublicObject::Schema(Identifier::new("service").unwrap()),
            &[Usage, Create],
        ),
        (PublicObject::Routine(routine), &[Execute]),
        (PublicObject::Type(name), &[Usage]),
        (
            PublicObject::Parameter(ParameterName::new("work_mem").unwrap()),
            &[Set, AlterSystem],
        ),
        (PublicObject::Database, &[Connect, Create, Temporary]),
    ];
    let all = [
        Select,
        Insert,
        Update,
        Delete,
        Truncate,
        References,
        Trigger,
        Maintain,
        Usage,
        Create,
        Connect,
        Temporary,
        Execute,
        Set,
        AlterSystem,
    ];

    for (object, allowed) in cases {
        for privilege in all {
            let result = AuthorityPolicyBuilder {
                public_grants: vec![PublicGrant {
                    object: object.clone(),
                    privilege: AllowedPrivilege::new(privilege, false),
                }],
                ..AuthorityPolicyBuilder::default()
            }
            .build();
            assert_eq!(
                result.is_ok(),
                allowed.contains(&privilege),
                "unexpected {privilege:?} result for {object:?}"
            );
        }
    }
}

#[test]
fn generic_authority_duplicates_normalize_or_reject_one_meaning() {
    let select = AllowedPrivilege::new(ObjectPrivilege::Select, false);
    let relation = QualifiedName::new("service", "records").unwrap();
    let normalized = AuthorityPolicyBuilder {
        relations: vec![RelationPolicy {
            relation: relation.clone(),
            privileges: vec![select, select],
            columns: Vec::new(),
            allow_owner: false,
            allow_row_type_public_usage: false,
        }],
        public_grants: vec![
            PublicGrant {
                object: PublicObject::Relation(relation.clone()),
                privilege: select,
            },
            PublicGrant {
                object: PublicObject::Relation(relation.clone()),
                privilege: select,
            },
        ],
        ..AuthorityPolicyBuilder::default()
    }
    .build()
    .unwrap();
    assert_eq!(normalized.relations()[0].privileges, [select]);
    assert_eq!(normalized.public_grants().len(), 1);

    let conflicting = AuthorityPolicyBuilder {
        relations: vec![RelationPolicy {
            relation: relation.clone(),
            privileges: vec![select, AllowedPrivilege::new(ObjectPrivilege::Select, true)],
            columns: Vec::new(),
            allow_owner: false,
            allow_row_type_public_usage: false,
        }],
        ..AuthorityPolicyBuilder::default()
    };
    assert_eq!(
        conflicting.build(),
        Err(PolicyError::ContradictoryAuthorityPrivilege)
    );

    let conflicting_public = AuthorityPolicyBuilder {
        public_grants: vec![
            PublicGrant {
                object: PublicObject::Relation(relation.clone()),
                privilege: select,
            },
            PublicGrant {
                object: PublicObject::Relation(relation),
                privilege: AllowedPrivilege::new(ObjectPrivilege::Select, true),
            },
        ],
        ..AuthorityPolicyBuilder::default()
    };
    assert_eq!(
        conflicting_public.build(),
        Err(PolicyError::ContradictoryAuthorityPrivilege)
    );
}

#[test]
fn set_like_inputs_normalize_while_exact_public_identities_reject_duplicates() {
    let schema = Identifier::new("service").unwrap();
    let role = Identifier::new("service_operator").unwrap();
    let normalized = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::Schemas(vec![schema.clone(), schema]),
        roles: RolePolicy {
            allowed_admin_roles: vec![role.clone(), role.clone()],
            allowed_predefined_roles: vec![role.clone(), role],
            ..RolePolicy::default()
        },
        required_surfaces: vec![
            RequiredSurface::SecurityDefinerBody,
            RequiredSurface::SecurityDefinerBody,
        ],
        ..AuthorityPolicyBuilder::default()
    }
    .build()
    .unwrap();
    assert!(matches!(
        normalized.discovery(),
        DiscoveryScope::Schemas(schemas) if schemas.len() == 1
    ));
    assert_eq!(normalized.roles().allowed_admin_roles.len(), 1);
    assert_eq!(normalized.roles().allowed_predefined_roles.len(), 1);
    assert_eq!(normalized.required_surfaces().len(), 1);

    let object = PublicObject::Relation(QualifiedName::new("service", "records").unwrap());
    let duplicate = AuthorityPolicyBuilder {
        public_overrides: vec![
            PublicAllowance {
                object: object.clone(),
                privileges: Vec::new(),
            },
            PublicAllowance {
                object,
                privileges: Vec::new(),
            },
        ],
        ..AuthorityPolicyBuilder::default()
    };
    assert_eq!(
        duplicate.build(),
        Err(PolicyError::DuplicateAuthorityObject)
    );
}

#[test]
fn exact_empty_public_allowance_rejects_a_requirement_allowed_only_by_default_public() {
    let object = PublicObject::Relation(QualifiedName::new("service", "records").unwrap());
    let mut authority = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::UserSchemas,
        required_privileges: vec![RequiredPrivilege {
            object: object.clone(),
            privilege: ObjectPrivilege::Select,
        }],
        ..AuthorityPolicyBuilder::default()
    };
    authority
        .defaults
        .relations
        .public_privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    assert!(authority.clone().build().is_ok());
    authority.public_overrides.push(PublicAllowance {
        object,
        privileges: Vec::new(),
    });
    assert_eq!(
        authority.build(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

#[test]
fn required_column_can_use_public_relation_allowance_and_exact_role_override_keeps_public_default()
{
    let name = QualifiedName::new("service", "records").unwrap();
    let mut authority = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::UserSchemas,
        relations: vec![RelationPolicy {
            relation: name.clone(),
            privileges: Vec::new(),
            columns: Vec::new(),
            allow_owner: false,
            allow_row_type_public_usage: false,
        }],
        required_privileges: vec![RequiredPrivilege {
            object: PublicObject::Column(name, Identifier::new("value").unwrap()),
            privilege: ObjectPrivilege::Select,
        }],
        ..AuthorityPolicyBuilder::default()
    };
    authority
        .defaults
        .relations
        .public_privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    assert!(authority.build().is_ok());
}

fn add_all_kind_group(policy: &mut AuthorityPolicyBuilder, ordinal: usize) {
    let allowed = |privilege| vec![AllowedPrivilege::new(privilege, false)];
    let relation = QualifiedName::new("service", format!("records_{ordinal}")).unwrap();
    let columns = [
        Identifier::new("first").unwrap(),
        Identifier::new("second").unwrap(),
    ];
    let sequence = QualifiedName::new("service", format!("counter_{ordinal}")).unwrap();
    let schema = Identifier::new(format!("schema_{ordinal}")).unwrap();
    let routine = RoutineSignature::new(
        "service",
        format!("perform_{ordinal}"),
        std::iter::empty::<RoutineType>(),
    )
    .unwrap();
    let type_name = QualifiedName::new("service", format!("payload_{ordinal}")).unwrap();
    let parameter = ParameterName::new(format!("parameter_{ordinal}")).unwrap();
    for (object, privilege) in [
        (
            PublicObject::Relation(relation.clone()),
            ObjectPrivilege::Delete,
        ),
        (
            PublicObject::Column(relation.clone(), columns[0].clone()),
            ObjectPrivilege::Select,
        ),
        (
            PublicObject::Column(relation.clone(), columns[1].clone()),
            ObjectPrivilege::Select,
        ),
        (
            PublicObject::Sequence(sequence.clone()),
            ObjectPrivilege::Usage,
        ),
        (PublicObject::Schema(schema.clone()), ObjectPrivilege::Usage),
        (
            PublicObject::Routine(routine.clone()),
            ObjectPrivilege::Execute,
        ),
        (
            PublicObject::Type(type_name.clone()),
            ObjectPrivilege::Usage,
        ),
        (
            PublicObject::Parameter(parameter.clone()),
            ObjectPrivilege::Set,
        ),
    ] {
        policy
            .required_privileges
            .push(RequiredPrivilege { object, privilege });
    }
    policy.relations.push(RelationPolicy {
        relation,
        // DELETE cannot satisfy the column SELECT requirements.
        privileges: allowed(ObjectPrivilege::Delete),
        columns: columns
            .into_iter()
            .map(|column| ColumnPolicy {
                column,
                privileges: allowed(ObjectPrivilege::Select),
            })
            .collect(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    });
    policy.sequences.push(SequencePolicy {
        sequence,
        privileges: allowed(ObjectPrivilege::Usage),
        allow_owner: false,
    });
    policy.schemas.push(SchemaPolicy {
        schema,
        privileges: allowed(ObjectPrivilege::Usage),
        allow_owner: false,
    });
    policy.routines.push(RoutinePolicy {
        routine,
        privileges: allowed(ObjectPrivilege::Execute),
        allow_owner: false,
        allow_security_definer: false,
    });
    policy.types.push(TypePolicy {
        type_name,
        privileges: allowed(ObjectPrivilege::Usage),
        allow_owner: false,
    });
    policy.parameters.push(ParameterPolicy {
        parameter,
        privileges: allowed(ObjectPrivilege::Set),
    });
}

fn near_capacity_all_kind_policy() -> AuthorityPolicyBuilder {
    let mut policy = AuthorityPolicyBuilder::default();
    for ordinal in 0..400 {
        add_all_kind_group(&mut policy, ordinal);
    }
    policy
        .parameters
        .extend((0..398).map(|ordinal| ParameterPolicy {
            parameter: ParameterName::new(format!("unused_{ordinal}")).unwrap(),
            privileges: Vec::new(),
        }));
    policy.database.privileges = vec![AllowedPrivilege::new(ObjectPrivilege::Connect, false)];
    policy.required_privileges.push(RequiredPrivilege {
        object: PublicObject::Database,
        privilege: ObjectPrivilege::Connect,
    });
    policy
}

#[test]
fn near_capacity_validation_indexes_every_declared_object_kind_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const EXPECTED_LOGICAL_OPERATIONS: usize = 13_998;
    let policy = near_capacity_all_kind_policy().build().unwrap();
    let operations = AtomicUsize::new(0);

    policy.validate_counted(&operations).unwrap();

    // 400 groups of eight declarations, allowances and requirements, plus
    // 398 unused declarations and two database entries fill the 10,000 budget.
    // Check 2,400 relation-kind identities and index 3,598 declarations. Each
    // group makes 20 decisions: relation 2,
    // columns 4 each, sequence 2, schema 2, routine 3, type 2, parameter 1.
    // Every keyed inventory has hundreds of distinct required lookups.
    assert_eq!(
        operations.load(Ordering::Relaxed),
        EXPECTED_LOGICAL_OPERATIONS
    );
    let mut overflow = policy.to_builder();
    overflow.parameters.push(ParameterPolicy {
        parameter: ParameterName::new("overflow").unwrap(),
        privileges: Vec::new(),
    });
    assert_eq!(overflow.build(), Err(PolicyError::AuthorityCapacity));
    let mut denied = policy.to_builder();
    denied.relations.last_mut().unwrap().columns[1]
        .privileges
        .clear();
    assert_eq!(
        denied.build(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

fn near_capacity_public_discovery_policy() -> AuthorityPolicyBuilder {
    let select = AllowedPrivilege::new(ObjectPrivilege::Select, false);
    let object = |schema: &str, ordinal| {
        PublicObject::Relation(QualifiedName::new(schema, format!("record_{ordinal}")).unwrap())
    };
    let mut policy = AuthorityPolicyBuilder {
        discovery: DiscoveryScope::Schemas(
            (0..2_000)
                .map(|ordinal| Identifier::new(format!("schema_{ordinal}")).unwrap())
                .collect(),
        ),
        public_grants: (0..2_000)
            .map(|ordinal| PublicGrant {
                object: object("granted", ordinal),
                privilege: select,
            })
            .collect(),
        public_overrides: (0..2_000)
            .map(|ordinal| PublicAllowance {
                object: object("denied", ordinal),
                privileges: Vec::new(),
            })
            .collect(),
        required_privileges: (0..3_999)
            .map(|ordinal| RequiredPrivilege {
                object: object(&format!("schema_{}", ordinal % 2_000), ordinal),
                privilege: ObjectPrivilege::Select,
            })
            .collect(),
        ..AuthorityPolicyBuilder::default()
    };
    policy.required_privileges.last_mut().unwrap().object = object("granted", 1_999);
    policy.defaults.relations.privileges.push(select);
    policy
}

#[test]
fn near_capacity_validation_counts_public_conflicts_and_schema_discovery() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let policy = near_capacity_public_discovery_policy().build().unwrap();
    let operations = AtomicUsize::new(0);
    policy.validate_counted(&operations).unwrap();
    // Exactly 10,000 aggregate entries. Index 2,000 grants and 2,000 schemas;
    // probe 2,000 overrides. Each of 3,998 default-backed requirements performs
    // two discovery decisions and one relation lookup; the final PUBLIC grant
    // supplies the remaining requirement without a discovery or role lookup.
    assert_eq!(operations.load(Ordering::Relaxed), 13_999 + 3 * 3_998);

    let mut duplicate = policy.to_builder();
    duplicate.public_overrides.last_mut().unwrap().object =
        duplicate.public_grants.last().unwrap().object.clone();
    assert_eq!(
        duplicate.build(),
        Err(PolicyError::DuplicateAuthorityObject)
    );
    let mut outside = policy.to_builder();
    outside.required_privileges.last_mut().unwrap().object =
        PublicObject::Relation(QualifiedName::new("outside_scope", "records").unwrap());
    assert_eq!(
        outside.build(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

#[test]
fn large_validation_preserves_user_schema_discovery_and_system_schema_rejection() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut draft = near_capacity_public_discovery_policy();
    draft.discovery = DiscoveryScope::UserSchemas;
    let policy = draft.build().unwrap();
    let operations = AtomicUsize::new(0);
    policy.validate_counted(&operations).unwrap();
    // UserSchemas needs no schema-name index; the other decisions are identical.
    assert_eq!(operations.load(Ordering::Relaxed), 11_999 + 3 * 3_998);
    let mut rejected = policy.to_builder();
    rejected.required_privileges.last_mut().unwrap().object =
        PublicObject::Relation(QualifiedName::new("pg_catalog", "records").unwrap());
    assert_eq!(
        rejected.build(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}
