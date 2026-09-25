"""Failure and coverage controls for concurrent local verification."""

import errno
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

import parallel_process as parallel
import check_runlimit_features as runlimit_features
import scheduling_controls as controls
from scheduling_process import ProcessOutcome
import test_matrix as matrix


class RunlimitFeatureInventoryTests(unittest.TestCase):
    def test_manifest_feature_addition_fails_until_graph_expectations_are_updated(self):
        metadata = {"packages": [{"name": "batter-runlimit", "features": {
            "default": [], "memory": [], "postgres": [], "axum": []}}]}
        self.assertEqual(runlimit_features.declared_features(metadata),
                         ("axum", "memory", "postgres"))
        metadata["packages"][0]["features"]["additional"] = []
        with self.assertRaisesRegex(RuntimeError, "unmapped=\\['additional'\\]"):
            runlimit_features.declared_features(metadata)


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
    success = ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())
    failure = ProcessOutcome(7, b"failure", b"", 0, False, False, True, True, ())

    def setUp(self):
        prerequisite = mock.patch.object(matrix, "nextest_ready", return_value=True)
        self.nextest_ready = prerequisite.start()
        self.addCleanup(prerequisite.stop)

    def test_nextest_prerequisite_failure_prevents_core_batch(self):
        self.nextest_ready.return_value = False
        with mock.patch.object(sys, "argv", ["test_matrix.py", "no-default-features"]), \
                mock.patch.object(matrix, "run_parallel") as execute:
            self.assertEqual(matrix.main(), 1)
        execute.assert_not_called()

    def test_other_parts_do_not_require_nextest(self):
        self.nextest_ready.return_value = False
        self.assertEqual(self.run_part("workspace", lambda commands, **kwargs: [self.success] * len(commands)), 0)
        self.nextest_ready.assert_not_called()

    def run_part(self, part, execute):
        with mock.patch.object(sys, "argv", ["test_matrix.py", part]), \
                mock.patch.object(matrix, "run_parallel", execute), \
                mock.patch.object(matrix, "render_outcomes"):
            return matrix.main()

    def test_matrix_scale_overflow_renders_context_and_final_failure(self):
        command = python("import sys; sys.stdout.buffer.write(b'BUILD_START\\n' + "
                         "b'x' * (9 * 1024 * 1024) + b'\\nFINAL_TEST_FAILURE\\n'); "
                         "sys.stdout.flush(); sys.stderr.write('failure details\\n'); "
                         "raise SystemExit(7)")
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, "argv", ["test_matrix.py", "no-default-features"]), \
                mock.patch.object(matrix, "CORE_CHECK", command), \
                mock.patch.object(matrix, "FACADE_CHECK", python("print('peer completed')")), \
                mock.patch.object(matrix, "CORE_TESTS", python("print('tests peer completed')")), \
                redirect_stdout(stdout), redirect_stderr(stderr):
            self.assertEqual(matrix.main(), 1)
        rendered = stdout.getvalue()
        self.assertIn("BUILD_START\n", rendered)
        self.assertIn("FINAL_TEST_FAILURE\n", rendered)
        self.assertIn("failure details\n", rendered)
        self.assertIn("peer completed\n", rendered)
        self.assertIn("tests peer completed\n", rendered)
        self.assertIn(parallel.Capture.OMITTED.decode(), rendered)
        self.assertIn('"overflow": true', stderr.getvalue())
        self.assertIn('"status": 7', stderr.getvalue())
        self.assertLess(len(rendered), 8 * 1024 * 1024 + 1024)

    def test_parts_partition_every_command_into_bounded_batches(self):
        batches = {}
        for part in matrix.parts():
            recorded = batches.setdefault(part, [])

            def execute(commands, recorded=recorded, **kwargs):
                recorded.append(commands)
                return [self.success] * len(commands)

            self.assertEqual(self.run_part(part, execute), 0)
        self.assertEqual({part: [len(batch) for batch in recorded] for part, recorded in batches.items()},
                         {"workspace": [3], "no-default-features": [3], "doctests": [1],
                          "consumers": [4, 4], "runlimit": [3], "scripts": [4, 1]})
        commands = [command for recorded in batches.values() for batch in recorded for command in batch]
        self.assertCountEqual(commands, [
            matrix.CORE_CHECK, matrix.FACADE_CHECK, matrix.CORE_TESTS, matrix.WORKSPACE_TESTS,
            matrix.HOSTILE_CONFIGURATION, matrix.DOC_TESTS, matrix.RUNNER_TESTS, matrix.SMOKE_TESTS,
            matrix.REFERENCE_RUNNER_TESTS, matrix.SQLX_RUNNER_TESTS, matrix.RUNLIMIT_FEATURES,
            matrix.RUNLEDGER_GRAPH, matrix.RUNLEDGER_CONTROLS, matrix.RUNLEDGER_CONSUMER,
            matrix.RUNLEDGER_TOOL_CONTROLS, matrix.RUNLIMIT_CONSUMER, matrix.RUNLIMIT_CONSUMER_CONTROLS,
            matrix.RUNLIMIT_GRAPH, matrix.RUNLIMIT_CONTROLS, matrix.RUNLIMIT_DEFAULT,
            matrix.RUNLIMIT_RELEASE, matrix.FACADE_FEATURES, matrix.FACADE_CACHE_CONTROLS,
        ])
        self.assertEqual(len(commands), 23)
        workspace, = batches["workspace"]
        self.assertIn("--all-targets", workspace[0])
        self.assertIn("--workspace", workspace[0])
        self.assertEqual(workspace[1][0], "env")
        self.assertIn("PGDATA=/unused-configuration-fixture", workspace[1])
        self.assertIn("PGPASSWORD=parent-secret-marker", workspace[1])
        self.assertIn("configuration", workspace[1])
        self.assertIn("--locked", workspace[1])
        self.assertTrue(all("--no-default-features" in command
                            for command in batches["no-default-features"][0]))
        self.assertIn("--doc", batches["doctests"][0][0])
        self.assertIn("test_parallel_process.py", batches["scripts"][0][0])
        self.assertIn("scripts/test_smoke_postgres.py", batches["scripts"][0][1])
        self.assertTrue(all("runlimit" in " ".join(command) for command in batches["runlimit"][0]))
        self.assertTrue(all("--locked" in command for command in commands if command[0] == "cargo"))

    def test_failure_stops_later_batches_of_its_part(self):
        for part, planned in matrix.parts().items():
            sizes = [len(commands) for _, commands in planned]
            for failing_batch, size in enumerate(sizes):
                for failing_command in range(size):
                    results = [[self.success] * earlier for earlier in sizes[:failing_batch]]
                    failed = [self.success] * size
                    failed[failing_command] = self.failure
                    results.append(failed)
                    execute = mock.Mock(side_effect=results)
                    with self.subTest(part=part, batch=failing_batch, command=failing_command):
                        self.assertEqual(self.run_part(part, execute), 1)
                        self.assertEqual(execute.call_count, failing_batch + 1)

    def test_a_known_part_is_required(self):
        for argv in (["test_matrix.py"], ["test_matrix.py", "all"]):
            execute = mock.Mock()
            with self.subTest(argv=argv), mock.patch.object(sys, "argv", argv), \
                    mock.patch.object(matrix, "run_parallel", execute), \
                    redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
                matrix.main()
            self.assertEqual(raised.exception.code, 2)
            execute.assert_not_called()

    def test_verify_profile_runs_every_part_exactly_once(self):
        root = Path(__file__).resolve().parent.parent
        contract = json.loads((root / ".agent/jig-contract.json").read_text())
        verify, = [profile for profile in contract["profiles"] if profile["id"] == "verify"]
        runners = {(action["target"]["component"], action["target"]["action"]): action["runner"].get("command")
                   for action in contract["actions"]}
        commands = dict(re.findall(r'(?m)^([a-z][a-z0-9_]*_command) = "([^"\\]*)"$',
                                   (root / ".jig.toml").read_text()))
        invoked = {}
        for target in verify["targets"]:
            key = (target["component"], target["action"])
            selected = re.fullmatch(r"python3 scripts/test_matrix\.py (\S+)",
                                    commands.get(runners.get(key), ""))
            if selected:
                invoked.setdefault(selected.group(1), []).append(key)
        self.assertEqual(sorted(invoked), sorted(matrix.parts()))
        self.assertTrue(all(len(targets) == 1 for targets in invoked.values()), invoked)


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
