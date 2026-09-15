#!/usr/bin/env python3
import argparse
import http.client
import os
import signal
import subprocess
import sys
import time
from urllib.parse import urlsplit


def fail(message, process=None):
    if process is not None and process.poll() is None:
        process.kill()
        process.wait(timeout=5)
    raise AssertionError(message)


def launch(binary, extra_env=None):
    env = os.environ.copy()
    env["RUST_LOG"] = "off"
    if extra_env:
        env.update(extra_env)
    return subprocess.Popen(
        [binary],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )


def listening(process, timeout=10):
    deadline = time.monotonic() + timeout
    lines = []
    while time.monotonic() < deadline:
        line = process.stdout.readline()
        if line:
            lines.append(line.rstrip("\n"))
            if line.startswith("consumer-listening:"):
                return line.strip().split(":", 1)[1], lines
        elif process.poll() is not None:
            break
    fail(f"no listening line before exit; stdout={lines!r}", process)


def request(url, path):
    parsed = urlsplit(url)
    connection = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=5)
    connection.request("GET", path)
    response = connection.getresponse()
    body = response.read().decode("utf-8")
    connection.close()
    return response.status, body


def expected_cleanup(variant):
    if variant == "initial":
        return ["consumer-cleanup:database.close:succeeded"]
    return [
        "consumer-cleanup:secondary.close:succeeded",
        "consumer-cleanup:database.close:succeeded",
    ]


def assert_tail(lines, variant, result):
    expected = expected_cleanup(variant) + [f"consumer-result:{result}"]
    if lines[-len(expected):] != expected:
        fail(f"wrong cleanup/result tail: expected={expected!r} actual={lines!r}")
    for marker in expected_cleanup(variant):
        if lines.count(marker) != 1:
            fail(f"cleanup marker must occur exactly once: {marker!r} in {lines!r}")


def service_scenario(binary, variant, trigger_failure):
    process = launch(binary)
    url, prefix = listening(process)
    status, body = request(url, "/query")
    if (status, body) != (200, "42"):
        fail(f"/query returned {(status, body)!r}", process)
    if trigger_failure:
        status, body = request(url, "/trigger-failure")
        if (status, body) != (202, "accepted"):
            fail(f"/trigger-failure returned {(status, body)!r}", process)
    else:
        os.kill(process.pid, signal.SIGTERM)
    try:
        stdout, stderr = process.communicate(timeout=12)
    except subprocess.TimeoutExpired:
        fail("service did not settle within 12 seconds", process)
    lines = prefix + stdout.splitlines()
    if trigger_failure:
        if process.returncode == 0:
            fail("native failure exited zero")
        if stderr != "consumer failed\n":
            fail(f"native failure diagnostic was not fixed: {stderr!r}")
        assert_tail(lines, variant, "failure")
    else:
        if process.returncode != 0:
            fail(f"TERM scenario exited {process.returncode}; stderr={stderr!r}")
        if stderr:
            fail(f"TERM scenario wrote stderr: {stderr!r}")
        assert_tail(lines, variant, "success")


def startup_failure(binary, variant):
    if variant != "modified":
        return
    process = launch(binary, {"CONSUMER_STARTUP_FAILURE": "1"})
    try:
        stdout, stderr = process.communicate(timeout=12)
    except subprocess.TimeoutExpired:
        fail("startup failure did not settle within 12 seconds", process)
    lines = stdout.splitlines()
    if process.returncode == 0:
        fail("fallible startup stage exited zero")
    if any(line.startswith("consumer-listening:") for line in lines):
        fail("fallible startup stage published a listener")
    if stderr != "consumer failed\n":
        fail(f"startup failure diagnostic was not fixed: {stderr!r}")
    assert_tail(lines, variant, "failure")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--variant", choices=("initial", "modified"), required=True)
    args = parser.parse_args()
    if "DATABASE_URL" not in os.environ:
        fail("DATABASE_URL is required")
    service_scenario(args.binary, args.variant, False)
    service_scenario(args.binary, args.variant, True)
    startup_failure(args.binary, args.variant)
    print(f"oracle:{args.variant}:ok")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"oracle failed: {error}", file=sys.stderr)
        sys.exit(1)
