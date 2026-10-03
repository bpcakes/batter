//! Offline controls for this schema version's exact-role grant requirements.
//!
//! `tests/grants/*.sql` is the frozen expected privilege inventory for each
//! operation group. It is a literal review artifact, not a value recomputed from
//! `runledger_postgres::grants`: a change to the exported requirements shows up
//! here as a reviewable diff of exact relations, columns and privileges. The
//! native migration bundle is pinned alongside it, so a migration change cannot
//! land without touching this file and reviewing grant coverage.

use batter_sqlx::verification::{
    DeclarationPurpose, DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
    ObjectPrivilege, PublicDelivery, QualifiedName, RelationGrantGroup,
};
use runledger_postgres::grants::{RunledgerGrantError, RunledgerOperation, grant_fragment};

/// The exact migration bundle these requirements were reviewed against.
const PINNED_BUNDLE: &str = "a4fc356878542c81b29151d3636a7d267141a0b2573c0204257f0813733aac6f";

const EVERY_OPERATION: [(RunledgerOperation, &str); 6] = [
    (
        RunledgerOperation::IntentSubmission,
        include_str!("grants/intent_submission.sql"),
    ),
    (
        RunledgerOperation::SchemaSnapshot,
        include_str!("grants/schema_snapshot.sql"),
    ),
    (
        RunledgerOperation::DirectJobExecution,
        include_str!("grants/direct_job_execution.sql"),
    ),
    (
        RunledgerOperation::IntentPromotion,
        include_str!("grants/intent_promotion.sql"),
    ),
    (
        RunledgerOperation::CatalogSync,
        include_str!("grants/catalog_sync.sql"),
    ),
    (
        RunledgerOperation::ScheduledDispatch,
        include_str!("grants/scheduled_dispatch.sql"),
    ),
];

fn policy(schema: &str) -> FragmentObjectPolicy {
    FragmentObjectPolicy::new(Identifier::new(schema).expect("valid schema identifier"))
}

fn rendered(
    policy: &FragmentObjectPolicy,
    operations: impl IntoIterator<Item = RunledgerOperation>,
) -> String {
    ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .expect("valid manifest")
        .with_fragment(grant_fragment(policy, operations).expect("valid fragment"))
        .expect("composable fragment")
        .compile()
        .expect("compilable manifest")
        .grant_plan()
        .render(
            &Identifier::new("jobs_service").expect("valid role identifier"),
            None,
        )
        .expect("renderable plan")
}

fn expected(fixture: &str) -> String {
    fixture
        .lines()
        .filter(|line| !line.starts_with("--"))
        .map(|line| format!("{line}\n"))
        .collect()
}

#[test]
fn every_operation_group_matches_its_frozen_privilege_inventory() {
    for (operation, fixture) in EVERY_OPERATION {
        assert_eq!(
            rendered(&policy("jobs"), [operation]),
            expected(fixture),
            "{operation:?} grant inventory changed",
        );
    }
}

#[test]
fn the_native_migration_bundle_is_pinned_for_grant_coverage_review() {
    let observed: String = runledger_postgres::migration_bundle()
        .bundle_fingerprint()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        observed, PINNED_BUNDLE,
        "native migrations changed; review every operation group's grant coverage",
    );
}

#[test]
fn intent_submission_carries_no_queue_promotion_or_payload_authority() {
    let intents = rendered(&policy("jobs"), [RunledgerOperation::IntentSubmission]);
    for forbidden in [
        "job_queue",
        "job_definitions",
        "job_attempts",
        "job_events",
        "job_schedules",
        "workflow",
        "_sqlx_migrations",
        // The submitted request is readable for duplicate resolution; the
        // separate decoded payload column is not.
        "SELECT (\"payload\")",
        "UPDATE (\"status\")",
        "UPDATE (\"promoted_job_id\")",
        "GRANT DELETE",
        "ALL PRIVILEGES",
        "WITH GRANT OPTION",
    ] {
        assert!(
            !intents.contains(forbidden),
            "unexpected authority: {forbidden}"
        );
    }
}

