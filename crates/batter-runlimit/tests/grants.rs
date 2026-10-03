//! Offline controls for the native Runlimit exact-role grant requirements.
//!
//! The expected privilege inventory below is written independently from
//! `batter_runlimit::grants`: each statement was read off the native
//! `runlimit-postgres` SQL for this source version. A change to the exported
//! fragment that is not also a change to the native statements fails here.

use batter_runlimit::grants::{RunlimitGrantError, RunlimitOperation, grant_fragment};
use batter_sqlx::verification::{
    DeclarationPurpose, DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
    ObjectPrivilege, PublicDelivery, RelationGrantGroup,
};

const EVERY_OPERATION: [RunlimitOperation; 5] = [
    RunlimitOperation::FixedWindowAdmission,
    RunlimitOperation::FixedWindowExpiryCleanup,
    RunlimitOperation::GcraAdmission,
    RunlimitOperation::GcraExpiryCleanup,
    RunlimitOperation::AuthenticationAttempts,
];

/// Pinned native migration inventory. A native migration change must be paired
/// with a grant-coverage review of the operations above, so updating these
/// values without that review is the failure this control reports.
const PINNED_MIGRATIONS: [(&str, &[(i64, &str)]); 3] = [
    (
        "fixed-window",
        &[
            (
                20_260_723_000_000,
                "31d93fde98c2364a33062e373508f563b486df2898f8e77365918e81da8980a8e74bf9c63022d251aeb3d58067e980ec",
            ),
            (
                20_260_725_000_000,
                "8c1ea2d669f8d183e44ffd13b69180502118d5fc4e85ddee7f8e678de173c19293bf324c055d76c91050548a22cc3b75",
            ),
            (
                20_260_726_000_000,
                "9bb37051aa94e5afa386799cef55579fcdfd7cfce974b305480eec8872b390d1aaa67727516c69c55121d3658b640664",
            ),
        ],
    ),
    (
        "gcra",
        &[
            (
                20_260_922_000_000,
                "3668f609b059435548091273050b850c68192007cd9c3c98e1155b4ed1938354543375919d5cacf93c898cdb88848d3c",
            ),
            (
                20_260_922_000_001,
                "ba41cdd57747de6ec69eddfeda3b9d41e486c890e2a0fc7fe825997e29edb25f7e3c73c00e6fc634e52ed6f3a719f9a8",
            ),
        ],
    ),
    (
        "attempts",
        &[(
            20_260_921_000_001,
            "c949c351283010167fa46431ed5d2f8125270af2b65768ed94b644a67afc10051baa8f1d8ef615e7e5c19bb8eaa9002c",
        )],
    ),
];

fn policy(schema: &str) -> FragmentObjectPolicy {
    FragmentObjectPolicy::new(Identifier::new(schema).unwrap())
}

fn rendered(
    policy: &FragmentObjectPolicy,
    operations: impl IntoIterator<Item = RunlimitOperation>,
) -> String {
    let schema = policy.schema().clone();
    ExactRoleManifest::new(schema, DiscoveryScope::Declared)
        .unwrap()
        .with_fragment(grant_fragment(policy, operations).unwrap())
        .unwrap()
        .compile()
        .unwrap()
        .grant_plan()
        .render(&Identifier::new("quota_service").unwrap(), None)
        .unwrap()
}

#[test]
fn fixed_window_admission_grants_exactly_the_upsert_and_shard_lock_authority() {
    assert_eq!(
        rendered(&policy("quotas"), [RunlimitOperation::FixedWindowAdmission]),
        concat!(
            "GRANT USAGE ON SCHEMA \"quotas\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\"), UPDATE (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_capacity_shards\" TO \"quota_service\";\n",
            "GRANT SELECT (\"row_count\") ON TABLE \"quotas\".\"runlimit_capacity_shards\" TO \"quota_service\";\n",
            "GRANT SELECT (\"config_fingerprint\"), INSERT (\"config_fingerprint\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"policy_id\"), INSERT (\"policy_id\"), UPDATE (\"policy_id\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"scope_id\"), INSERT (\"scope_id\"), UPDATE (\"scope_id\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"subject_key\"), INSERT (\"subject_key\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"used\"), INSERT (\"used\"), UPDATE (\"used\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"window_expires_at\"), INSERT (\"window_expires_at\"), UPDATE (\"window_expires_at\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"window_started_at\"), INSERT (\"window_started_at\"), UPDATE (\"window_started_at\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
        ),
    );
}

