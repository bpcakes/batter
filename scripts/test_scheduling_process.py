"""Real-process regressions for scheduling containment and evidence boundaries."""

import argparse
from contextlib import contextmanager
import errno
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

import check_scheduling_mutation as mutation
import scheduling_process as processes
import stress_scheduling as scheduling

BINARY = None
CONTROL_TIMEOUT = 3
PROCESS_SLACK = 3
CONTROL_BOUND = CONTROL_TIMEOUT + scheduling.REAP_ALLOWANCE + PROCESS_SLACK


def python(code, **kwargs):
    return processes.run_process(
        [sys.executable, "-c", code], timeout=kwargs.pop("timeout", CONTROL_TIMEOUT),
        output_limit=kwargs.pop("output_limit", 4096), **kwargs,
    )


def executable(directory, code):
    path = Path(directory) / "fixture"
    path.write_text(f"#!{sys.executable}\n{code}\n")
    path.chmod(0o755)
    return path


def background(command):
    # Pass arguments without shell interpolation; the asynchronous command
    # inherits SIG_IGN even when this non-interactive shell started with SIG_DFL.
    return ["/bin/sh", "-c", '"$@" & process=$!; wait "$process"',
            "scheduling-background", *command]


def stop_owned_fixture(child):
    errors = []
    processes._kill_group(child, errors)
    try:
        status = child.wait(timeout=scheduling.REAP_ALLOWANCE)
    except subprocess.TimeoutExpired as error:
        raise AssertionError(f"fixture reap timeout; signal errors={errors}") from error
    if errors:
        raise AssertionError(f"fixture signal errors={errors}")
    return status


@contextmanager
def owned_pipe_writer(stream, code):
    # This writer is a direct child in a separate session. Retain its waitable
    # identity until the final signal; a PID file would not provide ownership.
    child = subprocess.Popen(
        [sys.executable, "-c", code], stdin=subprocess.DEVNULL,
        stdout=stream, stderr=subprocess.DEVNULL, start_new_session=True,
    )
    try:
        yield child
    finally:
        stop_owned_fixture(child)


