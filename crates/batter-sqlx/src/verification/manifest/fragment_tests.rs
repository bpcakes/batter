use super::*;
use crate::verification::{
    DiscoveryScope, FragmentObjectPolicy, PublicObject, RoutineSignature, RoutineType,
};

fn identifier(value: impl Into<String>) -> Identifier {
    Identifier::new(value).unwrap()
}

fn name(value: &str) -> QualifiedName {
    QualifiedName::new("service", value).unwrap()
}

fn manifest() -> ExactRoleManifest {
    ExactRoleManifest::new(identifier("service"), DiscoveryScope::Declared).unwrap()
}

fn required(privileges: impl IntoIterator<Item = ObjectPrivilege>) -> RelationGrantGroup {
    RelationGrantGroup::new(
        [name("ledger")],
        privileges,
        DeclarationPurpose::RequiredAndProvisioned,
    )
    .unwrap()
}

fn ledger_columns(
    columns: impl IntoIterator<Item = &'static str>,
    privileges: impl IntoIterator<Item = ObjectPrivilege>,
) -> ColumnGrantGroup {
    ColumnGrantGroup::new(
        name("ledger"),
        columns.into_iter().map(identifier),
        privileges,
        DeclarationPurpose::RequiredAndProvisioned,
    )
    .unwrap()
}

fn schema_usage() -> SchemaGrantSpec {
    SchemaGrantSpec::new(
        identifier("service"),
        [ObjectPrivilege::Usage],
        DeclarationPurpose::RequiredAndProvisioned,
    )
    .unwrap()
}

#[test]
fn fragment_column_group_requires_its_own_parent_relation() {
    let orphan =
        GrantFragment::new().with_columns(ledger_columns(["id"], [ObjectPrivilege::Select]));
    assert_eq!(
        orphan.unwrap_err(),
        ManifestError::MissingRelationDeclaration
    );

    let parented = GrantFragment::new()
        .with_relations(required([]))
        .unwrap()
        .with_columns(ledger_columns(["id"], [ObjectPrivilege::Select]));
    assert!(parented.is_ok());
}

#[test]
fn fragment_order_and_duplication_compile_to_one_deterministic_plan() {
    let schema = Identifier::new("service_reader").unwrap();
    let one = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_schema(schema_usage())
                .unwrap()
                .with_relations(required([ObjectPrivilege::Select]))
                .unwrap()
                .with_columns(ledger_columns(["id", "state"], [ObjectPrivilege::Update]))
                .unwrap(),
        )
        .unwrap()
        .compile()
        .unwrap();
    let repeated = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_relations(required([ObjectPrivilege::Select]))
                .unwrap()
                .with_columns(ledger_columns(["state"], [ObjectPrivilege::Update]))
                .unwrap()
                .with_columns(ledger_columns(["id"], [ObjectPrivilege::Update]))
                .unwrap(),
        )
        .unwrap()
        .with_fragment(
            GrantFragment::new()
                .with_relations(required([ObjectPrivilege::Select]))
                .unwrap()
                .with_schema(schema_usage())
                .unwrap(),
        )
        .unwrap()
        .compile()
        .unwrap();

    assert_eq!(one, repeated);
    assert_eq!(
        one.grant_plan().render(&schema, None).unwrap(),
        repeated.grant_plan().render(&schema, None).unwrap(),
    );
}

#[test]
fn fragments_never_carry_application_database_discovery_or_role_policy() {
    let fragment = GrantFragment::new()
        .with_relations(required([ObjectPrivilege::Select]))
        .unwrap();
    let plain = manifest()
        .with_fragment(fragment.clone())
        .unwrap()
        .compile()
        .unwrap();

    let mut narrowed = ExactRoleManifest::new(
        identifier("service"),
        DiscoveryScope::Schemas(vec![identifier("service")]),
    )
    .unwrap();
    narrowed
        .set_role_policy(RolePolicy {
            allow_superuser: false,
            ..RolePolicy::default()
        })
        .unwrap();
    narrowed.deny_current_database_ownership(true);
    narrowed
        .add_database(
            DatabaseGrantSpec::new(
                [ObjectPrivilege::Connect],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap(),
        )
        .unwrap();
    let composed = narrowed.with_fragment(fragment).unwrap().compile().unwrap();

    // The fragment contributed the same relation grant to both manifests and
    // changed neither application policy.
    assert_eq!(plain.grant_plan().len(), 1);
    assert!(!plain.grant_plan().requires_database_context());
    assert!(composed.grant_plan().requires_database_context());
    assert!(composed.denies_current_database_ownership());
    assert!(!plain.denies_current_database_ownership());
}

#[test]
fn contradictory_fragment_and_application_options_fail_before_any_plan_exists() {
    let owned = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_relations(required([ObjectPrivilege::Select]).allow_owner(true))
                .unwrap(),
        )
        .unwrap()
        .with_fragment(
            GrantFragment::new()
                .with_relations(required([ObjectPrivilege::Select]))
                .unwrap(),
        )
        .unwrap()
        .compile();
    assert_eq!(owned.unwrap_err(), ManifestError::ContradictoryDeclaration);

    let public = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_relations(
                    required([ObjectPrivilege::Select])
                        .public_delivery(PublicDelivery::AllowDeclared),
                )
                .unwrap()
                .with_columns(ledger_columns(["id"], [ObjectPrivilege::Select]))
                .unwrap(),
        )
        .unwrap()
        .compile();
    assert_eq!(public.unwrap_err(), ManifestError::ContradictoryDeclaration);

    let row_type = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_relations(required([]).allow_row_type_public_usage(true))
                .unwrap(),
        )
        .unwrap()
        .with_fragment(GrantFragment::new().with_relations(required([])).unwrap())
        .unwrap()
        .compile();
    assert_eq!(
        row_type.unwrap_err(),
        ManifestError::ContradictoryDeclaration
    );
}

