"""Exercise Jig/CI boundaries in disposable repositories; requires Jig doctor first."""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

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
            raise RuntimeError("Run scripts/jig doctor before these integration tests.")
        cls.runtime = Path(result.stdout.strip())

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="batter-jig-test-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        for name in [".jig.toml", ".agent/jig-contract.json", ".gitattributes", ".mcp.json",
                     "scripts/jig", "scripts/install-jig.sh", "scripts/check_file_budget.sh"]:
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
        return subprocess.run(["bash", "scripts/check_file_budget.sh"], cwd=self.repo,
                              env=env, text=True, capture_output=True, timeout=30)

    def latest_findings(self):
        lines = (self.repo / ".agent/state/receipts.jsonl").read_text().splitlines()
        return json.loads(lines[-1])["findings"]

    def test_event_comparisons_detect_budget_violations_without_local_master(self):
        self.assertNotEqual(self.git("rev-parse", "--verify", "master", check=False).returncode, 0)
        for event in ["workflow_dispatch", "pull_request", "push", "merge_group"]:
            with self.subTest(event=event):
                (self.repo / "sample.rs").write_text("fn main() {}\n")
                result = self.check_budget(event)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                (self.repo / "sample.rs").write_text("// expanded source\n" * 25)
                result = self.check_budget(event)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                errors = [finding for finding in self.latest_findings() if finding["severity"] == "error"]
                self.assertEqual(len(errors), 1, errors)
                self.assertEqual(errors[0]["code"], "file_budget.debt_growth_lines")
                self.assertEqual(errors[0]["location"]["path"], "sample.rs")

    def test_new_branch_push_accepts_zero_before(self):
        result = self.check_budget("push", JIG_PUSH_BEFORE="0" * 40)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_event_bases_do_not_fall_back_to_a_passing_comparison(self):
        for event, key in [("pull_request", "JIG_PULL_REQUEST_BASE"),
                           ("push", "JIG_PUSH_BEFORE"), ("merge_group", "JIG_MERGE_GROUP_BASE")]:
            with self.subTest(event=event):
                result = self.check_budget(event, **{key: "a" * 40})
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("file_budget.baseline_unavailable",
                              [finding["code"] for finding in self.latest_findings()])

    def jig(self, *args):
        result = subprocess.run([str(self.runtime), "--json", *args], cwd=self.repo,
                                env=self.env, text=True, capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def prepare_freshness(self):
        # Keep the actual repository target scopes/profile. Replace only expensive
        # command bodies with observable executions; native policy checks stay real.
        shutil.copy2(ROOT / ".gitignore", self.repo / ".gitignore")
        runner = self.repo / "scripts/fixture_check.py"
        runner.write_text(
            "from pathlib import Path\nimport sys\n"
            "with Path('.agent/fixture-executions').open('a') as output:\n"
            "    output.write(sys.argv[1] + '\\n')\n"
        )
        config = self.repo / ".jig.toml"
        text = config.read_text()
        for label in ["clippy", "fmt", "test", "test_locked"]:
            text = re.sub(rf'^api_{label}_command = .*$',
                          lambda _, label=label: f'api_{label}_command = '
                          + json.dumps(f"python3 scripts/fixture_check.py {label}"),
                          text, flags=re.MULTILINE)
        config.write_text(text)
        for name, content in {
            "crates/example/src/lib.rs": "pub fn example() {}\n",
            "crates/example/tests/fixture.txt": "example fixture\n",
            "examples/example/migrations/001.sql": "SELECT 1;\n",
            "test-support/temp_dir.rs": "// shared test fixture\n",
            ".beads/issues.jsonl": '{"status":"open"}\n',
            "README.md": "Example documentation\n",
            "scripts/example_helper.py": "# example helper\n",
        }.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        # Ignored build/cache contents must not make the exhaustive scopes unknown.
        for name in ["target/debug/generated.rs", "scripts/__pycache__/example.pyc"]:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("ignored build output\n")
        self.commit("freshness fixture")
        result = subprocess.run(
            [str(self.runtime), "work", "start", "--title", "Example freshness",
             "--body", "Exercise actual target input scopes.", "--print-plan-id"],
            cwd=self.repo, env=self.env, text=True, capture_output=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout.strip()

    def freshness_check(self, plan):
        result = self.jig("work", "check", "--plan-id", plan)
        self.assertTrue(result["ok"], result)
        return {entry["target"]["action"]: entry for entry in result["target_evidence"]}

    def execution_count(self, label):
        return (self.repo / ".agent/fixture-executions").read_text().splitlines().count(label)

    def test_tracker_closeout_reuses_rust_passes_across_git_states(self):
        plan = self.prepare_freshness()
        original = self.freshness_check(plan)
        for state in ["dirty", "staged", "committed"]:
            with self.subTest(state=state):
                if state == "dirty":
                    (self.repo / ".beads/issues.jsonl").write_text('{"status":"closed"}\n')
                    (self.repo / "README.md").write_text("Updated example documentation\n")
                elif state == "staged":
                    self.git("add", ".beads", "README.md")
                else:
                    self.commit("tracker closeout")
                current = self.freshness_check(plan)
                for action in ["clippy", "fmt", "test"]:
                    self.assertEqual(self.execution_count(action), 1)
                    self.assertEqual(current[action]["receipt_id"], original[action]["receipt_id"])
                    self.assertEqual(current[action]["disposition"], "reused")
                self.assertNotEqual(current["file-budget"]["receipt_id"],
                                    original["file-budget"]["receipt_id"])

    def test_relevant_source_and_helpers_invalidate_test_pass(self):
        plan = self.prepare_freshness()
        previous = self.freshness_check(plan)
        for count, name in enumerate([
            "crates/example/src/lib.rs", "crates/example/tests/fixture.txt",
            "examples/example/migrations/001.sql", "test-support/temp_dir.rs",
            "scripts/example_helper.py", "scripts/new_helper.py",
        ], start=2):
            with self.subTest(path=name):
                path = self.repo / name
                path.write_text((path.read_text() if path.exists() else "") + "\n")
                current = self.freshness_check(plan)
                self.assertEqual(self.execution_count("test"), count)
                self.assertNotEqual(current["test"]["receipt_id"], previous["test"]["receipt_id"])
                previous = current

    def test_native_policy_refresh_does_not_execute_rust_targets(self):
        plan = self.prepare_freshness()
        original = self.freshness_check(plan)
        (self.repo / ".beads/issues.jsonl").write_text('{"status":"closed"}\n')
        self.jig("check", "repo:file-budget", "--plan-id", plan)
        current = self.freshness_check(plan)
        for action in ["clippy", "fmt", "test"]:
            self.assertEqual(self.execution_count(action), 1)
            self.assertEqual(current[action]["receipt_id"], original[action]["receipt_id"])

    def test_restored_runtime_cache_runs_without_cargo(self):
        cache = self.runtime.parent.parent
        contract = json.loads((self.repo / ".agent/jig-contract.json").read_text())
        cache_name = f'contract-{contract["contract_version"]}-runtime'
        shim_dir = self.repo / "shims"
        shim_dir.mkdir()
        cargo = shim_dir / "cargo"
        cargo.write_text('#!/bin/sh\ntouch cargo-was-invoked\nexit 99\n')
        cargo.chmod(0o755)
        env = dict(self.env, PATH=str(shim_dir) + os.pathsep + self.env["PATH"],
                   JIG_EVENT_NAME="workflow_dispatch")
        cold = subprocess.run(["bash", "scripts/check_file_budget.sh"], cwd=self.repo,
                              env=env, text=True, capture_output=True, timeout=30)
        self.assertNotEqual(cold.returncode, 0, cold.stdout + cold.stderr)
        self.assertTrue((self.repo / "cargo-was-invoked").exists())
        (self.repo / "cargo-was-invoked").unlink()
        # A full local runtime is also compatible with the CI runtime profile.
        shutil.copytree(cache, self.repo / ".git/jig-tools" / cache_name, dirs_exist_ok=True)
        result = subprocess.run(["bash", "scripts/check_file_budget.sh"], cwd=self.repo,
                                env=env, text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.repo / "cargo-was-invoked").exists())

    def merge_edits(self, relative, initial, left, right):
        path = self.repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(initial)
        self.commit("shared content")
        self.git("branch", "other-edit")
        path.write_text(left)
        self.commit("first edit")
        self.git("checkout", "other-edit")
        path.write_text(right)
        self.commit("second edit")
        result = self.git("merge", "--no-commit", "review-feature", check=False)
        return result, path.read_text()

    def test_conflicting_plan_edits_require_resolution(self):
        attributes = self.repo / ".gitattributes"
        attributes.write_text(attributes.read_text().replace(
            "# BEGIN JIG MANAGED BLOCK",
            "# BEGIN JIG MANAGED BLOCK\n.agent/plans/*.md merge=union",
        ))
        result, text = self.merge_edits(".agent/plans/example.md", "Timeout: 5\n",
                                       "Timeout: 10\n", "Timeout: 20\n")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("<<<<<<<", text)
        self.assertIn("=======", text)

    def test_append_only_records_still_merge_with_union(self):
        initial = '{"event": "start"}\n'
        result, text = self.merge_edits(".agent/state/events.jsonl", initial,
                                       initial + '{"event": "left"}\n',
                                       initial + '{"event": "right"}\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertCountEqual([json.loads(line)["event"] for line in text.splitlines()],
                              ["start", "left", "right"])


class ArchiveTests(unittest.TestCase):
    def test_archive_keeps_jig_sources_and_excludes_local_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="batter-package-test-") as directory:
            repo = Path(directory) / "repo"
            (repo / "scripts").mkdir(parents=True)
            shutil.copy2(ROOT / "scripts/package.py", repo / "scripts/package.py")
            included = ["Cargo.toml", "Cargo.lock", "scripts/jig", "scripts/install-jig.sh", ".mcp.json", ".jig.toml",
                        ".gitattributes", ".agent/jig-contract.json", ".agent/PLANS.md",
                        ".agent/state/receipts.jsonl", ".agent/plans/.gitkeep",
                        "crates/batter/Cargo.toml", "crates/batter/src/lib.rs",
                        "crates/batter/examples/worker.rs", "crates/batter/tests/lifecycle.rs",
                        "test-support/process/watchdog.rs",
                        "crates/batter-axum/Cargo.toml", "crates/batter-axum/src/lib.rs",
                        "crates/batter-axum/examples/http_service.rs",
                        "crates/batter-test-support/Cargo.toml", "crates/batter-test-support/src/lib.rs",
                        "examples/postgres-lifecycle/Cargo.toml", "examples/postgres-lifecycle/src/main.rs"]
            excluded = [".agent/.cache/runtime.json", ".agent/.cache/adopt/backup.md",
                        ".agent/runtime/session.json", ".agent/tmp/note.md",
                        ".agent/state/adopt-last.json", ".agent/plans/example.md.lock",
                        ".env", "target/cache.rs", "validation/local/check.json",
                        "crates/batter/target/cache.rs", "examples/postgres-lifecycle/target/cache.rs",
                        "examples/postgres-lifecycle/.env"]
            for name in included + excluded:
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture sentinel\n")
            output = Path(directory) / "source.zip"
            subprocess.run([sys.executable, str(repo / "scripts/package.py"), "--output", str(output)],
                           check=True, capture_output=True, text=True, timeout=30)
            with zipfile.ZipFile(output) as archive:
                names = set(archive.namelist())
                inventory = archive.read("batter/SHA256SUMS").decode()
                for name in included:
                    self.assertIn("batter/" + name, names)
                    self.assertIn("  " + name + "\n", inventory)
                for name in excluded:
                    self.assertNotIn("batter/" + name, names)
                    self.assertNotIn("  " + name + "\n", inventory)
                self.assertTrue(archive.getinfo("batter/scripts/jig").external_attr >> 16 & 0o111)


if __name__ == "__main__":
    unittest.main()
