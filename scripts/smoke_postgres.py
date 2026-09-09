#!/usr/bin/env python3
"""Verify the SQLx executable against an externally provisioned PostgreSQL database."""

import argparse
import os
from pathlib import Path
import re
import signal
from scheduling_process import SignalAfterReady, run_process


READY = "PostgreSQL lifecycle ready"


def check_output(status: int, output: str) -> None:
    output = re.sub(r"\x1b\[[0-9;]*m", "", output)
    if status != 0:
        raise RuntimeError(f"Example did not exit successfully (status {status}).")
    if READY not in output:
        raise RuntimeError("Example did not acknowledge initialized components.")
    if not any("cleanup observed" in line and 'cleanup="postgres.pool"' in line
               and "outcome=Succeeded" in line for line in output.splitlines()):
        raise RuntimeError("Example did not report successful pool cleanup.")


def run_smoke(command, signum, *, startup_timeout=15, shutdown_timeout=25,
              output_limit=65536, reap_allowance=5):
    return run_process(
        command, timeout=startup_timeout, output_limit=output_limit,
        reap_allowance=reap_allowance,
        ready_signal=SignalAfterReady(READY.encode(), signum, shutdown_timeout),
    )


def check_run(result):
    if not result.ok:
        # Only metadata reaches diagnostics; a failing process may print secrets.
        raise RuntimeError(f"Incomplete PostgreSQL smoke run: {result.metadata()}")
    check_output(result.status, result.output)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--signal", choices=("SIGTERM", "SIGINT"), default="SIGTERM")
    args = parser.parse_args()
    if not os.environ.get("DATABASE_URL"):
        parser.error("DATABASE_URL is required; no database was provisioned or test skipped.")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error("Build the PostgreSQL lifecycle example before running this check.")
    check_run(run_smoke([str(binary)], getattr(signal, args.signal)))
    print(f"PASS: PostgreSQL readiness, pool cleanup, and {args.signal} exit 0.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
