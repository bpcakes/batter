#!/usr/bin/env python3
"""Build the HTTP example and exercise all five process smoke profiles."""

import json
from pathlib import Path
import sys

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parents[1]
PROFILES = [[], ["--signal", "SIGINT"], ["--deadline"],
            ["--warn-filter"], ["--warn-filter", "--deadline"]]


def main():
    build = run_parallel(
        [["cargo", "build", "-p", "batter", "--features", "axum", "--example",
          "http_service", "--locked", "--message-format=json"]],
        timeout=900, output_limit=8 * 1024 * 1024, cwd=ROOT,
    )[0]
    if not build.ok:
        render_outcomes(["http-build"], [build])
        return 1
    sys.stderr.write(build.stderr.decode(errors="replace"))
    # Cargo reports the actual artifact path, including target-dir configuration
    # and an explicit build target. Never smoke an old hardcoded target/debug file.
    executables = set()
    for line in build.stdout.splitlines():
        artifact = json.loads(line)
        if (artifact.get("reason") == "compiler-artifact"
                and artifact.get("target", {}).get("name") == "http_service"
                and "example" in artifact["target"].get("kind", [])
                and artifact.get("executable")):
            executables.add(artifact["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"expected one HTTP executable, received {sorted(executables)}")
    executable = executables.pop()
    for profile in PROFILES:
        outcome = run_parallel(
            [[sys.executable, "scripts/smoke_http.py", "--binary", executable, *profile]],
            timeout=120, output_limit=2 * 1024 * 1024, cwd=ROOT,
        )[0]
        render_outcomes(["http-smoke " + (" ".join(profile) or "normal")], [outcome])
        if not outcome.ok:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
