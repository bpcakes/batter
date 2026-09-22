"""Source-copy and bounded command support for native workspace developer tools."""
from pathlib import Path
import shutil

from package import eligible
from parallel_process import render_outcomes, run_parallel


def copy_source(root, destination):
    root, destination = Path(root).resolve(), Path(destination).resolve()
    if destination.is_relative_to(root):
        raise ValueError("source copy must be outside the source checkout")
    destination.mkdir(parents=True)
    for path in root.rglob("*"):
        if eligible(path, root):
            target = destination / path.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
    return destination


def run(command, cwd, *, echo=False):
    outcome = run_parallel([command], cwd=cwd, timeout=1200,
                           output_limit=8 * 1024 * 1024, retain_tail=True)[0]
    if not outcome.ok or echo:
        render_outcomes(["native-source-tool"], [outcome])
    if not outcome.ok:
        raise RuntimeError("Native source tool command failed; see captured output")
    return outcome.stdout.decode()


def external_sources(metadata):
    return {(p["name"], p["version"], p["source"])
            for p in metadata["packages"] if p.get("source")}


RUNLEDGER_IDENTITIES = ("batter", "batter-core", "batter-sqlx", "batter-runledger",
                        "runledger-core", "runledger-postgres", "runledger-runtime")


def validate_consumer(metadata, source, consumer, known_sources, *,
                      required_identities=RUNLEDGER_IDENTITIES):
    source, consumer = Path(source).resolve(), Path(consumer).resolve()
    if external_sources(metadata) - known_sources:
        raise ValueError("standalone consumer changed locked external dependencies")
    for package in metadata["packages"]:
        if package.get("source") is None:
            manifest = Path(package["manifest_path"]).resolve()
            if manifest != consumer / "Cargo.toml" and not manifest.is_relative_to(source):
                raise ValueError("consumer resolved a local package outside the source copy")
    for name in required_identities:
        matches = [p for p in metadata["packages"] if p["name"] == name]
        if len(matches) != 1 or matches[0].get("source") is not None:
            raise ValueError("consumer requires one copied package identity: " + name)
