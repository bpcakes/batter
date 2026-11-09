#!/usr/bin/env python3
"""Build real producers, provision pinned VictoriaMetrics, and query stored metrics."""

import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from contextlib import contextmanager
from dataclasses import replace

from parallel_process import interrupt_scope, render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parents[1]
IMAGE = ("victoriametrics/victoria-metrics:v1.156.0@sha256:"
         "da8a90ac0912f16589be2b96a6441a2fca4bca415c83783d972d92e3b5b66970")
TEST = "exports_cumulative_metrics_through_service_completion"
INSTANCES = {"worker-1": 2, "worker-2": 5}
# One work child is active at a time. Its caller must allow settlement followed
# by diagnostics and removal before escalating the harness itself.
WORK_GRACE = 10
WORK_REAP_ALLOWANCE = 5
LOG_TIMEOUT = 2
REMOVE_TIMEOUT = 10
POLL_TIMEOUT = 5
MAX_POLL_RESPONSE = 2 * 1024 * 1024
STOP_GRACE = WORK_GRACE + WORK_REAP_ALLOWANCE + LOG_TIMEOUT + REMOVE_TIMEOUT + 5


class DockerCommandFailure(RuntimeError):
    """Retain bounded process evidence without putting its output in the message."""

    def __init__(self, outcome):
        super().__init__("Docker command did not complete successfully")
        self.outcome = outcome


def docker(*args, timeout=60, interruptible=False):
    if interruptible:
        outcome, = run_parallel(
            [["docker", *args]], cwd=ROOT, timeout=timeout, output_limit=1024 * 1024,
            grace=WORK_GRACE, reap_allowance=WORK_REAP_ALLOWANCE,
        )
        if not outcome.ok:
            raise DockerCommandFailure(outcome)
        return outcome.stdout.decode().strip()
    result = subprocess.run(["docker", *args], cwd=ROOT, check=True, timeout=timeout,
                            capture_output=True, text=True, start_new_session=True)
    return result.stdout.strip()


def get(url):
    # Fixture requests are explicitly local and must not use ambient proxies.
    # This blocking operation runs only in the parent's bounded polling child.
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=2) as response:
        body = response.read(MAX_POLL_RESPONSE + 1)
        if len(body) > MAX_POLL_RESPONSE:
            raise ValueError("poll response exceeds its byte limit")
        return body


class PollingNotReady(AssertionError):
    """An expected, completed read-only probe that did not establish readiness."""

    def __init__(self, outcome):
        super().__init__("VictoriaMetrics polling did not establish readiness")
        self.outcome = outcome


class PollingCommandFailure(RuntimeError):
    """Retain timeout/interruption/crash evidence without automatically replaying it."""

    def __init__(self, outcome):
        super().__init__("VictoriaMetrics polling command did not complete successfully")
        self.outcome = outcome


def poll(base, mode):
    """Own an entire urllib exchange/query sequence under one subprocess deadline."""
    worker = Path(__file__).with_name("victoriametrics_poll.py")
    outcome, = run_parallel(
        [[sys.executable, "-B", str(worker), mode, base]], timeout=POLL_TIMEOUT,
        output_limit=64 * 1024, cwd=ROOT, grace=WORK_GRACE,
        reap_allowance=WORK_REAP_ALLOWANCE,
    )
    if (outcome.status == 3 and not outcome.watchdog and not outcome.overflow
            and not outcome.errors and outcome.reaped and outcome.output_eof):
        raise PollingNotReady(outcome)
    if not outcome.ok:
        raise PollingCommandFailure(outcome)


def query(base, metric):
    params = urllib.parse.urlencode({"query": '{__name__="' + metric + '"}', "nocache": "1"})
    response = json.loads(get(base + "/api/v1/query?" + params))
    if response.get("status") != "success" or response["data"]["resultType"] != "vector":
        raise AssertionError(f"query failed: {response}")
    return response["data"]["result"]


def samples(base, metric, labels):
    values = {}
    for series in query(base, metric):
        attrs = series["metric"]
        if attrs.get("service.name") != "example-service":
            continue
        if not all(attrs.get(key) == value for key, value in labels.items()):
            continue
        instance = attrs.get("service.instance.id")
        if instance in values:
            raise AssertionError(f"duplicate series for {metric}: {instance}")
        values[instance] = float(series["value"][1])
    return values


def verify(base):
    labels = {"operation": "live.operation", "outcome": "succeeded"}
    checks = [
        ("batter_operation_completions_total", labels, INSTANCES),
        ("batter_operation_duration_seconds_count", labels, INSTANCES),
        ("batter_operation_duration_seconds_bucket", {**labels, "le": "+Inf"}, INSTANCES),
        ("batter_cleanup_hooks_total", {"outcome": "succeeded"}, dict.fromkeys(INSTANCES, 1)),
        ("batter_shutdowns_total", {"result": "success"}, dict.fromkeys(INSTANCES, 1)),
        ("batter_shutdown_duration_seconds_count", {"result": "success"}, dict.fromkeys(INSTANCES, 1)),
    ]
    for metric, selected, expected in checks:
        actual = samples(base, metric, selected)
        if actual != expected:
            raise AssertionError(f"{metric}: expected {expected}, received {actual}")
    sums = samples(base, "batter_operation_duration_seconds_sum", labels)
    if sums.keys() != INSTANCES.keys() or not all(value > 0 for value in sums.values()):
        raise AssertionError(f"missing or invalid duration sums: {sums}")


