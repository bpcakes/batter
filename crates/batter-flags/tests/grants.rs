//! Offline controls for the flags exact-role grant requirements.
//!
//! The expected privileges were read off the `batter-flags-postgres`
//! statements for this source version, independently of the inventory.
#![cfg(feature = "postgres")]
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use batter_flags::grants::{FlagsGrantError, FlagsOperation, grant_fragment};
use batter_sqlx::verification::{DiscoveryScope, ExactRoleManifest, Identifier};

fn rendered(operations: impl IntoIterator<Item = FlagsOperation>) -> String {
    ExactRoleManifest::new(Identifier::new("flags").unwrap(), DiscoveryScope::Declared)
        .unwrap()
        .with_fragment(grant_fragment(operations).unwrap())
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
        rendered([FlagsOperation::PresencePrune]),
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
    assert_eq!(
        rendered([FlagsOperation::HistoryRead]),
        concat!(
            "GRANT USAGE ON SCHEMA \"flags\" TO \"flags_service\";\n",
            "GRANT SELECT ON TABLE \"flags\".\"flag_audit\" TO \"flags_service\";\n",
        ),
    );
    let everything = rendered([
        FlagsOperation::OperatorCli,
        FlagsOperation::SnapshotRead,
        FlagsOperation::Presence,
        FlagsOperation::PresencePrune,
    ]);
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
fn the_serving_selection_publishes_presence_without_any_write_authority() {
    let serving = rendered([
        FlagsOperation::SnapshotRead,
        FlagsOperation::Presence,
        FlagsOperation::PresencePrune,
    ]);
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
    let cli = rendered([FlagsOperation::OperatorCli]);
    let parts = rendered([
        FlagsOperation::AuditedWrite,
        FlagsOperation::SnapshotRead,
        FlagsOperation::CatalogRead,
        FlagsOperation::HistoryRead,
        FlagsOperation::FleetRead,
    ]);
    assert_eq!(cli, parts);
    assert!(cli.contains("UPDATE (\"version\") ON TABLE \"flags\".\"flag_control\""));
}

#[test]
fn an_empty_selection_is_refused() {
    assert!(matches!(
        grant_fragment([]),
        Err(FlagsGrantError::EmptySelection)
    ));
}
