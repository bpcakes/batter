#!/usr/bin/env python3
"""Execute the single-dependency facade consumer from a Git-free source copy.

The acceptance consumer declares only `batter` from this workspace, plus ordinary
registry crates, and no `[patch]` section. A separate harness crate owns the
disposable PostgreSQL 18 database through Runledger's existing native test
support, so the consumer's own dependency claim stays intact.
"""
import argparse
import json
import os
from pathlib import Path
import tempfile

from check_runledger_consumer import selected_cargo
from runledger_source import copy_source, external_sources, run, validate_consumer

ROOT = Path(__file__).resolve().parent.parent
CONSUMER_SOURCE = "consumers/single_facade_consumer.rs"
HARNESS_SOURCE = "consumers/single_facade_harness.rs"
FACADE_FEATURES = ("runledger", "runlimit-postgres", "runlimit-native-http")
HARNESS_FEATURES = ("runledger-test-support", "test-support")
PACKAGE_LINTS = '[lints.rust]\nwarnings = "deny"\n'
# One source identity for the facade, every adapter it selects, and every native
# package the consumer reaches without declaring it.
IDENTITIES = ("batter", "batter-core", "batter-sqlx", "batter-runledger", "batter-runlimit",
              "runledger-core", "runledger-postgres", "runledger-runtime",
              "runlimit-core", "runlimit-postgres", "runlimit-http")
HARNESS_IDENTITIES = ("batter", "batter-runledger", "runledger-test-support", "batter-test-support")
# Ordinary registry crates. These are not native-workspace side pins.
ECOSYSTEM = ('tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "sync"] }',
             'sqlx = { version = "0.9.0", features = ["runtime-tokio", "postgres"] }',
             'serde_json = "1"')
MARKERS = ("facade consumer: runledger and runlimit migrations applied",
           "facade consumer: durable intent and atomic enqueue committed",
           "facade consumer: invocation exit cancelled each job's derived phases",
           "facade consumer: worker executed both durable jobs",
           "facade consumer: postgres quota admitted then denied",
           "facade consumer: native draft-11 response metadata encoded",
           "single batter dependency reached runledger and runlimit",
           "single-facade harness: disposable database created, consumed and dropped")


def workspace_names(metadata):
    members = set(metadata["workspace_members"])
    return {package["name"] for package in metadata["packages"] if package["id"] in members}


def consumer_manifest(source):
    facade = ('batter = { path = ' + json.dumps(str(Path(source) / "crates/batter"))
              + ', default-features = false, features = ' + json.dumps(list(FACADE_FEATURES)) + ' }')
    dependencies = "\n".join([facade, *ECOSYSTEM])
    return ('[package]\nname = "single-facade-consumer"\nversion = "0.0.0"\n'
            'edition = "2024"\nrust-version = "1.94"\npublish = false\n'
            '[workspace]\nresolver = "3"\n[dependencies]\n' + dependencies + '\n'
            '[dev-dependencies]\ntokio = { version = "1", features = ["test-util"] }\n'
            + PACKAGE_LINTS + '[[bin]]\nname = "single-facade-consumer"\npath = '
            + json.dumps(str(Path(source) / CONSUMER_SOURCE)) + '\n')


def harness_manifest(source):
    facade = ('batter = { path = ' + json.dumps(str(Path(source) / "crates/batter"))
              + ', default-features = false, features = ' + json.dumps(list(HARNESS_FEATURES)) + ' }')
    tokio = 'tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }'
    return ('[package]\nname = "single-facade-harness"\nversion = "0.0.0"\n'
            'edition = "2024"\nrust-version = "1.94"\npublish = false\n'
            '[workspace]\nresolver = "3"\n[dependencies]\n' + facade + '\n' + tokio + '\n'
            + PACKAGE_LINTS + '[[bin]]\nname = "single-facade-harness"\npath = '
            + json.dumps(str(Path(source) / HARNESS_SOURCE)) + '\n')


