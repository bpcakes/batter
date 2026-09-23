#!/usr/bin/env python3
"""Require and execute every external PostgreSQL adapter contract."""

import os
from pathlib import Path
import re
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "atomic_live": {
        "policy_abandoned_operation_cannot_be_caught_into_success",
        "policy_infers_errors_and_preserves_recovery_and_rejection",
        "policy_caught_loss_cannot_run_more_work_or_commit",
        "policy_recovery_loss_keeps_rejection_and_native_cause",
        "policy_completion_uncertainty_keeps_output_and_rejection",
        "policy_begin_failure_does_not_invoke_body",

        "combined_validation_rejects_each_setting_and_recovers_local_changes",
        "combined_validation_rejects_role_and_schema_privilege_drift",
        "atomic_scope_statement_counts_include_recovery_and_completion",
        "profiled_scope_checks_external_schema_loss_before_work",
        "profiled_schema_lookup_remains_index_eligible",
        "transaction_profiles_distinguish_startup_defaults_session_settings_and_zero",
        "transaction_profiles_cover_pool_hooks_atomic_and_snapshot_work",
        "transaction_timeout_drift_poison_retains_cause_and_retires",
        "snapshot_timeout_drift_rejects_success_and_preserves_recovered_errors",
        "declared_idle_timeout_releases_server_locks_but_not_held_lease",
        "declared_transaction_timeout_bounds_multiple_short_statements",
        "owned_pool_replaces_hooks_and_restores_fast_acquisition_and_atomic_policy",
        "profile_setup_errors_redact_setting_and_role_values",
        "profile_after_connect_logging_redacts_setting_values",
        "profile_before_acquire_logging_redacts_setting_values",
        "mixed_case_profile_settings_preserve_values",
        "transaction_control_is_terminal",
        "application_and_library_operations_commit_together",
        "savepoint_errors_recover_without_partial_writes",
        "swallowed_sql_error_and_application_error_are_terminal",
        "deferred_and_transport_commit_failures_are_unconfirmed",
        "unpolled_consuming_scopes_retire_the_owner",
        "cancelled_application_operation_and_snapshot_retire",
        "panicking_scope_retires_the_owner",
        "snapshot_is_coherent_read_only_and_normalized",
        "snapshot_transaction_control_never_returns_evidence",
        "commit_in_flight_cancellation_and_disconnect_are_uncertain",
        "snapshot_error_requires_original_guard",
        "runner_releases_only_acknowledged_outputs",
        "runner_uncertainty_retains_output_and_rejection",
        "runner_cannot_commit_after_caught_operation_cancellation",
        "caught_boundary_loss_retains_first_cause",
        "caught_recovery_failure_retains_poison_and_callback_rejection",
        "poisoned_scope_repeats_original_cause_without_invoking_work",
        "atomic_and_snapshot_completion_retire_session_state",
        "acquisition_resets_inherited_state_and_statement_cache",
        "failed_session_normalization_retires_instead_of_returning",
    },
    "migrations_live": {
        "migration_preserves_native_history_and_retires_checksum_failure",
        "interrupted_migration_retires_without_waiting_for_server_lock",
    },
    "postgres_live": {
        "blocked_cancellation_releases_capacity",
        "blocked_deadline_releases_capacity",
        "blocked_error_releases_capacity",
        "blocked_panic_releases_capacity",
        "blocked_outer_drop_releases_capacity",
        "repeated_interruptions_leave_independent_residual_sessions",
        "acknowledged_transactions_reuse_and_abandoned_transactions_retire",
        "verification_uses_one_read_only_snapshot_and_preserves_ledger_policy",
        "native_database_failure_retires_and_preserves_cause",
        "rejected_commit_preserves_native_cause_and_retires",
        "ordinary_return_control_retains_blocked_capacity",
    },
    "pool_ownership_live": {
        "command_query_closes_owned_pool",
        "startup_query_joins_before_pool_close",
        "invalid_slot_prevents_pool_construction",
        "duplicate_slot_prevents_second_pool",
        "native_options_and_maintenance_are_preserved",
        "authentication_error_keeps_pool_cleanup",
        "cancelled_acquisition_keeps_pool_cleanup",
        "later_application_error_keeps_pool_cleanup",
        "application_panic_keeps_pool_cleanup",
        "two_pools_close_after_dependents_in_lifo_order",
        "held_checkout_delays_successful_close",
        "held_checkout_timeout_is_not_success",
        "work_and_cleanup_failures_are_both_retained",
        "ownership_oracles_reject_missing_and_premature_cleanup",
    },
    "verification_live": {
        "verification_temporary_namespace_requests_are_incomplete",
        "verification_public_relation_overrides_control_column_defaults",
        "verification_required_and_excess_authority_share_captured_snapshot",
        "verification_cross_schema_aliases_preserve_scope_and_exact_policy",
        "verification_idle_reset_has_one_expected_native_notice",
        "verification_checks_real_login_reachability_and_two_fixture_policies",
        "verification_cancellation_releases_one_slot_capacity",
        "verification_uses_one_snapshot_for_concurrent_acl_changes",
        "verification_keeps_many_declared_objects_set_oriented",
        "verification_uses_the_initial_authenticated_role",
        "verification_detects_createrole_admin_across_unusable_memberships",
        "verification_rejects_row_security_on_the_migration_ledger",
        "verification_preserves_separate_caller_raw_transactions",
        "verification_occupied_pool_acquisition_preserves_caller_transaction",
        "verification_normalizes_abandoned_pooled_raw_transactions",
        "verification_ledger_ddl_before_lock_is_observed",
        "verification_ledger_ddl_after_lock_waits_for_snapshot",
        "verification_catalog_resolution_preserves_serving_state",
        "verification_ledger_descendant_truncate_waits_for_snapshot",
        "verification_late_ledger_attachment_cannot_supply_snapshot_rows",
        "verification_checks_mixed_case_builtin_parameter_names",
        "verification_rejects_oversized_parameter_acl_catalog",
        "verification_rejects_oversized_parameter_name",
        "verification_required_privileges_follow_current_inheritance",
        "verification_discovery_checks_unlisted_objects_and_exact_overrides",
        "verification_distinguishes_hidden_parameters_from_missing_objects",
        "verification_reserved_custom_parameter_requirement",
        "verification_discovery_dependent_types_follow_native_acl",
        "protected_sqlx_ledger_checks_exact_shape_and_nonprefix_subset",
        "protected_schema_checks_all_definers_and_exact_stored_search_path",
        "protected_exact_role_rejects_set_reachable_ownership",
        "protected_sqlx_absent_lock_cannot_admit_late_ledger",
        "protected_sqlx_history_rejects_post_snapshot_name_replacement",
        "protected_sqlx_name_replacement_is_not_mistaken_for_locked_ledger",
        "protected_sqlx_guards_reject_rls_inheritance_and_overflow",
        "protected_sqlx_late_attachment_cannot_supply_snapshot_rows",
    },
}


