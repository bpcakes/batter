"""Exercise receipt-free file-budget enforcement in disposable repositories."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class JigIntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "JIG_"))}
        cls.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
                       GIT_TERMINAL_PROMPT="0")
        result = subprocess.run(
            ["scripts/install-jig.sh", "--repository-scope", "--profile", "runtime", "--resolve-only"],
            cwd=ROOT, env=cls.env, text=True, capture_output=True, timeout=30,
        )
        if result.returncode:
            raise RuntimeError("Run scripts/jig file-budget validate before these integration tests.")
        cls.runtime = Path(result.stdout.strip())

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="batter-jig-test-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        for name in [".jig.toml", ".agent/jig-contract.json",
                     "scripts/jig", "scripts/install-jig.sh", "scripts/check_file_budget.sh",
                     "scripts/verify.sh"]:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, path)
        (self.repo / ".jig").mkdir()
        (self.repo / ".jig/file-budget.toml").write_text(
            'version = 1\n[[rules]]\nid = "rust"\ncategory = "source"\n'
            'include = ["**/*.rs"]\nexclude = []\nnotice_lines = 5\nwarn_lines = 10\nmax_lines = 20\n'
        )
        (self.repo / "sample.rs").write_text("fn main() {}\n")
        self.git("init", "--template=", "-b", "master")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Fixture")
        self.commit("baseline")
        self.base = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("branch", "-m", "review-feature")
        self.git("update-ref", "refs/remotes/origin/master", self.base)
        # Missing push-before fetches stay local and cannot contact a network service.
        self.git("remote", "add", "origin", str(self.repo))

    def git(self, *args, check=True):
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env, check=check,
                              text=True, capture_output=True, timeout=30)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-m", message)

    def check_budget(self, event, **overrides):
        env = dict(self.env, JIG_DEV_BIN=str(self.runtime), JIG_EVENT_NAME=event,
                   JIG_PULL_REQUEST_BASE=self.base, JIG_PUSH_BEFORE=self.base,
                   JIG_MERGE_GROUP_BASE=self.base)
        env.update(overrides)
        result = subprocess.run(["bash", "scripts/check_file_budget.sh"], cwd=self.repo,
                                env=env, text=True, capture_output=True, timeout=30)
        self.assertFalse((self.repo / ".agent/state").exists())
        self.assertFalse((self.repo / ".agent/plans").exists())
        return result

    def test_event_comparisons_detect_budget_violations_without_local_master(self):
        self.assertNotEqual(self.git("rev-parse", "--verify", "master", check=False).returncode, 0)
        for event in ["local", "workflow_dispatch", "pull_request", "push", "merge_group"]:
            with self.subTest(event=event):
                (self.repo / "sample.rs").write_text("fn main() {}\n")
                result = self.check_budget(event)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                (self.repo / "sample.rs").write_text("// expanded source\n" * 25)
                result = self.check_budget(event)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("file_budget.debt_growth_lines", result.stdout + result.stderr)
                self.assertIn("sample.rs", result.stdout + result.stderr)

    def test_inherited_overage_cannot_grow(self):
        source = self.repo / "sample.rs"
        source.write_text("// inherited source\n" * 25)
        self.commit("inherited overage")
        self.git("update-ref", "refs/remotes/origin/master", "HEAD")
        result = self.check_budget("local")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        source.write_text("// inherited source\n" * 26)
        result = self.check_budget("local")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("file_budget.debt_growth_lines", result.stdout + result.stderr)

    def test_new_branch_push_accepts_zero_before(self):
        result = self.check_budget("push", JIG_PUSH_BEFORE="0" * 40)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_event_bases_do_not_fall_back_to_a_passing_comparison(self):
        for event, key in [("pull_request", "JIG_PULL_REQUEST_BASE"),
                           ("push", "JIG_PUSH_BEFORE"), ("merge_group", "JIG_MERGE_GROUP_BASE")]:
            with self.subTest(event=event):
                result = self.check_budget(event, **{key: "a" * 40})
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("git rev-parse exact object failed", result.stdout + result.stderr)

    def test_new_untracked_source_cannot_exceed_budget(self):
        (self.repo / "new.rs").write_text("// source\n" * 25)
        result = self.check_budget("local")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("new.rs", result.stdout + result.stderr)

    def test_import_ceiling_still_blocks_growth(self):
        policy = self.repo / ".jig/file-budget.toml"
        policy.write_text(policy.read_text().replace('exclude = []', 'exclude = ["imported.rs"]') +
                          '\n[[rules]]\nid="imported"\ncategory="source"\n'
                          'include=["imported.rs"]\nexclude=[]\n'
                          'notice_lines=5\nwarn_lines=10\nmax_lines=30\n')
        (self.repo / "imported.rs").write_text("// imported\n" * 30)
        self.commit("import ceiling")
        result = self.check_budget("local")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (self.repo / "imported.rs").write_text("// imported\n" * 31)
        result = self.check_budget("local")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("imported.rs", result.stdout + result.stderr)

    def test_workflow_commands_are_unavailable(self):
        for args in [("work", "start"), ("check",), ("mcp",)]:
            result = subprocess.run(["scripts/jig", *args], cwd=self.repo, env=self.env,
                                    text=True, capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertFalse((self.repo / ".agent/state").exists())


if __name__ == "__main__":
    unittest.main()
