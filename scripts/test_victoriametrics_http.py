"""Real HTTP parser progress must not postpone parent-owned fixture cleanup."""

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import check_victoriametrics as live
from parallel_process import interrupt_scope


@contextmanager
def progressing_server(mode, phase):
    started = threading.Event()
    finish = threading.Event()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            if mode == "metrics" and self.path == "/health":
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"OK")
                return
            try:
                if phase == "headers":
                    self.connection.sendall(b"HTTP/1.1 200 OK\r\nX-Progress: ")
                    fragment = b"a"
                else:
                    self.connection.sendall(
                        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                    fragment = b"1\r\nx\r\n"
                # Every read can make progress within urllib's two-second idle
                # timeout, but neither the header nor the body ever completes.
                while not finish.is_set():
                    self.connection.sendall(fragment)
                    started.set()
                    finish.wait(0.02)
            except OSError:
                pass  # The bounded polling child is intentionally interrupted.

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.02})
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", started
    finally:
        finish.set()
        server.shutdown()
        server.server_close()  # Join the finite handlers after releasing them.
        worker.join(timeout=2)
        if worker.is_alive():
            raise AssertionError("HTTP fixture did not stop")


HTTP_HARNESS = r'''
import json, signal, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import check_victoriametrics as live
from scheduling_process import ProcessOutcome

base, control = sys.argv[2], Path(sys.argv[3])
signal.signal(signal.SIGINT, signal.default_int_handler)
signal.signal(signal.SIGTERM, signal.SIG_DFL)
previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
live.build = lambda: "unused-producer"
owner = None

def docker(*args, **kwargs):
    global owner
    if args[0] == "run":
        owner = args[args.index("--name") + 1]
    elif args[0] == "inspect":
        return json.dumps([{"NetworkSettings":{"Ports":{"8428/tcp":[{
            "HostIp":"127.0.0.1", "HostPort":base.rsplit(":", 1)[1]}]}}}])
    elif args[0] == "rm":
        assert args == ("rm", "--force", "--volumes", owner)
        (control / "removed").write_text(owner)
    return ""

runner = live.run_parallel
def without_rust_producers(commands, **kwargs):
    if commands[0][0] == "env":
        return [ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())]
    # HTTP polling is actual urllib in the actual bounded worker, never a
    # cooperative get/verify checkpoint substituted by this fixture.
    return runner(commands, **kwargs)

live.docker = docker
live.run_parallel = without_rust_producers
live.render_outcomes = lambda *_args: None
try:
    live.main()
except BaseException as error:
    (control / "failure").write_text(type(error).__name__)
    raise SystemExit(1)
finally:
    assert {sig: signal.getsignal(sig) for sig in previous} == previous
    (control / "restored").touch()
'''


class HttpPollingTests(unittest.TestCase):
    def test_stop_during_progressing_headers_and_body_reaches_parent_cleanup(self):
        for mode in ("health", "metrics"):
            for phase in ("headers", "body"):
                with self.subTest(mode=mode, phase=phase):
                    self.check_stop(mode, phase)

    def check_stop(self, mode, phase):
        with progressing_server(mode, phase) as (base, started), \
                tempfile.TemporaryDirectory(prefix="victoriametrics-http-") as directory:
            control = Path(directory)
            child = subprocess.Popen(
                [sys.executable, "-B", "-u", "-c", HTTP_HARNESS,
                 str(Path(live.__file__).resolve().parent), base, directory],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
            )
            try:
                self.assertTrue(started.wait(10), "actual HTTP exchange did not start")
                child.send_signal(signal.SIGTERM)
                stdout, stderr = child.communicate(timeout=20)
                self.assertEqual(child.returncode, 1, (stdout, stderr))
                self.assertTrue((control / "removed").exists(), (stdout, stderr))
                self.assertTrue((control / "restored").exists(), (stdout, stderr))
            finally:
                if child.poll() is None:
                    child.send_signal(signal.SIGTERM)
                    try:
                        child.communicate(timeout=20)
                    except subprocess.TimeoutExpired:
                        child.kill()
                child.communicate(timeout=5)

    def test_complete_attempt_timeout_is_not_extended_by_progressing_body(self):
        with progressing_server("health", "body") as (base, started), \
                patch.object(live, "POLL_TIMEOUT", 0.3), interrupt_scope():
            begin = time.monotonic()
            with self.assertRaises(live.PollingCommandFailure) as raised:
                live.poll(base, "health")
            self.assertTrue(started.is_set(), "actual response body was exercised")
            self.assertTrue(raised.exception.outcome.watchdog)
            self.assertTrue(raised.exception.outcome.reaped)
            self.assertLess(time.monotonic() - begin,
                            0.3 + live.WORK_GRACE + live.WORK_REAP_ALLOWANCE + 1)


if __name__ == "__main__":
    unittest.main()
