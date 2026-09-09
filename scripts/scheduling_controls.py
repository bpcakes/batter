"""Discovery, partitioning and completion evidence for isolated unittest shards."""

import json
from pathlib import Path
import sys
import unittest

from parallel_process import render_outcomes, run_parallel

MARKER = "CONTROL_RESULT "
# Scheduling hints only; these never alter a test's deadline or assertions.
SLOW_CONTROLS = {
    "test_missing_record_has_a_native_startup_deadline": 5,
    "test_escaped_pipe_deadline_keeps_checkpoints": 3.2,
    "test_unobserved_exit_is_reported_without_an_unbounded_wait": 3.1,
    "test_stdout_eof_is_not_process_exit": 3,
    "test_build_timeout_persists_partial_logs_and_classification": 3,
    "test_descendant_pipe_is_contained_by_the_owned_group": 3,
    "test_replay_timeout_is_retained_and_cannot_reject_a_mutant": 3,
    "test_timeout_retains_output_and_reaps": 3,
    "test_delayed_start_still_reaches_the_success_checkpoint": 1,
    "test_sigint_during_cleanup_preserves_deadline_evidence_and_resources": 0.8,
}


def cases(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from cases(test)
        else:
            yield test


def partition(suite, count):
    tests = list(cases(suite))
    ids = [test.id() for test in tests]
    if not tests or len(ids) != len(set(ids)) or not 1 <= count <= 4:
        raise ValueError("shards require unique nonempty discovery and 1..4 workers")
    shards, loads = [[] for _ in range(count)], [0.0] * count
    for test in sorted(tests, key=lambda t: (-SLOW_CONTROLS.get(t.id().split('.')[-1], 0.1),
                                           t.id())):
        index = min(range(count), key=loads.__getitem__)
        shards[index].append(test)
        loads[index] += SLOW_CONTROLS.get(test.id().split('.')[-1], 0.1)
    return shards


class RecordedResult(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.executed = []

    def startTest(self, test):
        self.executed.append(test.id())
        super().startTest(test)


def run_shard(tests):
    result = unittest.TextTestRunner(verbosity=2, resultclass=RecordedResult).run(
        unittest.TestSuite(tests))
    success = result.wasSuccessful() and not result.skipped
    print(MARKER + json.dumps({"tests": sorted(result.executed), "successful": success}),
          flush=True)
    return 0 if success else 1


def completed(outcome, expected):
    if not outcome.ok:
        return False
    records = [line[len(MARKER):] for line in outcome.stdout.decode("utf-8", errors="replace")
               .splitlines() if line.startswith(MARKER)]
    try:
        return len(records) == 1 and json.loads(records[0]) == {
            "tests": sorted(test.id() for test in expected), "successful": True,
        }
    except (ValueError, TypeError):
        return False


def run_shards(suite, *, binary, count, shard=None):
    shards = partition(suite, count)
    if shard is not None:
        return run_shard(shards[shard])
    script = Path(__file__).with_name("test_scheduling_process.py")
    commands = [[sys.executable, str(script), "--jobs", str(count), "--shard", str(index),
                 *(["--binary", str(binary)] if binary is not None else [])]
                for index in range(count)]
    # Keep bounded settlement below the existing independent 60-second backstop.
    outcomes = run_parallel(commands, timeout=45, output_limit=128 * 1024,
                            grace=5, reap_allowance=1)
    render_outcomes([f"controls-{index}" for index in range(count)], outcomes)
    valid = len(outcomes) == len(shards) and all(
        completed(outcome, tests) for outcome, tests in zip(outcomes, shards))
    if not valid:
        print("Scheduling controls failed or returned incomplete shard evidence.", file=sys.stderr)
    return 0 if valid else 1
