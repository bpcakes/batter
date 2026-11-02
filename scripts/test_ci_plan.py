"""Controls for Rust CI job selection: skips need proof, and failures run everything."""
import contextlib
import io
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from ci_plan import (
    GLOBAL_INPUTS,
    MARKER_PREFIX,
    RUNLIMIT_PACKAGE,
    WORKFLOW_PATH,
    cargo_metadata,
    changed_paths,
    local_closure,
    plan,
    touches,
    verifying_run,
    write_outputs,
)

TREE = "a" * 40
CLOSURE = ["runlimit/runlimit-core/", "runlimit/runlimit-postgres/"]


def fail(*_args):
    raise RuntimeError("unavailable")


class ClosureTests(unittest.TestCase):
    def package(self, name, *dependencies):
        return dict(name=name, manifest_path=f"/workspace/{name}/Cargo.toml",
                    dependencies=[dict(name=dep, kind=kind, path=f"/workspace/{dep}")
                                  for dep, kind in dependencies])

    def test_root_dev_dependencies_count_but_transitive_dev_dependencies_do_not(self):
        metadata = dict(workspace_root="/workspace", packages=[
            self.package("root", ("normal", None), ("dev", "dev")),
            self.package("normal", ("normal-dev", "dev"), ("normal-build", "build")),
            self.package("dev"), self.package("normal-dev"), self.package("normal-build"),
            self.package("unrelated")])
        self.assertEqual(local_closure(metadata, "root"),
                         ["dev/", "normal-build/", "normal/", "root/"])

    def test_registry_dependencies_and_cycles_are_ignored(self):
        root = self.package("root", ("peer", None))
        root["dependencies"].append(dict(name="serde", kind=None))
        metadata = dict(workspace_root="/workspace",
                        packages=[root, self.package("peer", ("root", None))])
        self.assertEqual(local_closure(metadata, "root"), ["peer/", "root/"])

    def test_path_dependency_outside_the_workspace_raises(self):
        metadata = dict(workspace_root="/workspace",
                        packages=[self.package("root", ("outside", None))])
        with self.assertRaises(KeyError):
            local_closure(metadata, "root")

    @unittest.skipUnless(shutil.which("cargo"), "cargo is required for workspace metadata")
    def test_workspace_closure_contains_the_package_and_existing_directories(self):
        closure = local_closure(cargo_metadata(), RUNLIMIT_PACKAGE)
        root = Path(__file__).resolve().parents[1]
        self.assertIn("runlimit/runlimit-postgres/", closure)
        self.assertTrue(all((root / entry).is_dir() for entry in closure), closure)


class TouchTests(unittest.TestCase):
    def test_directories_match_descendants_and_files_match_exactly(self):
        inputs = (*CLOSURE, *GLOBAL_INPUTS)
        self.assertTrue(touches(["runlimit/runlimit-core/src/lib.rs"], inputs))
        self.assertTrue(touches(["Cargo.lock"], inputs))
        self.assertTrue(touches([".cargo/config.toml"], inputs))
        self.assertTrue(touches([WORKFLOW_PATH], inputs))
        self.assertFalse(touches(["Cargo.toml.orig", "runlimit/runlimit-core-extra/x.rs"], inputs))
        self.assertFalse(touches(["crates/batter/src/lib.rs", "runlimit/AGENTS.md"], inputs))
        self.assertFalse(touches([], inputs))


class ChangedPathTests(unittest.TestCase):
    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True,
                              text=True).stdout.strip()

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="batter-ci-plan-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.git("init", "-q")
        self.git("config", "user.email", "ci@example.invalid")
        self.git("config", "user.name", "CI plan control")
        (self.repo / "kept.txt").write_text("one\n")
        (self.repo / "moved.txt").write_text("moved\n")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "base")
        self.base = self.git("rev-parse", "HEAD")
        (self.repo / "kept.txt").write_text("two\n")
        self.git("mv", "moved.txt", "renamed.txt")
        self.git("commit", "-q", "-am", "change")

    def test_renames_report_both_sides(self):
        self.assertEqual(sorted(changed_paths(self.base, self.repo)),
                         ["kept.txt", "moved.txt", "renamed.txt"])

    def test_missing_null_and_unknown_bases_are_unknown(self):
        for base in ("", "0" * 40, "f" * 40):
            self.assertIsNone(changed_paths(base, self.repo), base)


