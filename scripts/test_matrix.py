#!/usr/bin/env python3
"""Run one part of the locked verification matrix; each part is one Jig target."""

import argparse
from pathlib import Path
import subprocess
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
CORE_CHECK = ["cargo", "check", "-p", "batter-core", "--lib", "--no-default-features", "--locked"]
FACADE_CHECK = ["cargo", "check", "-p", "batter", "--lib", "--no-default-features", "--locked"]
CORE_TESTS = ["cargo", "nextest", "run", "--profile", "gate", "-p", "batter-core",
              "--no-default-features", "--lib", "--tests", "--locked"]
WORKSPACE_TESTS = ["cargo", "test", "--workspace", "--all-features", "--all-targets", "--locked"]
# A clean developer shell must not hide ambient-state dependencies in tests.
HOSTILE_CONFIGURATION = ["env", "PGDATA=/unused-configuration-fixture", "PGUSER=parent-fixture",
                         "PGPASSWORD=parent-secret-marker", "cargo", "test", "-p",
                         "batter-example-reference-service", "--test", "configuration", "--locked"]
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
RUNLIMIT_CONSUMER = [sys.executable, "scripts/check_runlimit_consumer.py"]
RUNLIMIT_CONSUMER_CONTROLS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                              "-p", "test_runlimit_consumer.py", "-v"]
RUNLIMIT_GRAPH = [sys.executable, "scripts/check_runlimit_workspace.py"]
RUNLIMIT_CONTROLS = [sys.executable, "-m", "unittest", "discover", "-s", "scripts",
                     "-p", "test_runlimit_workspace.py", "-v"]
RUNLIMIT_DEFAULT = ["cargo", "test", "-p", "runlimit-core", "-p", "runlimit-memory",
                    "-p", "runlimit-postgres", "-p", "runlimit-http", "-p", "runlimit-axum", "--locked"]
RUNLIMIT_RELEASE = ["cargo", "test", "-p", "runlimit-memory", "--release", "--locked",
                    "corrupt_quota_state_fails_closed_in_release_builds"]
FACADE_FEATURES = [sys.executable, "scripts/check_facade_features.py"]
FACADE_CACHE_CONTROLS = [sys.executable, "scripts/test_facade_features.py", "-v"]


def nextest_ready():
    try:
        subprocess.run(["cargo", "nextest", "show-config", "version"], cwd=ROOT,
                       check=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        print("Nextest prerequisite failed. Install with: "
              "cargo install cargo-nextest --locked --version 0.9.130", file=sys.stderr)
        return False
    return True


def parts():
    """Map each part to ordered batches; a failed batch stops only its own part.

    Jig runs the parts concurrently with one receipt each, so their exhaustive
    input scopes must cover every file their commands read. Python controls that
    read repository files belong to the part whose scope covers those files;
    `scripts` holds only controls confined to `scripts/`.
    """
    return {
        "workspace": [(["workspace-tests", "configuration-hostile-environment", "reference-runner-controls"],
                       [WORKSPACE_TESTS, HOSTILE_CONFIGURATION, REFERENCE_RUNNER_TESTS])],
        "no-default-features": [(["core-library", "facade-library", "core-tests"],
                                 [CORE_CHECK, FACADE_CHECK, CORE_TESTS])],
        "doctests": [(["doctests"], [DOC_TESTS])],
        # Consumer builds use private target directories. Two batches bound the
        # concurrent builds and keep the shared facade cache users apart.
        "consumers": [(["facade-feature-controls", "runledger-consumer", "runledger-workspace", "runledger-graph-controls"],
                       [FACADE_FEATURES, RUNLEDGER_CONSUMER, RUNLEDGER_GRAPH, RUNLEDGER_CONTROLS]),
                      (["facade-cache-controls", "runlimit-consumer", "runlimit-isolated-features", "runledger-tool-controls"],
                       [FACADE_CACHE_CONTROLS, RUNLIMIT_CONSUMER, RUNLIMIT_FEATURES, RUNLEDGER_TOOL_CONTROLS])],
        "runlimit": [(["runlimit-default", "runlimit-release", "runlimit-workspace"],
                      [RUNLIMIT_DEFAULT, RUNLIMIT_RELEASE, RUNLIMIT_GRAPH])],
        "scripts": [(["runner-controls", "smoke-controls", "sqlx-runner-controls", "runlimit-controls"],
                     [RUNNER_TESTS, SMOKE_TESTS, SQLX_RUNNER_TESTS, RUNLIMIT_CONTROLS]),
                    (["runlimit-consumer-controls"], [RUNLIMIT_CONSUMER_CONTROLS])],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("part", choices=sorted(parts()))
    selected = parser.parse_args().part
    if selected == "no-default-features" and not nextest_ready():
        return 1
    for labels, commands in parts()[selected]:
        print(f"Running {', '.join(labels)}", file=sys.stderr, flush=True)
        outcomes = run_parallel(commands, timeout=1500, output_limit=8 * 1024 * 1024,
                                cwd=ROOT, retain_tail=True)
        render_outcomes(labels, outcomes)
        if not all(outcome.ok for outcome in outcomes):
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
