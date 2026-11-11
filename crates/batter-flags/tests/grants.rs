//! Offline controls for the flags exact-role grant requirements.
//!
//! The expected privileges were read off the `batter-flags-postgres`
//! statements for this source version, independently of the inventory.
#![cfg(feature = "postgres")]
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags::grants::{
    FlagsGrant, FlagsGrantError, FlagsOperation, FlagsRole, SCHEMA, grant_fragment,
};
use batter_sqlx::verification::{
    DiscoveryScope, ExactRoleManifest, FragmentObjectPolicy, Identifier,
};

fn policy() -> FragmentObjectPolicy {
    FragmentObjectPolicy::new(Identifier::new(SCHEMA).unwrap())
}

fn rendered(grant: impl Into<FlagsGrant>) -> String {
    ExactRoleManifest::new(Identifier::new("flags").unwrap(), DiscoveryScope::Declared)
        .unwrap()
        .with_fragment(grant_fragment(&policy(), &grant.into()).unwrap())
        .unwrap()
        .compile()
        .unwrap()
        .grant_plan()
        .render(&Identifier::new("flags_service").unwrap(), None)
        .unwrap()
}

#[test]
fn prune_deletes_by_its_predicate_columns_and_locks_one_column_each() {
    assert_eq!(
        rendered(FlagsGrant::operations(FlagsOperation::PresencePrune, [])),
        concat!(
            "GRANT USAGE ON SCHEMA \"flags\" TO \"flags_service\";\n",
            "GRANT DELETE ON TABLE \"flags\".\"flag_catalogs\" TO \"flags_service\";\n",
            "GRANT DELETE ON TABLE \"flags\".\"flag_processes\" TO \"flags_service\";\n",
            "GRANT SELECT (\"last_referenced_at\"), UPDATE (\"last_referenced_at\") ON TABLE \"flags\".\"flag_catalogs\" TO \"flags_service\";\n",
            "GRANT SELECT (\"manifest_digest\") ON TABLE \"flags\".\"flag_catalogs\" TO \"flags_service\";\n",
            "GRANT SELECT (\"expires_at\"), UPDATE (\"expires_at\") ON TABLE \"flags\".\"flag_processes\" TO \"flags_service\";\n",
            "GRANT SELECT (\"manifest_digest\") ON TABLE \"flags\".\"flag_processes\" TO \"flags_service\";\n",
            "GRANT SELECT (\"process_id\") ON TABLE \"flags\".\"flag_processes\" TO \"flags_service\";\n",
            "GRANT SELECT (\"retention_seconds\") ON TABLE \"flags\".\"flag_processes\" TO \"flags_service\";\n",
        ),
    );
}

#[test]
fn history_is_read_whole_and_nothing_writes_the_audit() {
    // The read guard first reads the installed schema revision.
    assert_eq!(
        rendered(FlagsGrant::operations(FlagsOperation::HistoryRead, [])),
        concat!(
            "GRANT USAGE ON SCHEMA \"flags\" TO \"flags_service\";\n",
            "GRANT SELECT ON TABLE \"flags\".\"flag_audit\" TO \"flags_service\";\n",
            "GRANT SELECT (\"schema_revision\") ON TABLE \"flags\".\"flag_control\" TO \"flags_service\";\n",
            "GRANT SELECT (\"singleton\") ON TABLE \"flags\".\"flag_control\" TO \"flags_service\";\n",
        ),
    );
    let everything =
        rendered(FlagsGrant::role(FlagsRole::OperatorCli).and(FlagsRole::ServingProcess.into()));
    for line in everything
        .lines()
        .filter(|line| line.contains("\"flag_audit\""))
    {
        assert!(
            !line.contains("INSERT") && !line.contains("UPDATE") && !line.contains("DELETE"),
            "{line}"
        );
    }
}

#[test]
fn the_serving_role_publishes_presence_without_any_write_authority() {
    let serving = rendered(FlagsRole::ServingProcess);
    assert_eq!(
        serving,
        rendered(FlagsGrant::operations(
            FlagsOperation::SnapshotRead,
            [FlagsOperation::Presence, FlagsOperation::PresencePrune],
        )),
        "the role is exactly what register_in's components run"
    );
    // A serving process never writes a state row or bumps the counter.
    for line in serving.lines() {
        if line.contains("\"flag_state\"") || line.contains("\"flag_control\"") {
            assert!(
                !line.contains("INSERT") && !line.contains("UPDATE") && !line.contains("DELETE"),
                "{line}"
            );
        }
    }
    assert!(serving.contains("INSERT (\"manifest\")"));
    assert!(serving.contains("UPDATE (\"stopped_at\")"));
}

#[test]
fn the_operator_cli_is_the_union_of_its_parts() {
    let cli = rendered(FlagsRole::OperatorCli);
    let parts = rendered(FlagsGrant::operations(
        FlagsOperation::AuditedWrite,
        [
            FlagsOperation::SnapshotRead,
            FlagsOperation::CatalogRead,
            FlagsOperation::HistoryRead,
            FlagsOperation::FleetRead,
        ],
    ));
    assert_eq!(cli, parts);
    assert!(cli.contains("UPDATE (\"version\") ON TABLE \"flags\".\"flag_control\""));
}

/// Every operator read runs `SELECT schema_revision FROM flags.flag_control
/// WHERE singleton` in its own snapshot before reading anything else, so a
/// login granted only that read can run it.
#[test]
fn every_operator_read_can_run_the_schema_guard() {
    for operation in [
        FlagsOperation::CatalogRead,
        FlagsOperation::HistoryRead,
        FlagsOperation::FleetRead,
    ] {
        let read = rendered(FlagsGrant::operations(operation, []));
        for column in ["schema_revision", "singleton"] {
            assert!(
                read.contains(&format!(
                    "SELECT (\"{column}\") ON TABLE \"flags\".\"flag_control\""
                )),
                "{operation:?} lacks {column}: {read}"
            );
        }
    }
}

#[test]
fn another_schema_is_refused() {
    let elsewhere = FragmentObjectPolicy::new(Identifier::new("public").unwrap());
    assert!(matches!(
        grant_fragment(&elsewhere, &FlagsRole::ServingProcess.into()),
        Err(FlagsGrantError::WrongSchema)
    ));
}

#[test]
fn an_audited_write_reads_what_its_judgements_need() {
    let write = rendered(FlagsGrant::operations(FlagsOperation::AuditedWrite, []));
    // The coordinated-flip refusal reads the flip discipline, and the
    // invariant and governance judgement reads each running catalog's manifest.
    assert!(
        write.contains("SELECT (\"flip_discipline\") ON TABLE \"flags\".\"flag_catalog_entries\""),
        "{write}"
    );
    assert!(
        write.contains("SELECT (\"manifest\") ON TABLE \"flags\".\"flag_catalogs\""),
        "{write}"
    );
    assert!(!write.contains("UPDATE (\"manifest\")") && !write.contains("INSERT (\"manifest\")"));
}
