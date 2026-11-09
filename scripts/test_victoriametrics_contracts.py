"""Execution-policy controls independent of Docker and producer implementation."""

from dataclasses import replace
import os
import signal
import sys
import unittest
from unittest.mock import Mock, patch

import check_victoriametrics as live
import test_matrix as matrix
from parallel_process import interrupt_scope
from scheduling_process import ProcessOutcome


class ExecutionContracts(unittest.TestCase):
    completed = ProcessOutcome(3, b"", b"not ready", 0, False, False, True, True, ())

    def test_only_explicit_not_ready_is_retried(self):
        with interrupt_scope() as stop, patch.object(live.time, "sleep"):
            action = Mock(side_effect=[live.PollingNotReady(self.completed), "ready"])
            self.assertEqual(live.eventually(action, stop), "ready")
            self.assertEqual(action.call_count, 2)
            for error in (OSError("unexpected I/O"), ValueError("invalid configuration"),
                          AssertionError("broken invariant"),
                          live.PollingCommandFailure(self.completed)):
                with self.subTest(error=type(error).__name__):
                    action = Mock(side_effect=error)
                    with self.assertRaises(type(error)) as raised:
                        live.eventually(action, stop)
                    self.assertIs(raised.exception, error)
                    self.assertEqual(action.call_count, 1)

    def test_not_ready_requires_complete_clean_child_settlement(self):
        with patch.object(live, "run_parallel", return_value=[self.completed]):
            with self.assertRaises(live.PollingNotReady):
                live.poll("unused", "health")
        for change in ({"watchdog": True}, {"overflow": True}, {"reaped": False},
                       {"output_eof": False}, {"errors": ("interrupted",)}, {"status": 1}):
            outcome = replace(self.completed, **change)
            with self.subTest(change=change), \
                    patch.object(live, "run_parallel", return_value=[outcome]):
                with self.assertRaises(live.PollingCommandFailure) as raised:
                    live.poll("unused", "health")
                self.assertIs(raised.exception.outcome, outcome)

    def test_fixture_work_cannot_launch_after_stop_but_removal_still_runs(self):
        def work(_binary, _name, _stop):
            os.kill(os.getpid(), signal.SIGTERM)
            live.docker_work("inspect", "unused")

        with patch.object(live, "_exercise", side_effect=work), \
                patch.object(live, "_fixture_logs"), \
                patch.object(live, "_remove_fixture") as remove, \
                patch("parallel_process.subprocess.Popen") as launch:
            with self.assertRaises(live.DockerCommandFailure):
                live.run_fixture("producer")
        launch.assert_not_called()
        remove.assert_called_once()

    def test_stop_grace_travels_with_commands_when_labels_and_batches_change(self):
        fast = matrix.Command(["quick"], stop_grace=2)
        fixture = matrix.VICTORIAMETRICS
        completed = replace(self.completed, status=0)
        for labels, commands in [(["renamed-backend"], [fixture]),
                                 (["peer", "another-name"], [fast, fixture])]:
            planned = {"workspace": [(labels, commands)]}
            with self.subTest(labels=labels), patch.object(matrix, "parts", return_value=planned), \
                    patch.object(sys, "argv", ["test_matrix.py", "workspace"]), \
                    patch.object(matrix, "render_outcomes"), \
                    patch.object(matrix, "run_parallel", return_value=[completed] * len(commands)) as run:
                self.assertEqual(matrix.main(), 0)
                self.assertEqual(run.call_args.args[0], [command.argv for command in commands])
                self.assertEqual(run.call_args.kwargs["grace"], live.STOP_GRACE)

    def test_command_rejects_missing_arguments_and_unbounded_stop_grace(self):
        for argv, grace in [([], 10), (["command"], 0), (["command"], float("inf")),
                            (["command"], float("nan"))]:
            with self.subTest(argv=argv, grace=grace), self.assertRaises(ValueError):
                matrix.Command(argv, stop_grace=grace)


if __name__ == "__main__":
    unittest.main()
