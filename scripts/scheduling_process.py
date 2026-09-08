"""Bounded Unix process observation for the scheduling tools, not application work."""

from __future__ import annotations

from dataclasses import dataclass, replace
import os
import selectors
import signal
import subprocess
import threading
import time


@dataclass(frozen=True)
class ProcessOutcome:
    status: int | None
    stdout: bytes
    stderr: bytes
    elapsed_seconds: float
    watchdog: bool
    overflow: bool
    reaped: bool
    output_eof: bool
    errors: tuple[str, ...]

    @property
    def ok(self):
        return (self.status == 0 and self.reaped and self.output_eof
                and not self.watchdog and not self.overflow and not self.errors)

    @property
    def output(self):
        # Scheduling action/checkpoint records all use stderr, preserving their
        # order. Keep stdout separate as well so Cargo JSON need not be scraped
        # out of compiler diagnostics.
        separator = b"\n" if self.stdout and self.stderr and not self.stdout.endswith(b"\n") else b""
        return (self.stdout + separator + self.stderr).decode("utf-8", errors="replace")

    def metadata(self):
        return {
            "status": self.status,
            "watchdog": self.watchdog,
            "overflow": self.overflow,
            "reaped": self.reaped,
            "output_eof": self.output_eof,
            "errors": list(self.errors),
            "elapsed_seconds": round(self.elapsed_seconds, 3),
            "captured_bytes": len(self.stdout) + len(self.stderr),
        }


class Capture:
    def __init__(self, child, limit):
        self.buffers = [bytearray(), bytearray()]
        self.remaining = limit
        self.overflow = False
        self.selector = selectors.DefaultSelector()
        try:
            for index, stream in enumerate((child.stdout, child.stderr)):
                os.set_blocking(stream.fileno(), False)
                self.selector.register(stream, selectors.EVENT_READ, index)
        except BaseException:
            # The caller cannot own this Capture until construction returns.
            # Close even on interruption; selector cycles can defer GC cleanup.
            self.selector.close()
            raise

    @property
    def eof(self):
        return not self.selector.get_map()

    def complete(self, child):
        # poll() reaps an exited leader. Retain that leader (and therefore its
        # PID/group identity) while a pipe writer may still require group
        # termination. Only reap after EOF or after _settle has sent its signal.
        return self.eof and child.poll() is not None

    def observe_until(self, child, deadline, *, stop=None):
        while True:
            if stop is not None and stop.requested:
                return
            if self.complete(child):
                return
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            for key, _ in self.selector.select(min(0.02, remaining)):
                try:
                    data = os.read(key.fd, 8192)
                except BlockingIOError:
                    continue
                if not data:
                    self.selector.unregister(key.fileobj)
                    continue
                kept = data[:self.remaining]
                self.buffers[key.data].extend(kept)
                self.remaining -= len(kept)
                self.overflow |= len(data) > len(kept)


def _kill_group(child, errors):
    # Do not poll here: an exited but unreaped leader still reserves the ID.
    # A reaped leader cannot authorize any subsequent numeric group signal.
    if child.returncode is not None:
        return
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except OSError as error:
        errors.append(f"kill:{error.errno}")


def _settle(child, capture, allowance, errors):
    """One cleanup deadline covers pipe draining and observing direct-child exit."""
    deadline = time.monotonic() + allowance
    _kill_group(child, errors)
    if capture is not None:
        try:
            capture.observe_until(child, deadline)
        except OSError as error:
            errors.append(f"cleanup-output:{error.errno}")
    # A read failure must not skip reaping. Conversely, no hidden context-manager
    # exit may wait forever after this deadline. Escaped pipe owners are reported.
    while child.poll() is None and time.monotonic() < deadline:
        time.sleep(min(0.02, max(0, deadline - time.monotonic())))


@dataclass
class _Interrupt:
    requested: bool = False

    def __call__(self, _signal, _frame):
        # A SIGINT must not inject an exception into acquisition or cleanup.
        self.requested = True


def _unstarted(error):
    return ProcessOutcome(
        status=None, stdout=b"", stderr=b"", elapsed_seconds=0,
        watchdog=False, overflow=False, reaped=False, output_eof=False,
        errors=(error,),
    )


