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
  `runlimit-postgres`, a source file outside it that the closure's Rust
  sources load by path, or a workspace-wide build input.

Whenever the planner cannot prove a skip, it selects the full work: if a job's
inputs are unknown, every job runs and no earlier verification is reused. The
workflow also runs every test job when this planner fails.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
import subprocess
import sys
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_PATH = ".github/workflows/ci.yml"
MARKER_PREFIX = "verified-tree-"
RUNLIMIT_PACKAGE = "runlimit-postgres"
# Files and directories outside package directories that can change any Rust
# job's outcome. Directory entries end with "/".
GLOBAL_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/",
                 WORKFLOW_PATH, "scripts/ci_plan.py")
DIFF_EVENTS = ("pull_request", "merge_group", "push")
RECORDING_EVENTS = ("pull_request", "merge_group")
NULL_SHA = "0" * 40
# A `#[path]` module or an `include!`, `include_str!` or `include_bytes!` of a
# string literal. Both resolve against the directory of the file that holds them.
PATH_REFERENCE = re.compile(
    r'#\[path\s*=\s*"([^"]+)"\s*\]|\binclude(?:_str|_bytes)?!\(\s*"([^"]+)"\s*\)')
# An include whose argument is not a plain string literal, such as `concat!`.
COMPUTED_INCLUDE = re.compile(r'\binclude(?:_str|_bytes)?!\(\s*[^"\s]')


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


def referenced_sources(root, directories):
    """Return repository-relative files outside `directories` that their Rust sources load.

    Every `.rs` file below `directories`, and every Rust file it loads in turn,
    is scanned for `#[path]` modules and literal `include!`-family macros, which
    resolve against the holding file's directory. A computed include, a
    reference that leaves the repository or one that names no existing file
    raises, which makes the caller select the full work.
    """
    root = Path(root).resolve()

    def inside(relative):
        return any(relative.startswith(directory) for directory in directories)

    pending = [source for directory in directories for source in (root / directory).rglob("*.rs")]
    seen, outside = set(), set()
    while pending:
        source = pending.pop()
        if source in seen:
            continue
        seen.add(source)
        text = source.read_text(encoding="utf-8")
        if COMPUTED_INCLUDE.search(text):
            raise ValueError(f"{source.relative_to(root)} includes a computed path")
        for match in PATH_REFERENCE.finditer(text):
            target = (source.parent / (match.group(1) or match.group(2))).resolve()
            relative = target.relative_to(root).as_posix()
            if not target.is_file():
                raise FileNotFoundError(f"{source.relative_to(root)} loads missing {relative}")
            if not inside(relative):
                outside.add(relative)
                if target.suffix == ".rs":
                    pending.append(target)
    return sorted(outside)


def package_inputs(metadata, packages):
    """Return the local closures of `packages` and the outside sources they load."""
    directories = set()
    for package in packages:
        directories.update(local_closure(metadata, package))
    directories = sorted(directories)
    return [*directories, *referenced_sources(metadata["workspace_root"], directories)]


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


def plan(event, paths, tree, find_run, closure):
    """Return the step outputs and the notes that explain them.

    `find_run` maps a tree to a verifying run URL and `closure` returns the
    Runlimit PostgreSQL inputs; either may raise. A closure failure returns the
    full-run outputs at once, so no skip or earlier verification rests on a
    partial plan.
    """
    outputs = {"tree": tree, "tree_verified": "false", "verified_run": "",
               "runlimit_postgres": "true"}
    notes = []
    if event not in DIFF_EVENTS:
        notes.append(f"{event} runs every job.")
        return outputs, notes
    if paths is None:
        notes.append("The changed files are unknown, so every job runs.")
    else:
        try:
            inputs = (*closure(), *GLOBAL_INPUTS)
        except Exception as error:  # noqa: BLE001 - any failure selects the full run
            notes.append(f"The Runlimit PostgreSQL inputs are unknown ({error!r}), so every job runs.")
            return outputs, notes
        if touches(paths, inputs):
            notes.append("The change touches Runlimit PostgreSQL inputs.")
        else:
            outputs["runlimit_postgres"] = "false"
            notes.append("The change leaves Runlimit PostgreSQL inputs untouched; it is skipped.")
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
                          lambda: package_inputs(cargo_metadata(), (RUNLIMIT_PACKAGE,)))
    write_outputs(outputs, notes, environ)
    return 0


if __name__ == "__main__":
    sys.exit(main())