def validate_single_dependency(manifest_text, metadata, names, *, package="single-facade-consumer"):
    """The consumer may name exactly one package from this workspace, and no patch."""
    if "[patch" in manifest_text:
        raise ValueError("the single-dependency consumer must declare no [patch] section")
    roots = [entry for entry in metadata["packages"] if entry["name"] == package]
    if len(roots) != 1:
        raise ValueError("expected exactly one package named " + package)
    declared = {dependency["name"] for dependency in roots[0]["dependencies"]}
    workspace_declared = declared & names
    if workspace_declared != {"batter"}:
        raise ValueError("consumer must declare only batter from this workspace, not "
                         + ", ".join(sorted(workspace_declared) or ["nothing"]))
    for dependency in roots[0]["dependencies"]:
        if dependency["name"] == "batter" and dependency.get("uses_default_features", True):
            raise ValueError("the facade dependency must keep default features disabled")
    return sorted(declared - names)


def executable(output, name):
    for line in output.splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get("reason") == "compiler-artifact" and record.get("executable") \
                and record["target"]["name"] == name:
            return record["executable"]
    raise RuntimeError("cargo did not report an executable for " + name)


def require_markers(output):
    for marker in MARKERS:
        if output.splitlines().count(marker) != 1:
            raise RuntimeError("consumer did not execute its expected step exactly once: " + marker)


def build(cargo, directory, target, name):
    return executable(run(["env", "SQLX_OFFLINE=true", *cargo, "build", "--locked", "--offline",
                           "--message-format", "json", "--target-dir", str(target)], directory), name)


def check_format(cargo, directory):
    run([*cargo, "fmt", "--", "--check"], directory, echo=True)


def check_completion(cargo, directory, target, name="single-facade-consumer"):
    run(["env", "SQLX_OFFLINE=true", "RUST_TEST_NOCAPTURE=0", *cargo, "test", "--locked", "--offline",
         "--target-dir", str(target), "--bin", name], directory, echo=True)


def run_harness(cargo, directory, target, binary):
    return run(["env", "SQLX_OFFLINE=true", *cargo, "run", "--locked", "--offline",
                "--target-dir", str(target), "--", binary], directory, echo=True)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    cargo = selected_cargo(ROOT)
    print("Single-facade consumer toolchain=" + cargo[1], flush=True)
    locked = (ROOT / "Cargo.lock").read_bytes()
    metadata = json.loads(run([*cargo, "metadata", "--format-version", "1",
                               "--all-features", "--locked"], ROOT))
    names = workspace_names(metadata)
    known = external_sources(metadata)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "single-facade"
    with tempfile.TemporaryDirectory(prefix="batter-single-facade-") as directory:
        parent = Path(directory)
        source = copy_source(ROOT, parent / "source")
        if (source / ".git").exists():
            raise RuntimeError("source copy must not contain Git metadata")
        consumer, harness = parent / "consumer", parent / "harness"
        for case, text in ((consumer, consumer_manifest(source)), (harness, harness_manifest(source))):
            case.mkdir()
            (case / "Cargo.toml").write_text(text)
            (case / "Cargo.lock").write_bytes(locked)
            check_format(cargo, case)
        # Cargo may prune the seeded temporary lock; it must retain every selected source.
        resolved = json.loads(run([*cargo, "metadata", "--format-version", "1", "--offline"], consumer))
        validate_consumer(resolved, source, consumer, known, required_identities=IDENTITIES)
        registry = validate_single_dependency((consumer / "Cargo.toml").read_text(), resolved, names)
        print("Consumer declares one workspace package (batter) plus registry crates: "
              + ", ".join(registry), flush=True)
        resolved_harness = json.loads(
            run([*cargo, "metadata", "--format-version", "1", "--offline"], harness))
        validate_consumer(resolved_harness, source, harness, known,
                          required_identities=HARNESS_IDENTITIES)
        validate_single_dependency((harness / "Cargo.toml").read_text(), resolved_harness, names,
                                   package="single-facade-harness")
        binary = build(cargo, consumer, target, "single-facade-consumer")
        check_completion(cargo, consumer, target)
        check_completion(cargo, harness, target, "single-facade-harness")
        require_markers(run_harness(cargo, harness, target, binary))
    if (ROOT / "Cargo.lock").read_bytes() != locked:
        raise RuntimeError("source lockfile changed during single-facade verification")
    print("Git-free source copy: one batter dependency executed Runledger migrations, durable "
          "intents, a registered worker and a PostgreSQL quota")


if __name__ == "__main__":
    main()
