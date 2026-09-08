#!/usr/bin/env python3
"""Smoke-test only a local, explicitly supplied Batter example process."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import signal
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--signal", choices=("SIGTERM", "SIGINT"), default="SIGTERM")
    parser.add_argument("--deadline", action="store_true", help="Exercise the 1 ms middleware deadline and custom error envelope.")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if os.name != "posix":
        parser.error("This process smoke test requires POSIX SIGTERM; use the in-process tests elsewhere.")
    if not binary.is_file():
        parser.error(f"Example binary not found: {binary}. Build it first; no test was run.")
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    env = dict(os.environ, BATTER_BIND=f"127.0.0.1:{port}", BATTER_REQUEST_TIMEOUT_MS="1" if args.deadline else "2000")
    # Exercise the example's ordinary INFO subscriber, not ambient verbose logging.
    env.pop("RUST_LOG", None)
    # Bypass unrelated proxy configuration; this test is strictly loopback.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with tempfile.TemporaryFile(mode="w+b") as logs:
        process = subprocess.Popen([str(binary)], env=env, stdout=logs, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 15
            while True:
                if process.poll() is not None:
                    raise RuntimeError(f"Example exited before readiness (status {process.returncode}).")
                try:
                    with opener.open(f"http://127.0.0.1:{port}/ready", timeout=1) as response:
                        if response.status == 200:
                            break
                except (urllib.error.URLError, TimeoutError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Readiness did not become available within the smoke-test allowance.")
                time.sleep(0.05)
            # Readiness now acknowledges signal registration; no startup sleep.
            cases = [("/live", 200, None), ("/ready", 200, None),
                     ("/work", 503 if args.deadline else 200, "deadline_exceeded" if args.deadline else None)]
            # A 1 ms policy may legitimately time out even the immediate /fail
            # handler under preemption; check its 500 only with the normal budget.
            if not args.deadline:
                cases.append(("/fail", 500, "internal_error"))
            for path, expected_status, expected_code in cases:
                try:
                    request = urllib.request.Request(
                        f"http://127.0.0.1:{port}{path}",
                        headers={"x-request-id": "untrusted-correlation-value"},
                    )
                    response = opener.open(request, timeout=3)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    if response.status != expected_status:
                        raise RuntimeError(f"Unexpected response for {path}: {response.status}")
                    body = response.read(4096)
                    if expected_code is not None:
                        payload = json.loads(body)
                        if set(payload) != {"code", "message", "request_id"} or payload["code"] != expected_code:
                            raise RuntimeError(f"Unexpected application envelope for {path}: {payload}")
                        request_id = payload["request_id"]
                        if not isinstance(request_id, str) or re.fullmatch(r"example-[0-9]+", request_id) is None:
                            raise RuntimeError(f"Missing generated correlation ID for {path}.")
                        if request_id != response.headers.get("x-request-id"):
                            raise RuntimeError(f"Correlation header/body mismatch for {path}.")
                        if response.headers.get("cache-control") != "no-store":
                            raise RuntimeError(f"Infrastructure failure may be cached for {path}.")
            process.send_signal(getattr(signal, args.signal))
            status = process.wait(timeout=25)
            if status != 0:
                raise RuntimeError(f"Graceful shutdown returned nonzero status {status}.")
            logs.seek(0)
            output = re.sub(r"\x1b\[[0-9;]*m", "", logs.read().decode("utf-8", errors="replace"))
            required = ["operation boundary finished", "http_outcome", "latency_ms", "request_id=example-"]
            if args.deadline:
                required += ["status=503", "deadline_exceeded"]
            else:
                required += ["status=500"]
            if any(field not in output for field in required):
                raise RuntimeError(f"Default INFO telemetry missing one of: {required}")
            if "untrusted-correlation-value" in output:
                raise RuntimeError("Untrusted correlation header leaked into telemetry.")
            print(f"PASS: probes, custom error envelopes, {'deadline' if args.deadline else 'work'}, default INFO telemetry, and {args.signal} exit 0.")
            return 0
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
            print(f"FAIL: {error}", file=sys.stderr)
            logs.seek(0)
            print(logs.read().decode("utf-8", errors="replace"), file=sys.stderr)
            return 1
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