#[test]
fn the_direct_worker_composition_excludes_history_and_workflow_authorship() {
    let worker = rendered(
        &policy("jobs"),
        [
            RunledgerOperation::DirectJobExecution,
            RunledgerOperation::IntentPromotion,
        ],
    );
    for forbidden in [
        "GRANT DELETE ON TABLE \"jobs\".\"job_events\"",
        "GRANT DELETE ON TABLE \"jobs\".\"job_logs\"",
        "GRANT DELETE ON TABLE \"jobs\".\"job_queue\"",
        "GRANT DELETE ON TABLE \"jobs\".\"job_enqueue_intents\"",
        "job_logs",
        "job_replays",
        "workflow_runs",
        "workflow_step_dependencies",
        "workflow_run_mutations",
        "INSERT (\"workflow_run_id\")",
        "ALL PRIVILEGES",
        "WITH GRANT OPTION",
    ] {
        assert!(
            !worker.contains(forbidden),
            "unexpected authority: {forbidden}"
        );
    }
    // Releasing an unstarted claim deletes its attempt row, which is relation
    // wide authority over attempt history for those rows.
    assert!(worker.contains("GRANT DELETE ON TABLE \"jobs\".\"job_attempts\""));
    assert_eq!(worker.matches("GRANT DELETE ON TABLE").count(), 2);
}

#[test]
fn the_full_snapshot_is_a_separate_relation_wide_selection() {
    let snapshot = rendered(&policy("jobs"), [RunledgerOperation::SchemaSnapshot]);
    // LOCK TABLE ... IN ACCESS SHARE MODE checks relation privileges, so this
    // group cannot be expressed per column.
    assert_eq!(snapshot.matches("GRANT SELECT ON TABLE").count(), 5);
    assert!(!snapshot.contains('('));
    for group in [
        RunledgerOperation::IntentSubmission,
        RunledgerOperation::DirectJobExecution,
        RunledgerOperation::IntentPromotion,
        RunledgerOperation::CatalogSync,
        RunledgerOperation::ScheduledDispatch,
    ] {
        assert!(
            !rendered(&policy("jobs"), [group]).contains("GRANT SELECT ON TABLE"),
            "{group:?} must not require relation-wide SELECT",
        );
    }
}

#[test]
fn overlapping_and_reordered_selections_render_one_deterministic_plan() {
    let groups: Vec<RunledgerOperation> = EVERY_OPERATION
        .iter()
        .map(|(operation, _)| *operation)
        .collect();
    let ascending = rendered(&policy("jobs"), groups.clone());
    let mut shuffled: Vec<RunledgerOperation> = groups.iter().rev().copied().collect();
    shuffled.extend(groups.iter().copied());
    assert_eq!(ascending, rendered(&policy("jobs"), shuffled));

    let policy = policy("jobs");
    let split = ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .expect("valid manifest")
        .with_fragment(grant_fragment(&policy, groups[..3].to_vec()).expect("valid fragment"))
        .expect("composable fragment")
        .with_fragment(grant_fragment(&policy, groups[3..].to_vec()).expect("valid fragment"))
        .expect("composable fragment")
        .compile()
        .expect("compilable manifest");
    assert_eq!(
        split
            .grant_plan()
            .render(
                &Identifier::new("jobs_service").expect("valid role identifier"),
                None,
            )
            .expect("renderable plan"),
        ascending,
    );
}

#[test]
fn empty_selections_and_contradictory_application_options_fail_closed() {
    assert!(matches!(
        grant_fragment(&policy("jobs"), []),
        Err(RunledgerGrantError::EmptySelection),
    ));

    let policy = policy("jobs");
    let mut manifest = ExactRoleManifest::new(policy.schema().clone(), DiscoveryScope::Declared)
        .expect("valid manifest")
        .with_fragment(
            grant_fragment(&policy, [RunledgerOperation::IntentSubmission])
                .expect("valid fragment"),
        )
        .expect("composable fragment");
    manifest
        .add_relations(
            RelationGrantGroup::new(
                [QualifiedName::new("jobs", "job_enqueue_intents").expect("valid relation")],
                [ObjectPrivilege::Update],
                DeclarationPurpose::RequiredAndProvisioned,
            )
            .expect("valid relation group")
            .public_delivery(PublicDelivery::AllowDeclared),
        )
        .expect("addable relation group");
    assert!(manifest.compile().is_err());
}

#[test]
fn a_non_public_mixed_case_schema_is_quoted_and_never_shadowed() {
    let mixed = rendered(&policy("Job Store"), [RunledgerOperation::IntentSubmission]);
    assert!(mixed.starts_with("GRANT USAGE ON SCHEMA \"Job Store\" TO \"jobs_service\";\n"));
    assert!(mixed.contains("ON TABLE \"Job Store\".\"job_enqueue_intents\""));
    assert!(!mixed.contains("\"public\""));
    assert_ne!(
        mixed,
        rendered(&policy("job store"), [RunledgerOperation::IntentSubmission]),
    );
}
