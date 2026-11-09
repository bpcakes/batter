"""Negative controls for live-backend evidence and fixture cleanup."""

import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import check_victoriametrics as live
import test_matrix as matrix


class EvidenceTests(unittest.TestCase):
    def sample(self, _base, name, _labels):
        if name.endswith("_sum"):
            return dict.fromkeys(live.INSTANCES, 0.01)
        if name.startswith("batter_operation_"):
            return live.INSTANCES.copy()
        return dict.fromkeys(live.INSTANCES, 1)

    def test_missing_identity_and_wrong_cumulative_count_cannot_pass(self):
        with patch.object(live, "samples", side_effect=self.sample):
            live.verify("fixture")  # Positive control for the independent oracle.
        for invalid in [{"worker-1": 2}, {None: 7}, {"worker-1": 4, "worker-2": 10}]:
            def samples(base, name, labels):
                if name == "batter_operation_completions_total":
                    return invalid
                return self.sample(base, name, labels)
            with self.subTest(invalid=invalid), patch.object(live, "samples", side_effect=samples):
                with self.assertRaises(AssertionError):
                    live.verify("fixture")

    def test_failed_producer_and_ambiguous_launch_both_attempt_owned_name_removal(self):
        for launch_failure in [False, True]:
            calls = []

            def docker(*args, **_kwargs):
                calls.append(args)
                if args[0] == "run" and launch_failure:
                    raise subprocess.TimeoutExpired("docker run", 180)
                if args[0] == "inspect":
                    return '[{"NetworkSettings":{"Ports":{"8428/tcp":[{"HostIp":"127.0.0.1","HostPort":"1234"}]}}}]'
                if args[0] == "ps":
                    self.fail("cleanup must not depend on a Docker listing")
                return ""

            with (self.subTest(launch_failure=launch_failure),
                  patch.object(live, "build", return_value="producer"),
                  patch.object(live, "docker", side_effect=docker),
                  patch.object(live, "poll", return_value=None),
                  patch.object(live, "run_parallel", return_value=[SimpleNamespace(ok=False)]),
                  patch.object(live, "render_outcomes"),
                  patch("builtins.print")):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    live.main()
            name = calls[0][calls[0].index("--name") + 1]
            self.assertTrue(name.startswith("batter-victoriametrics-"))
            self.assertEqual(calls[-1], ("rm", "--force", "--volumes", name))

    def test_cleanup_failure_retains_the_primary_error_and_its_cause(self):
        primary = RuntimeError("producer failed")
        cause = ValueError("retained underlying cause")
        primary.__cause__ = cause
        cleanup = subprocess.CalledProcessError(1, ["docker", "rm"], stderr="daemon failed")
        before = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        with patch.object(live, "docker", side_effect=cleanup) as docker:
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                with live.fixture() as (name, _stop):
                    raise primary
        self.assertIs(raised.exception, cleanup)
        self.assertIs(cleanup.__context__, primary)
        self.assertIs(primary.__cause__, cause)
        docker.assert_called_once_with("rm", "--force", "--volumes", name, timeout=10)
        self.assertEqual({sig: signal.getsignal(sig) for sig in before}, before)

    def test_cleanup_failure_cannot_turn_success_into_a_pass(self):
        cleanup = subprocess.TimeoutExpired(["docker", "rm"], 10)
        with patch.object(live, "docker", side_effect=cleanup):
            with self.assertRaises(subprocess.TimeoutExpired) as raised:
                with live.fixture():
                    pass
        self.assertIs(raised.exception, cleanup)
        self.assertIsNone(cleanup.__context__)

    def test_native_success_for_an_absent_container_needs_no_special_error_parser(self):
        before = signal.getsignal(signal.SIGTERM)
        with patch.object(live, "docker", return_value="") as docker:
            with live.fixture() as (name, stop):
                self.assertIs(signal.getsignal(signal.SIGTERM), stop)
        docker.assert_called_once_with("rm", "--force", "--volumes", name, timeout=10)
        self.assertEqual(signal.getsignal(signal.SIGTERM), before)

    def test_late_stop_is_checked_after_cleanup_without_interrupting_removal(self):
        removed = []

        def remove(*args, **_kwargs):
            os.kill(os.getpid(), signal.SIGTERM)
            os.kill(os.getpid(), signal.SIGTERM)
            removed.append(args)

        with patch.object(live, "docker", side_effect=remove):
            with self.assertRaises(KeyboardInterrupt):
                with live.fixture() as (name, _stop):
                    pass
        self.assertEqual(removed, [("rm", "--force", "--volumes", name)])

    def test_interruptible_docker_preserves_failed_process_evidence(self):
        from scheduling_process import ProcessOutcome
        outcome = ProcessOutcome(7, b"native stdout", b"native stderr", 0, False,
                                 False, True, True, ())
        with patch.object(live, "run_parallel", return_value=[outcome]), \
                patch.object(live.subprocess, "run", side_effect=AssertionError("unbounded path")):
            with self.assertRaises(live.DockerCommandFailure) as raised:
                live.docker("run", "fixture", timeout=180, interruptible=True)
        self.assertIs(raised.exception.outcome, outcome)
        self.assertNotIn("native stderr", str(raised.exception))


