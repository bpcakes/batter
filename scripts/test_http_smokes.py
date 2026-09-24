"""Controls for build-artifact selection and fail-fast HTTP smoke execution."""

from contextlib import redirect_stderr
import io
import json
import unittest
from unittest.mock import patch

import check_http_smokes as smokes
from parallel_process import ProcessOutcome


class HttpSmokeRunnerTests(unittest.TestCase):
    def outcomes(self):
        artifact = {"reason": "compiler-artifact", "target": {
            "name": "http_service", "kind": ["example"]},
            "executable": "/configured target/custom/http_service"}
        build = ProcessOutcome(0, json.dumps(artifact).encode(), b"", 0,
                               False, False, True, True, ())
        success = ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())
        return [build] + [success] * 5

    def test_uses_cargo_artifact_for_every_profile(self):
        with patch.object(smokes, "run_parallel", side_effect=[[x] for x in self.outcomes()]) as run, \
                patch.object(smokes, "render_outcomes"):
            self.assertEqual(smokes.main(), 0)
        for call, profile in zip(run.call_args_list[1:], smokes.PROFILES):
            command = call.args[0][0]
            self.assertEqual(command[2:], ["--binary", "/configured target/custom/http_service", *profile])
        self.assertEqual(run.call_count, 6)

    def test_build_or_any_smoke_failure_stops_later_commands(self):
        for index in range(6):
            with self.subTest(index=index):
                outcomes = self.outcomes()
                outcomes[index] = ProcessOutcome(9, b"", b"failure", 0,
                                                  False, False, True, True, ())
                with patch.object(smokes, "run_parallel", side_effect=[[x] for x in outcomes]) as run, \
                        patch.object(smokes, "render_outcomes"):
                    self.assertEqual(smokes.main(), 1)
                self.assertEqual(run.call_count, index + 1)

    def test_missing_artifact_cannot_smoke_a_stale_default_binary(self):
        build = ProcessOutcome(0, b"", b"", 0, False, False, True, True, ())
        with patch.object(smokes, "run_parallel", return_value=[build]) as run, \
                redirect_stderr(io.StringIO()), \
                self.assertRaisesRegex(RuntimeError, "expected one HTTP executable"):
            smokes.main()
        self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
