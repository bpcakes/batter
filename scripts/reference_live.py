"""Run the explicit reference probes with prerequisites and a wall-clock bound."""

import re
import sys

from parallel_process import run_parallel

CASES = frozenset({
    "configured_pool_capacity_and_acquire_timeout",
    "configured_startup_pool_close_before_lease",
    "configured_worker_concurrency",
    "fixture_abandoned_producer_failure_retained",
    "fixture_assertion_and_script_failures_retained",
    "fixture_autovacuum_retains_lease",
    "fixture_body_and_cleanup_failures_retained",
    "fixture_cancelled_creation_retains_producers",
    "fixture_close_order_and_resumable_wait",
    "fixture_detached_sessions_retained",
    "fixture_distinct_deferred_and_consuming_failures",
    "fixture_driver_error_closes_admin_pools",
    "fixture_finish_preserves_body_failure",
    "fixture_foreign_template_rejected",
    "fixture_handled_pool_error_retained",
    "fixture_observer_failure_preserves_body",
    "fixture_one_slot_lock_operation",
    "fixture_partial_acquisition_and_panic",
    "fixture_pending_body_keeps_diagnostics",
    "fixture_pending_completion_recovers",
    "fixture_pool_error_is_pending_cleanup",
    "fixture_premature_shared_close_recovers",
    "fixture_replacement_before_first_attempt",
    "fixture_restricted_observer_sees_other_role",
    "fixture_retry_during_active_attempts",
    "fixture_retry_during_active_attempts_multithread",
    "fixture_role_cleanup_after_assertion",
    "fixture_runtime_loss_exposes_native_drop",
    "fixture_session_observer_failure_resumes",
    "fixture_shared_harness_admission",
    "fixture_shared_retry_and_close",
    "fixture_startup_is_not_a_connection_fence",
    "fixture_template_observation_recovers",
    "fixture_template_reuse_and_isolation",
    "fixture_waiter_loss_keeps_cleanup_driven",
    "fixture_wrong_server_retains_lease",
    "initialized_schema_upgrade",
    "lease_cleanup_defer_and_drop",
    "migrations_and_transactional_enqueue",
    "worker_startup_witness_and_shutdown",
})

COMMAND = ["cargo", "test", "-p", "batter-example-reference-service", "--test", "reference_live", "--locked"]


def complete_inventory(output):
    return set(re.findall(r"^([a-z_]+): test$", output, re.MULTILINE)) == CASES


def complete_execution(output):
    executed = re.findall(r"^test ([a-z_]+) \.\.\. ok$", output, re.MULTILINE)
    return (len(executed) == len(CASES) and set(executed) == CASES
            and re.search(rf"^test result: ok\. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;", output, re.MULTILINE) is not None)


def run(command, timeout):
    # Reuse the repository's existing Unix process owner for timeout, capture,
    # signal escalation and group reaping. A watchdog exit is failed evidence.
    return run_parallel([command], timeout=timeout, output_limit=4 * 1024 * 1024,
                        retain_tail=True)[0]


def main():
    # The native executable uses the same Rust validator and SQLx credentials as
    # fixture acquisition. Python only owns process budgets and exact inventory.
    check = run(["cargo", "run", "--quiet", "-p", "batter-example-reference-service",
                 "--example", "reference_preflight", "--locked"], timeout=300)
    if not check.ok or check.stdout.strip() != b"reference-preflight:ok":
        print(check.output, file=sys.stderr)
        sys.exit("Native PostgreSQL preflight failed; no fixtures were started.")
    inventory = run(COMMAND + ["--", "--ignored", "--list"], timeout=300)
    if not inventory.ok or not complete_inventory(inventory.stdout.decode("utf-8", errors="replace")):
        print(inventory.output, file=sys.stderr)
        sys.exit("Live probe inventory failed: required ignored cases must all compile and exist.")
    print(f"PostgreSQL 18 prerequisite passed; executing {len(CASES)} required live cases.", flush=True)
    result = run(COMMAND + ["--", "--ignored", "--test-threads=1"], timeout=180)
    print(result.output, end="")
    if not result.ok or not complete_execution(result.stdout.decode("utf-8", errors="replace")):
        sys.exit("Live probes did not all execute successfully.")


if __name__ == "__main__":
    main()
