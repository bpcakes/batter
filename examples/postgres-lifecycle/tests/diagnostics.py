"""External watchdog for executable exit tests; no PostgreSQL is contacted."""

import os
from pathlib import Path
import signal
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "scripts"))
from scheduling_process import run_process


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def observe(command, *, env=None, timeout=10):
    result = run_process(command, env=env, timeout=timeout,
                         output_limit=65536, reap_allowance=2)
    # Assert metadata only: raw failing output might contain credentials.
    require(result.reaped and result.output_eof, result.metadata())
    require(not result.overflow and not result.errors, result.metadata())
    return result


def configuration(binary):
    for value in (None, b"postgres://test:credential-marker@localhost/db\xff",
                  b"postgres://test:credential-marker@127.0.0.1:invalid-port/db"):
        env = os.environb.copy()
        env.pop(b"DATABASE_URL", None)
        if value is not None:
            env[b"DATABASE_URL"] = value
        result = observe([binary], env=env)
        require(not result.watchdog and result.status == 1, result.metadata())
        require(result.stderr == b"Error: process failed\n", "unexpected exit diagnostic")
        require(b"credential-marker" not in result.stdout + result.stderr, "credential escaped")


def missing_configuration(binary):
    env = os.environb.copy()
    env.pop(b"DATABASE_URL", None)
    result = observe([binary], env=env)
    require(not result.watchdog and result.status == 1, result.metadata())
    require(not result.stdout, "unexpected standard output")
    require(result.stderr == b"Error: process failed\n", "unexpected exit diagnostic")


def watchdog():
    result = observe([sys.executable, "-c", "import time; time.sleep(60)"], timeout=0.2)
    require(result.watchdog and result.status == -signal.SIGKILL, result.metadata())
    require(result.elapsed_seconds < 5, result.metadata())


if __name__ == "__main__":
    if sys.argv[1] == "configuration":
        configuration(sys.argv[2])
    elif sys.argv[1] == "watchdog":
        watchdog()
    elif sys.argv[1] == "missing-configuration":
        missing_configuration(sys.argv[2])
    else:
        raise SystemExit("unknown diagnostic control")