def run_process(command, *, timeout, output_limit, cwd=None, env=None,
                launch_record=None, keep_stdin=False, reap_allowance=5):
    """Observe a child with scoped SIGINT ownership and bounded cleanup.

    Requires the main thread, default SIGCHLD, and Python-default, SIG_DFL or
    SIG_IGN for SIGINT. Custom/unknown SIGINT ownership is rejected before launch.
    Ignored SIGINT stays ignored, including in children. For either default,
    SIGINT requests observation to stop; cleanup retains its original deadline.
    The exact prior disposition is restored after owned resources are released.
    A launch_record(pid) callback may provide at most 256 bytes. The stdin writer
    stays open until observation ends so a fixture can detect parent death.
    Process creation and OS scheduling cannot be preempted by a Python deadline.
    """
    if timeout <= 0 or reap_allowance <= 0 or output_limit <= 0:
        raise ValueError("process bounds must be positive")
    # CPython may synthesize returncode 0 after ECHILD if another owner reaps.
    if signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
        return _unstarted("unobservable-child-status")
    disposition = signal.getsignal(signal.SIGINT)
    if (threading.current_thread() is not threading.main_thread()
            or (disposition is not signal.default_int_handler
                and disposition not in (signal.SIG_DFL, signal.SIG_IGN))):
        return _unstarted("unsupported-sigint-owner")
    interrupt = _Interrupt()
    # An inherited ignore is Unix launch policy, not a competing callback.
    # Keeping SIG_IGN also preserves it across exec in the owned child.
    previous = signal.signal(
        signal.SIGINT, signal.SIG_IGN if disposition == signal.SIG_IGN else interrupt,
    )
    try:
        outcome = _run_owned_process(
            command, timeout=timeout, output_limit=output_limit, cwd=cwd, env=env,
            launch_record=launch_record, keep_stdin=keep_stdin,
            reap_allowance=reap_allowance, interrupt=interrupt,
        )
    finally:
        signal.signal(signal.SIGINT, previous)
    if interrupt.requested and "interrupted" not in outcome.errors:
        outcome = replace(outcome, errors=(*outcome.errors, "interrupted"))
    return outcome


def _close_resources(child, capture, errors):
    try:
        if capture is not None:
            try:
                capture.selector.close()
            except OSError as error:
                errors.append(f"close-selector:{error.errno}")
    finally:
        for stream in (child.stdin, child.stdout, child.stderr):
            if stream is not None:
                try:
                    stream.close()
                except OSError as error:
                    errors.append(f"close:{error.errno}")


def _run_owned_process(command, *, timeout, output_limit, cwd, env,
                       launch_record, keep_stdin, reap_allowance, interrupt):
    if interrupt.requested:
        return _unstarted("interrupted")
    started = time.monotonic()
    errors = []
    capture = None
    child = None
    watchdog = False
    output_eof = False
    try:
        child = subprocess.Popen(
            command, cwd=cwd, env=env, start_new_session=True, bufsize=0,
            stdin=subprocess.PIPE if launch_record or keep_stdin else subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        capture = Capture(child, output_limit)
        if launch_record is not None and not interrupt.requested:
            record = launch_record(child.pid)
            if not 0 < len(record) <= 256:
                raise ValueError("launch record exceeds bound")
            os.set_blocking(child.stdin.fileno(), False)
            if os.write(child.stdin.fileno(), record) != len(record):
                raise OSError("incomplete launch record")
        capture.observe_until(child, started + timeout, stop=interrupt)
        watchdog = not interrupt.requested and not capture.complete(child)
    except OSError as error:
        errors.append(f"{'spawn' if child is None else 'io'}:{error.errno}")
    except ValueError:
        errors.append("invalid-launch-record")
    except KeyboardInterrupt:
        errors.append("interrupted")
    finally:
        if child is not None:
            try:
                if capture is None or not capture.complete(child):
                    _settle(child, capture, reap_allowance, errors)
            finally:
                if capture is not None:
                    output_eof = capture.eof
                _close_resources(child, capture, errors)
    status = child.poll() if child is not None else None
    return ProcessOutcome(
        status=status,
        stdout=bytes(capture.buffers[0]) if capture else b"",
        stderr=bytes(capture.buffers[1]) if capture else b"",
        elapsed_seconds=time.monotonic() - started,
        watchdog=watchdog,
        overflow=capture.overflow if capture else False,
        reaped=child is not None and status is not None,
        output_eof=output_eof,
        errors=tuple(errors),
    )