class ProcessTests(unittest.TestCase):
    def setUp(self):
        # Interrupt controls choose an active handler independently of how the
        # test suite was launched. Dedicated controls exercise inherited ignore.
        previous = signal.signal(signal.SIGINT, signal.default_int_handler)
        self.addCleanup(signal.signal, signal.SIGINT, previous)

    def test_partial_capture_setup_closes_selector_and_reaps_child(self):
        for stage in ("set-blocking", "register"):
            for interrupted in (False, True):
                with self.subTest(stage=stage, interrupted=interrupted):
                    selector = selectors.DefaultSelector()
                    set_blocking, register = os.set_blocking, selector.register
                    calls = {"set-blocking": 0, "register": 0}

                    def step(name, function, *args):
                        calls[name] += 1
                        if name == stage and calls[name] == 2:
                            if interrupted:
                                raise KeyboardInterrupt
                            raise OSError(errno.EIO, "injected capture setup failure")
                        return function(*args)

                    try:
                        with mock.patch.object(processes.selectors, "DefaultSelector", return_value=selector), \
                                mock.patch.object(processes.os, "set_blocking", side_effect=lambda *a: step("set-blocking", set_blocking, *a)), \
                                mock.patch.object(selector, "register", side_effect=lambda *a: step("register", register, *a)):
                            result = python("import time; time.sleep(30)")
                        self.assertEqual(result.status, -signal.SIGKILL)
                        self.assertTrue(result.reaped)
                        self.assertEqual(result.errors, ("interrupted" if interrupted else f"io:{errno.EIO}",))
                        # Keep the real selector alive so cyclic GC cannot mask
                        # a missing close. Linux epoll and macOS kqueue both
                        # reject fileno() once the underlying resource is closed.
                        with self.assertRaises(ValueError):
                            selector.fileno()
                    finally:
                        selector.close()

    def test_sigint_during_cleanup_preserves_deadline_evidence_and_resources(self):
        for signal_count in (1, 3):
            with self.subTest(signals=signal_count):
                read_fd, write_fd = os.pipe()
                with os.fdopen(read_fd, "rb", buffering=0) as reader, \
                        os.fdopen(write_fd, "wb", buffering=0) as writer:
                    writer.write(b"cleanup checkpoint\n")
                    with owned_pipe_writer(writer, "import time; time.sleep(30)"):
                        selector = selectors.DefaultSelector()
                        real_select = selector.select
                        real_popen = subprocess.Popen
                        real_settle = processes._settle
                        real_observe = processes.Capture.observe_until
                        previous = signal.getsignal(signal.SIGINT)
                        children, deadlines = [], []
                        settling, sent = False, 0

                        def spawn_observed(*args, **kwargs):
                            child = real_popen(*args, **{**kwargs, "stdout": writer})
                            children.append(child)
                            child.stdout = reader
                            writer.close()
                            return child

                        def settle(*args, **kwargs):
                            nonlocal settling
                            settling = True
                            return real_settle(*args, **kwargs)

                        def observe(capture, child, deadline, **kwargs):
                            if settling:
                                deadlines.append(deadline)
                            return real_observe(capture, child, deadline, **kwargs)

                        def select_then_signal(timeout):
                            nonlocal sent
                            ready = real_select(timeout)
                            if settling and sent < signal_count:
                                sent += 1
                                os.kill(os.getpid(), signal.SIGINT)
                            return ready

                        try:
                            with mock.patch.object(processes.subprocess, "Popen", spawn_observed), \
                                    mock.patch.object(processes.selectors, "DefaultSelector", return_value=selector), \
                                    mock.patch.object(selector, "select", select_then_signal), \
                                    mock.patch.object(processes, "_settle", settle), \
                                    mock.patch.object(processes.Capture, "observe_until", observe):
                                try:
                                    result = python("import time; time.sleep(30)", timeout=0.1,
                                                    reap_allowance=0.3)
                                except KeyboardInterrupt:
                                    self.fail("SIGINT escaped cleanup without an outcome")
                            self.assertEqual(sent, signal_count)
                            self.assertEqual(len(set(deadlines)), 1, "cleanup deadline restarted")
                            self.assertEqual(result.status, -signal.SIGKILL)
                            self.assertTrue(result.watchdog and result.reaped)
                            self.assertFalse(result.ok or result.output_eof)
                            self.assertEqual(result.errors, ("interrupted",))
                            self.assertIn("cleanup checkpoint", result.output)
                            self.assertLess(result.elapsed_seconds, 0.4 + PROCESS_SLACK)
                            self.assertTrue(reader.closed and children[0].stderr.closed)
                            with self.assertRaises(ValueError):
                                selector.fileno()
                            self.assertIs(signal.getsignal(signal.SIGINT), previous)
                        finally:
                            selector.close()
                            for child in children:
                                stop_owned_fixture(child)
                                child.stdout.close()
                                child.stderr.close()

    def test_sigint_during_setup_observation_and_close_restores_handler(self):
        for stage in ("setup", "observation", "close"):
            with self.subTest(stage=stage):
                selector = selectors.DefaultSelector()
                register, select, close = selector.register, selector.select, selector.close
                previous = signal.getsignal(signal.SIGINT)
                real_popen = subprocess.Popen
                children = []
                sent = False

                def interrupt():
                    nonlocal sent
                    if not sent:
                        sent = True
                        os.kill(os.getpid(), signal.SIGINT)

                def spawned(*args, **kwargs):
                    child = real_popen(*args, **kwargs)
                    children.append(child)
                    return child

                def registered(*args):
                    key = register(*args)
                    if stage == "setup":
                        interrupt()
                    return key

                def selected(timeout):
                    ready = select(timeout)
                    if stage == "observation" and ready:
                        interrupt()
                    return ready

                def closed():
                    if stage == "close":
                        interrupt()
                    close()

                try:
                    with mock.patch.object(processes.subprocess, "Popen", spawned), \
                            mock.patch.object(processes.selectors, "DefaultSelector", return_value=selector), \
                            mock.patch.object(selector, "register", registered), \
                            mock.patch.object(selector, "select", selected), \
                            mock.patch.object(selector, "close", closed):
                        code = "print('observed checkpoint',flush=True)"
                        if stage != "close":
                            code += "; import time; time.sleep(30)"
                        result = python(code)
                    self.assertTrue(sent)
                    self.assertEqual(result.errors, ("interrupted",))
                    self.assertFalse(result.ok or result.watchdog)
                    self.assertTrue(result.reaped and result.output_eof)
                    self.assertEqual(result.status, 0 if stage == "close" else -signal.SIGKILL)
                    if stage == "observation":
                        self.assertIn("observed checkpoint", result.output)
                    self.assertIs(signal.getsignal(signal.SIGINT), previous)
                    self.assertTrue(children[0].stdout.closed and children[0].stderr.closed)
                    with self.assertRaises(ValueError):
                        selector.fileno()
                finally:
                    close()
                    for child in children:
                        stop_owned_fixture(child)
                        child.stdout.close()
                        child.stderr.close()

    def test_incompatible_sigint_owner_is_rejected_before_launch(self):
        previous = signal.getsignal(signal.SIGINT)
        for handler in (lambda *_: None,):
            with self.subTest(handler=handler):
                try:
                    signal.signal(signal.SIGINT, handler)
                    with mock.patch.object(processes.subprocess, "Popen") as spawn:
                        result = python("raise SystemExit(0)")
                    self.assertEqual(result.errors, ("unsupported-sigint-owner",))
                    self.assertIsNone(result.status)
                    spawn.assert_not_called()
                    self.assertIs(signal.getsignal(signal.SIGINT), handler)
                finally:
                    signal.signal(signal.SIGINT, previous)
        results = []
        with mock.patch.object(processes.subprocess, "Popen") as spawn:
            worker = threading.Thread(target=lambda: results.append(python("pass")))
            worker.start()
            worker.join(timeout=CONTROL_TIMEOUT)
            self.assertFalse(worker.is_alive())
            spawn.assert_not_called()
        self.assertEqual(results[0].errors, ("unsupported-sigint-owner",))
        self.assertIs(signal.getsignal(signal.SIGINT), previous)

    def test_ignored_sigint_survives_background_shell_launch(self):
        child_code = ("import os,signal; "
                      "print(signal.getsignal(signal.SIGINT)==signal.SIG_IGN,flush=True); "
                      "os.kill(os.getppid(),signal.SIGINT); "
                      "os.kill(os.getpid(),signal.SIGINT); print('completed',flush=True)")
        owner_code = f"""
import json,signal,sys
sys.path.insert(0, {str(Path(processes.__file__).parent)!r})
from scheduling_process import run_process
before = signal.getsignal(signal.SIGINT) == signal.SIG_IGN
result = run_process([sys.executable, '-c', {child_code!r}], timeout=3, output_limit=4096)
print(json.dumps({{'ignored_before': before,
                  'ignored_after': signal.getsignal(signal.SIGINT) == signal.SIG_IGN,
                  'stdout': result.stdout.decode(), **result.metadata()}}))
sys.exit(0 if result.ok else 1)
"""
        result = processes.run_process(
            background([sys.executable, "-c", owner_code]), timeout=CONTROL_BOUND,
            output_limit=4096,
        )
        self.assertTrue(result.ok, result.output)
        record = json.loads(result.stdout)
        self.assertTrue(record["ignored_before"] and record["ignored_after"])
        self.assertEqual(record["stdout"], "True\ncompleted\n")
        self.assertEqual(record["errors"], [])
        self.assertFalse(record["watchdog"])
        self.assertTrue(record["reaped"] and record["output_eof"])

    def test_compatible_sigint_restores_disposition_on_each_exit_path(self):
        previous = signal.getsignal(signal.SIGINT)

        def failed_record(_pid):
            raise RuntimeError("injected record failure")

        for disposition in (signal.default_int_handler, signal.SIG_DFL, signal.SIG_IGN):
            for path in ("success", "spawn-failure", "callback-failure", "timeout"):
                with self.subTest(disposition=disposition, path=path):
                    try:
                        signal.signal(signal.SIGINT, disposition)
                        if path == "callback-failure":
                            with self.assertRaisesRegex(RuntimeError, "injected record failure"):
                                python("import time; time.sleep(30)", launch_record=failed_record)
                        elif path == "spawn-failure":
                            result = processes.run_process(
                                ["/nonexistent/scheduling-fixture"], timeout=1, output_limit=64,
                            )
                            self.assertEqual(result.errors, (f"spawn:{errno.ENOENT}",))
                        elif path == "timeout":
                            result = python("import time; time.sleep(30)", timeout=0.1)
                            self.assertEqual(result.status, -signal.SIGKILL)
                            self.assertTrue(result.watchdog and result.reaped and result.output_eof)
                            self.assertEqual(result.errors, ())
                        else:
                            result = python("print('completed')")
                            self.assertTrue(result.ok, result.metadata())
                            self.assertEqual(result.stdout, b"completed\n")
                        self.assertIs(signal.getsignal(signal.SIGINT), disposition)
                    finally:
                        signal.signal(signal.SIGINT, previous)

    def test_unix_default_sigint_is_deferred_through_cleanup(self):
        previous = signal.signal(signal.SIGINT, signal.SIG_DFL)

        def interrupt(_pid):
            os.kill(os.getpid(), signal.SIGINT)
            return b"interrupt checkpoint\n"

        try:
            result = python("import time; time.sleep(30)", launch_record=interrupt)
            self.assertEqual(result.status, -signal.SIGKILL)
            self.assertTrue(result.reaped and result.output_eof)
            self.assertFalse(result.ok or result.watchdog)
            self.assertEqual(result.errors, ("interrupted",))
            self.assertIs(signal.getsignal(signal.SIGINT), signal.SIG_DFL)
        finally:
            signal.signal(signal.SIGINT, previous)

    def test_callback_exception_closes_resources_and_restores_sigint(self):
        selector = selectors.DefaultSelector()
        previous = signal.getsignal(signal.SIGINT)
        real_popen = subprocess.Popen
        children = []

        def spawned(*args, **kwargs):
            child = real_popen(*args, **kwargs)
            children.append(child)
            return child

        def failed_record(_pid):
            raise RuntimeError("injected record failure")

        try:
            with mock.patch.object(processes.subprocess, "Popen", spawned), \
                    mock.patch.object(processes.selectors, "DefaultSelector", return_value=selector):
                with self.assertRaisesRegex(RuntimeError, "injected record failure"):
                    python("import time; time.sleep(30)", launch_record=failed_record)
            self.assertEqual(children[0].returncode, -signal.SIGKILL)
            self.assertTrue(all(stream.closed for stream in
                                (children[0].stdin, children[0].stdout, children[0].stderr)))
            self.assertIs(signal.getsignal(signal.SIGINT), previous)
            with self.assertRaises(ValueError):
                selector.fileno()
        finally:
            selector.close()
            for child in children:
                stop_owned_fixture(child)
                for stream in (child.stdin, child.stdout, child.stderr):
                    stream.close()

    def test_success_retains_separate_streams(self):
        result = python("import sys; print('result'); print('checkpoint', file=sys.stderr)")
        self.assertTrue(result.ok, result.metadata())
        self.assertIs(signal.getsignal(signal.SIGINT), signal.default_int_handler)
        self.assertEqual(result.stdout, b"result\n")
        self.assertEqual(result.stderr, b"checkpoint\n")

    def test_delayed_start_still_reaches_the_success_checkpoint(self):
        # Model interpreter/site initialization taking longer than the former
        # 300 ms budget, before any fixture checkpoint or application action.
        result = python("import time; time.sleep(1); print('started',flush=True)")
        self.assertTrue(result.ok, result.metadata())
        self.assertEqual(result.stdout, b"started\n")

    def test_reaped_child_is_never_signalled(self):
        with owned_pipe_writer(subprocess.DEVNULL, "pass") as child:
            self.assertEqual(child.wait(timeout=CONTROL_TIMEOUT), 0)
            with mock.patch.object(processes.os, "killpg") as kill:
                self.assertEqual(stop_owned_fixture(child), 0)
                kill.assert_not_called()

    def test_owned_writer_is_reaped_when_observation_raises(self):
        with self.assertRaisesRegex(RuntimeError, "observation fixture failed"):
            with owned_pipe_writer(subprocess.DEVNULL, "import time; time.sleep(30)") as child:
                raise RuntimeError("observation fixture failed")
        self.assertEqual(child.returncode, -signal.SIGKILL)

    def test_timeout_retains_output_and_reaps(self):
        result = python("import time; print('checkpoint', flush=True); time.sleep(30)")
        self.assertEqual(result.status, -signal.SIGKILL)
        self.assertTrue(result.watchdog and result.reaped and result.output_eof)
        self.assertIn("checkpoint", result.output)
        self.assertLess(result.elapsed_seconds, CONTROL_BOUND)

    def test_stdout_eof_is_not_process_exit(self):
        result = python("import os,time; print('checkpoint',flush=True); "
                        "os.close(1); os.close(2); time.sleep(30)")
        self.assertEqual(result.status, -signal.SIGKILL)
        self.assertTrue(result.watchdog and result.reaped and result.output_eof)
        self.assertIn("checkpoint", result.output)

    def test_descendant_pipe_is_contained_by_the_owned_group(self):
        real_popen, real_killpg = subprocess.Popen, os.killpg
        children, status_at_signal = [], []

        def spawned(*args, **kwargs):
            child = real_popen(*args, **kwargs)
            children.append(child)
            return child

        def signal_owned_group(pid, sig):
            self.assertEqual(pid, children[0].pid)
            status_at_signal.append(children[0].returncode)
            real_killpg(pid, sig)

        with mock.patch.object(processes.subprocess, "Popen", spawned), \
                mock.patch.object(processes.os, "killpg", signal_owned_group):
            result = python("import os,subprocess,sys; "
                            "subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
                            "print('leader completed',flush=True); os._exit(0)")
        # The leader's retained status proves it exited successfully; it must
        # remain unreaped until signalling its still-owned process group.
        self.assertEqual(status_at_signal, [None])
        self.assertEqual(result.status, 0)
        self.assertTrue(result.watchdog and result.reaped and result.output_eof)
        self.assertFalse(result.ok)
        self.assertIn("leader completed", result.output)

    def test_escaped_pipe_deadline_keeps_checkpoints(self):
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, "rb", buffering=0) as reader, \
                os.fdopen(write_fd, "wb", buffering=0) as writer:
            code = "import time; print('escaped pipe checkpoint',flush=True); time.sleep(30)"
            with owned_pipe_writer(writer, code) as escaped:
                real_popen = subprocess.Popen

                def spawn_observed_child(*args, **kwargs):
                    # Both children share this pipe, but only the observed
                    # child's group belongs to run_process. Transfer the reader
                    # to its capture and release the test's extra write handle.
                    child = real_popen(*args, **{**kwargs, "stdout": writer})
                    child.stdout = reader
                    writer.close()
                    return child

                with mock.patch.object(processes.subprocess, "Popen", spawn_observed_child):
                    result = python("print('leader completed',flush=True)", reap_allowance=0.2)
                self.assertEqual(result.status, 0)
                self.assertTrue(result.watchdog and result.reaped)
                self.assertFalse(result.output_eof)
                self.assertIn("escaped pipe checkpoint", result.output)
                self.assertIsNone(escaped.poll(), "the separately owned writer must still run")
                self.assertLess(result.elapsed_seconds, CONTROL_BOUND)
                rendered = scheduling.render(result.metadata(), result.output)
                self.assertFalse(json.loads(rendered.splitlines()[-1].removeprefix(
                    "WATCHDOG_RESULT "))["output_eof"])
            self.assertEqual(escaped.returncode, -signal.SIGKILL)

    def test_overflow_still_drains_and_summary_starts_its_own_line(self):
        result = python("import os; os.write(1,b'PROFILE_OK fake\\n'+b'x'*200000)", output_limit=64)
        self.assertEqual(result.status, 0)
        self.assertTrue(result.overflow and result.output_eof and result.reaped)
        self.assertFalse(result.ok)
        self.assertEqual(len(result.stdout) + len(result.stderr), 64)
        rendered = scheduling.render(result.metadata(), result.output)
        lines = [line for line in rendered.splitlines() if line.startswith("WATCHDOG_RESULT ")]
        self.assertEqual(len(lines), 1)
        self.assertTrue(json.loads(lines[0].removeprefix("WATCHDOG_RESULT "))["overflow"])

    def test_spawn_failure_is_a_structured_outcome(self):
        result = processes.run_process(["/nonexistent/scheduling-fixture"], timeout=1, output_limit=64)
        self.assertFalse(result.ok or result.reaped)
        self.assertEqual(result.errors, (f"spawn:{errno.ENOENT}",))
        self.assertIs(signal.getsignal(signal.SIGINT), signal.default_int_handler)

    def test_ignored_sigchld_cannot_turn_a_failed_exit_into_success(self):
        result = python(f"""
import json,signal,sys
sys.path.insert(0,{str(Path(processes.__file__).parent)!r})
from scheduling_process import run_process
signal.signal(signal.SIGCHLD,signal.SIG_IGN)
r=run_process([sys.executable,'-c',"print('PROFILE_OK fake'); raise SystemExit(7)"],
              timeout=1,output_limit=4096)
print(json.dumps({{'ok':r.ok,**r.metadata()}}))
""", timeout=3)
        self.assertTrue(result.ok, result.metadata())
        evidence = json.loads(result.stdout)
        self.assertFalse(evidence["ok"])
        self.assertIsNone(evidence["status"])
        self.assertIn("unobservable-child-status", evidence["errors"])

    def test_close_failure_preserves_already_captured_output(self):
        real_popen = subprocess.Popen
        selector = selectors.DefaultSelector()
        real_close = selector.close
        children = []

        class FailingClose:
            def __init__(self, stream):
                self.stream = stream

            def fileno(self):
                return self.stream.fileno()

            def close(self):
                self.stream.close()
                raise OSError(errno.EIO, "injected close failure")

        def spawned(*args, **kwargs):
            child = real_popen(*args, **kwargs)
            child.stdout = FailingClose(child.stdout)
            children.append(child)
            return child

        def failed_selector_close():
            real_close()
            raise OSError(errno.EIO, "injected selector close failure")

        try:
            with mock.patch.object(processes.subprocess, "Popen", spawned), \
                    mock.patch.object(processes.selectors, "DefaultSelector", return_value=selector), \
                    mock.patch.object(selector, "close", failed_selector_close):
                result = python("print('retained checkpoint')")
        finally:
            real_close()
        self.assertFalse(result.ok)
        self.assertEqual(result.status, 0)
        self.assertIn("retained checkpoint", result.output)
        self.assertEqual(result.errors, (f"close-selector:{errno.EIO}", f"close:{errno.EIO}"))
        self.assertTrue(children[0].stdout.stream.closed and children[0].stderr.closed)

    def test_read_failure_still_reaps_the_real_child(self):
        observe = processes.Capture.observe_until
        called = False

        def fail_once(capture, child, deadline, **kwargs):
            nonlocal called
            if not called:
                called = True
                raise OSError(errno.EIO, "injected read failure")
            return observe(capture, child, deadline, **kwargs)

        with mock.patch.object(processes.Capture, "observe_until", fail_once):
            result = python("import time; time.sleep(30)")
        self.assertEqual(result.status, -signal.SIGKILL)
        self.assertTrue(result.reaped)
        self.assertIn(f"io:{errno.EIO}", result.errors)

    def test_unobserved_exit_is_reported_without_an_unbounded_wait(self):
        real_popen = subprocess.Popen
        children = []

        class Unobserved:
            def __init__(self, *args, **kwargs):
                self.child = real_popen(*args, **kwargs)
                children.append(self.child)

            def __getattr__(self, name):
                return getattr(self.child, name)

            def poll(self):
                return None

            def wait(self, *args, **kwargs):
                raise AssertionError("observation deadline must not call an unbounded wait")

        try:
            with mock.patch.object(processes.subprocess, "Popen", Unobserved):
                result = python("print('completed')", reap_allowance=0.1)
            self.assertFalse(result.reaped or result.ok)
            self.assertIsNone(result.status)
            self.assertIn("completed", result.output)
            self.assertLess(result.elapsed_seconds, CONTROL_BOUND)
        finally:
            for child in children:
                child.wait(timeout=3)

    def test_build_timeout_persists_partial_logs_and_classification(self):
        def timed_build(command, **kwargs):
            return python("import sys,time; print('partial build output',flush=True); "
                          "print('compiler checkpoint',file=sys.stderr,flush=True); time.sleep(30)")

        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "build.log"
            with mock.patch.object(mutation, "run_process", timed_build):
                with self.assertRaisesRegex(RuntimeError, "not oracle evidence"):
                    mutation.build(Path(directory), log)
            self.assertIn("partial build output", log.read_text())
            self.assertIn("compiler checkpoint", log.read_text())
            metadata = json.loads(log.with_suffix(".json").read_text())
            self.assertTrue(metadata["watchdog"] and metadata["reaped"])
            self.assertEqual(metadata["command"][0], "cargo")

    def test_replay_timeout_is_retained_and_cannot_reject_a_mutant(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = executable(directory, "import time; "
                                "print('shared capacity violated: descendant must be Full',flush=True); "
                                "time.sleep(30)")
            with mock.patch.object(scheduling, "MAX_WATCHDOG", CONTROL_TIMEOUT):
                with self.assertRaisesRegex(RuntimeError, "unexpected mutant oracle result"):
                    mutation.check(Path(directory), binary, Path(directory), "mutant", 2, 0)
            self.assertIn("shared capacity violated", (Path(directory) / "mutant-2-0.log").read_text())
            record = json.loads((Path(directory) / "mutant-2-0.json").read_text())
            self.assertTrue(record["watchdog"] and record["reaped"])
            self.assertEqual(record["status"], -signal.SIGKILL)


class FixtureLaunchTests(unittest.TestCase):
    def test_background_runner_starts_real_fixture(self):
        result = processes.run_process(
            background([sys.executable, str(Path(scheduling.__file__)), "--binary", str(BINARY),
                        "--workers", "2", "--seed", "17", "--schedule", "capacity-after-drain"]),
            timeout=CONTROL_BOUND, output_limit=16384,
        )
        self.assertTrue(result.ok, result.output)
        self.assertIn("PROFILE_OK ", result.output)
        record = json.loads(result.output.splitlines()[-1].removeprefix("WATCHDOG_RESULT "))
        self.assertEqual(record["status"], 0)
        self.assertEqual(record["errors"], [])
        self.assertTrue(record["reaped"] and record["output_eof"])

    def test_ambient_flag_leaves_ordinary_discovery_inert_with_stdin_open(self):
        result = processes.run_process(
            [str(BINARY), "--exact", "scheduling_child"], timeout=3, output_limit=4096,
            env={**os.environ, "BATTER_SCHEDULING_CHILD": "1"}, keep_stdin=True,
        )
        self.assertTrue(result.ok, result.metadata())
        self.assertNotIn("PROFILE_OK", result.output)

    def test_missing_record_has_a_native_startup_deadline(self):
        result = processes.run_process(
            [str(BINARY), *scheduling.CHILD_ARGS], timeout=8, output_limit=4096, keep_stdin=True,
        )
        self.assertEqual(result.status, 86, result.metadata())
        self.assertFalse(result.watchdog)
        self.assertNotIn("SCHEDULE", result.output)

    def test_invalid_records_never_start_application_work(self):
        for record in (b"v1 2 17 corpus\n", b"batter-scheduling-v1 0 2 17 corpus\n", b"x" * 128):
            with self.subTest(record=record):
                result = processes.run_process(
                    [str(BINARY), *scheduling.CHILD_ARGS], timeout=3, output_limit=4096,
                    launch_record=lambda pid: record,
                )
                self.assertEqual(result.status, 86, result.metadata())
                self.assertNotIn("SCHEDULE", result.output)

    def test_parent_eof_terminates_an_authorized_blocked_fixture(self):
        child = subprocess.Popen(
            [str(BINARY), *scheduling.CHILD_ARGS], stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True, bufsize=0,
        )
        try:
            child.stdin.write(f"batter-scheduling-v1 {child.pid} 2 17 stuck\n".encode())
            capture = bytearray()
            deadline = time.monotonic() + 3
            with selectors.DefaultSelector() as selector:
                selector.register(child.stderr, selectors.EVENT_READ)
                while b"action=runtime-blocked" not in capture and time.monotonic() < deadline:
                    for key, _ in selector.select(0.02):
                        capture.extend(os.read(key.fd, 4096))
                        self.assertLess(len(capture), 16384)
            self.assertIn(b"action=runtime-blocked", capture)
            child.stdin.close()
            self.assertEqual(child.wait(timeout=3), 87)
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=3)
            child.stdin.close()
            child.stderr.close()

    def test_watchdog_ceiling_leaves_room_for_reaping_before_emergency_exit(self):
        result = processes.run_process(
            [sys.executable, str(Path(scheduling.__file__)), "--binary", str(BINARY),
             "--workers", "2", "--watchdog", "140.1"], timeout=3, output_limit=4096,
        )
        self.assertEqual(result.status, 2)
        self.assertIn("at most 140 seconds", result.output)


def load_tests(loader, tests, pattern):
    suite = loader.loadTestsFromTestCase(ProcessTests)
    if BINARY is not None:
        suite.addTests(loader.loadTestsFromTestCase(FixtureLaunchTests))
    return suite


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    options, remaining = parser.parse_known_args()
    BINARY = options.binary.resolve() if options.binary else None
    # This deadline is outside the process owner being tested. A regression in
    # that owner must fail Cargo visibly rather than hang its parent test.
    def emergency_exit():
        time.sleep(60)
        os._exit(89)
    threading.Thread(target=emergency_exit, daemon=True).start()
    unittest.main(argv=[sys.argv[0], *remaining], verbosity=2)
