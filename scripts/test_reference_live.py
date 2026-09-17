"""Controls preventing compile-only or skipped probes from becoming live evidence."""

import json
from pathlib import Path
import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from reference_live import (CASES, DATABASE_CASES, DISPATCH_CASES, EXECUTABLE_CASES, SESSION_CASE,
                            SYNTHETIC_ACQUISITION_CASES, complete_execution, complete_inventory,
                            complete_session_execution)
import reference_live

ROOT = Path(__file__).resolve().parent.parent

PROTECTED_STARTUP = {
    "protected_startup_acquisition_sigterm": SYNTHETIC_ACQUISITION_CASES,
    "protected_startup_acquisition_sigint": SYNTHETIC_ACQUISITION_CASES,
    "protected_startup_schema_sigterm": DATABASE_CASES,
    "protected_startup_schema_sigint": DATABASE_CASES,
    "protected_startup_waiter_loss": DATABASE_CASES,
    "protected_startup_owner_loss": DATABASE_CASES,
    "protected_startup_executable_sigterm": EXECUTABLE_CASES,
    "protected_startup_executable_sigint": EXECUTABLE_CASES,
}


def successful_outputs():
    inventory = "\n".join(f"{case}: test" for case in CASES)
    execution = "\n".join(f"test {case} ... ok" for case in CASES)
    execution += f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"
    session = f"test {SESSION_CASE} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out;"
    state = session.replace(SESSION_CASE, reference_live.STATE_CASE)
    return [b"reference-preflight:ok\n", b"", inventory.encode(), execution.encode(), session.encode(), state.encode()]


class ReferenceLiveControls(unittest.TestCase):
    def test_documented_run_selects_the_production_binary(self):
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        package = next(
            package
            for package in json.loads(result.stdout)["packages"]
            if package["name"] == "batter-example-reference-service"
        )
        self.assertEqual(package["default_run"], "batter-example-reference-service")

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

    def test_native_preflight_and_binary_build_precede_inventory_and_exact_execution(self):
        results = [SimpleNamespace(ok=True, stdout=output, output=output.decode()) for output in successful_outputs()]
        with patch.object(reference_live, "run", side_effect=results) as run, patch("builtins.print") as printed:
            reference_live.main()
        self.assertEqual(run.call_count, 6)
        self.assertIn("reference_preflight", run.call_args_list[0].args[0])
        self.assertEqual(run.call_args_list[1].args[0], reference_live.BINARY_COMMAND)
        self.assertEqual(run.call_args_list[2].args[0][-2:], ["--", "--list"])
        self.assertEqual(run.call_args_list[3].args[0][-2:], ["--include-ignored", "--test-threads=1"])
        self.assertEqual(run.call_args_list[4].args[0], reference_live.SESSION_COMMAND)
        self.assertEqual(run.call_args_list[5].args[0], reference_live.STATE_COMMAND)
        printed.assert_any_call(reference_live.announcement(), flush=True)

    def test_failed_binary_build_stops_before_inventory(self):
        results = [SimpleNamespace(ok=True, stdout=b"reference-preflight:ok\n", output=""),
                   SimpleNamespace(ok=False, stdout=b"", output="build failed")]
        with patch.object(reference_live, "run", side_effect=results) as run, patch("builtins.print"):
            with self.assertRaisesRegex(SystemExit, "Reference process-fixture build failed"):
                reference_live.main()
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[1].args[0][:4], ["cargo", "build", "-p", "batter-example-reference-service"])
        self.assertIn("--bins", run.call_args_list[1].args[0])

    def test_session_probe_cannot_be_skipped_or_replaced_by_summary(self):
        output = f"test {SESSION_CASE} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out;"
        self.assertTrue(complete_session_execution(output))
        self.assertFalse(complete_session_execution(output.split("\n", 1)[1]))
        self.assertFalse(complete_session_execution(output.replace("... ok", "... ignored")))
        self.assertFalse(complete_session_execution(output.replace("1 passed", "0 passed")))

    def test_state_probe_is_required_after_the_main_and_session_suites(self):
        for invalid in (b"", b"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;"):
            outputs = successful_outputs()
            outputs[-1] = invalid
            results = [SimpleNamespace(ok=True, stdout=output, output=output.decode()) for output in outputs]
            with patch.object(reference_live, "run", side_effect=results), patch("builtins.print"):
                with self.assertRaisesRegex(SystemExit, "Provider state boundary probe"):
                    reference_live.main()

    def test_case_classes_are_exact_and_disjoint(self):
        self.assertEqual(len(DATABASE_CASES), 61)
        self.assertEqual(len(SYNTHETIC_ACQUISITION_CASES), 2)
        self.assertEqual(len(EXECUTABLE_CASES), 2)
        self.assertEqual(DISPATCH_CASES, {"child_fixture"})
        self.assertEqual(len(CASES), 66)
        classes = (DATABASE_CASES, SYNTHETIC_ACQUISITION_CASES, EXECUTABLE_CASES, DISPATCH_CASES)
        self.assertEqual(sum(len(cases) for cases in classes), len(CASES))
        for case, expected in PROTECTED_STARTUP.items():
            self.assertIn(case, expected)
        announcement = reference_live.announcement()
        self.assertIn("66 required cases", announcement)
        self.assertIn("61 live database probes", announcement)

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
                      "production_root_registers_provider_worker",
                      "provider_effect_crash_and_restart",
                      "provider_effect_outcome_contracts",
                      "child_fixture"} | set(PROTECTED_STARTUP)
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
        self.assertFalse(complete_inventory(output + "\nunexpected_case: test"))
        self.assertFalse(complete_inventory("0 tests, 0 benchmarks"))

    def test_success_requires_each_case_and_unfiltered_summary(self):
        output = "\n".join(f"test {case} ... ok" for case in CASES)
        summary = f"\ntest result: ok. {len(CASES)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n"
        self.assertTrue(complete_execution(output + summary))
        duplicated = output + f"\ntest {next(iter(CASES))} ... ok"
        self.assertFalse(complete_execution(duplicated + summary))
        self.assertFalse(complete_execution(summary))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) - 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace(f"{len(CASES)} passed", f"{len(CASES) + 1} passed")))
        self.assertFalse(complete_execution(output + summary.replace("0 ignored", "1 ignored")))
        self.assertFalse(complete_execution(output + summary.replace("0 filtered", "1 filtered")))
        self.assertFalse(complete_execution(output + summary.replace("0 failed", "1 failed")))
        self.assertFalse(complete_execution(output.replace("... ok", "... ignored", 1) + summary))
        self.assertFalse(complete_execution(output.replace("... ok", "... FAILED", 1) + summary))


if __name__ == "__main__":
    unittest.main()
