#!/usr/bin/env python3
"""Require and execute the complete external PostgreSQL adapter contract."""

import os
from pathlib import Path
import re
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
CASES = {
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
}
COMMAND = ["cargo", "test", "-p", "batter-sqlx", "--test", "postgres_live", "--locked", "--"]


def invoke(arguments):
    outcome, = run_parallel([COMMAND + arguments], timeout=180, output_limit=1024 * 1024, cwd=ROOT)
    render_outcomes(["sqlx-live"], [outcome])
    return outcome


def main():
    if not os.environ.get("DATABASE_URL"):
        print("DATABASE_URL must identify an externally provisioned disposable PostgreSQL database", file=sys.stderr)
        return 1
    inventory = invoke(["--ignored", "--list", "--format", "terse"])
    names = set(re.findall(rb"^([a-z_]+): test$", inventory.stdout, re.MULTILINE))
    if not inventory.ok or names != {case.encode() for case in CASES}:
        print("live case inventory mismatch", file=sys.stderr)
        return 1
    outcome = invoke(["--ignored", "--test-threads=1", "--nocapture"])
    summary = f"test result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
    if not outcome.ok or summary.encode() not in outcome.stdout:
        print("incomplete or failed live PostgreSQL evidence", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