SIGNAL_CHILD = r'''
import json, signal, sys, time
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import check_victoriametrics as live

stage = sys.argv[2]
control = Path(sys.argv[3]) if len(sys.argv) > 3 else None
# Model a foreground CLI regardless of the test runner's inherited dispositions.
signal.signal(signal.SIGINT, signal.default_int_handler)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
owned = None
live.build = lambda: "producer"

def checkpoint():
    print("POLLING", flush=True)
    if control:
        (control / "polling").touch()
    stop = signal.getsignal(signal.SIGTERM)
    while True:
        stop.check()
        time.sleep(0.01)

def docker(*args, **kwargs):
    global owned
    if args[0] == "run":
        owned = args[args.index("--name") + 1]
        assert kwargs["interruptible"]
    elif args[0] == "inspect":
        assert kwargs["interruptible"]
        return json.dumps([{"NetworkSettings": {"Ports": {"8428/tcp": [
            {"HostIp": "127.0.0.1", "HostPort": "1234"}]}}}])
    elif args[0] == "ps":
        raise AssertionError("cleanup cannot require a listing")
    elif args[0] == "logs" and control:
        time.sleep(live.LOG_TIMEOUT * 0.1)
    elif args[0] == "logs" and stage == "entry":
        print("FINALIZING", flush=True)
        assert sys.stdin.readline().strip() == "continue"
    elif args[0] == "rm":
        assert args == ("rm", "--force", "--volumes", owned)
        assert kwargs == {"timeout": 10}
        print("REMOVING", flush=True)
        if control:
            time.sleep(live.REMOVE_TIMEOUT * 0.09)
            (control / "removed").write_text(owned)
        else:
            assert sys.stdin.readline().strip() == "continue"
        print("REMOVED", flush=True)
    return ""

def health(_url):
    if stage in ("health", "entry"):
        checkpoint()
    return b"OK"

original_runner = live.run_parallel
def producer(_commands, **_kwargs):
    # Exercise actual runner signal ownership/restoration before read-back.
    command = [sys.executable, "-c", "pass"]
    if stage == "producer":
        command = [sys.executable, "-c",
            "import signal,sys,time; from pathlib import Path; "
            "signal.signal(signal.SIGINT, lambda *_: (time.sleep(0.8), sys.exit(0))); "
            "Path(sys.argv[1]).touch(); time.sleep(5)", str(control / "polling")]
    results = original_runner([command], timeout=5, output_limit=1024,
                              grace=live.WORK_GRACE * 0.1,
                              reap_allowance=live.WORK_REAP_ALLOWANCE * 0.1)
    assert results[0].ok == (stage != "producer"), results[0]
    return results

live.docker = docker
live.poll = lambda base, mode: health(base) if mode == "health" else checkpoint()
live.run_parallel = producer
live.render_outcomes = lambda *_args: None
try:
    live.main()
except KeyboardInterrupt:
    print("INTERRUPTED", flush=True)
    raise SystemExit(130)
finally:
    assert {sig: signal.getsignal(sig) for sig in previous} == previous
    print("RESTORED", flush=True)
'''


