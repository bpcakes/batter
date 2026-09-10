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


def check_completion_fields(line: str, route: str, status: int, request_id=None) -> None:
    """Require HTTP facts on the event, independently of its formatted spans."""
    _, boundary, event = line.partition("HTTP response boundary finished")
    outcome = "server_error" if status >= 500 else "client_error" if status >= 400 else "completed"
    fields = ('method="GET"', f'route="{route}"', f"status={status}",
              f'http_outcome="{outcome}"')
    if request_id is not None:
        fields += (f'request_id="{request_id}"',)
    if not boundary or re.search(r"(?:^|\s)latency_ms=\S+", event) is None or any(
        re.search(r"(?:^|\s)" + re.escape(field) + r"(?=\s|$)", event) is None
        for field in fields
    ):
        raise RuntimeError(f"HTTP completion event fields mismatch for {route}: {line}")


def check_operation_filter(output: str, request_ids: set[str], deadline_id=None) -> None:
    """Require correlated WARN operations without enabling INFO completions."""
    deadline_seen = False
    for line in output.splitlines():
        if "operation boundary finished" not in line:
            continue
        if re.match(r"\s*(?:\d{4}-\d{2}-\d{2}T\S+\s+)?INFO\b", line):
            raise RuntimeError(f"INFO operation event escaped the WARN filter: {line}")
        ids = re.findall(r'\brequest_id="?([A-Za-z0-9-]+)', line)
        if not ids or any(value not in request_ids for value in ids):
            raise RuntimeError(f"Operation warning lost request correlation: {line}")
        event = line.partition("operation boundary finished")[2]
        if deadline_id in ids and 'outcome="deadline_exceeded"' in event:
            deadline_seen = True
    if deadline_id is not None and not deadline_seen:
        raise RuntimeError("Missing correlated deadline operation completion.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--signal", choices=("SIGTERM", "SIGINT"), default="SIGTERM")
    parser.add_argument("--deadline", action="store_true", help="Exercise the 1 ms middleware deadline and custom error envelope.")
    parser.add_argument("--warn-filter", action="store_true", help="Use info,batter=warn,batter::request=info to retain nested correlation with Batter INFO completions disabled.")
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
    # Select a known filter profile, independent of ambient verbose logging.
    env.pop("RUST_LOG", None)
    if args.warn_filter:
        env["RUST_LOG"] = "info,batter=warn,batter::request=info"
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
                     ("/secret-missing?secret-query=value", 404, None),
                     ("/work", 503 if args.deadline else 200, "deadline_exceeded" if args.deadline else None)]
            observations = []
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
                    request_id = response.headers.get("x-request-id")
                    if not isinstance(request_id, str) or re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}", request_id) is None:
                        raise RuntimeError(f"Missing generated correlation ID for {path}.")
                    route = "<unmatched>" if expected_status == 404 else path
                    observations.append((request_id, route, expected_status))
                    body = response.read(4096)
                    if expected_code is not None:
                        payload = json.loads(body)
                        if set(payload) != {"code", "message", "request_id"} or payload["code"] != expected_code:
                            raise RuntimeError(f"Unexpected application envelope for {path}: {payload}")
                        if payload["request_id"] != request_id:
                            raise RuntimeError(f"Correlation header/body mismatch for {path}.")
                        if response.headers.get("cache-control") != "no-store":
                            raise RuntimeError(f"Infrastructure failure may be cached for {path}.")
            process.send_signal(getattr(signal, args.signal))
            status = process.wait(timeout=25)
            if status != 0:
                raise RuntimeError(f"Graceful shutdown returned nonzero status {status}.")
            logs.seek(0)
            output = re.sub(r"\x1b\[[0-9;]*m", "", logs.read().decode("utf-8", errors="replace"))
            required = ["http_outcome", "latency_ms", "request_id="]
            if not args.warn_filter:
                required += ["operation boundary finished"]
            else:
                deadline_id = next((identity for identity, route, _ in observations
                                    if route == "/work"), None) if args.deadline else None
                check_operation_filter(output, {identity for identity, _, _ in observations}, deadline_id)
            if args.deadline:
                required += ["status=503", "deadline_exceeded"]
            else:
                required += ["status=500"]
            if any(field not in output for field in required):
                raise RuntimeError(f"Telemetry missing one of: {required}")
            for request_id, route, expected_status in observations:
                events = [line for line in output.splitlines()
                          if "HTTP response boundary finished" in line
                          and re.search(r'\brequest_id="?' + re.escape(request_id) + r"\b", line)]
                expected_count = 0 if args.warn_filter and expected_status < 500 else 1
                if len(events) != expected_count:
                    raise RuntimeError(f"Expected {expected_count} HTTP completions for {route}/{request_id}, got {len(events)}.")
                if not events:
                    continue
                check_completion_fields(events[0], route, expected_status, request_id)
                if args.warn_filter:
                    if re.search(r"\bWARN\b", events[0]) is None or "batter.http" in events[0]:
                        raise RuntimeError(f"Filtered HTTP event depends on its disabled span: {events[0]}")
            if "untrusted-correlation-value" in output or "secret-" in output:
                raise RuntimeError("Untrusted request metadata leaked into telemetry.")
            telemetry = "WARN filtering with retained request correlation" if args.warn_filter else "default INFO telemetry with one HTTP event per request"
            print(f"PASS: probes/fallback, custom error envelopes, {'deadline' if args.deadline else 'work'}, {telemetry}, and {args.signal} exit 0.")
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
