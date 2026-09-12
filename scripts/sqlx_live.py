#!/usr/bin/env python3
"""Require and execute every external PostgreSQL adapter contract."""

import os
from pathlib import Path
import re
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "postgres_live": {
        "blocked_cancellation_releases_capacity",
        "blocked_deadline_releases_capacity",
        "blocked_error_releases_capacity",
        "blocked_panic_releases_capacity",
        "blocked_outer_drop_releases_capacity",
        "repeated_interruptions_leave_independent_residual_sessions",
        "success_and_acknowledged_transactions_reuse",
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
}


def command(target):
    return ["cargo", "test", "-p", "batter-sqlx", "--test", target, "--locked", "--"]


def invoke(target, arguments):
    outcome, = run_parallel([command(target) + arguments], timeout=180,
                            output_limit=1024 * 1024, cwd=ROOT)
    render_outcomes([f"sqlx-live:{target}"], [outcome])
    return outcome


def complete_inventory(output, cases):
    names = set(re.findall(rb"^([a-z_]+): test$", output, re.MULTILINE))
    return names == {case.encode() for case in cases}


def complete_execution(output, cases):
    completed = set(re.findall(rb"^test ([a-z_]+) \.\.\. ok$", output, re.MULTILINE))
    summary = (f"test result: ok. {len(cases)} passed; 0 failed; 0 ignored; "
               "0 measured; 0 filtered out;").encode()
    return completed == {case.encode() for case in cases} and summary in output


def main():
    if not os.environ.get("DATABASE_URL"):
        print("DATABASE_URL must identify an externally provisioned disposable PostgreSQL database",
              file=sys.stderr)
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
