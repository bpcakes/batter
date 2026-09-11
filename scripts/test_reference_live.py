"""Controls preventing compile-only or skipped probes from becoming live evidence."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from reference_live import CASES, complete_execution, complete_inventory
import reference_live


class ReferenceLiveControls(unittest.TestCase):
    def test_failed_native_preflight_stops_before_inventory_or_fixtures(self):
        for result in (SimpleNamespace(ok=True, stdout=b"wrong-marker", output="rejected"),
                       SimpleNamespace(ok=False, stdout=b"reference-preflight:ok", output="rejected")):
            with patch.object(reference_live, "run", return_value=result) as run, patch("builtins.print"):
                with self.assertRaisesRegex(SystemExit, "Native PostgreSQL preflight failed"):
                    reference_live.main()
                self.assertEqual(run.call_count, 1)
                command = run.call_args.args[0]
                self.assertEqual(command[:3], ["cargo", "run", "--quiet"])
                self.assertIn("reference_preflight", command)

    def test_native_preflight_precedes_inventory_and_exact_execution(self):
        inventory = "\n".join(f"{case}: test" for case in CASES)
        execution = "\n".join(f"test {case} ... ok" for case in CASES)
        execution += f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
        outputs = [b"reference-preflight:ok\n", inventory.encode(), execution.encode()]
        results = [SimpleNamespace(ok=True, stdout=output, output=output.decode()) for output in outputs]
        with patch.object(reference_live, "run", side_effect=results) as run, patch("builtins.print"):
            reference_live.main()
        self.assertEqual(run.call_count, 3)
        self.assertIn("reference_preflight", run.call_args_list[0].args[0])
        self.assertEqual(run.call_args_list[1].args[0][-2:], ["--ignored", "--list"])
        self.assertEqual(run.call_args_list[2].args[0][-2:], ["--ignored", "--test-threads=1"])

    def test_configuration_cases_cannot_be_omitted_from_inventory_or_execution(self):
        configured = {"configured_pool_capacity_and_acquire_timeout",
                      "configured_startup_pool_close_before_lease",
                      "configured_worker_concurrency"}
        self.assertTrue(configured <= CASES)
        for missing in configured:
            inventory = "\n".join(f"{case}: test" for case in CASES if case != missing)
            self.assertFalse(complete_inventory(inventory))
            output = "\n".join(f"test {case} ... ok" for case in CASES if case != missing)
            summary = f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
            self.assertFalse(complete_execution(output + summary))

    def test_inventory_requires_every_named_ignored_case(self):
        output = "\n".join(f"{case}: test" for case in CASES)
        self.assertTrue(complete_inventory(output))
        self.assertFalse(complete_inventory(output.replace(next(iter(CASES)), "unexpected")))
        self.assertFalse(complete_inventory("0 tests, 0 benchmarks"))

    def test_success_requires_each_case_and_unfiltered_summary(self):
        output = "\n".join(f"test {case} ... ok" for case in CASES)
        summary = f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n"
        self.assertTrue(complete_execution(output + summary))
        self.assertFalse(complete_execution(summary))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) - 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) + 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace("0 ignored", "1 ignored")))
        self.assertFalse(complete_execution(output + summary.replace("0 filtered", "1 filtered")))
        self.assertFalse(complete_execution(output.replace("... ok", "... ignored", 1) + summary))


if __name__ == "__main__":
    unittest.main()
