"""Exercise facade rejection diagnostics after warming the real Cargo cache."""

from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import tempfile
import unittest

import check_facade_features as facade


class CachedConsumerTests(unittest.TestCase):
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