#[test]
fn each_remaining_family_renders_its_own_independently_selected_authority() {
    assert_eq!(
        rendered(
            &policy("quotas"),
            [RunlimitOperation::FixedWindowExpiryCleanup]
        ),
        concat!(
            "GRANT USAGE ON SCHEMA \"quotas\" TO \"quota_service\";\n",
            "GRANT SELECT, DELETE ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\"), UPDATE (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_capacity_shards\" TO \"quota_service\";\n",
            "GRANT UPDATE (\"window_expires_at\") ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";\n",
        ),
    );
    assert_eq!(
        rendered(&policy("quotas"), [RunlimitOperation::GcraAdmission]),
        concat!(
            "GRANT USAGE ON SCHEMA \"quotas\" TO \"quota_service\";\n",
            "GRANT SELECT (\"config_fingerprint\"), INSERT (\"config_fingerprint\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"expires_at_ms\"), INSERT (\"expires_at_ms\"), UPDATE (\"expires_at_ms\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"subject_key\"), INSERT (\"subject_key\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"tat_scaled\"), INSERT (\"tat_scaled\"), UPDATE (\"tat_scaled\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";\n",
            "GRANT SELECT (\"observed_at_ms\"), UPDATE (\"observed_at_ms\") ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";\n",
            "GRANT SELECT (\"row_count\") ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";\n",
        ),
    );
    assert_eq!(
        rendered(&policy("quotas"), [RunlimitOperation::GcraExpiryCleanup]),
        concat!(
            "GRANT USAGE ON SCHEMA \"quotas\" TO \"quota_service\";\n",
            "GRANT DELETE ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"config_fingerprint\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"expires_at_ms\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"subject_key\") ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";\n",
            "GRANT SELECT (\"observed_at_ms\"), UPDATE (\"observed_at_ms\") ON TABLE \"quotas\".\"runlimit_gcra_shards\" TO \"quota_service\";\n",
        ),
    );
    assert_eq!(
        rendered(
            &policy("quotas"),
            [RunlimitOperation::AuthenticationAttempts]
        ),
        concat!(
            "GRANT USAGE ON SCHEMA \"quotas\" TO \"quota_service\";\n",
            "GRANT DELETE ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_shard\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"capacity_slot\"), INSERT (\"capacity_slot\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"config_fingerprint\"), INSERT (\"config_fingerprint\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"failures\"), INSERT (\"failures\"), UPDATE (\"failures\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"last_failure_ms\"), INSERT (\"last_failure_ms\"), UPDATE (\"last_failure_ms\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"lease_token\"), INSERT (\"lease_token\"), UPDATE (\"lease_token\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"lease_until_ms\"), INSERT (\"lease_until_ms\"), UPDATE (\"lease_until_ms\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"quiet_ms\"), INSERT (\"quiet_ms\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"retry_at_ms\"), INSERT (\"retry_at_ms\"), UPDATE (\"retry_at_ms\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
            "GRANT SELECT (\"subject_key\"), INSERT (\"subject_key\") ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";\n",
        ),
    );
}

#[test]
fn no_selection_grants_counter_key_capacity_or_ledger_authority() {
    let every = rendered(&policy("quotas"), EVERY_OPERATION);
    for forbidden in [
        "UPDATE (\"row_count\")",
        "UPDATE (\"config_fingerprint\")",
        "UPDATE (\"subject_key\")",
        "UPDATE (\"capacity_slot\")",
        "UPDATE (\"quiet_ms\")",
        "ALL PRIVILEGES",
        "WITH GRANT OPTION",
        "_sqlx_migrations",
        "runledger",
    ] {
        assert!(
            !every.contains(forbidden),
            "unexpected authority: {forbidden}"
        );
    }
    // Relation-level privileges are only the ones PostgreSQL cannot express per
    // column: DELETE, and the SELECT that the fixed-window cleanup's `ctid` read
    // requires. No selection widens INSERT or UPDATE to a whole relation.
    for statement in every.lines() {
        let relation_wide =
            statement.starts_with("GRANT UPDATE ON") || statement.starts_with("GRANT INSERT ON");
        assert!(!relation_wide, "relation-wide grant: {statement}");
    }
    let relation_level: Vec<&str> = every
        .lines()
        .filter(|statement| statement.contains("ON TABLE") && !statement.contains('('))
        .collect();
    assert_eq!(
        relation_level,
        [
            "GRANT DELETE ON TABLE \"quotas\".\"runlimit_attempts\" TO \"quota_service\";",
            "GRANT SELECT, DELETE ON TABLE \"quotas\".\"runlimit_fixed_windows\" TO \"quota_service\";",
            "GRANT DELETE ON TABLE \"quotas\".\"runlimit_gcra\" TO \"quota_service\";",
        ],
    );
}

