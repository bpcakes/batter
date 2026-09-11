#!/usr/bin/env python3
"""Run the locked core/workspace test matrix with concurrent runtime passes."""

import argparse
from pathlib import Path
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
CORE_CHECK = ["cargo", "check", "-p", "batter", "--lib", "--no-default-features", "--locked"]
RUNTIME_TESTS = [
    ["cargo", "test", "-p", "batter", "--no-default-features", "--lib", "--tests", "--locked"],
    ["cargo", "test", "--workspace", "--all-features", "--all-targets", "--locked"],
    # A clean developer shell must not hide ambient-state dependencies in tests.
    ["env", "PGDATA=/unused-configuration-fixture", "PGUSER=parent-fixture",
     "PGPASSWORD=parent-secret-marker", "cargo", "test", "-p",
     "batter-example-reference-service", "--test", "configuration", "--locked"],
]
DOC_TESTS = ["cargo", "test", "--workspace", "--all-features", "--doc", "--locked"]
RUNNER_TESTS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                "-p", "test_parallel_process.py", "-v"]
SMOKE_TESTS = [sys.executable, "scripts/test_smoke_postgres.py", "-v"]
REFERENCE_RUNNER_TESTS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                          "-p", "test_reference_live.py", "-v"]


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    for labels, commands in [(["core-library", "runner-controls", "smoke-controls", "reference-runner-controls"],
                              [CORE_CHECK, RUNNER_TESTS, SMOKE_TESTS, REFERENCE_RUNNER_TESTS]),
                             (["core-tests", "workspace-tests", "configuration-hostile-environment"], RUNTIME_TESTS),
                             (["doctests"], [DOC_TESTS])]:
        print(f"Running {', '.join(labels)}", file=sys.stderr, flush=True)
        outcomes = run_parallel(commands, timeout=1500, output_limit=8 * 1024 * 1024,
                                cwd=ROOT, retain_tail=True)
        render_outcomes(labels, outcomes)
        if not all(outcome.ok for outcome in outcomes):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
