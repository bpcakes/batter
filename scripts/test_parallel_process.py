"""Failure and coverage controls for concurrent local verification."""

import errno
from contextlib import ExitStack, redirect_stderr, redirect_stdout
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
            "default": [], "memory": [], "postgres": [], "axum": [],
            "native-http": [], "native-axum": []}}]}
        self.assertEqual(runlimit_features.declared_features(metadata),
                         ("axum", "memory", "native-axum", "native-http", "postgres"))
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
                         {"workspace": [2, 2], "no-default-features": [3], "doctests": [1],
                          "consumers": [4, 4, 1], "runlimit": [3], "scripts": [4, 2]})
        planned = matrix.parts()
        # Each part executes exactly the batches it plans, in the planned order.
        self.assertEqual({part: [commands for _, commands in plan] for part, plan in planned.items()},
                         batches)
        commands = {label: command for plan in planned.values() for labels, batch in plan
                    for label, command in zip(labels, batch)}
        self.assertCountEqual(commands.values(), [
            matrix.CORE_CHECK, matrix.FACADE_CHECK, matrix.CORE_TESTS, matrix.WORKSPACE_TESTS,
            matrix.HOSTILE_CONFIGURATION, matrix.DOC_TESTS, matrix.RUNNER_TESTS, matrix.SMOKE_TESTS,
            matrix.REFERENCE_RUNNER_TESTS, matrix.SQLX_RUNNER_TESTS, matrix.RUNLIMIT_FEATURES,
            matrix.RUNLEDGER_GRAPH, matrix.RUNLEDGER_CONTROLS, matrix.RUNLEDGER_CONSUMER,
            matrix.RUNLEDGER_TOOL_CONTROLS, matrix.RUNLIMIT_CONSUMER, matrix.RUNLIMIT_CONSUMER_CONTROLS,
            matrix.RUNLIMIT_GRAPH, matrix.RUNLIMIT_CONTROLS, matrix.RUNLIMIT_DEFAULT,
            matrix.RUNLIMIT_RELEASE, matrix.FACADE_FEATURES, matrix.FACADE_CACHE_CONTROLS,
            matrix.FUTURE_SIZE_RELEASE, matrix.SINGLE_FACADE_CONSUMER, matrix.SINGLE_FACADE_CONTROLS,
        ])
        self.assertEqual(len(commands), 26)
        self.assertIn("--all-targets", commands["workspace-tests"])
        self.assertIn("--workspace", commands["workspace-tests"])
        hostile = commands["configuration-hostile-environment"]
        self.assertEqual(hostile[0], "env")
        self.assertIn("PGDATA=/unused-configuration-fixture", hostile)
        self.assertIn("PGPASSWORD=parent-secret-marker", hostile)
        self.assertIn("configuration", hostile)
        self.assertIn("--locked", hostile)
        future_sizes = commands["release-future-sizes"]
        self.assertEqual(future_sizes[future_sizes.index("--test") + 1], "future_size")
        self.assertIn("--release", future_sizes)
        self.assertIn("test_reference_live.py", commands["reference-runner-controls"])
        self.assertTrue(all("--no-default-features" in command
                            for command in batches["no-default-features"][0]))
        self.assertIn("--doc", batches["doctests"][0][0])
        self.assertIn("test_parallel_process.py", batches["scripts"][0][0])
        self.assertIn("scripts/test_smoke_postgres.py", batches["scripts"][0][1])
        self.assertTrue(all("runlimit" in " ".join(command) for command in batches["runlimit"][0]))
        self.assertTrue(all("--locked" in command for command in commands.values()
                            if command[0] == "cargo"))

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

    def test_verification_runs_every_part_exactly_once(self):
        root = Path(__file__).resolve().parent.parent
        script = (root / "scripts/verify.sh").read_text()
        selected, = re.findall(r"for part in ([^;]+); do", script)
        self.assertCountEqual(selected.split(), matrix.parts())
        self.assertIn('python3 scripts/test_matrix.py "$part"', script)


