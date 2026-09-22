#!/usr/bin/env python3
"""Compile isolated Runlimit consumers; reject optional-dependency leakage and lock drift."""

import itertools
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

from parallel_process import render_outcomes, run_parallel
from consumer_manifest import consumer_patches

ROOT = Path(__file__).resolve().parent.parent
FEATURE_PACKAGES = {"memory": ("runlimit-memory",),
                    "postgres": ("runlimit-postgres", "sqlx", "batter-sqlx"),
                    "axum": ("batter-axum", "axum")}


def execute(command, cwd):
    outcome = run_parallel([command], timeout=600, output_limit=8 * 1024 * 1024,
                           cwd=cwd, retain_tail=True)[0]
    if not outcome.ok:
        render_outcomes(["isolated-consumer"], [outcome])
        raise RuntimeError("consumer command failed")
    return outcome.stdout.decode()


def normal_names(metadata):
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    packages = {package["id"]: package for package in metadata["packages"]}
    pending = [metadata["resolve"]["root"]]
    visited = set()
    while pending:
        ident = pending.pop()
        if ident in visited:
            continue
        visited.add(ident)
        for dependency in nodes[ident]["deps"]:
            if any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return {packages[ident]["name"] for ident in visited}


def declared_features(metadata):
    packages = [package for package in metadata["packages"]
                if package["name"] == "batter-runlimit"]
    if len(packages) != 1:
        raise RuntimeError("expected exactly one batter-runlimit package in Cargo metadata")
    manifest_features = packages[0]["features"]
    if manifest_features.get("default") != []:
        raise RuntimeError("batter-runlimit default features must remain empty")
    declared = set(manifest_features) - {"default"}
    mapped = set(FEATURE_PACKAGES)
    if declared != mapped:
        raise RuntimeError(f"feature expectation map differs from manifest: "
                           f"unmapped={sorted(declared - mapped)}, "
                           f"removed={sorted(mapped - declared)}")
    return tuple(sorted(declared))


def main():
    toolchain = os.environ.get("RUSTUP_TOOLCHAIN") or execute(
        ["rustup", "show", "active-toolchain"], ROOT).split()[0]
    cargo = ["cargo", "+" + toolchain]
    host = next(line.split(": ", 1)[1] for line in execute(
        ["rustc", "+" + toolchain, "-vV"], ROOT).splitlines() if line.startswith("host: "))
    metadata_args = ["metadata", "--format-version", "1", "--filter-platform", host]
    baseline = json.loads(execute(cargo + metadata_args + ["--all-features", "--locked"], ROOT))
    features = declared_features(baseline)
    known = {(p["name"], p["version"], p["source"]) for p in baseline["packages"] if p["source"]}
    with tempfile.TemporaryDirectory(prefix="batter-runlimit-features-") as directory:
        fixture = Path(directory)
        (fixture / "src").mkdir()
        for size in range(len(features) + 1):
            for selected in itertools.combinations(features, size):
                manifest = ('[package]\nname="runlimit-feature-consumer"\nversion="0.0.0"\n'
                            'edition="2024"\npublish=false\n[workspace]\nresolver="3"\n'
                            '[dependencies]\nbatter-runlimit={path=' + json.dumps(str(ROOT / "crates/batter-runlimit")) +
                            ', default-features=false, features=' + json.dumps(selected) + '}\n'
                            + consumer_patches({"runlimit-core"} | {
                                "runlimit-" + feature for feature in selected
                                if feature in {"memory", "postgres"}
                            }))
                (fixture / "Cargo.toml").write_text(manifest)
                shutil.copyfile(ROOT / "Cargo.lock", fixture / "Cargo.lock")
                source = "use batter_runlimit::{Checks, Quota, RunResult};\n"
                if "axum" in selected:
                    source += "use batter_runlimit::http::{HttpQuota, PreparedHttp, TestClient};\n"
                (fixture / "src/main.rs").write_text(source + "fn main() {}\n")
                # The disposable root must reconcile its own package in the copied lock.
                metadata = json.loads(execute(cargo + metadata_args + ["--offline"], fixture))
                drift = {(p["name"], p["version"], p["source"]) for p in metadata["packages"] if p["source"]} - known
                if drift:
                    raise RuntimeError(f"fixture dependency drift: {drift}")
                names = normal_names(metadata)
                for feature, packages in FEATURE_PACKAGES.items():
                    for package in packages:
                        if (package in names) != (feature in selected):
                            raise RuntimeError(f"{selected}: unexpected graph membership for {package}")
                check = cargo + ["check", "--locked", "--offline", "--target-dir", str(ROOT / "target/runlimit-features")]
                execute(check, fixture)
                if "axum" not in selected:
                    (fixture / "src/main.rs").write_text("use batter_runlimit::http;\nfn main() {}\n")
                    outcome = run_parallel([check], timeout=600, output_limit=1024 * 1024, cwd=fixture)[0]
                    error = outcome.stderr.decode()
                    if (outcome.status != 101 or outcome.watchdog or outcome.overflow or outcome.errors
                            or not outcome.reaped or not outcome.output_eof
                            or "error[E0432]" not in error or "unresolved import `batter_runlimit::http`" not in error):
                        render_outcomes(["disabled-http-negative-control"], [outcome])
                        raise RuntimeError("expected specifically the disabled HTTP import failure")
                print(f"isolated features {','.join(selected) or 'none'}: graph and compilation passed", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