#[test]
fn fragment_capacity_overflow_retains_no_partial_declaration() {
    let bounded = GrantFragment::new()
        .with_relations(required([ObjectPrivilege::Select]))
        .unwrap()
        .with_columns(ledger_columns(["id"], [ObjectPrivilege::Update]))
        .unwrap();
    let oversized = ColumnGrantGroup::new(
        name("ledger"),
        (0..MAX_MANIFEST_INPUTS).map(|index| identifier(format!("c{index}"))),
        [ObjectPrivilege::Select],
        DeclarationPurpose::RequiredAndProvisioned,
    )
    .unwrap();

    assert_eq!(
        bounded.clone().with_columns(oversized.clone()).unwrap_err(),
        ManifestError::Policy(PolicyError::AuthorityCapacity),
    );
    // Rejecting the append consumed the attempt, not the fragment, so the
    // retained declarations still compile and render on their own.
    let compiled = manifest()
        .with_fragment(bounded)
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(compiled.grant_plan().len(), 2);

    let half = MAX_MANIFEST_INPUTS / 2;
    let bulk = |prefix: char| {
        ColumnGrantGroup::new(
            name("ledger"),
            (0..half).map(|index| identifier(format!("{prefix}{index}"))),
            [ObjectPrivilege::Select],
            DeclarationPurpose::RequiredAndProvisioned,
        )
        .unwrap()
    };
    let mut populated = manifest();
    populated.add_relations(required([])).unwrap();
    populated.add_columns(bulk('a')).unwrap();
    let large = GrantFragment::new()
        .with_relations(required([]))
        .unwrap()
        .with_columns(bulk('b'))
        .unwrap();
    assert_eq!(
        populated.with_fragment(large).unwrap_err(),
        ManifestError::Policy(PolicyError::AuthorityCapacity),
    );
}

#[test]
fn permitted_public_delivery_never_widens_another_schema_privilege() {
    let policy = FragmentObjectPolicy::new(identifier("service"))
        .with_schema_privileges([ObjectPrivilege::Create])
        .unwrap()
        .with_public_delivered_privileges([ObjectPrivilege::Usage])
        .unwrap();
    let compiled = manifest()
        .with_fragment(policy.declare_schema(GrantFragment::new()).unwrap())
        .unwrap()
        .compile()
        .unwrap();

    // PUBLIC may deliver only the privilege the application named. The schema's
    // CREATE declaration keeps its deny, so an observed PUBLIC CREATE grant stays
    // a violation.
    let allowances = compiled
        .authority_policy()
        .public_overrides
        .iter()
        .filter(|allowance| allowance.object == PublicObject::Schema(identifier("service")))
        .flat_map(|allowance| allowance.privileges.iter().map(|allowed| allowed.privilege))
        .collect::<Vec<_>>();
    assert_eq!(allowances, vec![ObjectPrivilege::Usage]);
}

#[test]
fn a_fragment_routine_reaches_the_compiled_policy_and_the_grant_plan() {
    let signature = RoutineSignature::new(
        "service",
        "normalize",
        [RoutineType::new("pg_catalog", "text").unwrap()],
    )
    .unwrap();
    let compiled = manifest()
        .with_fragment(
            GrantFragment::new()
                .with_routine(
                    RoutineGrantSpec::new(
                        signature.clone(),
                        [ObjectPrivilege::Execute],
                        DeclarationPurpose::RequiredAndProvisioned,
                    )
                    .unwrap()
                    .allow_security_definer(true),
                )
                .unwrap(),
        )
        .unwrap()
        .compile()
        .unwrap();

    // The declaration has to survive the transfer into the manifest, the
    // compiler and the renderer, with its exact overload identity and its
    // definer allowance.
    let routines = &compiled.authority_policy().routines;
    assert_eq!(routines.len(), 1);
    assert_eq!(routines[0].routine, signature);
    assert!(routines[0].allow_security_definer);
    assert!(!routines[0].allow_owner);
    assert_eq!(
        compiled
            .grant_plan()
            .render(&identifier("service_reader"), None)
            .unwrap(),
        "GRANT EXECUTE ON ROUTINE \"service\".\"normalize\"(\"pg_catalog\".\"text\") \
         TO \"service_reader\";\n",
    );
}

#[test]
fn schema_and_relation_ownership_choices_propagate_independently() {
    let policy = |schema_owner, relation_owner| {
        FragmentObjectPolicy::new(identifier("service"))
            .allow_schema_owner(schema_owner)
            .allow_relation_owner(relation_owner)
    };
    let compiled = |schema_owner, relation_owner| {
        let policy = policy(schema_owner, relation_owner);
        let fragment = policy
            .declare_relations(
                policy.declare_schema(GrantFragment::new()).unwrap(),
                ["ledger"],
                [ObjectPrivilege::Select],
            )
            .unwrap();
        manifest()
            .with_fragment(fragment)
            .unwrap()
            .compile()
            .unwrap()
    };

    // The application's two ownership choices are separate: neither implies the
    // other, and verification must accept an owner-reachable login only where
    // the application said so.
    for (schema_owner, relation_owner) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let compiled = compiled(schema_owner, relation_owner);
        let policy = compiled.authority_policy();
        assert_eq!(policy.schemas.len(), 1);
        assert_eq!(policy.relations.len(), 1);
        assert_eq!(
            policy.schemas[0].allow_owner, schema_owner,
            "schema ownership choice {schema_owner} did not reach the policy",
        );
        assert_eq!(
            policy.relations[0].allow_owner, relation_owner,
            "relation ownership choice {relation_owner} did not reach the policy",
        );
    }
}