#[test]
fn overlapping_and_reordered_selections_render_one_deterministic_plan() {
    let ascending = rendered(&policy("quotas"), EVERY_OPERATION);
    let mut descending = EVERY_OPERATION;
    descending.reverse();
    let mut repeated = Vec::from(descending);
    repeated.extend(EVERY_OPERATION);
    assert_eq!(ascending, rendered(&policy("quotas"), repeated));

    // The same composition reached through two fragments is also identical.
    let policy = policy("quotas");
    let split = ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .unwrap()
        .with_fragment(
            grant_fragment(
                &policy,
                [
                    RunlimitOperation::FixedWindowAdmission,
                    RunlimitOperation::GcraAdmission,
                ],
            )
            .unwrap(),
        )
        .unwrap()
        .with_fragment(
            grant_fragment(
                &policy,
                [
                    RunlimitOperation::FixedWindowExpiryCleanup,
                    RunlimitOperation::GcraExpiryCleanup,
                    RunlimitOperation::AuthenticationAttempts,
                ],
            )
            .unwrap(),
        )
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(
        split
            .grant_plan()
            .render(&Identifier::new("quota_service").unwrap(), None)
            .unwrap(),
        ascending,
    );
}

#[test]
fn empty_selections_and_contradictory_application_options_fail_closed() {
    assert!(matches!(
        grant_fragment(&policy("quotas"), []),
        Err(RunlimitGrantError::EmptySelection),
    ));

    // The application cannot quietly widen one native relation to table-level
    // UPDATE beside the fragment's column-scoped declarations.
    let policy = policy("quotas");
    let mut manifest = ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .unwrap()
        .with_fragment(grant_fragment(&policy, [RunlimitOperation::GcraAdmission]).unwrap())
        .unwrap();
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [
                    batter_sqlx::verification::QualifiedName::new("quotas", "runlimit_gcra")
                        .unwrap(),
                ],
                [ObjectPrivilege::Update],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .unwrap()
            .public_delivery(PublicDelivery::AllowDeclared),
        )
        .unwrap();
    assert!(manifest.compile().is_err());
}

#[test]
fn a_non_public_mixed_case_schema_is_quoted_and_never_shadowed() {
    let mixed = rendered(&policy("Quota Store"), [RunlimitOperation::GcraAdmission]);
    assert!(mixed.starts_with("GRANT USAGE ON SCHEMA \"Quota Store\" TO \"quota_service\";\n"));
    assert!(mixed.contains("ON TABLE \"Quota Store\".\"runlimit_gcra\""));
    assert!(!mixed.contains("\"public\""));
    // A same-named store in another schema is a different declared object.
    assert_ne!(
        mixed,
        rendered(&policy("quota store"), [RunlimitOperation::GcraAdmission]),
    );
}

#[test]
fn application_public_delivery_and_row_type_choices_reach_the_declarations() {
    let strict = rendered(&policy("quotas"), [RunlimitOperation::GcraAdmission]);
    let permissive = rendered(
        &policy("quotas")
            .allow_row_type_public_usage(true)
            .with_public_delivered_privileges([ObjectPrivilege::Select])
            .unwrap(),
        [RunlimitOperation::GcraAdmission],
    );
    // PUBLIC delivery and row-type USAGE change what verification accepts, never
    // the role-targeted grant plan.
    assert_eq!(strict, permissive);

    let compiled = |policy: &FragmentObjectPolicy| {
        ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
            .unwrap()
            .with_fragment(grant_fragment(policy, [RunlimitOperation::GcraAdmission]).unwrap())
            .unwrap()
            .compile()
            .unwrap()
    };
    assert_ne!(
        compiled(&policy("quotas")),
        compiled(&policy("quotas").allow_row_type_public_usage(true)),
    );
}

#[test]
fn the_native_migration_inventory_is_pinned_for_grant_coverage_review() {
    let migrators = [
        &runlimit_postgres::MIGRATOR,
        &runlimit_postgres::GCRA_MIGRATOR,
        &runlimit_postgres::attempts::ATTEMPTS_MIGRATOR,
    ];
    for ((family, expected), migrator) in PINNED_MIGRATIONS.iter().zip(migrators) {
        let observed: Vec<(i64, String)> = migrator
            .iter()
            .map(|migration| {
                (
                    migration.version,
                    migration
                        .checksum
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                )
            })
            .collect();
        let expected: Vec<(i64, String)> = expected
            .iter()
            .map(|(version, checksum)| (*version, (*checksum).to_owned()))
            .collect();
        assert_eq!(observed, expected, "{family} migrations changed");
    }
}
