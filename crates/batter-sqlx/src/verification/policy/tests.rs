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
