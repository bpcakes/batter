"""Exercise the verification entry point with observable command boundaries."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class VerifyTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="batter-verify-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        (self.repo / "scripts").mkdir()
        (self.repo / "bin").mkdir()
        shutil.copy2(ROOT / "scripts/verify.sh", self.repo / "scripts/verify.sh")
        (self.repo / "Cargo.lock").write_text("# retained fixture lock\n")
        stub = f'''#!{sys.executable}
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
if name == "python3" and sys.argv[1:2] == ["-c"]:
    os.execv({sys.executable!r}, [{sys.executable!r}, *sys.argv[1:]])
with open("calls.jsonl", "a") as output:
    output.write(json.dumps([name, *sys.argv[1:]]) + "\\n")
if os.environ.get("FAIL_COMMAND") and os.environ["FAIL_COMMAND"] in " ".join([name, *sys.argv[1:]]):
    sys.exit(9)
'''
        for name in ["cargo", "rustc", "rustfmt", "python3", "budget"]:
            path = self.repo / "bin" / name
            path.write_text(stub)
            path.chmod(0o755)
        (self.repo / "scripts/check_file_budget.sh").write_text("#!/bin/sh\nexec budget\n")
        self.env = dict(os.environ, PATH=str(self.repo / "bin") + os.pathsep + os.environ["PATH"])

    def run_verify(self, *args, **env):
        result = subprocess.run(["bash", "scripts/verify.sh", *args], cwd=self.repo,
                                env=dict(self.env, **env), capture_output=True, text=True, timeout=30)
        calls = self.repo / "calls.jsonl"
        return result, [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else []

    def test_complete_verification_runs_checks_without_work_state(self):
        result, calls = self.run_verify()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ["rustc", "--version", "--verbose"], ["cargo", "--version"],
            ["cargo", "fmt", "--all", "--", "--check"], ["budget"],
            ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked",
             "--", "-D", "warnings", "-D", "clippy::mod_module_files"],
            ["cargo", "clippy", "-p", "runlimit-core", "-p", "runlimit-memory", "-p",
             "runlimit-postgres", "-p", "runlimit-http", "-p", "runlimit-axum",
             "--all-targets", "--locked", "--", "-D", "warnings"],
            *[["python3", "scripts/test_matrix.py", part] for part in
              ["workspace", "no-default-features", "doctests", "consumers", "runlimit", "scripts"]],
            ["cargo", "doc", "--workspace", "--all-features", "--no-deps", "--locked"],
            ["python3", "scripts/check_http_smokes.py"],
        ])
        self.assertFalse((self.repo / ".agent").exists())
        self.assertEqual((self.repo / "Cargo.lock").read_text(), "# retained fixture lock\n")

    def test_check_failure_is_not_reported_as_success(self):
        for command in ["budget", "clippy", "test_matrix.py workspace", "test_matrix.py consumers",
                        "cargo doc", "check_http_smokes.py"]:
            with self.subTest(command=command):
                result, calls = self.run_verify(FAIL_COMMAND=command)
                self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
                self.assertIn(command, " ".join(calls[-1]))

    def test_missing_lock_and_removed_plan_option_fail_before_checks(self):
        result, calls = self.run_verify("--plan-id", "old-plan")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])
        (self.repo / "Cargo.lock").unlink()
        result, calls = self.run_verify()
        self.assertEqual(result.returncode, 2)
        self.assertFalse(any(call[:2] == ["cargo", "fmt"] for call in calls))


if __name__ == "__main__":
    unittest.main()
