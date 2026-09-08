#!/usr/bin/env python3
"""Challenge the real scheduling oracle in an isolated copy of this workspace."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

from scheduling_process import run_process
import stress_scheduling

BUILD_LIMIT = 180
BUILD_OUTPUT_LIMIT = 16 * 1024 * 1024


def build(subject, log):
    command = ["cargo", "test", "-p", "batter", "--test", "scheduling", "--no-run",
               "--locked", "--offline", "--message-format=json"]
    result = run_process(
        command, cwd=subject, timeout=BUILD_LIMIT, output_limit=BUILD_OUTPUT_LIMIT,
        env={**os.environ, "CARGO_TARGET_DIR": str(subject / "target")},
    )
    # Persist partial evidence before any oracle classification, on every result.
    log.write_text(result.output)
    log.with_suffix(".json").write_text(json.dumps({
        "command": command, "cwd": str(subject), **result.metadata(),
    }, indent=2))
    if not result.ok:
        raise RuntimeError(f"mutation build failed; this is not oracle evidence: {log}")
    try:
        executables = [record["executable"] for line in result.stdout.splitlines()
                       if (record := json.loads(line)).get("executable")
                       and record.get("target", {}).get("name") == "scheduling"]
    except (ValueError, KeyError, TypeError) as error:
        raise RuntimeError(f"invalid build evidence; inspect {log}") from error
    if len(executables) != 1:
        raise RuntimeError("expected exactly one scheduling test executable")
    return executables[0]


def check(subject, binary, output, variant, workers, iteration):
    passed, summary, text = stress_scheduling.run(
        binary, workers, 17, "capacity-after-drain", stress_scheduling.MAX_WATCHDOG, cwd=subject,
    )
    log = output / f"{variant}-{workers}-{iteration}.log"
    log.write_text(stress_scheduling.render(summary, text))
    record = {"variant": variant, "iteration": iteration,
              "command": [str(binary), *stress_scheduling.CHILD_ARGS],
              "cwd": str(subject), **summary}
    log.with_suffix(".json").write_text(json.dumps(record, indent=2))
    if variant == "original":
        valid = passed
    else:
        valid = (not passed and summary["status"] == 101
                 and "shared capacity violated: descendant must be Full" in text
                 and "PROFILE_OK " not in text)
    if (not valid or summary["watchdog"] or summary["overflow"] or summary["errors"]
            or not summary["reaped"] or not summary["output_eof"]):
        raise RuntimeError(f"unexpected {variant} oracle result; inspect {output}")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[1]
    version = run_process(["rustc", "-Vv"], timeout=10, output_limit=65536)
    evidence = {"toolchain": version.output,
                "toolchain_check": {"command": ["rustc", "-Vv"], **version.metadata()},
                "lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
                "cargo_config_sha256": hashlib.sha256((root / ".cargo/config.toml").read_bytes()).hexdigest(),
                "runs": []}
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2))
    if not version.ok:
        raise RuntimeError(f"toolchain inspection failed; this is not oracle evidence: {output}")
    with tempfile.TemporaryDirectory(prefix="batter-capacity-mutation-") as temporary:
        subject = Path(temporary)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            shutil.copy2(root / name, subject / name)
        (subject / ".cargo").mkdir()
        shutil.copy2(root / ".cargo/config.toml", subject / ".cargo/config.toml")
        for name in ("crates", "examples"):
            shutil.copytree(root / name, subject / name,
                            ignore=shutil.ignore_patterns("target", "__pycache__"))
        (subject / "scripts").mkdir()
        for name in ("stress_scheduling.py", "scheduling_process.py",
                     "test_scheduling_process.py", "check_scheduling_mutation.py"):
            shutil.copy2(root / "scripts" / name, subject / "scripts")
        source = subject / "crates/batter/src/lifecycle/process.rs"
        original = source.read_text()
        old = """let permit = self
            .permits
            .clone()
            .try_acquire_owned()"""
        replacement = """let permit = if ancestor.is_some() {
            Arc::new(Semaphore::new(1))
        } else {
            self.permits.clone()
        }.try_acquire_owned()"""
        if original.count(old) != 1:
            raise RuntimeError("capacity implementation changed; review mutation before adapting it")
        import difflib
        mutant = original.replace(old, replacement)
        (output / "mutation.patch").write_text("".join(difflib.unified_diff(
            original.splitlines(True), mutant.splitlines(True),
            fromfile="a/crates/batter/src/lifecycle/process.rs",
            tofile="b/crates/batter/src/lifecycle/process.rs")))
        for variant, content in (("original", original), ("mutant", mutant)):
            source.write_text(content)
            binary = build(subject, output / f"{variant}-build.log")
            for workers in (2, 4):
                for iteration in range(3):
                    evidence["runs"].append(check(subject, binary, output, variant, workers, iteration))
                    (output / "evidence.json").write_text(json.dumps(evidence, indent=2))
        # Production source was never edited; retain its hash alongside the patch.
        evidence["original_source_sha256"] = hashlib.sha256(original.encode()).hexdigest()
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2))
    print(f"Capacity oracle rejected all six mutant replays; six original replays passed: {output}")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
