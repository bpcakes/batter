#!/usr/bin/env python3
"""Run the locked core/workspace test matrix with concurrent runtime passes."""

import argparse
from pathlib import Path
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
CORE_CHECK = ["cargo", "check", "-p", "batter-core", "--lib", "--no-default-features", "--locked"]
FACADE_CHECK = ["cargo", "check", "-p", "batter", "--lib", "--no-default-features", "--locked"]
RUNTIME_TESTS = [
    ["cargo", "test", "-p", "batter-core", "--no-default-features", "--lib", "--tests", "--locked"],
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
SQLX_RUNNER_TESTS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                     "-p", "test_sqlx_live.py", "-v"]
RUNLIMIT_FEATURES = [sys.executable, "scripts/check_runlimit_features.py"]
RUNLEDGER_GRAPH = [sys.executable, "scripts/check_runledger_workspace.py"]
RUNLEDGER_CONTROLS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                      "-p", "test_runledger_workspace.py", "-v"]
RUNLEDGER_CONSUMER = [sys.executable, "scripts/check_runledger_consumer.py"]
RUNLEDGER_TOOL_CONTROLS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                          "-p", "test_runledger_tools.py", "-v"]
RUNLIMIT_GRAPH = [sys.executable, "scripts/check_runlimit_workspace.py"]
RUNLIMIT_CONTROLS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                     "-p", "test_runlimit_workspace.py", "-v"]
RUNLIMIT_DEFAULT = ["cargo", "test", "-p", "runlimit-core", "-p", "runlimit-memory",
                    "-p", "runlimit-postgres", "-p", "runlimit-http", "-p", "runlimit-axum", "--locked"]
RUNLIMIT_RELEASE = ["cargo", "test", "-p", "runlimit-memory", "--release", "--locked",
                    "corrupt_quota_state_fails_closed_in_release_builds"]
FACADE_FEATURES = [sys.executable, "scripts/check_facade_features.py"]


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    for labels, commands in [(["core-library", "facade-library", "runner-controls", "smoke-controls"],
                              [CORE_CHECK, FACADE_CHECK, RUNNER_TESTS, SMOKE_TESTS]),
                             (["reference-runner-controls", "sqlx-runner-controls", "facade-feature-controls"],
                              [REFERENCE_RUNNER_TESTS, SQLX_RUNNER_TESTS, FACADE_FEATURES]),
                             (["runledger-workspace", "runledger-graph-controls", "runledger-consumer", "runledger-tool-controls"],
                              [RUNLEDGER_GRAPH, RUNLEDGER_CONTROLS, RUNLEDGER_CONSUMER, RUNLEDGER_TOOL_CONTROLS]),
                             (["core-tests", "workspace-tests", "configuration-hostile-environment"], RUNTIME_TESTS),
                             (["doctests"], [DOC_TESTS]),
                             (["runlimit-isolated-features", "runlimit-workspace", "runlimit-controls", "runlimit-default"],
                              [RUNLIMIT_FEATURES, RUNLIMIT_GRAPH, RUNLIMIT_CONTROLS, RUNLIMIT_DEFAULT]),
                             (["runlimit-release"], [RUNLIMIT_RELEASE])]:
        print(f"Running {', '.join(labels)}", file=sys.stderr, flush=True)
        outcomes = run_parallel(commands, timeout=1500, output_limit=8 * 1024 * 1024,
                                cwd=ROOT, retain_tail=True)
        render_outcomes(labels, outcomes)
        if not all(outcome.ok for outcome in outcomes):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
