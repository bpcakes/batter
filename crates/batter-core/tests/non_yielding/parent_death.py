"""Linux regression: SIGKILL only the fixture owner; adopt and reap its child.

Called by the bounded Rust integration test. Python 3 is already required by
the workspace's local verification. No global process state in Cargo's test
runner is changed: subreaping applies only to this separate probe process.
All evidence checks use explicit failures so Python optimization cannot remove
them. Arguments name the child command; the Rust controls also supply a Python
command that deliberately violates the termination contract.
"""

import ctypes
import os
import select
import signal
import subprocess
import sys
import time


def read_until(fd, marker, deadline):
    output = bytearray()
    while marker not in output:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
            raise SystemExit(f"missing {marker!r}: {output!r}")
        chunk = os.read(fd, 4096)
        if not chunk:
            raise SystemExit(f"EOF before {marker!r}: {output!r}")
        output.extend(chunk)
        if len(output) > 64 * 1024:
            raise SystemExit("probe output overflow")
    return bytes(output)


def kill_and_reap(pid):
    try:
        waited, _ = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return
    if waited == 0:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)


def main():
    # Independent of the Rust parent, including while handling a probe failure.
    signal.alarm(14)
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    # PR_SET_CHILD_SUBREAPER: orphaned descendants become waitable here.
    if libc.prctl(36, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")

    ready_read, ready_write = os.pipe()
    output_read, output_write = os.pipe()
    owner = os.fork()
    if owner == 0:
        # If the outer probe dies, this owner still cannot live indefinitely.
        signal.alarm(12)
        os.close(ready_read)
        os.close(output_read)
        child = subprocess.Popen(
            [*sys.argv[1:], "--exact", "child_fixture", "--nocapture", "--test-threads=1"],
            stdin=subprocess.PIPE,
            stdout=output_write,
            stderr=subprocess.STDOUT,
            env={**os.environ, "RUST_BACKTRACE": "0"},
        )
        os.write(ready_write, f"{child.pid}\n".encode())
        os.close(ready_write)
        os.close(output_write)
        child.stdin.write(f"batter-fixture-v1 {child.pid} non-yielding\n".encode())
        child.stdin.flush()
        while True:
            signal.pause()

    os.close(ready_write)
    os.close(output_write)
    child_pid = None
    try:
        deadline = time.monotonic() + 5
        child_pid = int(read_until(ready_read, b"\n", deadline))
        output = read_until(output_read, b"runtime-drop-started", deadline)
        if b"report-unjoined-cleanup-skipped" not in output:
            raise SystemExit(f"missing unjoined report: {output!r}")
        # Kill exactly the owner PID, with no process-group signal and no
        # Python/Rust unwinding. Its stdin writer closes in the kernel.
        os.kill(owner, signal.SIGKILL)
        _, owner_status = os.waitpid(owner, 0)
        if not (os.WIFSIGNALED(owner_status) and os.WTERMSIG(owner_status) == signal.SIGKILL):
            raise SystemExit(f"unexpected owner status: {owner_status}")
        deadline = time.monotonic() + 3
        while True:
            waited, status = os.waitpid(child_pid, os.WNOHANG)
            if waited:
                break
            if time.monotonic() >= deadline:
                raise SystemExit("orphan did not observe parent death")
            time.sleep(0.01)
        # 74 is the launch protocol's parent-disconnected exit, distinct from
        # the ten-second emergency deadline and the parent's SIGKILL status.
        if not (os.WIFEXITED(status) and os.WEXITSTATUS(status) == 74):
            raise SystemExit(f"unexpected orphan status: {status}")
        while chunk := os.read(output_read, 4096):
            output += chunk
            if len(output) > 64 * 1024:
                raise SystemExit("probe output overflow")
        for forbidden in (b"task-dropped", b"cleanup-invoked", b"runtime-dropped", b"panicked at"):
            if forbidden in output:
                raise SystemExit(f"forbidden orphan event {forbidden!r}: {output!r}")
        print("batter-fixture:parent-death-reaped", flush=True)
    finally:
        kill_and_reap(owner)
        if child_pid is not None:
            kill_and_reap(child_pid)
        # Also reap a child adopted if the owner died before reporting its PID.
        # Such a fixture has closed stdin and its own emergency deadline.
        while True:
            try:
                os.waitpid(-1, 0)
            except ChildProcessError:
                break
        os.close(ready_read)
        os.close(output_read)


if __name__ == "__main__":
    main()
