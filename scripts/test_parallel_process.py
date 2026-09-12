"""Failure and coverage controls for concurrent local verification."""

import errno
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

import parallel_process as parallel
import scheduling_controls as controls
from scheduling_process import ProcessOutcome
import test_matrix as matrix


def python(code):
    return [sys.executable, "-c", code]


def run(commands, **kwargs):
    return parallel.run_parallel(commands, timeout=kwargs.pop("timeout", 5),
                                 output_limit=kwargs.pop("output_limit", 4096),
                                 grace=0.1, reap_allowance=0.5, **kwargs)


class ParallelTests(unittest.TestCase):
    def test_commands_overlap_and_preserve_ordered_results(self):
        with tempfile.TemporaryDirectory() as directory:
            commands = []
            for index in range(2):
                commands.append(python(f"""
import time
from pathlib import Path
root = Path({directory!r})
(root / '{index}').touch()
deadline = time.monotonic() + 3
while not (root / '{1-index}').exists():
    if time.monotonic() >= deadline:
        raise SystemExit(8)
    time.sleep(.01)
print('worker-{index}')
"""))
            outcomes = run(commands)
        self.assertTrue(all(outcome.ok for outcome in outcomes), outcomes)
        self.assertEqual([outcome.stdout for outcome in outcomes], [b"worker-0\n", b"worker-1\n"])

    def test_all_failures_and_spawn_failure_are_retained(self):
        outcomes = run([python("print('first'); raise SystemExit(3)"),
                        ["/nonexistent/batter-command"],
                        python("print('second'); raise SystemExit(7)")])
        self.assertEqual([o.status for o in outcomes], [3, None, 7])
        self.assertEqual(outcomes[0].stdout, b"first\n")
        self.assertEqual(outcomes[2].stdout, b"second\n")
        self.assertIn("launch:FileNotFoundError", outcomes[1].errors[0])

    def test_overflow_keeps_draining_but_cannot_pass(self):
        outcome, = run([python("import os; os.write(1, b'x' * 200000)")], output_limit=32)
        self.assertEqual(outcome.status, 0)
        self.assertTrue(outcome.reaped and outcome.output_eof and outcome.overflow)
        self.assertFalse(outcome.ok)
        self.assertEqual(len(outcome.stdout) + len(outcome.stderr), 32)

    def test_head_tail_is_exact_without_overflow_and_marks_only_real_gaps(self):
        for limit in (1, 7, 8, 9):
            with self.subTest(limit=limit):
                outcome, = run([python("import os; os.write(1, b'abcdefgh')")],
                                output_limit=limit, retain_tail=True)
                if limit >= 8:
                    self.assertTrue(outcome.ok)
                    self.assertEqual(outcome.stdout, b"abcdefgh")
                else:
                    self.assertFalse(outcome.ok)
                    self.assertTrue(outcome.overflow and outcome.reaped and outcome.output_eof)
                    head = b"abcdefgh"[:limit // 2]
                    tail = b"abcdefgh"[-(limit - limit // 2):]
                    self.assertEqual(outcome.stdout, head + parallel.Capture.OMITTED + tail)
                self.assertEqual(outcome.stderr, b"")

    def test_head_tail_preserves_separate_streams_at_the_shared_limit(self):
        outcome, = run([python("import os; os.write(1, b'abcd'); os.write(2, b'WXYZ')")],
                        output_limit=8, retain_tail=True)
        self.assertTrue(outcome.ok)
        self.assertEqual((outcome.stdout, outcome.stderr), (b"abcd", b"WXYZ"))

    def test_head_tail_keeps_final_errors_on_both_streams_after_overflow(self):
        code = """
import sys
sys.stdout.buffer.write(b'HEAD\\n' + b'x' * 200000 + b'\\nSTDOUT_END\\n')
sys.stdout.flush()
sys.stderr.buffer.write(b'y' * 200000 + b'\\nSTDERR_ERROR\\n')
sys.stderr.flush()
sys.stdout.buffer.write(b'FINAL_FAILURE\\n')
sys.stdout.flush()
raise SystemExit(7)
"""
        outcome, = run([python(code)], output_limit=4096, retain_tail=True)
        self.assertEqual(outcome.status, 7)
        self.assertTrue(outcome.overflow and outcome.reaped and outcome.output_eof)
        self.assertFalse(outcome.ok)
        self.assertTrue(outcome.stdout.startswith(b"HEAD\n"))
        self.assertIn(b"FINAL_FAILURE\n", outcome.stdout)
        self.assertTrue(outcome.stderr.endswith(b"STDERR_ERROR\n"))
        self.assertIn(parallel.Capture.OMITTED, outcome.stdout)
        self.assertIn(parallel.Capture.OMITTED, outcome.stderr)
        retained = sum(len(stream.replace(parallel.Capture.OMITTED, b""))
                       for stream in (outcome.stdout, outcome.stderr))
        self.assertEqual(retained, 4096)

    def test_timeout_kills_and_reaps_an_interrupt_ignoring_child(self):
        outcome, = run([python("import signal,time; signal.signal(signal.SIGINT, signal.SIG_IGN); "
                               "print('started',flush=True); time.sleep(30)")], timeout=0.3)
        self.assertEqual(outcome.status, -signal.SIGKILL)
        self.assertTrue(outcome.watchdog and outcome.reaped and outcome.output_eof)
        self.assertIn(b"started", outcome.stdout)
        self.assertLess(outcome.elapsed_seconds, 2)

    def test_capture_setup_failure_settles_every_acquired_child(self):
        children = []
        popen, capture = subprocess.Popen, parallel.Capture

        def spawn(*args, **kwargs):
            child = popen(*args, **kwargs)
            children.append(child)
            return child

        def acquire(child, limit, **kwargs):
            if len(children) == 2:
                raise OSError(errno.EMFILE, "capture fixture")
            return capture(child, limit, **kwargs)

        with mock.patch.object(parallel.subprocess, "Popen", spawn), \
                mock.patch.object(parallel, "Capture", acquire):
            outcomes = run([python("print('peer')"), python("import time; time.sleep(30)")])
        self.assertTrue(outcomes[0].ok)
        self.assertFalse(outcomes[1].ok)
        self.assertTrue(all(child.poll() is not None for child in children))
        self.assertTrue(all(child.stdout.closed and child.stderr.closed for child in children))

    def test_interrupt_during_launch_stops_admission_and_restores_handlers(self):
        popen = subprocess.Popen
        children = []
        previous = signal.signal(signal.SIGTERM, signal.SIG_DFL)
        original_int = signal.getsignal(signal.SIGINT)

        def spawn(*args, **kwargs):
            child = popen(*args, **kwargs)
            children.append(child)
            os.kill(os.getpid(), signal.SIGTERM)
            return child

        try:
            with mock.patch.object(parallel.subprocess, "Popen", spawn):
                outcomes = run([python("import time; time.sleep(30)"), python("print('forbidden')")])
            self.assertEqual(len(children), 1)
            self.assertTrue(outcomes[0].reaped)
            self.assertIsNone(outcomes[1].status)
            self.assertFalse(any(outcome.ok for outcome in outcomes))
            self.assertIn("interrupted", outcomes[1].errors)
            self.assertEqual(signal.getsignal(signal.SIGTERM), signal.SIG_DFL)
            self.assertIs(signal.getsignal(signal.SIGINT), original_int)
        finally:
            signal.signal(signal.SIGTERM, previous)

    def test_reaped_group_is_not_signalled_when_a_later_worker_times_out(self):
        signals = []
        killpg = os.killpg

        def kill(group, sig):
            signals.append(group)
            return killpg(group, sig)

        with mock.patch.object(parallel.os, "killpg", kill):
            outcomes = run([python("import os; print(os.getpid())"),
                            python("import time; time.sleep(30)")], timeout=0.2)
        self.assertTrue(outcomes[0].ok)
        self.assertNotIn(int(outcomes[0].stdout), signals)
        self.assertFalse(outcomes[1].ok)

    def test_repeated_signals_do_not_restart_settlement(self):
        observe = parallel.Capture.observe_until
        previous = signal.signal(signal.SIGTERM, signal.SIG_DFL)
        sent = 0

        def observe_and_signal(capture, child, deadline, **kwargs):
            nonlocal sent
            observe(capture, child, deadline, **kwargs)
            if b"armed" in capture.buffers[0]:
                os.kill(os.getpid(), signal.SIGTERM)
                sent += 1

        try:
            with mock.patch.object(parallel.Capture, "observe_until", observe_and_signal):
                outcome, = run([python("import signal,time; "
                                       "signal.signal(signal.SIGINT, signal.SIG_IGN); "
                                       "print('armed',flush=True); time.sleep(30)")])
            self.assertGreater(sent, 2)
            self.assertEqual(outcome.status, -signal.SIGKILL)
            self.assertTrue(outcome.reaped)
            self.assertIn("interrupted", outcome.errors)
            self.assertLess(outcome.elapsed_seconds, 2)
        finally:
            signal.signal(signal.SIGTERM, previous)

    def test_ignored_sigint_is_preserved_in_parent_and_child(self):
        previous = signal.signal(signal.SIGINT, signal.SIG_IGN)
        try:
            outcome, = run([python("import signal; "
                                   "assert signal.getsignal(signal.SIGINT) == signal.SIG_IGN")])
            self.assertTrue(outcome.ok)
            self.assertEqual(signal.getsignal(signal.SIGINT), signal.SIG_IGN)
        finally:
            signal.signal(signal.SIGINT, previous)

    def test_read_failure_retains_partial_output_and_reaps_child(self):
        observe = parallel.Capture.observe_until

        def broken(capture, child, deadline, **kwargs):
            observe(capture, child, deadline, **kwargs)
            if capture.buffers[0]:
                raise OSError(errno.EIO, "read fixture")

        with mock.patch.object(parallel.Capture, "observe_until", broken):
            outcome, = run([python("import time; print('checkpoint',flush=True); time.sleep(30)")])
        self.assertFalse(outcome.ok)
        self.assertTrue(outcome.reaped)
        self.assertIn(b"checkpoint", outcome.stdout)
        self.assertIn(f"capture:{errno.EIO}", outcome.errors)

    def test_invalid_bounds_and_custom_handler_launch_nothing(self):
        for timeout in (0, float("inf"), float("nan")):
            with self.assertRaises(ValueError):
                run([python("pass")], timeout=timeout)
        previous = signal.signal(signal.SIGINT, lambda *_: None)
        try:
            with mock.patch.object(parallel.subprocess, "Popen") as spawn:
                outcome, = run([python("pass")])
            spawn.assert_not_called()
            self.assertEqual(outcome.errors, ("unsupported-signal-owner",))
        finally:
            signal.signal(signal.SIGINT, previous)


class ShardTests(unittest.TestCase):
    def suite(self):
        import test_scheduling_process as tests
        return tests.load_tests(unittest.defaultTestLoader, None, None)

    def test_partition_is_complete_disjoint_and_deterministic(self):
        for binary in (None, Path("fixture")):
            with mock.patch("test_scheduling_process.BINARY", binary):
                suite = self.suite()
                expected = sorted(t.id() for t in controls.cases(suite))
                self.assertEqual(len(expected), 24 if binary is None else 30)
                for count in range(1, 5):
                    first = [[t.id() for t in shard] for shard in controls.partition(suite, count)]
                    second = [[t.id() for t in shard] for shard in controls.partition(suite, count)]
                    self.assertEqual(first, second)
                    self.assertEqual(sorted(t for shard in first for t in shard), expected)

    def test_new_discovered_control_is_never_omitted_by_scheduling_hints(self):
        suite = self.suite()
        extra = unittest.FunctionTestCase(lambda: None)
        suite.addTest(extra)
        assigned = [test for shard in controls.partition(suite, 4) for test in shard]
        self.assertEqual(sum(test is extra for test in assigned), 1)

    def test_missing_duplicate_partial_or_failed_completion_cannot_pass(self):
        expected = list(controls.cases(self.suite()))[:2]
        valid = {"tests": sorted(t.id() for t in expected), "successful": True}

        def outcome(text, status=0):
            return ProcessOutcome(status, text.encode(), b"", 0, False, False, True, True, ())

        text = controls.MARKER + json.dumps(valid) + "\n"
        self.assertTrue(controls.completed(outcome(text), expected))
        for bad in ("", controls.MARKER + "{", text + text,
                    controls.MARKER + json.dumps({**valid, "tests": valid["tests"][:1]}),
                    controls.MARKER + json.dumps({**valid, "successful": False})):
            self.assertFalse(controls.completed(outcome(bad), expected))
        self.assertFalse(controls.completed(outcome(text, 7), expected))

    def test_duplicate_discovery_is_rejected(self):
        test = next(controls.cases(self.suite()))
        with self.assertRaises(ValueError):
            controls.partition(unittest.TestSuite([test, test]), 4)


class MatrixTests(unittest.TestCase):
    def test_matrix_scale_overflow_renders_context_and_final_failure(self):
        command = python("import sys; sys.stdout.buffer.write(b'BUILD_START\\n' + "
                         "b'x' * (9 * 1024 * 1024) + b'\\nFINAL_TEST_FAILURE\\n'); "
                         "sys.stdout.flush(); sys.stderr.write('failure details\\n'); "
                         "raise SystemExit(7)")
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, "argv", ["test_matrix.py"]), \
                mock.patch.object(matrix, "CORE_CHECK", command), \
                mock.patch.object(matrix, "RUNNER_TESTS", python("print('peer completed')")), \
                mock.patch.object(matrix, "SMOKE_TESTS", python("print('smoke peer completed')")), \
                mock.patch.object(matrix, "REFERENCE_RUNNER_TESTS", python("print('reference controls completed')")), \
                mock.patch.object(matrix, "SQLX_RUNNER_TESTS", python("print('sqlx controls completed')")), \
                redirect_stdout(stdout), redirect_stderr(stderr):
            self.assertEqual(matrix.main(), 1)
        rendered = stdout.getvalue()
        self.assertIn("BUILD_START\n", rendered)
        self.assertIn("FINAL_TEST_FAILURE\n", rendered)
        self.assertIn("failure details\n", rendered)
        self.assertIn("peer completed\n", rendered)
        self.assertIn("smoke peer completed\n", rendered)
        self.assertIn("reference controls completed\n", rendered)
        self.assertIn(parallel.Capture.OMITTED.decode(), rendered)
        self.assertIn('"overflow": true', stderr.getvalue())
        self.assertIn('"status": 7', stderr.getvalue())
        self.assertLess(len(rendered), 8 * 1024 * 1024 + 1024)

    def test_runtime_passes_share_a_batch_between_check_and_doctests(self):
        success = ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())
        batches = []

        def execute(commands, **kwargs):
            batches.append(commands)
            return [success] * len(commands)

        with mock.patch.object(sys, "argv", ["test_matrix.py"]), \
                mock.patch.object(matrix, "run_parallel", execute), \
                mock.patch.object(matrix, "render_outcomes"):
            self.assertEqual(matrix.main(), 0)
        self.assertEqual([len(batch) for batch in batches], [4, 1, 3, 1])
        self.assertEqual(batches[0][1], matrix.RUNNER_TESTS)
        self.assertEqual(batches[0][3], matrix.REFERENCE_RUNNER_TESTS)
        self.assertEqual(batches[1][0], matrix.SQLX_RUNNER_TESTS)
        self.assertIn("--no-default-features", batches[0][0])
        self.assertIn("test_parallel_process.py", batches[0][1])
        self.assertIn("scripts/test_smoke_postgres.py", batches[0][2])
        self.assertIn("--no-default-features", batches[2][0])
        self.assertIn("--all-targets", batches[2][1])
        self.assertIn("--workspace", batches[2][1])
        self.assertEqual(batches[2][2][0], "env")
        self.assertIn("PGDATA=/unused-configuration-fixture", batches[2][2])
        self.assertIn("PGPASSWORD=parent-secret-marker", batches[2][2])
        self.assertIn("configuration", batches[2][2])
        self.assertIn("--locked", batches[2][2])
        self.assertIn("--doc", batches[3][0])
        self.assertTrue(all("--locked" in command for batch in batches for command in batch
                            if command[0] == "cargo"))

    def test_failure_of_any_prerequisite_or_runtime_pass_stops_later_batches(self):
        success = ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())
        failure = ProcessOutcome(7, b"failure", b"", 0, False, False, True, True, ())
        batches = [
            [[failure, success, success, success]],
            [[success, failure, success, success]],
            [[success, success, failure, success]],
            [[success, success, success, failure]],
            [[success, success, success, success], [failure, success, success]],
            [[success, success, success, success], [success, failure, success]],
            [[success, success, success, success], [success, success, failure]],
        ]
        for results in batches:
            with self.subTest(results=results), \
                    mock.patch.object(sys, "argv", ["test_matrix.py"]), \
                    mock.patch.object(matrix, "run_parallel",
                                      side_effect=results) as execute, \
                    mock.patch.object(matrix, "render_outcomes"):
                self.assertEqual(matrix.main(), 1)
                self.assertEqual(execute.call_count, len(results))


class MutationCopyTests(unittest.TestCase):
    def test_copied_control_entrypoint_runs_without_repository_imports(self):
        import check_scheduling_mutation as mutation
        root = Path(__file__).resolve().parent.parent
        with tempfile.TemporaryDirectory() as directory:
            subject = Path(directory)
            mutation.copy_scripts(root, subject)
            # Exercise the actual copied shard entrypoint, selecting a quick real
            # control through unittest discovery rather than mocking execution.
            command = [sys.executable, "-I", "-c", """
import runpy, sys, unittest
from pathlib import Path
script = Path.cwd() / 'scripts/test_scheduling_process.py'
sys.path.insert(0, str(script.parent))
unittest.defaultTestLoader.testNamePatterns = ['*.test_success_retains_separate_streams']
sys.argv = [str(script), '--jobs', '1', '--shard', '0']
runpy.run_path(str(script), run_name='__main__')
"""]
            outcome, = run([command], cwd=subject)
        self.assertTrue(outcome.ok, outcome.output)
        self.assertIn('CONTROL_RESULT ', outcome.output)
        self.assertIn('test_success_retains_separate_streams', outcome.output)
        self.assertIn('Ran 1 test', outcome.output)


if __name__ == "__main__":
    unittest.main(verbosity=2)
