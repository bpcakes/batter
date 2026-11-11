#!/usr/bin/env python3
"""Select the Rust CI work that a workflow run needs.

`.github/workflows/ci.yml` runs this planner first and reads its step outputs:

- `tree`: the tree hash of the checked-out commit.
- `tree_verified`: `true` only for a push whose exact tree already passed every
  Rust test job in a same-repository pull-request or merge-group run of this
  workflow. Test jobs then stop once the default-branch dependency cache is an
  exact hit; a cache miss runs the full job, which also refreshes the cache.
- `verified_run`: the URL of that earlier run, when there is one.
- `runlimit_postgres`: `true` when the change can affect the native Runlimit
  PostgreSQL tests, meaning a file in the local package closure of
  `runlimit-postgres` or a workspace-wide build input.
- `flags_postgres`: `true` when the change can affect the flags PostgreSQL
  suites, meaning a file in the local package closure of `batter-flags-postgres`,
  `batter-flags` or `batter-flags-cli`, or a workspace-wide build input.

Whenever the planner cannot prove a skip, it selects the full work. The
workflow also runs every test job when this planner fails.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_PATH = ".github/workflows/ci.yml"
MARKER_PREFIX = "verified-tree-"
RUNLIMIT_PACKAGE = "runlimit-postgres"
FLAGS_PACKAGES = ("batter-flags-postgres", "batter-flags", "batter-flags-cli")
# Files and directories outside package directories that can change any Rust
# job's outcome. Directory entries end with "/".
GLOBAL_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/",
                 WORKFLOW_PATH, "scripts/ci_plan.py")
DIFF_EVENTS = ("pull_request", "merge_group", "push")
RECORDING_EVENTS = ("pull_request", "merge_group")
NULL_SHA = "0" * 40


def local_closure(metadata, package):
    """Return repository-relative directories of `package` and its local build inputs.

    The package contributes every dependency kind because its tests compile
    dev-dependencies. Other local packages contribute normal and build
    dependencies only. A path dependency outside the workspace metadata raises
    `KeyError`, which makes the caller select the full work.
    """
    root = Path(metadata["workspace_root"])
    by_name = {entry["name"]: entry for entry in metadata["packages"]}
    by_dir = {Path(entry["manifest_path"]).parent: entry for entry in metadata["packages"]}
    seen = set()
    pending = [(by_name[package], True)]
    while pending:
        current, is_root = pending.pop()
        directory = Path(current["manifest_path"]).parent
        if directory in seen:
            continue
        seen.add(directory)
        for dependency in current["dependencies"]:
            if dependency.get("path") and (is_root or dependency.get("kind") in (None, "build")):
                pending.append((by_dir[Path(dependency["path"])], False))
    return sorted(directory.relative_to(root).as_posix() + "/" for directory in seen)


def flags_closure(metadata):
    """Return the union of the flags PostgreSQL packages' local closures."""
    directories = set()
    for package in FLAGS_PACKAGES:
        directories.update(local_closure(metadata, package))
    return sorted(directories)


def touches(paths, inputs):
    """Return whether any changed path is one of `inputs` or lies below one."""
    return any(path.startswith(entry) if entry.endswith("/") else path == entry
               for path in paths for entry in inputs)


