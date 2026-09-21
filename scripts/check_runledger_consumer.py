#!/usr/bin/env python3
"""Build and run native/facade consumers from exported sources without Git or patches."""
import argparse
import json
import os
from pathlib import Path
import tempfile

from runledger_source import copy_source, external_sources, run, validate_consumer

ROOT = Path(__file__).resolve().parent.parent
ROUND_TRIP = "tests::shared_contract_transaction_and_worker_round_trip"
IDENTITY = """
fn report(value: &batter::runledger::NativeReport) -> &runledger_runtime::RuntimeShutdownReport { value.native() }
fn scope<'a>(value: batter::runledger::PgIntentScope<'a>) -> runledger_postgres::PgIntentScope<'a> { value }
fn error(value: batter::sqlx::PgScopeError<()>) -> runledger_postgres::PgScopeError<()> { value }
fn context(value: batter::operation::OperationContext) -> batter_core::operation::OperationContext { value }
fn main() {
    let _ = (report, scope, error, context);
    assert!(runledger_runtime::RuntimeShutdownBudget::new(
        std::time::Duration::from_secs(1), std::time::Duration::from_millis(100)).is_ok());
    println!("direct native and facade identities execute together");
}
"""


def require_round_trip(output):
    if (f"test {ROUND_TRIP} ... ok" not in output
            or "test result: ok. 1 passed; 0 failed; 0 ignored" not in output):
        raise RuntimeError("copied worker round-trip test did not execute successfully")


def selected_cargo(root):
    toolchain = os.environ.get("RUSTUP_TOOLCHAIN") or run(
        ["rustup", "show", "active-toolchain"], root).split()[0]
    return ["cargo", "+" + toolchain]


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    cargo = selected_cargo(ROOT)
    print("Native standalone consumer toolchain=" + cargo[1])
    locked = (ROOT / "Cargo.lock").read_bytes()
    metadata = json.loads(run([*cargo, "metadata", "--format-version", "1",
                               "--all-features", "--locked"], ROOT))
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "runledger-consumer"
    with tempfile.TemporaryDirectory(prefix="batter-native-consumer-") as directory:
        parent = Path(directory)
        source = copy_source(ROOT, parent / "source")
        if (source / ".git").exists():
            raise RuntimeError("source copy must not contain Git metadata")
        consumer = parent / "consumer"
        (consumer / "src").mkdir(parents=True)
        dependencies = []
        for name in ("batter", "batter-core", "runledger-core", "runledger-postgres", "runledger-runtime"):
            group = "runledger" if name.startswith("runledger-") else "crates"
            extra = ', default-features = false, features = ["runledger"]' if name == "batter" else ""
            dependencies.append(name + ' = { path = ' + json.dumps(str(source / group / name)) + extra + ' }')
        (consumer / "Cargo.toml").write_text(
            '[package]\nname = "native-source-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
            'rust-version = "1.94"\npublish = false\n[workspace]\nresolver = "3"\n'
            '[dependencies]\n' + '\n'.join(dependencies) + '\n')
        (consumer / "Cargo.lock").write_bytes(locked)
        (consumer / "src/main.rs").write_text(IDENTITY)
        # Cargo may prune the seeded temporary lock; it must retain every selected source/version.
        resolved = json.loads(run([*cargo, "metadata", "--format-version", "1", "--offline"], consumer))
        validate_consumer(resolved, source, consumer, external_sources(metadata))
        run(["env", "SQLX_OFFLINE=true", *cargo, "run", "--locked", "--offline",
             "--target-dir", str(target)], consumer, echo=True)
        # Capture application output so it cannot split libtest's result line.
        # A single selected test also exercises serial libtest formatting on every run.
        output = run(["env", "RUST_TEST_NOCAPTURE=0", *cargo, "test", "-p", "runledger-runtime", "--example", "worker",
                      "--locked", "--offline", "--target-dir", str(target), "--",
                      "--exact", ROUND_TRIP, "--test-threads=1"], source, echo=True)
        require_round_trip(output)
    if (ROOT / "Cargo.lock").read_bytes() != locked:
        raise RuntimeError("source lockfile changed during standalone consumer verification")
    print("Git-free source copy: native/facade identity and producer/worker round trip passed")


if __name__ == "__main__":
    main()