OUTER_CHILD = r'''
import json, signal, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import test_matrix as matrix
from test_victoriametrics import SIGNAL_CHILD

signal.signal(signal.SIGINT, signal.default_int_handler)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
control, stage, policy = Path(sys.argv[2]), sys.argv[3], sys.argv[4]
command = [sys.executable, "-B", "-u", "-c", SIGNAL_CHILD,
           sys.argv[1], stage, str(control)]
matrix.parts = lambda: {"workspace": [(["victoriametrics-live"], [command])]}
original = matrix.run_parallel

def scaled(commands, **kwargs):
    # Preserve production budget ratios while bounding this real-process test.
    # The negative control deliberately restores the old ten-second default.
    grace = 10 if policy == "old-default" else kwargs.get("grace", 10)
    kwargs.update(timeout=10, grace=grace * 0.1, reap_allowance=0.5)
    return original(commands, **kwargs)

def retain(_labels, outcomes):
    outcome, = outcomes
    (control / "outcome.json").write_text(json.dumps({
        "status": outcome.status, "errors": outcome.errors,
        "reaped": outcome.reaped, "output_eof": outcome.output_eof,
        "stdout": outcome.stdout.decode(), "stderr": outcome.stderr.decode(),
    }))

matrix.run_parallel = scaled
matrix.render_outcomes = retain
sys.argv = ["test_matrix.py", "workspace"]
raise SystemExit(matrix.main())
'''


CLEANUP_PROCESS_CHILD = r'''
import os, signal, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import check_victoriametrics as live
control = Path(sys.argv[2])
signal.signal(signal.SIGINT, signal.default_int_handler)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
native_run = subprocess.run

def substitute_command(command, **kwargs):
    assert command[:4] == ["docker", "rm", "--force", "--volumes"]
    worker = ("import sys; from pathlib import Path; "
              "p=Path(sys.argv[1]); (p/'cleanup-ready').touch(); "
              "assert sys.stdin.readline().strip() == 'continue'; "
              "(p/'cleanup-finished').touch()")
    # Preserve the actual helper's process-creation options, including session
    # ownership, timeout and capture; replace only the external Docker command.
    return native_run([sys.executable, "-B", "-u", "-c", worker, str(control)], **kwargs)

live.subprocess.run = substitute_command
try:
    with live.fixture() as (_name, stop):
        (control / "polling").touch()
        while True:
            stop.check()
            time.sleep(0.01)
except KeyboardInterrupt:
    raise SystemExit(130)
'''


