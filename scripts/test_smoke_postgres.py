import os
import signal
import sys
import unittest
from unittest import mock

from smoke_postgres import check_output, check_run, run_smoke


# Real child processes exercise the same readiness/signal protocol as the smoke.
# This fixture has no database behavior and cannot prove pool closure.
CHILD = '''
import os, signal, sys, time
mode = sys.argv[1]
def stop(signum, frame):
    print("received=" + str(signum), flush=True)
    if mode == "overflow-after":
        os.write(1, b"x" * 8192)
    if mode != "no-cleanup":
        print('INFO cleanup observed cleanup="postgres.pool" outcome=Succeeded', flush=True)
    sys.exit(1 if mode == "failed-exit" else 0)
signal.signal(signal.SIGINT, signal.SIG_IGN if mode == "ignore" else stop)
signal.signal(signal.SIGTERM, signal.SIG_IGN if mode == "ignore" else stop)
print("child-pid=" + str(os.getpid()), file=sys.stderr, flush=True)
if mode == "interrupt":
    os.kill(os.getppid(), signal.SIGINT)
if mode == "early-exit":
    sys.exit(0)
if mode == "overflow-before":
    os.write(1, b"x" * 8192)
elif mode == "partial":
    sys.stdout.write("PostgreSQL lifecycle ready")
    sys.stdout.flush()
elif mode == "trailing-fields":
    print("INFO PostgreSQL lifecycle ready field=value", flush=True)
elif mode not in ("never-ready", "interrupt"):
    # stderr, ANSI styling, CRLF, and split writes are legal tracing output.
    sys.stderr.write("INFO \\x1b[32mPostgreSQL lifecycle ")
    sys.stderr.flush()
    time.sleep(0.03)
    sys.stderr.write("ready\\x1b[0m\\r\\n")
    sys.stderr.flush()
while True:
    signal.pause()
'''


class ExitEvidence(unittest.TestCase):
    def test_completed_lifecycle_is_accepted(self):
        check_output(0, 'PostgreSQL lifecycle ready\n'
                        'INFO cleanup observed cleanup="postgres.pool" outcome=Succeeded\n')

    def test_failure_status_is_not_hidden_by_successful_cleanup(self):
        with self.assertRaisesRegex(RuntimeError, "status 1"):
            check_output(1, 'PostgreSQL lifecycle ready\n'
                            'INFO cleanup observed cleanup="postgres.pool" outcome=Succeeded\n')

    def test_exit_before_readiness_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "initialized"):
            check_output(0, 'INFO cleanup observed cleanup="postgres.pool" outcome=Succeeded\n')

    def test_failed_or_unrelated_cleanup_is_rejected(self):
        for cleanup in ['cleanup="postgres.pool" outcome=Failed',
                        'cleanup="other" outcome=Succeeded']:
            with self.subTest(cleanup=cleanup), self.assertRaisesRegex(RuntimeError, "pool cleanup"):
                check_output(0, f'PostgreSQL lifecycle ready\nWARN cleanup observed {cleanup}\n')