class VerifyingRunTests(unittest.TestCase):
    def artifact(self, run_id, **overrides):
        return dict(dict(name=MARKER_PREFIX + TREE, expired=False,
                         workflow_run=dict(id=run_id, repository_id=1, head_repository_id=1)),
                    **overrides)

    def getter(self, artifacts, runs):
        self.calls = []

        def get(path):
            self.calls.append(path)
            if path.startswith("actions/artifacts?"):
                return dict(artifacts=artifacts)
            return runs[int(path.rsplit("/", 1)[1])]
        return get

    def run_record(self, run_id, path=WORKFLOW_PATH, event="pull_request"):
        return dict(path=path, event=event, html_url=f"https://example.invalid/runs/{run_id}")

    def test_same_repository_pull_request_and_merge_group_markers_verify(self):
        for event in ("pull_request", "merge_group"):
            get = self.getter([self.artifact(7)], {7: self.run_record(7, event=event)})
            self.assertEqual(verifying_run(TREE, get), "https://example.invalid/runs/7")
            self.assertIn(f"name={MARKER_PREFIX}{TREE}", self.calls[0])

    def test_untrusted_markers_are_rejected(self):
        artifacts = [
            self.artifact(1, workflow_run=dict(id=1, repository_id=1, head_repository_id=2)),
            self.artifact(2, expired=True),
            self.artifact(3, name=MARKER_PREFIX + "b" * 40),
            self.artifact(4), self.artifact(5),
            self.artifact(6, workflow_run=None),
        ]
        runs = {4: self.run_record(4, path=".github/workflows/repo-policy.yml"),
                5: self.run_record(5, event="push")}
        get = self.getter(artifacts, runs)
        self.assertIsNone(verifying_run(TREE, get))
        self.assertEqual([call for call in self.calls if call.startswith("actions/runs/")],
                         ["actions/runs/4", "actions/runs/5"])


class PlanTests(unittest.TestCase):
    def plan(self, event, paths, find_run=lambda _tree: None, closure=lambda: CLOSURE):
        return plan(event, paths, TREE, find_run, closure)

    def test_events_without_a_diff_run_everything_without_lookup(self):
        for event in ("schedule", "workflow_dispatch"):
            outputs, _ = self.plan(event, ["docs/testing.md"], find_run=fail, closure=fail)
            self.assertEqual(outputs, dict(tree=TREE, tree_verified="false", verified_run="",
                                           runlimit_postgres="true"))

    def test_runlimit_runs_only_for_its_inputs(self):
        cases = [(["docs/testing.md", "crates/batter/src/lib.rs"], "false"),
                 (["runlimit/runlimit-core/src/lib.rs"], "true"),
                 (["Cargo.lock"], "true"), (["scripts/ci_plan.py"], "true"),
                 (None, "true")]
        for event in ("pull_request", "merge_group", "push"):
            for paths, expected in cases:
                outputs, _ = self.plan(event, paths)
                self.assertEqual(outputs["runlimit_postgres"], expected, (event, paths))

    def test_unknown_runlimit_inputs_run_it(self):
        outputs, notes = self.plan("pull_request", ["docs/testing.md"], closure=fail)
        self.assertEqual(outputs["runlimit_postgres"], "true")
        self.assertIn("unavailable", " ".join(notes))

    def test_only_pushes_reuse_a_recorded_tree(self):
        found = lambda tree: f"https://example.invalid/{tree}"
        outputs, _ = self.plan("push", ["docs/testing.md"], find_run=found)
        self.assertEqual((outputs["tree_verified"], outputs["verified_run"]),
                         ("true", f"https://example.invalid/{TREE}"))
        for event in ("pull_request", "merge_group"):
            outputs, _ = self.plan(event, ["docs/testing.md"], find_run=found)
            self.assertEqual(outputs["tree_verified"], "false", event)

    def test_missing_or_failed_lookup_runs_everything(self):
        for find_run in (lambda _tree: None, fail):
            outputs, notes = self.plan("push", None, find_run=find_run)
            self.assertEqual((outputs["tree_verified"], outputs["runlimit_postgres"]),
                             ("false", "true"))
            self.assertIn("every job runs", " ".join(notes))


class OutputTests(unittest.TestCase):
    def test_outputs_and_summary_are_appended(self):
        with tempfile.TemporaryDirectory(prefix="batter-ci-plan-output-") as directory:
            output, summary = Path(directory, "output"), Path(directory, "summary")
            output.write_text("earlier=1\n")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                write_outputs(dict(tree=TREE, tree_verified="false"), ["A note."],
                              dict(GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary)))
            self.assertEqual(printed.getvalue(), "- A note.\n")
            self.assertEqual(output.read_text(),
                             f"earlier=1\ntree={TREE}\ntree_verified=false\n")
            self.assertEqual(summary.read_text(), "### Rust CI scope\n- A note.\n")


if __name__ == "__main__":
    unittest.main()