def changed_paths(base, cwd=ROOT):
    """Return the paths changed from `base` to HEAD, or None when unknown."""
    if not base or base == NULL_SHA:
        return None
    result = subprocess.run(["git", "diff", "--name-only", "--no-renames", base, "HEAD"],
                            cwd=cwd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        return None
    return [line for line in result.stdout.splitlines() if line]


def verifying_run(tree, get):
    """Return the URL of a trusted run that recorded `tree`, or None.

    Only markers from same-repository pull-request or merge-group runs of this
    workflow count; fork runs can upload artifacts under any name.
    """
    name = MARKER_PREFIX + tree
    listing = get("actions/artifacts?" + urllib.parse.urlencode({"name": name, "per_page": 100}))
    for artifact in listing.get("artifacts", []):
        run = artifact.get("workflow_run") or {}
        if artifact.get("name") != name or artifact.get("expired") or not run.get("id"):
            continue
        if run.get("head_repository_id") != run.get("repository_id"):
            continue
        details = get(f"actions/runs/{run['id']}")
        if details.get("path") == WORKFLOW_PATH and details.get("event") in RECORDING_EVENTS:
            return details.get("html_url") or f"run {run['id']}"
    return None


def plan(event, paths, tree, find_run, closure, flags_closure):
    """Return the step outputs and the notes that explain them.

    `find_run` maps a tree to a verifying run URL, `closure` returns the
    Runlimit PostgreSQL input directories and `flags_closure` the flags
    PostgreSQL ones; any of them may raise.
    """
    outputs = {"tree": tree, "tree_verified": "false", "verified_run": "",
               "runlimit_postgres": "true", "flags_postgres": "true"}
    notes = []
    if event not in DIFF_EVENTS:
        notes.append(f"{event} runs every job.")
        return outputs, notes
    if paths is None:
        notes.append("The changed files are unknown, so every job runs.")
    else:
        for key, label, inputs_of in (("runlimit_postgres", "Runlimit PostgreSQL", closure),
                                      ("flags_postgres", "flags PostgreSQL", flags_closure)):
            try:
                inputs = (*inputs_of(), *GLOBAL_INPUTS)
            except Exception as error:  # noqa: BLE001 - any failure selects the full run
                notes.append(f"The {label} inputs are unknown ({error!r}), so it runs.")
                continue
            if touches(paths, inputs):
                notes.append(f"The change touches {label} inputs.")
            else:
                outputs[key] = "false"
                notes.append(f"The change leaves {label} inputs untouched; it is skipped.")
    if event == "push":
        try:
            url = find_run(tree)
        except Exception as error:  # noqa: BLE001 - any failure selects the full run
            url = None
            notes.append(f"Looking up earlier verification failed ({error!r}).")
        if url:
            outputs.update(tree_verified="true", verified_run=url)
            notes.append(f"Tree {tree} already passed in {url}; jobs only confirm the cache.")
        else:
            notes.append(f"No trusted run recorded tree {tree}; every job runs in full.")
    return outputs, notes


def github_getter(api_url, repository, token):
    """Return a function that GETs a repository-relative REST path as JSON."""
    def get(path):
        request = urllib.request.Request(f"{api_url}/repos/{repository}/{path}", headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    return get


def cargo_metadata(cwd=ROOT):
    """Return workspace metadata without resolving registry dependencies."""
    result = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
                            cwd=cwd, capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def write_outputs(outputs, notes, environ):
    """Append outputs and notes where GitHub Actions reads them, else print them."""
    lines = "".join(f"{key}={value}\n" for key, value in outputs.items())
    summary = "".join(f"- {note}\n" for note in notes)
    if environ.get("GITHUB_OUTPUT"):
        with open(environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(lines)
    else:
        sys.stdout.write(lines)
    if environ.get("GITHUB_STEP_SUMMARY"):
        with open(environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write("### Rust CI scope\n" + summary)
    sys.stdout.write(summary)


def main(environ=os.environ):
    event = environ["GITHUB_EVENT_NAME"]
    tree = subprocess.run(["git", "rev-parse", "HEAD^{tree}"], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout.strip()
    paths = changed_paths(environ.get("PLAN_BASE", "")) if event in DIFF_EVENTS else None
    get = github_getter(environ.get("GITHUB_API_URL", "https://api.github.com"),
                        environ["GITHUB_REPOSITORY"], environ.get("GITHUB_TOKEN", ""))
    outputs, notes = plan(event, paths, tree, lambda value: verifying_run(value, get),
                          lambda: local_closure(cargo_metadata(), RUNLIMIT_PACKAGE),
                          lambda: flags_closure(cargo_metadata()))
    write_outputs(outputs, notes, environ)
    return 0


if __name__ == "__main__":
    sys.exit(main())
