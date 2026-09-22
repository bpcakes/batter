#!/usr/bin/env python3
"""Check the actual local Runledger/foundation Cargo graph and bundled assets."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ("core", "postgres", "runtime", "test-support", "tui")
EXPECTED_PUBLISH = ["crates-io"]
EXPECTED_VERSIONS = {
    **{"runledger-" + suffix: "0.13.0" for suffix in NATIVE},
    "batter-core": "0.0.1",
    "batter-sqlx": "0.0.1",
}


def validate_graph(metadata, root):
    root = Path(root).resolve()
    packages = metadata["packages"]
    selected = {}
    for name in ["runledger-" + suffix for suffix in NATIVE] + ["batter-core", "batter-sqlx"]:
        matches = [package for package in packages if package["name"] == name]
        if len(matches) != 1:
            raise ValueError("expected exactly one package identity: " + name)
        package = matches[0]
        directory = "runledger" if name.startswith("runledger-") else "crates"
        expected = root / directory / name / "Cargo.toml"
        if package.get("source") is not None or Path(package["manifest_path"]).resolve() != expected:
            raise ValueError("package must resolve from this workspace: " + name)
        if package["id"] not in metadata["workspace_members"]:
            raise ValueError("package must be a workspace member: " + name)
        if package.get("publish") != EXPECTED_PUBLISH:
            raise ValueError("package must target crates.io publication: " + name)
        if package["version"] != EXPECTED_VERSIONS[name]:
            raise ValueError("package has an unexpected release version: " + name)
        selected[name] = package

    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    names = {package["id"]: package["name"] for package in packages}

    def normal_closure(start):
        visited, pending = set(), [start]
        while pending:
            current = pending.pop()
            if current in visited:
                continue
            visited.add(current)
            for dependency in nodes[current]["deps"]:
                if any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                    pending.append(dependency["pkg"])
        return {names[identifier] for identifier in visited}

    for suffix in NATIVE:
        name = "runledger-" + suffix
        if "batter" in normal_closure(selected[name]["id"]):
            raise ValueError("native Runledger must not depend on the Batter facade: " + name)
    foundation = normal_closure(selected["batter-core"]["id"])
    if any(name.startswith("runledger-") or name.startswith("batter-") and name != "batter-core"
           for name in foundation):
        raise ValueError("foundation must not depend on adapters or native Runledger")
    if "batter-sqlx" not in normal_closure(selected["runledger-postgres"]["id"]):
        raise ValueError("Runledger persistence must use the workspace SQLx foundation")


def files(directory, suffix):
    return {path.name: path.read_bytes() for path in directory.glob("*" + suffix)}


def validate_assets(root):
    native = Path(root) / "runledger"
    migrations = files(native / "migrations", ".sql")
    cache = files(native / ".sqlx", ".json")
    if not migrations or not cache:
        raise ValueError("canonical migrations and SQLx metadata must not be empty")
    for name in ("runledger-postgres", "runledger-test-support"):
        if files(native / name / "migrations", ".sql") != migrations:
            raise ValueError("bundled migrations differ: " + name)
    for name in ("runledger-postgres", "runledger-runtime"):
        if files(native / name / ".sqlx", ".json") != cache:
            raise ValueError("bundled SQLx metadata differs: " + name)


def validate_readme(root):
    native = Path(root) / "runledger"
    snippets = re.findall(r"<!-- quick-start-source: ([^\n]+) -->\n```rust\n(.*?)\n```",
                          (native / "README.md").read_text(), re.DOTALL)
    expected = {"runledger-runtime/examples/producer_worker/" + name + ".rs"
                for name in ("shared", "producer", "worker")}
    expected.add("runledger-runtime/examples/support/database.rs")
    if len(snippets) != len(expected) or {path for path, _ in snippets} != expected:
        raise ValueError("README must retain all four compiled quick-start snippets")
    for path, snippet in snippets:
        compiled = (native / path).read_text().split("\n#[cfg(test)]", 1)[0].rstrip()
        if snippet != compiled:
            raise ValueError("README snippet differs from its compiled example: " + path)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    validate_readme(ROOT)
    print("Runledger: publishable local packages, one foundation identity, acyclic ownership, matching assets and snippets")


if __name__ == "__main__":
    main()