class ProcessEvidence(unittest.TestCase):
    def run_child(self, mode, signum=signal.SIGTERM, **bounds):
        options = dict(startup_timeout=3, shutdown_timeout=3,
                       output_limit=4096, reap_allowance=2)
        options.update(bounds)
        result = run_smoke([sys.executable, "-c", CHILD, mode], signum, **options)
        self.assertTrue(result.reaped, result.metadata())
        self.assertTrue(result.output_eof, result.metadata())
        self.assertLessEqual(len(result.stdout) + len(result.stderr), options["output_limit"])
        pids = [int(line.removeprefix(b"child-pid=")) for line in result.stderr.splitlines()
                if line.startswith(b"child-pid=")]
        if not result.overflow:
            self.assertEqual(len(pids), 1, result.metadata())
        for pid in pids:
            with self.assertRaises(ChildProcessError):
                os.waitpid(pid, os.WNOHANG)
        return result

    def test_readiness_then_each_signal_and_cleanup(self):
        for signum in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(signum=signum):
                result = self.run_child("success", signum)
                check_run(result)
                self.assertIn(f"received={signum}", result.output)

    def test_child_installs_sigint_handler_after_inherited_ignore(self):
        previous = signal.signal(signal.SIGINT, signal.SIG_IGN)
        try:
            result = self.run_child("success", signal.SIGINT)
            check_run(result)
            self.assertIn(f"received={signal.SIGINT}", result.output)
            self.assertEqual(signal.getsignal(signal.SIGINT), signal.SIG_IGN)
        finally:
            signal.signal(signal.SIGINT, previous)

    def test_absent_or_partial_readiness_times_out_without_signal(self):
        for mode in ("never-ready", "partial"):
            with self.subTest(mode=mode):
                result = self.run_child(mode, startup_timeout=1)
                self.assertTrue(result.watchdog)
                self.assertIn("readiness-timeout", result.errors)
                self.assertEqual(result.status, -signal.SIGKILL)
                self.assertNotIn("received=", result.output)
                if mode == "partial":
                    self.assertEqual(result.stdout, b"PostgreSQL lifecycle ready")
                with self.assertRaisesRegex(RuntimeError, "Incomplete"):
                    check_run(result)

    def test_exit_before_readiness_is_not_a_timeout_or_success(self):
        result = self.run_child("early-exit")
        self.assertEqual(result.status, 0)
        self.assertFalse(result.watchdog)
        self.assertIn("exited-before-readiness", result.errors)
        with self.assertRaisesRegex(RuntimeError, "Incomplete"):
            check_run(result)

    def test_output_overflow_cannot_pass_before_or_after_readiness(self):
        for mode in ("overflow-before", "overflow-after"):
            with self.subTest(mode=mode):
                result = self.run_child(mode)
                self.assertTrue(result.overflow)
                self.assertFalse(result.watchdog)
                if mode == "overflow-before":
                    self.assertIn("readiness-output-overflow", result.errors)
                    self.assertNotIn("received=", result.output)
                else:
                    self.assertEqual(result.status, 0)
                with self.assertRaisesRegex(RuntimeError, "Incomplete"):
                    check_run(result)

    def test_shutdown_has_its_own_deadline_and_forces_reaping(self):
        real_kill = os.kill
        for stopped in (False, True):
            with self.subTest(stopped=stopped):
                requested = []

                def send_signal(pid, signum):
                    # The owner retains this direct child's PID until reaping.
                    # SIGSTOP models a ready child that cannot run a callback;
                    # the owner's SIGKILL must still terminate and reap it.
                    if stopped:
                        real_kill(pid, signal.SIGSTOP)
                    real_kill(pid, signum)
                    requested.append(signum)

                with mock.patch("scheduling_process.os.kill", side_effect=send_signal):
                    result = self.run_child("ignore", startup_timeout=10, shutdown_timeout=0.15)
                self.assertEqual(requested, [signal.SIGTERM])
                self.assertEqual(result.errors, ())
                self.assertTrue(result.watchdog)
                self.assertEqual(result.status, -signal.SIGKILL)
                self.assertLess(result.elapsed_seconds, 5)
                with self.assertRaisesRegex(RuntimeError, "Incomplete"):
                    check_run(result)

    def test_parent_interrupt_during_readiness_still_kills_and_reaps(self):
        previous = signal.signal(signal.SIGINT, signal.default_int_handler)
        try:
            result = self.run_child("interrupt")
            self.assertEqual(result.errors, ("interrupted",))
            self.assertFalse(result.watchdog)
            self.assertEqual(result.status, -signal.SIGKILL)
            self.assertNotIn("received=", result.output)
            self.assertNotIn("PostgreSQL lifecycle ready", result.output)
            self.assertIs(signal.getsignal(signal.SIGINT), signal.default_int_handler)
            with self.assertRaisesRegex(RuntimeError, "Incomplete"):
                check_run(result)
        finally:
            signal.signal(signal.SIGINT, previous)

    def test_readiness_followed_by_fields_does_not_match_the_line_suffix(self):
        result = self.run_child("trailing-fields", startup_timeout=1)
        self.assertIn(b"INFO PostgreSQL lifecycle ready field=value\n", result.stdout)
        self.assertEqual(result.errors, ("readiness-timeout",))
        self.assertTrue(result.watchdog)
        self.assertEqual(result.status, -signal.SIGKILL)
        self.assertNotIn("received=", result.output)
        with self.assertRaisesRegex(RuntimeError, "Incomplete"):
            check_run(result)

    def test_successful_signal_delivery_does_not_replace_exit_and_cleanup_checks(self):
        for mode in ("failed-exit", "no-cleanup"):
            with self.subTest(mode=mode):
                result = self.run_child(mode)
                self.assertFalse(result.watchdog)
                self.assertIn(f"received={signal.SIGTERM}", result.output)
                self.assertEqual(result.status, 1 if mode == "failed-exit" else 0)
                with self.assertRaisesRegex(RuntimeError, "Incomplete|pool cleanup"):
                    check_run(result)


if __name__ == "__main__":
    unittest.main()
