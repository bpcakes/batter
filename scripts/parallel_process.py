"""Private bounded Unix command batches for local verification (at most four jobs)."""

from __future__ import annotations

from dataclasses import dataclass, field, replace
import math
import os
import signal
import subprocess
import threading
import time

from scheduling_process import Capture, ProcessOutcome, _close_resources, _settle, _unstarted


@dataclass
class _Job:
    started: float
    child: object = None
    capture: object = None
    errors: list = field(default_factory=list)
    watchdog: bool = False
    stopping_at: float | None = None
    outcome: ProcessOutcome | None = None

    def stop(self, reason):
        if reason not in self.errors:
            self.errors.append(reason)
        if self.stopping_at is not None:
            return
        self.stopping_at = time.monotonic()
        # Capture.complete retains the group leader until EOF. Never signal
        # after it has reaped the leader, even if a later failure is discovered.
        if self.child is not None and self.child.returncode is None:
            try:
                os.killpg(self.child.pid, signal.SIGINT)
            except ProcessLookupError:
                pass
            except OSError as error:
                self.errors.append(f"interrupt:{error.errno}")

    def finish(self, reap_allowance):
        if self.outcome is not None:
            return
        child, capture = self.child, self.capture
        try:
            complete = capture is not None and capture.eof and child.poll() is not None
            if child is not None and not complete:
                _settle(child, capture, reap_allowance, self.errors)
            status = child.poll() if child is not None else None
            stdout, stderr = capture.retained_buffers() if capture else (b"", b"")
            self.outcome = ProcessOutcome(
                status=status,
                stdout=stdout,
                stderr=stderr,
                elapsed_seconds=time.monotonic() - self.started,
                watchdog=self.watchdog,
                overflow=capture.overflow if capture else False,
                reaped=child is not None and status is not None,
                output_eof=capture.eof if capture else False,
                errors=tuple(self.errors),
            )
        finally:
            if child is not None:
                _close_resources(child, capture, self.errors)
        # Descriptor-close failures also invalidate otherwise successful evidence.
        if self.outcome is not None and self.outcome.errors != tuple(self.errors):
            self.outcome = replace(self.outcome, errors=tuple(self.errors))


def run_parallel(commands, *, timeout, output_limit, grace=10, reap_allowance=5,
                 cwd=None, retain_tail=False):
    """Run 1..4 commands concurrently, returning every outcome in input order.

    Bounds apply per command; output is drained after overflow but never accepted
    as successful evidence. `retain_tail` keeps bounded head/tail diagnostics with
    explicit gap markers instead of a prefix only. SIGINT/SIGTERM stop further
    launches and request SIGINT in owned groups, then allow `grace` seconds before
    kill/reap. An inherited ignored SIGINT stays ignored. Repeated signals never restart cleanup budgets.
    Each final reap/output observation is bounded by `reap_allowance` (at most four
    such observations). Arbitrarily detached descendants and OS process creation
    are not bounded by this owner. Call on the main thread with standard handlers.
    """
    if (not 1 <= len(commands) <= 4 or not all(commands)
            or any(not math.isfinite(x) or x <= 0 for x in (timeout, grace, reap_allowance))
            or output_limit <= 0):
        raise ValueError("a batch requires 1..4 commands and positive finite bounds")
    previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    supported = (signal.SIG_DFL, signal.SIG_IGN, signal.default_int_handler)
    if (threading.current_thread() is not threading.main_thread()
            or signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL
            or any(handler not in supported for handler in previous.values())):
        return [_unstarted("unsupported-signal-owner") for _ in commands]
    interrupted = False

    def interrupt(_sig, _frame):
        nonlocal interrupted
        interrupted = True

    jobs = []
    installed = []
    try:
        for sig, handler in previous.items():
            signal.signal(sig, handler if handler == signal.SIG_IGN else interrupt)
            installed.append(sig)
        for command in commands:
            job = _Job(time.monotonic())
            jobs.append(job)  # Retain ownership even when capture setup fails.
            if interrupted:
                job.errors.append("interrupted")
                job.finish(reap_allowance)
                continue
            try:
                job.child = subprocess.Popen(
                    command, cwd=cwd, start_new_session=True, bufsize=0,
                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                job.capture = Capture(job.child, output_limit, retain_tail=retain_tail)
            except (OSError, ValueError) as error:
                job.errors.append(f"launch:{type(error).__name__}:{getattr(error, 'errno', None)}")
                job.finish(reap_allowance)
        while any(job.outcome is None for job in jobs):
            for job in jobs:
                if job.outcome is not None:
                    continue
                if interrupted:
                    job.stop("interrupted")
                try:
                    job.capture.observe_until(job.child, time.monotonic() + 0.01)
                    complete = job.capture.complete(job.child)
                except OSError as error:
                    job.errors.append(f"capture:{error.errno}")
                    job.finish(reap_allowance)
                    continue
                if complete:
                    job.finish(reap_allowance)
                elif job.stopping_at is None and time.monotonic() >= job.started + timeout:
                    job.watchdog = True
                    job.stop("timeout")
                elif job.stopping_at is not None and time.monotonic() >= job.stopping_at + grace:
                    job.finish(reap_allowance)
    finally:
        try:
            failure = None
            for job in jobs:
                if job.outcome is None:
                    job.errors.append("batch-abandoned")
                    try:
                        job.finish(reap_allowance)
                    except BaseException as error:
                        failure = failure or error
            if failure is not None:
                raise failure
        finally:
            for sig in reversed(installed):
                signal.signal(sig, previous[sig])
    # Interruption at the last completion is still failed batch evidence.
    if interrupted:
        return [replace(job.outcome, errors=(*job.outcome.errors, "batch-interrupted"))
                for job in jobs]
    return [job.outcome for job in jobs]


def render_outcomes(labels, outcomes):
    """Emit each bounded test log and outcome together, without interleaving jobs."""
    import json
    import sys
    for label, outcome in zip(labels, outcomes):
        print(f"=== {label} ===", file=sys.stderr, flush=True)
        print(outcome.output, end="" if outcome.output.endswith("\n") else "\n", flush=True)
        print(f"{label}: {json.dumps(outcome.metadata(), sort_keys=True)}", file=sys.stderr,
              flush=True)
