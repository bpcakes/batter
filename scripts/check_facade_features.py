#!/usr/bin/env python3
"""Prove Batter facade feature isolation and cross-package type identity."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tempfile

from parallel_process import render_outcomes, run_parallel
from consumer_manifest import consumer_patches

ROOT = Path(__file__).resolve().parent.parent
EXPECTED_FEATURES = {
    "at-rest",
    "axum",
    "metrics",
    "sqlx",
    "runledger",
    "runlimit",
    "runlimit-memory",
    "runlimit-postgres",
    "runlimit-axum",
    "test-support",
    "sqlx-test-support",
}
RUNLIMIT_BRIDGES = ("runlimit-memory", "runlimit-postgres", "runlimit-axum")


def execute(command: list[str], cwd: Path, *, timeout: float = 600,
            output_limit: int = 8 * 1024 * 1024) -> str:
    outcome = run_parallel([command], timeout=timeout, output_limit=output_limit,
                           cwd=cwd, retain_tail=True)[0]
    if not outcome.ok:
        render_outcomes(["facade-consumer"], [outcome])
        raise RuntimeError(f"command failed: {command!r}")
    return outcome.stdout.decode(errors="replace")


def source_tuples(metadata: dict) -> set[tuple[str, str, str | None]]:
    return {
        (package["name"], package["version"], package.get("source"))
        for package in metadata["packages"]
        if package.get("source")
    }


def normal_names(metadata: dict) -> set[str]:
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    packages = {package["id"]: package for package in metadata["packages"]}
    pending = [metadata["resolve"]["root"]]
    visited: set[str] = set()
    while pending:
        identifier = pending.pop()
        if identifier in visited:
            continue
        visited.add(identifier)
        for dependency in nodes[identifier]["deps"]:
            if any(kind["kind"] is None for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return {packages[identifier]["name"] for identifier in visited}


def resolved_features(metadata: dict, package_name: str) -> set[str]:
    packages = {package["id"]: package for package in metadata["packages"]}
    matching = [
        node for node in metadata["resolve"]["nodes"]
        if packages[node["id"]]["name"] == package_name
    ]
    if len(matching) != 1:
        raise RuntimeError(
            f"expected exactly one resolved {package_name} package, observed {len(matching)}"
        )
    return set(matching[0]["features"])


def feature_cases() -> list[tuple[str, ...]]:
    """Cover each public feature and the few unions with distinct contracts.

    The adapter suites test their implementations. This runner only needs to
    prove facade selection, graph isolation, and the bridge combinations whose
    behavior differs from an individual feature. Testing every Cartesian
    product repeats those same checks without adding a different contract.
    """
    return [
        (),
        ("at-rest",),
        ("axum",),
        ("metrics",),
        ("sqlx",),
        ("runledger",),
        ("runlimit",),
        ("runlimit-memory",),
        ("runlimit-postgres",),
        ("runlimit-axum",),
        ("test-support",),
        ("sqlx-test-support",),
        ("axum", "sqlx"),
        ("axum", "runlimit"),
        ("sqlx", "runlimit-postgres"),
        tuple(sorted(EXPECTED_FEATURES)),
    ]


def expected_graph(selected: tuple[str, ...]) -> dict[str, bool]:
    chosen = set(selected)
    at_rest = "at-rest" in chosen
    axum = "axum" in chosen or "runlimit-axum" in chosen
    sqlx = bool(chosen & {"sqlx", "sqlx-test-support", "runledger", "runlimit-postgres"})
    batter_sqlx = sqlx
    runlimit = bool(chosen & {"runlimit", *RUNLIMIT_BRIDGES})
    return {
        "batter-core": True,
        "batter-at-rest": at_rest,
        "aes-gcm": at_rest,
        "aes": at_rest,
        "ghash": at_rest,
        "polyval": at_rest,
        "ctr": at_rest,
        "batter-axum": axum,
        "metrics": "metrics" in chosen,
        "batter-sqlx": batter_sqlx,
        "batter-runledger": "runledger" in chosen,
        "batter-runlimit": runlimit,
        "batter-test-support": "test-support" in chosen or "sqlx-test-support" in chosen,
        "axum": axum,
        "sqlx": sqlx,
        "runledger-postgres": "runledger" in chosen,
        "runledger-runtime": "runledger" in chosen,
        "runlimit-core": runlimit,
        "runlimit-memory": "runlimit-memory" in chosen,
        "runlimit-postgres": "runlimit-postgres" in chosen,
        "postgres-test-harness": "sqlx-test-support" in chosen,
    }


def facade_source(selected: tuple[str, ...]) -> str:
    chosen = set(selected)
    lines = [
        "#![allow(unused_imports)]",
        "use batter::operation::OperationContext;",
        "fn main() {}",
    ]
    if "metrics" in chosen:
        lines.insert(1, "use batter::telemetry::metrics::{MAX_SERIES, describe};")
    if "at-rest" in chosen:
        lines.insert(1, "use batter::at_rest::{BorrowedSealedPayload, Context, Keyring, MacKey};")
    if "axum" in chosen or "runlimit-axum" in chosen:
        lines.insert(1, "use batter::axum::{RequestPolicy, register_http_in};")
    if chosen & {"sqlx", "sqlx-test-support", "runledger"}:
        lines.insert(1, "use batter::sqlx::{PgLease, PgAtomicScope, PgReadOnlySnapshot, run_atomic, pool_in};")
    if "runledger" in chosen:
        lines.insert(1, "use batter::runledger::{NativeReport, register_in, PgIntentScope, PgQueueScope, run_atomic as run_runledger_atomic};")
    if "runlimit" in chosen or chosen & set(RUNLIMIT_BRIDGES):
        lines.insert(1, "use batter::runlimit::{ConsumptionError, EmptyChecks, Quota};")
    if "runlimit-axum" in chosen:
        lines.insert(1, "use batter::runlimit::http::{PreparedHttp, TestClient};")
    if "runlimit-postgres" in chosen:
        lines.insert(1, "use batter::runlimit::attempts::{AttemptRunner, Authentication};")
    if "test-support" in chosen or "sqlx-test-support" in chosen:
        lines.insert(1, "use batter::test_support::{Script, finish};")
    if "sqlx-test-support" in chosen:
        lines.insert(1, "use batter::sqlx::test_support::{ConnectionPlan, FixtureBody, FixtureScope, FixtureSuite, MigrationBundle, MigrationInput, template_spec};")
    return "\n".join(lines) + "\n"


def manifest(name: str, dependencies: list[str], selected: tuple[str, ...] = ()) -> str:
    # Runledger and its foundation resolve directly from the shared workspace.
    # Retain external Runlimit development patches only when selected.
    needed = set()
    if any(feature.startswith("runlimit") for feature in selected):
        needed.add("runlimit-core")
    needed.update("runlimit-" + backend for backend in ("memory", "postgres")
                  if "runlimit-" + backend in selected)
    return (
        "[package]\n"
        f"name = {json.dumps(name)}\n"
        "version = \"0.0.0\"\n"
        "edition = \"2024\"\n"
        "rust-version = \"1.94\"\n"
        "publish = false\n\n"
        "[workspace]\nresolver = \"3\"\n\n"
        "[dependencies]\n"
        + "\n".join(dependencies)
        + "\n"
        + consumer_patches(needed)
    )


def facade_dependency(selected: tuple[str, ...]) -> str:
    return (
        "batter = { path = " + json.dumps(str(ROOT / "crates/batter"))
        + ", default-features = false, features = " + json.dumps(list(selected)) + " }"
    )


def run_metadata(cargo: list[str], host: str, cwd: Path) -> dict:
    args = cargo + ["metadata", "--format-version", "1", "--filter-platform", host]
    return json.loads(execute(args + ["--offline"], cwd))


def check_graph(metadata: dict, selected: tuple[str, ...], known: set[tuple[str, str, str | None]]) -> None:
    drift = source_tuples(metadata) - known
    if drift:
        raise RuntimeError(f"{selected}: temporary consumer dependency drift: {sorted(drift)}")
    names = normal_names(metadata)
    if "batter-at-rest" in names and "test-support" in resolved_features(metadata, "batter-at-rest"):
        raise RuntimeError(f"{selected}: production graph enabled batter-at-rest/test-support")
    for package, expected in expected_graph(selected).items():
        if (package in names) != expected:
            raise RuntimeError(
                f"{selected}: normal graph expected {package}={expected}, "
                f"observed {package in names}; names={sorted(names)}"
            )


def run_positive_case(cargo: list[str], host: str, root_lock: bytes,
                      known: set[tuple[str, str, str | None]], parent: Path, target: Path,
                      selected: tuple[str, ...]) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"consumer-{label}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-feature-consumer", [facade_dependency(selected)], selected))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(facade_source(selected))
    metadata = run_metadata(cargo, host, case)
    check_graph(metadata, selected, known)
    execute(cargo + ["check", "--locked", "--offline", "--target-dir", str(target)], case)
    if (ROOT / "Cargo.lock").read_bytes() != root_lock:
        raise RuntimeError("repository Cargo.lock changed during facade consumer checks")


def identity_dependencies(selected: tuple[str, ...]) -> list[str]:
    chosen = set(selected)
    deps = [
        facade_dependency(selected),
        "batter-core = { path = " + json.dumps(str(ROOT / "crates/batter-core")) + " }",
    ]
    if "at-rest" in chosen:
        deps.append("batter-at-rest = { path = " + json.dumps(str(ROOT / "crates/batter-at-rest")) + " }")
    if "axum" in chosen or "runlimit-axum" in chosen:
        deps.append("batter-axum = { path = " + json.dumps(str(ROOT / "crates/batter-axum")) + " }")
    if chosen & {"sqlx", "sqlx-test-support", "runledger"}:
        sqlx_features = ", features = [\"test-support\"]" if "sqlx-test-support" in chosen else ""
        deps.append("batter-sqlx = { path = " + json.dumps(str(ROOT / "crates/batter-sqlx")) + sqlx_features + " }")
    if "runledger" in chosen:
        deps.append("batter-runledger = { path = " + json.dumps(str(ROOT / "crates/batter-runledger")) + " }")
        deps.append("runledger-runtime = { path = " + json.dumps(str(ROOT / "runledger/runledger-runtime")) + " }")
    if chosen & {"runlimit", *RUNLIMIT_BRIDGES}:
        native_features = [feature.removeprefix("runlimit-") for feature in RUNLIMIT_BRIDGES if feature in chosen]
        features = ", features = " + json.dumps(native_features) if native_features else ""
        deps.append("batter-runlimit = { path = " + json.dumps(str(ROOT / "crates/batter-runlimit")) + features + " }")
        if "runlimit-memory" in chosen:
            deps.append('runlimit-memory = { path = ' + json.dumps(str(ROOT / "runlimit/runlimit-memory")) + ' }')
            deps.append('runlimit-core = { path = ' + json.dumps(str(ROOT / "runlimit/runlimit-core")) + ' }')
        if "runlimit-postgres" in chosen:
            deps.append('runlimit-postgres = { path = ' + json.dumps(str(ROOT / "runlimit/runlimit-postgres")) + ' }')
    if "runlimit-axum" in chosen:
        deps.append("tokio = { version = \"1.53.1\", default-features = false, features = [\"net\"] }")
    if "test-support" in chosen or "sqlx-test-support" in chosen:
        deps.append("batter-test-support = { path = " + json.dumps(str(ROOT / "crates/batter-test-support")) + " }")
    return deps


def identity_source(selected: tuple[str, ...]) -> str:
    chosen = set(selected)
    lines = [
        "#![allow(dead_code, unused_imports)]",
        "use batter::operation::OperationContext;",
        "use batter_core::operation::OperationContext as CoreOperationContext;",
        "fn core_identity(_: CoreOperationContext) {}",
        "fn main() { let _: fn(OperationContext) = core_identity; }",
    ]
    if "at-rest" in chosen:
        lines += [
            "use batter::at_rest::Keyring;",
            "use batter_at_rest::Keyring as DirectKeyring;",
            "fn at_rest_identity(_: DirectKeyring) {}",
            "const _: fn(Keyring) = at_rest_identity;",
            "fn borrowed_identity(_: batter_at_rest::BorrowedSealedPayload<'_>) {}",
            "const _: for<'a> fn(batter::at_rest::BorrowedSealedPayload<'a>) = borrowed_identity;",
        ]
    if "axum" in chosen or "runlimit-axum" in chosen:
        lines += [
            "use batter::axum::RequestPolicy;",
            "use batter_axum::RequestPolicy as DirectRequestPolicy;",
            "fn axum_identity(_: DirectRequestPolicy) {}",
            "const _: fn(RequestPolicy) = axum_identity;",
        ]
    if chosen & {"sqlx", "sqlx-test-support", "runledger"}:
        lines += [
            "use batter::sqlx::PgLease;",
            "use batter_sqlx::PgLease as DirectPgLease;",
            "fn sqlx_identity(_: DirectPgLease) {}",
            "const _: fn(PgLease) = sqlx_identity;",
            "const _: fn(batter::sqlx::PgAtomicScope) = |_: batter_sqlx::PgAtomicScope| {};",
        ]
    if "sqlx-test-support" in chosen:
        lines += [
            "use batter::sqlx::test_support::ConnectionPlan;",
            "use batter_sqlx::test_support::ConnectionPlan as DirectConnectionPlan;",
            "fn fixture_identity(_: DirectConnectionPlan) {}",
            "const _: fn(ConnectionPlan) = fixture_identity;",
        ]
    if "runledger" in chosen:
        lines += [
            "use batter::runledger::NativeReport;",
            "use batter_runledger::NativeReport as DirectNativeReport;",
            "fn runledger_identity(_: DirectNativeReport) {}",
            "const _: fn(NativeReport) = runledger_identity;",
            "fn protected_registration(target: &mut batter::startup::ProtectedStartupScope, context: OperationContext, prepared: runledger_runtime::PreparedSupervisor) -> Result<(), batter::RegistrationError> { batter::runledger::register_in(target, \"worker\", context, prepared) }",
        ]
    if chosen & {"runlimit", *RUNLIMIT_BRIDGES}:
        lines += [
            "use batter::runlimit::EmptyChecks;",
            "use batter_runlimit::EmptyChecks as DirectEmptyChecks;",
            "fn runlimit_identity(_: DirectEmptyChecks) {}",
            "const _: fn(EmptyChecks) = runlimit_identity;",
        ]
    if "runlimit-memory" in chosen:
        lines += [
            "fn memory_bridge<E: batter::runlimit::ConsumptionError>() {}",
            "fn memory_quota(quota: &batter::runlimit::Quota<runlimit_memory::MemoryStore>, context: &OperationContext, checks: batter::runlimit::Checks<'_, runlimit_core::FixedWindowPolicy>) { let _future = quota.run(context, checks, |_| async { Ok::<(), std::convert::Infallible>(()) }); }",
            "const _: fn() = memory_bridge::<runlimit_memory::MemoryBatchError>;",
            "const _: fn() = memory_bridge::<runlimit_memory::GcraBatchError>;",
        ]
    if "runlimit-postgres" in chosen:
        lines += [
            "fn postgres_bridge<E: batter::runlimit::ConsumptionError>() {}",
            "const _: fn() = postgres_bridge::<runlimit_postgres::BatchCheckError>;",
            "const _: fn(batter::runlimit::attempts::AttemptRunner) = |_: batter_runlimit::attempts::AttemptRunner| {};",
        ]
    if "runlimit-axum" in chosen:
        lines += [
            "fn prepared_http<T: batter::registration::RegistrationTarget + ?Sized>(prepared: batter::runlimit::http::PreparedHttp, target: &mut T, listener: tokio::net::TcpListener) -> Result<(), batter::RegistrationError> { prepared.register_in(target, listener) }",
        ]
    if "test-support" in chosen or "sqlx-test-support" in chosen:
        lines += [
            "use batter::test_support::Script;",
            "use batter_test_support::Script as DirectScript;",
            "fn support_identity(_: DirectScript<(), ()>) {}",
            "const _: fn(Script<(), ()>) = support_identity;",
        ]
    return "\n".join(lines) + "\n"


def run_identity_case(cargo: list[str], host: str, root_lock: bytes, parent: Path, target: Path,
                      selected: tuple[str, ...]) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"identity-{label}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-identity-consumer", identity_dependencies(selected), selected))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(identity_source(selected))
    metadata = run_metadata(cargo, host, case)
    for name in ("batter-core", "batter-sqlx"):
        packages = [package for package in metadata["packages"] if package["name"] == name]
        if len(packages) != 1:
            raise RuntimeError(f"identity consumer resolved multiple {name} foundations")
    execute(cargo + ["check", "--locked", "--offline", "--target-dir", str(target)], case)


def run_checked_completion_case(cargo: list[str], host: str, root_lock: bytes,
                                known: set[tuple[str, str, str | None]], parent: Path, target: Path) -> None:
    """Execute the generic anyhow/BoxError lifecycle fixture as an external consumer."""
    case = parent / "checked-completion"
    (case / "src").mkdir(parents=True)
    (case / "tests").mkdir()
    dependencies = [
        facade_dependency(()),
        "batter-core = { path = " + json.dumps(str(ROOT / "crates/batter-core")) + " }",
        'anyhow = "1.0"',
        'tokio = { version = "1.53.1", features = ["macros", "rt-multi-thread", "time", "test-util", "sync"] }',
    ]
    (case / "Cargo.toml").write_text(manifest("facade-checked-consumer", dependencies))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text("fn main() {}\n")
    fixture = ROOT / "crates/batter/tests/checked_completion_consumer.rs"
    (case / "tests/checked_completion.rs").write_text(fixture.read_text())
    metadata = run_metadata(cargo, host, case)
    check_graph(metadata, (), known)
    test_output = execute(
        cargo + ["test", "--locked", "--offline", "--test", "checked_completion",
                 "--target-dir", str(target)], case, timeout=900,
    )
    result = re.search(
        r"(?m)^test result: ok\. (8) passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;",
        test_output,
    )
    if not result:
        raise RuntimeError("checked completion external consumer did not run all eight tests")

    negatives = [
        (
            "private-success",
            "use batter::lifecycle::{SharedShutdownReport, ShutdownSuccess};\n"
            "fn forge(report: SharedShutdownReport) -> ShutdownSuccess {\n"
            "    ShutdownSuccess { report }\n}\nfn main() {}\n",
            ("error[E0451]", "field `report`"),
        ),
        (
            "raw-discard",
            "#![deny(unused_must_use)]\n"
            "use batter::lifecycle::RunningSupervisor;\n"
            "async fn discard(running: &RunningSupervisor) -> Result<(), std::sync::Arc<tokio::task::JoinError>> {\n"
            "    running.wait_report().await?;\n    Ok(())\n}\nfn main() {}\n",
            ("unused_must_use", "SharedShutdownReport"),
        ),
    ]
    for label, source, expected in negatives:
        (case / "src/main.rs").write_text(source)
        outcome = run_parallel(
            [cargo + ["check", "--locked", "--offline", "--target-dir", str(target)]],
            timeout=600, output_limit=2 * 1024 * 1024, cwd=case, retain_tail=True,
        )[0]
        diagnostics = (outcome.stdout + outcome.stderr).decode(errors="replace")
        if (outcome.status != 101 or outcome.watchdog or outcome.overflow or outcome.errors
                or not outcome.reaped or not outcome.output_eof
                or any(token not in diagnostics for token in expected)):
            render_outcomes([label], [outcome])
            raise RuntimeError(f"checked completion {label} did not fail at the intended API")
    if (ROOT / "Cargo.lock").read_bytes() != root_lock:
        raise RuntimeError("repository Cargo.lock changed during checked completion consumer checks")
    print(f"facade checked completion: {result.group(1)} external runtime tests and two negative controls passed", flush=True)


def negative_cases() -> list[tuple[tuple[str, ...], str, str]]:
    return [
        ((), "at-rest", "batter::at_rest"),
        ((), "axum", "batter::axum"),
        ((), "sqlx", "batter::sqlx"),
        ((), "runledger", "batter::runledger"),
        ((), "runlimit", "batter::runlimit"),
        ((), "test-support", "batter::test_support"),
        (("sqlx",), "sqlx-test-support", "batter::sqlx::test_support"),
        (("runlimit",), "runlimit-axum", "batter::runlimit::http"),
        (("axum", "runlimit"), "runlimit-axum", "batter::runlimit::http"),
    ]


def run_negative_case(cargo: list[str], host: str, root_lock: bytes, parent: Path, target: Path,
                      selected: tuple[str, ...], disabled: str, symbol: str) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"negative-{label}-{disabled}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-negative-consumer", [facade_dependency(selected)], selected))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(f"use {symbol};\nfn main() {{}}\n")
    metadata = run_metadata(cargo, host, case)
    if (disabled == "runlimit-axum"
            and "axum" in resolved_features(metadata, "batter-runlimit")):
        raise RuntimeError(
            f"{selected}: disabled {disabled} activated batter-runlimit/axum"
        )
    outcome = run_parallel(
        [cargo + ["check", "--locked", "--offline", "--target-dir", str(target)]],
        timeout=600, output_limit=2 * 1024 * 1024, cwd=case, retain_tail=True,
    )[0]
    diagnostics = (outcome.stdout + outcome.stderr).decode(errors="replace")
    expected_token = symbol.replace("::", "::")
    if (outcome.status != 101 or outcome.watchdog or outcome.overflow or outcome.errors
            or not outcome.reaped or not outcome.output_eof
            or "error[E0432]" not in diagnostics
            or (expected_token not in diagnostics and symbol.rsplit("::", 1)[-1] not in diagnostics)):
        render_outcomes([f"negative-{symbol}"], [outcome])
        raise RuntimeError(f"{selected}: disabled {disabled} did not fail at the intended API")


def declared_features(metadata: dict) -> None:
    packages = [package for package in metadata["packages"] if package["name"] == "batter"]
    if len(packages) != 1:
        raise RuntimeError("expected exactly one batter facade package")
    features = packages[0]["features"]
    if features.get("default") != []:
        raise RuntimeError(f"batter default features must remain empty: {features.get('default')!r}")
    declared = set(features) - {"default"}
    if declared != EXPECTED_FEATURES:
        raise RuntimeError(
            f"facade feature expectation map differs from manifest: "
            f"unmapped={sorted(declared - EXPECTED_FEATURES)}, "
            f"removed={sorted(EXPECTED_FEATURES - declared)}"
        )


def internal_batter_crate_roots(metadata: dict) -> tuple[str, ...]:
    """Return Rust crate roots for facade implementation dependencies."""
    packages = [package for package in metadata["packages"] if package["name"] == "batter"]
    if len(packages) != 1:
        raise RuntimeError("expected exactly one batter facade package")
    roots = {
        (dependency.get("rename") or dependency["name"]).replace("-", "_")
        for dependency in packages[0]["dependencies"]
        if dependency["kind"] is None and dependency["name"].startswith("batter-")
    }
    if "batter_core" not in roots:
        raise RuntimeError(f"facade dependency discovery omitted batter-core: {sorted(roots)}")
    return tuple(sorted(roots))


def forbidden_example_roots(text: str, roots: tuple[str, ...]) -> set[str]:
    pattern = re.compile(r"\b(?:" + "|".join(map(re.escape, roots)) + r")\b")
    return set(pattern.findall(text))


def check_facade_import_detector(roots: tuple[str, ...]) -> None:
    """Prove path, alias and extern forms cannot bypass the recurrence guard."""
    if forbidden_example_roots("use batter::operation::OperationContext;", roots):
        raise RuntimeError("public facade path was classified as an internal dependency")
    for root in roots:
        for source in (
            f"use {root}::Thing;",
            f"use {root} as internal;",
            f"extern crate {root};",
        ):
            if forbidden_example_roots(source, roots) != {root}:
                raise RuntimeError(f"facade import detector missed {source!r}")


def check_facade_example_imports(roots: tuple[str, ...]) -> None:
    """Keep facade-owned examples on the public consumer path."""
    violations: list[str] = []
    for source in sorted((ROOT / "crates/batter/examples").rglob("*.rs")):
        text = source.read_text()
        for forbidden in sorted(forbidden_example_roots(text, roots)):
            violations.append(f"{source.relative_to(ROOT)}: {forbidden}")
    if violations:
        raise RuntimeError(
            "facade-owned examples bypass the public facade:\n" + "\n".join(violations)
        )


def consumer_target(metadata: dict, rustc_info: str) -> Path:
    # Cargo resolves target-directory environment/configuration relative to ROOT.
    # Keep artifacts across disposable consumers and separate compiler identities.
    compiler = hashlib.sha256(rustc_info.encode()).hexdigest()[:16]
    return Path(metadata["target_directory"]) / "facade-features" / compiler


def main() -> int:
    requested_toolchain = os.environ.get("RUSTUP_TOOLCHAIN")
    toolchain = requested_toolchain or execute(["rustup", "show", "active-toolchain"], ROOT).split()[0]
    cargo = ["cargo", "+" + toolchain]
    rustc_info = execute(["rustc", "+" + toolchain, "-vV"], ROOT)
    host = next(line.split(": ", 1)[1] for line in rustc_info.splitlines() if line.startswith("host: "))
    baseline = json.loads(execute(
        cargo + ["metadata", "--format-version", "1", "--filter-platform", host,
                 "--all-features", "--locked"], ROOT
    ))
    target = consumer_target(baseline, rustc_info)
    declared_features(baseline)
    internal_roots = internal_batter_crate_roots(baseline)
    check_facade_import_detector(internal_roots)
    check_facade_example_imports(internal_roots)
    known = source_tuples(baseline)
    root_lock = (ROOT / "Cargo.lock").read_bytes()
    print(
        f"facade feature consumers: toolchain={toolchain} host={host} "
        f"cargo-lock-sha256={hashlib.sha256(root_lock).hexdigest()} target={target}",
        flush=True,
    )
    with tempfile.TemporaryDirectory(prefix="batter-facade-features-") as directory:
        parent = Path(directory)
        for selected in feature_cases():
            run_positive_case(cargo, host, root_lock, known, parent, target, selected)
            print(f"facade features {','.join(selected) or 'none'}: graph and compilation passed", flush=True)
        for selected, disabled, symbol in negative_cases():
            run_negative_case(cargo, host, root_lock, parent, target, selected, disabled, symbol)
            print(f"facade negative {','.join(selected) or 'none'}: {symbol} gated as expected", flush=True)
        selected = tuple(sorted(EXPECTED_FEATURES))
        run_identity_case(cargo, host, root_lock, parent, target, selected)
        print("facade identity all-features: compatibility passed", flush=True)
        run_checked_completion_case(cargo, host, root_lock, known, parent, target)
    if (ROOT / "Cargo.lock").read_bytes() != root_lock:
        raise RuntimeError("repository Cargo.lock changed during facade feature checks")
    print("facade feature isolation, negative gating, and identity checks passed", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