class SignalTests(unittest.TestCase):
    def test_process_group_stops_cannot_interrupt_the_removal_command(self):
        with tempfile.TemporaryDirectory(prefix="victoriametrics-cleanup-") as directory:
            control = Path(directory)
            child = subprocess.Popen(
                [sys.executable, "-B", "-u", "-c", CLEANUP_PROCESS_CHILD,
                 str(Path(live.__file__).resolve().parent), directory],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                start_new_session=True,
            )
            try:
                for marker in ("polling", "cleanup-ready"):
                    deadline = time.monotonic() + 10
                    while not (control / marker).exists():
                        self.assertIsNone(child.poll(), marker)
                        self.assertLess(time.monotonic(), deadline, marker)
                        time.sleep(0.01)
                    os.killpg(child.pid, signal.SIGTERM)
                os.killpg(child.pid, signal.SIGINT)
                stdout, stderr = child.communicate(b"continue\n", timeout=15)
                self.assertEqual(child.returncode, 130, (stdout, stderr))
                self.assertTrue((control / "cleanup-finished").exists())
            finally:
                if child.poll() is None:
                    # Removal has its own ten-second subprocess deadline.
                    try:
                        child.communicate(b"continue\n", timeout=15)
                    except (subprocess.TimeoutExpired, ValueError):
                        child.kill()
                child.communicate(timeout=5)

    def test_matrix_grace_is_derived_from_the_complete_owned_stop_path(self):
        self.assertEqual(matrix.VICTORIAMETRICS_STOP_GRACE, live.STOP_GRACE)
        self.assertGreater(live.STOP_GRACE, live.WORK_GRACE +
                           live.WORK_REAP_ALLOWANCE + live.LOG_TIMEOUT + live.REMOVE_TIMEOUT)

    def test_outer_matrix_interrupt_allows_delayed_cleanup_and_producer_settlement(self):
        for stage in ("health", "read-back", "producer"):
            with self.subTest(stage=stage):
                outcome, removed = self.run_outer(stage, "current")
                self.assertTrue(removed, outcome)
                self.assertNotEqual(outcome["status"], -signal.SIGKILL, outcome)
                self.assertTrue(outcome["reaped"] and outcome["output_eof"], outcome)
                self.assertIn("batch-interrupted", outcome["errors"])
                self.assertIn("RESTORED", outcome["stdout"])

    def test_old_outer_grace_reproduces_cleanup_preemption(self):
        outcome, removed = self.run_outer("health", "old-default")
        self.assertFalse(removed, outcome)
        self.assertEqual(outcome["status"], -signal.SIGKILL, outcome)

    def run_outer(self, stage, policy):
        with tempfile.TemporaryDirectory(prefix="victoriametrics-signals-") as directory:
            control = Path(directory)
            child = subprocess.Popen(
                [sys.executable, "-B", "-u", "-c", OUTER_CHILD,
                 str(Path(live.__file__).resolve().parent), directory, stage, policy],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            try:
                deadline = time.monotonic() + 10
                while not (control / "polling").exists():
                    self.assertIsNone(child.poll(), "outer runner exited before checkpoint")
                    self.assertLess(time.monotonic(), deadline, "no polling checkpoint")
                    time.sleep(0.01)
                child.send_signal(signal.SIGTERM)
                stdout, stderr = child.communicate(timeout=15)
                self.assertEqual(child.returncode, 1, (stdout, stderr))
                return json.loads((control / "outcome.json").read_text()), (control / "removed").exists()
            finally:
                if child.poll() is None:
                    child.send_signal(signal.SIGTERM)
                    try:
                        child.communicate(timeout=15)
                    except subprocess.TimeoutExpired:
                        child.kill()
                child.communicate(timeout=5)

    def test_stop_at_each_polling_phase_cleans_up_despite_further_signals(self):
        for stage in ("health", "read-back", "entry"):
            for stop in (signal.SIGINT, signal.SIGTERM):
                with self.subTest(stage=stage, stop=stop):
                    self.check_stop(stage, stop)

    def check_stop(self, stage, stop):
        child = subprocess.Popen(
            [sys.executable, "-B", "-u", "-c", SIGNAL_CHILD,
             str(Path(live.__file__).resolve().parent), stage],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            bufsize=0, start_new_session=True,
        )
        output = b""

        def wait_for(marker):
            nonlocal output
            deadline = time.monotonic() + 10
            with selectors.DefaultSelector() as ready:
                ready.register(child.stdout, selectors.EVENT_READ)
                while marker not in output:
                    remaining = deadline - time.monotonic()
                    self.assertGreater(remaining, 0, (marker, output))
                    self.assertTrue(ready.select(remaining), (marker, output))
                    chunk = os.read(child.stdout.fileno(), 4096)
                    self.assertTrue(chunk, (marker, child.poll(), output))
                    output += chunk

        try:
            wait_for(b"POLLING\n")
            os.kill(child.pid, stop)
            if stage == "entry":
                # Signals here precede the removal call, while the work error
                # is unwinding. The same record-only owner must cover entry.
                wait_for(b"FINALIZING\n")
                os.killpg(child.pid, signal.SIGTERM)
                os.killpg(child.pid, signal.SIGINT)
                child.stdin.write(b"continue\n")
                child.stdin.flush()
            wait_for(b"REMOVING\n")
            # A second termination during cleanup must not prevent removal or
            # reset its deadline. Acknowledged checkpoints avoid timing sleeps.
            os.killpg(child.pid, signal.SIGTERM)
            os.killpg(child.pid, signal.SIGINT)
            tail, stderr = child.communicate(b"continue\n", timeout=10)
            output += tail
            self.assertEqual(child.returncode, 130, (output, stderr))
            self.assertEqual(output.count(b"REMOVED\n"), 1, output)
            self.assertIn(b"RESTORED\n", output)
        finally:
            if child.poll() is None:
                child.kill()
            child.communicate(timeout=5)


if __name__ == "__main__":
    unittest.main()
