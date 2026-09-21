#!/usr/bin/env python3
"""Check the actual local Runledger/foundation Cargo graph and bundled assets."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
NATIVE = ("core", "postgres", "runtime", "test-support", "tui")


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
        if package.get("publish") != []:
            raise ValueError("package must remain unpublished: " + name)
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


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"], cwd=ROOT))
    validate_graph(metadata, ROOT)
    validate_assets(ROOT)
    print("Runledger: five local packages, one foundation identity, acyclic ownership, matching assets")


if __name__ == "__main__":
    main()
