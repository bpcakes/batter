"""Exercise facade rejection diagnostics after warming the real Cargo cache."""

from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import check_facade_features as facade


class CachedConsumerTests(unittest.TestCase):
    def test_external_commands_force_offline_sqlx_without_changing_parent(self):
        inherited = {"SQLX_OFFLINE": "false", "DATABASE_URL": "ambient-endpoint-marker"}
        probe = ("import json, os; print(json.dumps([os.environ['SQLX_OFFLINE'], "
                 "os.environ['DATABASE_URL']]))")
        with tempfile.TemporaryDirectory(prefix="facade-environment-control-") as directory, \
                patch.dict(os.environ, inherited):
            observed = facade.execute([sys.executable, "-c", probe], Path(directory))
            self.assertEqual(json.loads(observed), ["true", inherited["DATABASE_URL"]])
            self.assertEqual(os.environ["SQLX_OFFLINE"], "false")
            self.assertEqual(os.environ["DATABASE_URL"], inherited["DATABASE_URL"])

    def test_negative_launches_force_offline_sqlx_without_changing_parent(self):
        inherited = {"SQLX_OFFLINE": "false", "DATABASE_URL": "ambient-endpoint-marker"}
        # Replace Cargo's output, not the runner: every launch is a real child.
        probe = """
import json, os, pathlib, sys
with pathlib.Path('environments.jsonl').open('a') as output:
    output.write(json.dumps([os.environ['SQLX_OFFLINE'], os.environ['DATABASE_URL']]) + '\\n')
if sys.argv[1] == 'test':
    print('test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;')
    sys.exit(0)
source = pathlib.Path('src/main.rs').read_text()
if 'fn forge(' in source:
    print('error[E0451]: field `report` is private', file=sys.stderr)
elif 'fn discard(' in source:
    print('unused_must_use: SharedShutdownReport', file=sys.stderr)
else:
    print('error[E0432]: unresolved import batter::runledger::native::test_support', file=sys.stderr)
sys.exit(101)
"""
        root_lock = (facade.ROOT / "Cargo.lock").read_bytes()
        for kind, count in (("feature", 1), ("checked-completion", 3)):
            with self.subTest(kind=kind), \
                    tempfile.TemporaryDirectory(prefix="facade-negative-environment-") as directory, \
                    patch.dict(os.environ, inherited), \
                    patch.object(facade, "run_metadata", return_value={}), \
                    patch.object(facade, "check_graph"), \
                    patch.object(facade, "normal_names", return_value=set()):
                parent = Path(directory)
                cargo = [sys.executable, "-c", probe]
                if kind == "feature":
                    facade.run_negative_case(cargo, "host", root_lock, parent, parent / "target",
                                             ("runledger",), "runledger-test-support",
                                             "batter::runledger::native::test_support")
                else:
                    with redirect_stdout(io.StringIO()):
                        facade.run_checked_completion_case(cargo, "host", root_lock, set(),
                                                           parent, parent / "target")
                captures = list(parent.glob("*/environments.jsonl"))
                self.assertEqual(len(captures), 1)
                observed = [json.loads(line) for line in captures[0].read_text().splitlines()]
                self.assertEqual(observed, [["true", inherited["DATABASE_URL"]]] * count)
                self.assertEqual(os.environ["SQLX_OFFLINE"], "false")
                self.assertEqual(os.environ["DATABASE_URL"], inherited["DATABASE_URL"])

    def test_warm_cache_preserves_disabled_imports_and_rejects_false_negatives(self):
        toolchain = os.environ.get("RUSTUP_TOOLCHAIN") or facade.execute(
            ["rustup", "show", "active-toolchain"], facade.ROOT).split()[0]
        cargo = ["cargo", "+" + toolchain]
        rustc = facade.execute(["rustc", "+" + toolchain, "-vV"], facade.ROOT)
        host = next(line.split(": ", 1)[1] for line in rustc.splitlines()
                    if line.startswith("host: "))
        baseline = json.loads(facade.execute(
            cargo + ["metadata", "--format-version", "1", "--filter-platform", host,
                     "--all-features", "--locked"], facade.ROOT))
        target = facade.consumer_target(baseline, rustc)
        root_lock = (facade.ROOT / "Cargo.lock").read_bytes()
        with tempfile.TemporaryDirectory(prefix="facade-cache-control-") as directory:
            parent = Path(directory)
            facade.run_positive_case(cargo, host, root_lock, facade.source_tuples(baseline),
                                     parent, target, ("at-rest",))
            facade.run_negative_case(cargo, host, root_lock, parent, target,
                                     (), "at-rest", "batter::at_rest")
            # An enabled import must invalidate a purported rejection, even
            # after a different consumer with the same package name failed.
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()), \
                    self.assertRaisesRegex(RuntimeError, "did not fail at the intended API"):
                facade.run_negative_case(cargo, host, root_lock, parent, target,
                                         (), "enabled-control", "batter::operation::OperationContext")
            # Nor may that successful consumer mask the next disabled import.
            facade.run_negative_case(cargo, host, root_lock, parent, target,
                                     (), "disabled-control", "batter::at_rest")
        self.assertTrue(target.is_dir())
        self.assertEqual((facade.ROOT / "Cargo.lock").read_bytes(), root_lock)


if __name__ == "__main__":
    unittest.main()
