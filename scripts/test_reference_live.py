"""Controls preventing compile-only or skipped probes from becoming live evidence."""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from reference_live import CASES, SESSION_CASE, complete_execution, complete_inventory, complete_session_execution
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
        session = f"test {SESSION_CASE} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out;"
        outputs = [b"reference-preflight:ok\n", inventory.encode(), execution.encode(), session.encode()]
        results = [SimpleNamespace(ok=True, stdout=output, output=output.decode()) for output in outputs]
        with patch.object(reference_live, "run", side_effect=results) as run, patch("builtins.print"):
            reference_live.main()
        self.assertEqual(run.call_count, 4)
        self.assertIn("reference_preflight", run.call_args_list[0].args[0])
        self.assertEqual(run.call_args_list[1].args[0][-2:], ["--", "--list"])
        self.assertEqual(run.call_args_list[2].args[0][-2:], ["--include-ignored", "--test-threads=1"])
        self.assertEqual(run.call_args_list[3].args[0], reference_live.SESSION_COMMAND)

    def test_session_probe_cannot_be_skipped_or_replaced_by_summary(self):
        output = f"test {SESSION_CASE} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out;"
        self.assertTrue(complete_session_execution(output))
        self.assertFalse(complete_session_execution(output.split("\n", 1)[1]))
        self.assertFalse(complete_session_execution(output.replace("... ok", "... ignored")))
        self.assertFalse(complete_session_execution(output.replace("1 passed", "0 passed")))

    def test_configuration_cases_cannot_be_omitted_from_inventory_or_execution(self):
        configured = {"retirement_preserves_history_and_disables_old_catalog",
                      "retirement_rejects_wrong_identity",
                      "retirement_rejects_hidden_sessions",
                      "retirement_observes_late_enqueue",
                      "retirement_rejects_prepared_enqueue",
                      "retirement_retains_commit_error_through_readback_cancellation",
                      "retirement_retains_lost_commit_acknowledgement",
                      "native_initialization_without_queue_writes",
                      "native_in_flight_finishes_after_drain",
                      "native_owner_drop_retains_settlement",
                      "native_business_failure_preserves_process",
                      "native_unjoined_callback_blocks_dependency_cleanup",
                      "configured_command_root_bounds",
                      "configured_pool_capacity_and_acquire_timeout",
                      "configured_startup_pool_close_before_lease",
                      "configured_worker_concurrency",
                      "startup_signal_during_schema_initialization",
                      "startup_signal_during_pool_acquisition"}
        self.assertTrue(configured <= CASES)
        for missing in configured:
            inventory = "\n".join(f"{case}: test" for case in CASES if case != missing)
            self.assertFalse(complete_inventory(inventory))
            output = "\n".join(f"test {case} ... ok" for case in CASES if case != missing)
            summary = f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
            self.assertFalse(complete_execution(output + summary))

    def test_inventory_requires_every_named_case(self):
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