class ExclusiveBuildTests(unittest.TestCase):
    """Controls for commands that uplift one binary under different selections.

    Cargo releases the build directory lock when linking finishes, so a peer
    command can replace an uplifted executable while an earlier command still
    spawns it from its tests. These controls execute the real scheduling with
    stand-in subprocesses that reproduce exactly that replacement.
    """

    fixture_timeout = 10

    def scheduled(self):
        return {label: command for plan in matrix.parts().values()
                for labels, batch in plan for label, command in zip(labels, batch)}

    def fixture(self, directory, label, body, observed, expected=None):
        """Hold every actual batch participant until all have done their work."""
        return python(f"""
import json, time
from pathlib import Path
root = Path({directory!r})
artifact = root / 'reference-executable'
started = time.monotonic()
deadline = started + {self.fixture_timeout}
expected = {expected!r}

def wait_for(marker):
    while not marker.exists():
        if time.monotonic() >= deadline:
            raise SystemExit('timed out waiting for ' + marker.name)
        time.sleep(0.01)

{body}
(root / '{label}.ready').touch()
for peer in json.loads((root / 'batch-members.json').read_text()):
    wait_for(root / (peer + '.ready'))
observed = {observed}
(root / '{label}.json').write_text(json.dumps(
    {{'started': started, 'finished': time.monotonic(), 'observed': observed}}))
raise SystemExit(0 if expected is None or observed == expected else 9)
""")

    def uplifting_fixture(self, directory):
        """Stand in for the all-feature run whose tests spawn the executable."""
        return self.fixture(directory, "workspace-tests", """
artifact.write_text('all-features')
(root / 'uplifted').touch()
""", "artifact.read_text()", expected="all-features")

    def replacing_fixture(self, directory):
        """Stand in for the default-feature run that rebuilds the same binary."""
        return self.fixture(directory, "configuration-hostile-environment", """
wait_for(root / 'uplifted')
artifact.write_text('default')
""", "'default'")

    def peer_fixture(self, directory, label):
        """Stand in for an unrelated command that may run beside either build."""
        return self.fixture(directory, label, "", "None")

    def execute_workspace(self, directory, prior=False):
        """Run the real workspace part with stand-in commands and record spans."""
        rendered = {}
        fixtures = [
            ("WORKSPACE_TESTS", "workspace-tests", self.uplifting_fixture(directory)),
            ("HOSTILE_CONFIGURATION", "configuration-hostile-environment",
             self.replacing_fixture(directory)),
            ("REFERENCE_RUNNER_TESTS", "reference-runner-controls",
             self.peer_fixture(directory, "reference-runner-controls")),
            ("FUTURE_SIZE_RELEASE", "release-future-sizes",
             self.peer_fixture(directory, "release-future-sizes")),
        ]
        labels_by_command = {tuple(command): label for _, label, command in fixtures}

        def bounded(commands, **kwargs):
            # The matrix keeps its own bounds; only the watchdog is shortened so a
            # stuck control cannot hold the suite for the full matrix allowance.
            self.assertEqual(kwargs["timeout"], 1500)
            self.assertEqual(kwargs["output_limit"], 8 * 1024 * 1024)
            self.assertTrue(kwargs["retain_tail"])
            # Derive the rendezvous from the batch actually launched by main(),
            # so the same fixtures work under both the current and prior plans.
            # The previous batch has settled before this file is replaced.
            labels = [labels_by_command[tuple(command)] for command in commands]
            (Path(directory) / "batch-members.json").write_text(json.dumps(labels))
            return parallel.run_parallel(commands, **{**kwargs, "timeout": 30})

        def render(labels, outcomes):
            rendered.update(zip(labels, outcomes))

        patches = [
            mock.patch.object(sys, "argv", ["test_matrix.py", "workspace"]),
            mock.patch.object(matrix, "run_parallel", bounded),
            mock.patch.object(matrix, "render_outcomes", render),
        ]
        patches.extend(mock.patch.object(matrix, name, command) for name, _, command in fixtures)
        with ExitStack() as stack:
            for patch in patches:
                stack.enter_context(patch)
            if prior:
                # Built here so the prior batch holds the stand-ins, not real Cargo runs.
                planned = self.prior_schedule()
                stack.enter_context(mock.patch.object(matrix, "parts", lambda: planned))
            status = matrix.main()
        spans = {path.stem: json.loads(path.read_text())
                 for path in Path(directory).glob("*.json") if path.name != "batch-members.json"}
        return status, rendered, spans

    def prior_schedule(self):
        """The single batch this task replaced, rebuilt from the same commands."""
        return {"workspace": [(["workspace-tests", "configuration-hostile-environment",
                                "reference-runner-controls", "release-future-sizes"],
                               [matrix.WORKSPACE_TESTS, matrix.HOSTILE_CONFIGURATION,
                                matrix.REFERENCE_RUNNER_TESTS, matrix.FUTURE_SIZE_RELEASE])]}

    @staticmethod
    def overlapping(first, second):
        return first["started"] < second["finished"] and second["started"] < first["finished"]

    def test_declared_uplifts_agree_with_the_scheduled_commands(self):
        commands = self.scheduled()
        for label, declared in matrix.UPLIFTED_BINARIES.items():
            with self.subTest(label=label):
                command = commands[label]
                selection, = [chosen for binary, chosen in declared
                              if binary == matrix.REFERENCE_BINARY]
                self.assertEqual(selection, "all-features" if "--all-features" in command
                                 else "default")
        for label, command in commands.items():
            # Cargo builds a package's binaries for its integration tests so they
            # can spawn CARGO_BIN_EXE_*; only doctest runs skip them.
            selects = "--workspace" in command or matrix.REFERENCE_BINARY in command
            with self.subTest(label=label):
                self.assertEqual(selects and "--doc" not in command,
                                 label in matrix.UPLIFTED_BINARIES)

    def test_no_batch_mixes_feature_selections_for_one_binary(self):
        placement = {}
        for part, plan in matrix.parts().items():
            for index, (labels, _) in enumerate(plan):
                self.assertEqual(matrix.conflicting_binaries(labels), ())
                for label in labels:
                    placement[label] = (part, index)
        self.assertNotEqual(placement["workspace-tests"],
                            placement["configuration-hostile-environment"])
        self.assertEqual(matrix.conflicting_binaries(
            ["workspace-tests", "configuration-hostile-environment"]),
            (matrix.REFERENCE_BINARY,))

    def test_prior_conflicting_schedule_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "conflicting feature selections"):
            matrix._scheduled(self.prior_schedule())

    def test_losing_or_repeating_a_declared_label_is_rejected(self):
        plan = {part: [(list(labels), list(commands)) for labels, commands in batches]
                for part, batches in matrix.parts().items()}
        renamed = json.loads(json.dumps(plan))
        renamed["workspace"][0][0][0] = "workspace-tests-renamed"
        with self.assertRaisesRegex(ValueError, "unscheduled"):
            matrix._scheduled(renamed)
        repeated = json.loads(json.dumps(plan))
        repeated["doctests"][0][0][0] = "workspace-tests"
        with self.assertRaisesRegex(ValueError, "unique"):
            matrix._scheduled(repeated)
        unlabelled = json.loads(json.dumps(plan))
        unlabelled["doctests"][0][0].clear()
        with self.assertRaisesRegex(ValueError, "exactly one label"):
            matrix._scheduled(unlabelled)

    def test_scheduled_builds_never_overlap_and_keep_the_uplifted_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            status, rendered, spans = self.execute_workspace(directory)
        self.assertEqual(status, 0)
        self.assertTrue(all(outcome.ok for outcome in rendered.values()), rendered)
        self.assertEqual(spans["workspace-tests"]["observed"], "all-features")
        self.assertGreaterEqual(spans["configuration-hostile-environment"]["started"],
                                spans["workspace-tests"]["finished"])
        # Unrelated commands still run beside each conflicting build.
        self.assertTrue(self.overlapping(spans["workspace-tests"],
                                         spans["reference-runner-controls"]))
        self.assertTrue(self.overlapping(spans["configuration-hostile-environment"],
                                         spans["release-future-sizes"]))

    def test_prior_schedule_still_replaces_the_artifact_under_the_same_control(self):
        with tempfile.TemporaryDirectory() as directory:
            status, rendered, spans = self.execute_workspace(directory, prior=True)
        self.assertEqual(status, 1)
        self.assertEqual(rendered["workspace-tests"].status, 9)
        self.assertFalse(rendered["workspace-tests"].ok)
        self.assertEqual(spans["workspace-tests"]["observed"], "default")
        self.assertTrue(self.overlapping(spans["workspace-tests"],
                                         spans["configuration-hostile-environment"]))

    @staticmethod
    def delayed(command, seconds):
        """Perturb startup beyond the old hold windows, without changing work."""
        return [*command[:2], f"import time; time.sleep({seconds})\n" + command[2]]

    def test_prior_schedule_waits_for_delayed_replacement(self):
        original = self.replacing_fixture
        with mock.patch.object(self, "replacing_fixture",
                               lambda directory: self.delayed(original(directory), 1.5)):
            self.test_prior_schedule_still_replaces_the_artifact_under_the_same_control()

    def test_current_schedule_waits_for_delayed_peer(self):
        original = self.peer_fixture

        def delayed_peer(directory, label):
            command = original(directory, label)
            return self.delayed(command, 0.9) if label == "release-future-sizes" else command

        with mock.patch.object(self, "peer_fixture", delayed_peer):
            self.test_scheduled_builds_never_overlap_and_keep_the_uplifted_artifact()

    def test_current_schedule_waits_for_delayed_owner(self):
        original = self.uplifting_fixture
        with mock.patch.object(self, "uplifting_fixture",
                               lambda directory: self.delayed(original(directory), 1.5)):
            self.test_scheduled_builds_never_overlap_and_keep_the_uplifted_artifact()

    def test_missing_peer_fails_the_handshake_and_stops_later_batches(self):
        original = self.peer_fixture

        def absent_peer(directory, label):
            return python("pass") if label == "reference-runner-controls" else original(directory, label)

        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(self, "fixture_timeout", 0.2), \
                mock.patch.object(self, "peer_fixture", absent_peer):
            status, rendered, _ = self.execute_workspace(directory)
        self.assertEqual(status, 1)
        self.assertEqual(set(rendered), {"workspace-tests", "reference-runner-controls"})
        owner = rendered["workspace-tests"]
        self.assertEqual(owner.status, 1)
        self.assertIn(b"timed out waiting for reference-runner-controls.ready", owner.stderr)
        self.assertFalse(owner.watchdog)
        self.assertTrue(owner.reaped and owner.output_eof)
        self.assertTrue(rendered["reference-runner-controls"].ok)


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
