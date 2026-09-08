#!/usr/bin/env python3
"""Run the scheduling test child with bounded output and an OS-process watchdog."""

import argparse
import json
import os
from pathlib import Path
import sys

from scheduling_process import run_process

OUTPUT_LIMIT = 1024 * 1024
MAX_WATCHDOG = 140
REAP_ALLOWANCE = 5
CHILD_ARGS = ["--exact", "scheduling_child", "--nocapture", "--test-threads=1"]
SCHEDULES = ("corpus", "capacity-before-drain", "capacity-after-drain", "stuck",
             "unjoined", "unjoined-delayed-start", "early-exit", "overflow")


def run(binary, workers, seed, schedule, watchdog, *, cwd=None):
    if not 0 < watchdog <= MAX_WATCHDOG:
        raise ValueError("watchdog must be positive and at most 140 seconds")
    # Preserve Cargo's runtime environment, including dynamic-library paths.
    # Environment variables do not authorize the fixture; argv and stdin do.
    env = {**os.environ, "RUST_BACKTRACE": "0"}
    env.pop("BATTER_SCHEDULING_CHILD", None)
    outcome = run_process(
        [str(binary), *CHILD_ARGS], timeout=watchdog, output_limit=OUTPUT_LIMIT,
        env=env, cwd=cwd, reap_allowance=REAP_ALLOWANCE,
        launch_record=lambda pid: (
            f"batter-scheduling-v1 {pid} {workers} "
            f"{seed if seed is not None else 'all'} {schedule}\n"
        ).encode(),
    )
    output = outcome.output
    summary = {
        "version": 1,
        "workers": workers,
        "seed": seed,
        "schedule": schedule,
        **outcome.metadata(),
    }
    # A child must reach the profile oracle, not merely exit successfully.
    success = outcome.ok and any(line.startswith("PROFILE_OK ") for line in output.splitlines())
    return success, summary, output


def render(summary, output):
    """Keep machine-readable completion separate from possibly truncated text."""
    separator = "" if not output or output.endswith("\n") else "\n"
    return output + separator + "WATCHDOG_RESULT " + json.dumps(summary, sort_keys=True) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--workers", type=int, choices=(2, 4), required=True)
    parser.add_argument("--seed", type=int)
    parser.add_argument("--schedule", choices=SCHEDULES, default="corpus")
    parser.add_argument("--watchdog", type=float, default=MAX_WATCHDOG)
    args = parser.parse_args()
    if not 0 < args.watchdog <= MAX_WATCHDOG:
        parser.error("watchdog must be positive and at most 140 seconds")
    if args.seed is not None and not 0 <= args.seed < 2**64:
        parser.error("seed must be an unsigned 64-bit integer")
    if args.schedule != "corpus" and args.seed is None:
        parser.error("a named schedule requires --seed")
    success, summary, output = run(
        args.binary.resolve(), args.workers, args.seed, args.schedule, args.watchdog
    )
    # Always retain checkpoints, including on a watchdog failure. Test fixtures
    # emit only generic names and numeric IDs, never application error contents.
    print(render(summary, output), end="", flush=True)
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
