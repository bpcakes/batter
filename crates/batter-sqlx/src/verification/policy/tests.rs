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
    let mut policy = VerificationPolicy::new(
        MigrationPolicy::new(ledger.clone(), vec![MigrationExpectation::new(1, vec![1])]),
        AuthorityPolicy::default(),
    );
    policy.migration.additional =
        AdditionalMigrations::AllowListed(vec![MigrationExpectation::new(1, vec![2])]);
    assert_eq!(
        policy.validate(),
        Err(PolicyError::DuplicateMigrationVersion)
    );
    assert_eq!(ledger.name(), "ledger");
}

#[test]
fn migration_policy_rejects_unbounded_expected_checksums() {
    let policy = VerificationPolicy::new(
        MigrationPolicy::new(
            QualifiedName::new("public", "ledger").unwrap(),
            vec![MigrationExpectation::new(
                1,
                vec![0; MAX_MIGRATION_CHECKSUM_BYTES + 1],
            )],
        ),
        AuthorityPolicy::default(),
    );
    assert_eq!(
        policy.validate(),
        Err(PolicyError::MigrationChecksumTooLarge)
    );
}

#[test]
fn authority_policy_rejects_duplicate_object_and_column_identities() {
    let ledger = QualifiedName::new("public", "ledger").unwrap();
    let relation = RelationPolicy {
        relation: QualifiedName::new("app", "records").unwrap(),
        privileges: Vec::new(),
        columns: Vec::new(),
        allow_owner: false,
        allow_row_type_public_usage: false,
    };
    let mut policy = VerificationPolicy::new(
        MigrationPolicy::new(ledger, Vec::new()),
        AuthorityPolicy {
            relations: vec![relation.clone(), relation],
            ..AuthorityPolicy::default()
        },
    );
    assert_eq!(
        policy.validate(),
        Err(PolicyError::DuplicateAuthorityObject)
    );

    policy.authority.relations = vec![RelationPolicy {
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
    assert_eq!(
        policy.validate(),
        Err(PolicyError::DuplicateAuthorityObject)
    );
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
    let mut authority = AuthorityPolicy {
        discovery: DiscoveryScope::UserSchemas,
        required_privileges: vec![required.clone()],
        ..AuthorityPolicy::default()
    };
    let make = |authority| {
        VerificationPolicy::new(
            MigrationPolicy::new(QualifiedName::new("service", "ledger").unwrap(), Vec::new()),
            authority,
        )
    };
    assert_eq!(
        make(authority.clone()).validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
    authority
        .defaults
        .relations
        .privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    assert_eq!(make(authority.clone()).validate(), Ok(()));
    authority.required_privileges.push(required);
    assert_eq!(
        make(authority.clone()).validate(),
        Err(PolicyError::DuplicateRequiredPrivilege)
    );
    authority.required_privileges.pop();
    authority.required_privileges[0].privilege = ObjectPrivilege::Execute;
    authority.roles.allow_superuser = true;
    assert_eq!(
        make(authority).validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

#[test]
fn exact_empty_public_allowance_rejects_a_requirement_allowed_only_by_default_public() {
    let object = PublicObject::Relation(QualifiedName::new("service", "records").unwrap());
    let mut authority = AuthorityPolicy {
        discovery: DiscoveryScope::UserSchemas,
        required_privileges: vec![RequiredPrivilege {
            object: object.clone(),
            privilege: ObjectPrivilege::Select,
        }],
        ..AuthorityPolicy::default()
    };
    authority
        .defaults
        .relations
        .public_privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    let mut policy = VerificationPolicy::new(
        MigrationPolicy::new(QualifiedName::new("service", "ledger").unwrap(), Vec::new()),
        authority,
    );
    assert_eq!(policy.validate(), Ok(()));
    policy.authority.public_overrides.push(PublicAllowance {
        object,
        privileges: Vec::new(),
    });
    assert_eq!(
        policy.validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

#[test]
fn required_column_can_use_public_relation_allowance_and_exact_role_override_keeps_public_default()
{
    let name = QualifiedName::new("service", "records").unwrap();
    let mut authority = AuthorityPolicy {
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
        ..AuthorityPolicy::default()
    };
    authority
        .defaults
        .relations
        .public_privileges
        .push(AllowedPrivilege::new(ObjectPrivilege::Select, false));
    assert_eq!(authority.validate(), Ok(()));
}

fn add_all_kind_group(policy: &mut AuthorityPolicy, ordinal: usize) {
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

fn near_capacity_all_kind_policy() -> AuthorityPolicy {
    let mut policy = AuthorityPolicy::default();
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

    const EXPECTED_LOGICAL_OPERATIONS: usize = 11_598;
    let mut policy = near_capacity_all_kind_policy();
    let operations = AtomicUsize::new(0);

    policy.validate_counted(&operations).unwrap();

    // 400 groups of eight declarations, allowances and requirements, plus
    // 398 unused declarations and two database entries fill the 10,000 budget.
    // Index 3,598 declarations. Each group makes 20 decisions: relation 2,
    // columns 4 each, sequence 2, schema 2, routine 3, type 2, parameter 1.
    // Every keyed inventory has hundreds of distinct required lookups.
    assert_eq!(
        operations.load(Ordering::Relaxed),
        EXPECTED_LOGICAL_OPERATIONS
    );
    policy.parameters.push(ParameterPolicy {
        parameter: ParameterName::new("overflow").unwrap(),
        privileges: Vec::new(),
    });
    assert_eq!(policy.validate(), Err(PolicyError::AuthorityCapacity));
    policy.parameters.pop();
    policy.relations.last_mut().unwrap().columns[1]
        .privileges
        .clear();
    assert_eq!(
        policy.validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

fn near_capacity_public_discovery_policy() -> AuthorityPolicy {
    let select = AllowedPrivilege::new(ObjectPrivilege::Select, false);
    let object = |schema: &str, ordinal| {
        PublicObject::Relation(QualifiedName::new(schema, format!("record_{ordinal}")).unwrap())
    };
    let mut policy = AuthorityPolicy {
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
        ..AuthorityPolicy::default()
    };
    policy.required_privileges.last_mut().unwrap().object = object("granted", 1_999);
    policy.defaults.relations.privileges.push(select);
    policy
}

#[test]
fn near_capacity_validation_counts_public_conflicts_and_schema_discovery() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut policy = near_capacity_public_discovery_policy();
    let operations = AtomicUsize::new(0);
    policy.validate_counted(&operations).unwrap();
    // Exactly 10,000 aggregate entries. Index 2,000 grants and 2,000 schemas;
    // probe 2,000 overrides. Each of 3,998 default-backed requirements performs
    // two discovery decisions and one relation lookup; the final PUBLIC grant
    // supplies the remaining requirement without a discovery or role lookup.
    assert_eq!(operations.load(Ordering::Relaxed), 6_000 + 3 * 3_998);

    let original = policy.public_overrides.last().unwrap().object.clone();
    policy.public_overrides.last_mut().unwrap().object =
        policy.public_grants.last().unwrap().object.clone();
    assert_eq!(
        policy.validate(),
        Err(PolicyError::DuplicateAuthorityObject)
    );
    policy.public_overrides.last_mut().unwrap().object = original;
    policy.required_privileges.last_mut().unwrap().object =
        PublicObject::Relation(QualifiedName::new("outside_scope", "records").unwrap());
    assert_eq!(
        policy.validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}

#[test]
fn large_validation_preserves_user_schema_discovery_and_system_schema_rejection() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut policy = near_capacity_public_discovery_policy();
    policy.discovery = DiscoveryScope::UserSchemas;
    let operations = AtomicUsize::new(0);
    policy.validate_counted(&operations).unwrap();
    // UserSchemas needs no schema-name index; the other decisions are identical.
    assert_eq!(operations.load(Ordering::Relaxed), 4_000 + 3 * 3_998);
    policy.required_privileges.last_mut().unwrap().object =
        PublicObject::Relation(QualifiedName::new("pg_catalog", "records").unwrap());
    assert_eq!(
        policy.validate(),
        Err(PolicyError::ContradictoryRequiredPrivilege)
    );
}