def eventually(action, stop, seconds=20):
    deadline = time.monotonic() + seconds
    while True:
        stop.check()
        try:
            result = action()
            stop.check()
            return result
        except (OSError, ValueError, AssertionError, urllib.error.URLError):
            stop.check()
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.1)


def build():
    outcome = run_parallel(
        [["cargo", "test", "-p", "batter-otlp", "--test", "victoriametrics", "--no-run",
          "--locked", "--message-format=json"]],
        timeout=900, output_limit=8 * 1024 * 1024, cwd=ROOT,
    )[0]
    if not outcome.ok:
        # Render compiler diagnostics, not thousands of artifact JSON lines.
        diagnostics = []
        for line in outcome.stdout.splitlines():
            try:
                message = json.loads(line).get("message", {}).get("rendered")
                if message:
                    diagnostics.append(message)
            except (ValueError, AttributeError):
                diagnostics.append(line.decode(errors="replace"))
        render_outcomes(["victoriametrics-build"], [replace(
            outcome, stdout="".join(diagnostics).encode(),
        )])
        raise RuntimeError("producer build failed")
    sys.stderr.write(outcome.stderr.decode(errors="replace"))
    executables = set()
    for line in outcome.stdout.splitlines():
        artifact = json.loads(line)
        if (artifact.get("reason") == "compiler-artifact"
                and artifact.get("target", {}).get("name") == "victoriametrics"
                and artifact.get("executable")):
            executables.add(artifact["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"expected one producer executable, received {executables}")
    return executables.pop()


@contextmanager
def fixture():
    """Own termination and one bounded removal attempt for our unique name.

    Docker's forced removal accepts an already-absent container. Other failures
    propagate; if work also failed, Python preserves it as exception context.
    A failed/ambiguous launch or an unavailable daemon is not settlement proof.
    """
    name = "batter-victoriametrics-" + uuid.uuid4().hex[:12]
    with interrupt_scope() as stop:
        try:
            yield name, stop
        finally:
            # The handler only records requests throughout the scope. There is
            # no exception-raising signal transition on entry to finalization.
            docker("rm", "--force", "--volumes", name, timeout=REMOVE_TIMEOUT)
        stop.check()  # A late stop cannot turn an otherwise successful run into a pass.


def main():
    binary = build()
    with fixture() as (name, stop):
        exercise(binary, name, stop)


def exercise(binary, name, stop):
    """Run the real producers while fixture owns signals and finalization."""
    try:
        stop.check()
        docker("run", "--detach", "--name", name, "--publish", "127.0.0.1::8428",
               IMAGE, "-storageDataPath=/storage", "-retentionPeriod=1d", "-search.latencyOffset=0",
               timeout=180, interruptible=True)
        stop.check()
        info = json.loads(docker("inspect", name, interruptible=True))[0]
        stop.check()
        binding = info["NetworkSettings"]["Ports"]["8428/tcp"][0]
        if binding["HostIp"] != "127.0.0.1":
            raise RuntimeError("fixture must bind only to loopback")
        base = "http://127.0.0.1:" + binding["HostPort"]
        eventually(lambda: poll(base, "health"), stop)
        print(f"VictoriaMetrics fixture: {IMAGE}", flush=True)
        clean = [item for key in os.environ if key.startswith("OTEL_") for item in ("-u", key)]
        for instance, count in INSTANCES.items():
            stop.check()
            outcome = run_parallel(
                [["env", *clean, f"BATTER_TEST_VM_ENDPOINT={base}/opentelemetry/v1/metrics",
                  f"BATTER_TEST_VM_INSTANCE={instance}", f"BATTER_TEST_VM_COUNT={count}",
                  binary, "--ignored", "--exact", TEST, "--nocapture"]],
                timeout=30, output_limit=1024 * 1024, cwd=ROOT,
                grace=WORK_GRACE, reap_allowance=WORK_REAP_ALLOWANCE,
            )[0]
            render_outcomes([instance], [outcome])
            if not outcome.ok:
                raise RuntimeError(f"producer {instance} failed")
        eventually(lambda: poll(base, "metrics"), stop)
        print("VictoriaMetrics read-back passed: two distinct instances, cumulative counters, "
              "histogram counts/buckets/sums, cleanup and final shutdown metrics.", flush=True)
    except BaseException:
        try:
            print(docker("logs", "--tail", "30", name, timeout=LOG_TIMEOUT), file=sys.stderr)
        except (OSError, subprocess.SubprocessError):
            pass
        raise


if __name__ == "__main__":
    main()