def command(target):
    command = ["cargo", "test", "-p", "batter-sqlx"]
    if target == "verification_live":
        command += ["--features", "test-support"]
    return command + ["--test", target, "--locked", "--"]


def invoke(target, arguments):
    outcome, = run_parallel([command(target) + arguments], timeout=180,
                            output_limit=1024 * 1024, cwd=ROOT)
    render_outcomes([f"sqlx-live:{target}"], [outcome])
    return outcome


def complete_inventory(output, cases):
    names = set(re.findall(rb"^(?:[a-z_]+::)*([a-z_]+): test$", output, re.MULTILINE))
    return names == {case.encode() for case in cases}


def complete_execution(output, cases):
    completed = set(re.findall(rb"^test (?:[a-z_]+::)*([a-z_]+) \.\.\. ok$", output, re.MULTILINE))
    summary = (f"test result: ok. {len(cases)} passed; 0 failed; 0 ignored; "
               "0 measured; 0 filtered out;").encode()
    return completed == {case.encode() for case in cases} and summary in output


def main():
    missing = [name for name in ("DATABASE_URL", "BATTER_SQLX_AUTH_ACCEPT_URL",
                                 "BATTER_SQLX_ADMIN_URL")
               if not os.environ.get(name)]
    if missing:
        print(("missing live PostgreSQL prerequisites: " + ", ".join(missing) +
               "; provide an ordinary disposable database and a known-good "
               "password-authenticated endpoint plus a PostgreSQL superuser "
               "connection to a dedicated disposable cluster"), file=sys.stderr)
        return 1
    for target, cases in TARGETS.items():
        inventory = invoke(target, ["--ignored", "--list", "--format", "terse"])
        if not inventory.ok or not complete_inventory(inventory.stdout, cases):
            print(f"live case inventory mismatch: {target}", file=sys.stderr)
            return 1
        outcome = invoke(target, ["--ignored", "--test-threads=1", "--nocapture"])
        if not outcome.ok or not complete_execution(outcome.stdout, cases):
            print(f"incomplete or failed live PostgreSQL evidence: {target}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
