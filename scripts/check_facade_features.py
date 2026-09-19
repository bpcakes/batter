#!/usr/bin/env python3
"""Prove Batter facade feature isolation and cross-package type identity."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile

from parallel_process import render_outcomes, run_parallel

ROOT = Path(__file__).resolve().parent.parent
EXPECTED_FEATURES = {
    "axum",
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
        ("axum",),
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
    axum = "axum" in chosen or "runlimit-axum" in chosen
    sqlx = bool(chosen & {"sqlx", "sqlx-test-support", "runledger", "runlimit-postgres"})
    runlimit = bool(chosen & {"runlimit", *RUNLIMIT_BRIDGES})
    return {
        "batter-core": True,
        "batter-axum": axum,
        "batter-sqlx": "sqlx" in chosen or "sqlx-test-support" in chosen,
        "batter-runledger": "runledger" in chosen,
        "batter-runlimit": runlimit,
        "batter-test-support": "test-support" in chosen or "sqlx-test-support" in chosen,
        "axum": axum,
        "sqlx": sqlx,
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
    if "axum" in chosen or "runlimit-axum" in chosen:
        lines.insert(1, "use batter::axum::{RequestPolicy, register_http_in};")
    if "sqlx" in chosen or "sqlx-test-support" in chosen:
        lines.insert(1, "use batter::sqlx::{PgLease, pool_in};")
    if "runledger" in chosen:
        lines.insert(1, "use batter::runledger::{NativeReport, register_in};")
    if "runlimit" in chosen or chosen & set(RUNLIMIT_BRIDGES):
        lines.insert(1, "use batter::runlimit::{ConsumptionError, EmptyChecks, Quota};")
    if "runlimit-axum" in chosen:
        lines.insert(1, "use batter::runlimit::http::{PreparedHttp, TestClient};")
    if "test-support" in chosen or "sqlx-test-support" in chosen:
        lines.insert(1, "use batter::test_support::{Script, finish};")
    if "sqlx-test-support" in chosen:
        lines.insert(1, "use batter::sqlx::test_support::{ConnectionPlan, FixtureBody, FixtureScope, FixtureSuite, MigrationBundle, MigrationInput, template_spec};")
    return "\n".join(lines) + "\n"


def manifest(name: str, dependencies: list[str]) -> str:
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
    for package, expected in expected_graph(selected).items():
        if (package in names) != expected:
            raise RuntimeError(
                f"{selected}: normal graph expected {package}={expected}, "
                f"observed {package in names}; names={sorted(names)}"
            )


def run_positive_case(cargo: list[str], host: str, root_lock: bytes,
                      known: set[tuple[str, str, str | None]], parent: Path,
                      selected: tuple[str, ...]) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"consumer-{label}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-feature-consumer", [facade_dependency(selected)]))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(facade_source(selected))
    metadata = run_metadata(cargo, host, case)
    check_graph(metadata, selected, known)
    execute(cargo + ["check", "--locked", "--offline", "--target-dir", str(case / "target")], case)
    if (ROOT / "Cargo.lock").read_bytes() != root_lock:
        raise RuntimeError("repository Cargo.lock changed during facade consumer checks")


def identity_dependencies(selected: tuple[str, ...]) -> list[str]:
    chosen = set(selected)
    deps = [
        facade_dependency(selected),
        "batter-core = { path = " + json.dumps(str(ROOT / "crates/batter-core")) + " }",
    ]
    if "axum" in chosen or "runlimit-axum" in chosen:
        deps.append("batter-axum = { path = " + json.dumps(str(ROOT / "crates/batter-axum")) + " }")
    if "sqlx" in chosen or "sqlx-test-support" in chosen:
        sqlx_features = ", features = [\"test-support\"]" if "sqlx-test-support" in chosen else ""
        deps.append("batter-sqlx = { path = " + json.dumps(str(ROOT / "crates/batter-sqlx")) + sqlx_features + " }")
    if "runledger" in chosen:
        deps.append("batter-runledger = { path = " + json.dumps(str(ROOT / "crates/batter-runledger")) + " }")
        deps.append("runledger-runtime = { git = \"https://github.com/bpcakes/runledger.git\", rev = \"d57ec6be61e9f00ccce373b19ca356cafe98f206\" }")
    if chosen & {"runlimit", *RUNLIMIT_BRIDGES}:
        native_features = [feature.removeprefix("runlimit-") for feature in RUNLIMIT_BRIDGES if feature in chosen]
        features = ", features = " + json.dumps(native_features) if native_features else ""
        deps.append("batter-runlimit = { path = " + json.dumps(str(ROOT / "crates/batter-runlimit")) + features + " }")
        if "runlimit-memory" in chosen:
            deps.append("runlimit-memory = { git = \"https://github.com/bpcakes/runlimit\", rev = \"0a9138fc72f210c2d2ab01d445734a92aaca6aee\" }")
            deps.append("runlimit-core = { git = \"https://github.com/bpcakes/runlimit\", rev = \"0a9138fc72f210c2d2ab01d445734a92aaca6aee\" }")
        if "runlimit-postgres" in chosen:
            deps.append("runlimit-postgres = { git = \"https://github.com/bpcakes/runlimit\", rev = \"0a9138fc72f210c2d2ab01d445734a92aaca6aee\" }")
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
    if "axum" in chosen or "runlimit-axum" in chosen:
        lines += [
            "use batter::axum::RequestPolicy;",
            "use batter_axum::RequestPolicy as DirectRequestPolicy;",
            "fn axum_identity(_: DirectRequestPolicy) {}",
            "const _: fn(RequestPolicy) = axum_identity;",
        ]
    if "sqlx" in chosen or "sqlx-test-support" in chosen:
        lines += [
            "use batter::sqlx::PgLease;",
            "use batter_sqlx::PgLease as DirectPgLease;",
            "fn sqlx_identity(_: DirectPgLease) {}",
            "const _: fn(PgLease) = sqlx_identity;",
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
            "const _: fn() = memory_bridge::<runlimit_memory::MemoryStoreError>;",
        ]
    if "runlimit-postgres" in chosen:
        lines += [
            "fn postgres_bridge<E: batter::runlimit::ConsumptionError>() {}",
            "const _: fn() = postgres_bridge::<runlimit_postgres::CheckError>;",
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


def run_identity_case(cargo: list[str], host: str, root_lock: bytes, parent: Path,
                      selected: tuple[str, ...]) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"identity-{label}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-identity-consumer", identity_dependencies(selected)))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(identity_source(selected))
    run_metadata(cargo, host, case)
    execute(cargo + ["check", "--locked", "--offline", "--target-dir", str(case / "target")], case)


def negative_cases() -> list[tuple[tuple[str, ...], str, str]]:
    return [
        ((), "axum", "batter::axum"),
        ((), "sqlx", "batter::sqlx"),
        ((), "runledger", "batter::runledger"),
        ((), "runlimit", "batter::runlimit"),
        ((), "test-support", "batter::test_support"),
        (("sqlx",), "sqlx-test-support", "batter::sqlx::test_support"),
        (("runlimit",), "runlimit-axum", "batter::runlimit::http"),
        (("axum", "runlimit"), "runlimit-axum", "batter::runlimit::http"),
    ]


def run_negative_case(cargo: list[str], host: str, root_lock: bytes, parent: Path,
                      selected: tuple[str, ...], disabled: str, symbol: str) -> None:
    label = "-".join(selected) or "default"
    case = parent / f"negative-{label}-{disabled}"
    (case / "src").mkdir(parents=True)
    (case / "Cargo.toml").write_text(manifest("facade-negative-consumer", [facade_dependency(selected)]))
    (case / "Cargo.lock").write_bytes(root_lock)
    (case / "src/main.rs").write_text(f"use {symbol};\nfn main() {{}}\n")
    metadata = run_metadata(cargo, host, case)
    if (disabled == "runlimit-axum"
            and "axum" in resolved_features(metadata, "batter-runlimit")):
        raise RuntimeError(
            f"{selected}: disabled {disabled} activated batter-runlimit/axum"
        )
    outcome = run_parallel(
        [cargo + ["check", "--locked", "--offline", "--target-dir", str(case / "target")]],
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
    declared_features(baseline)
    known = source_tuples(baseline)
    root_lock = (ROOT / "Cargo.lock").read_bytes()
    print(
        f"facade feature consumers: toolchain={toolchain} host={host} "
        f"cargo-lock-sha256={hashlib.sha256(root_lock).hexdigest()}",
        flush=True,
    )
    with tempfile.TemporaryDirectory(prefix="batter-facade-features-") as directory:
        parent = Path(directory)
        for selected in feature_cases():
            run_positive_case(cargo, host, root_lock, known, parent, selected)
            print(f"facade features {','.join(selected) or 'none'}: graph and compilation passed", flush=True)
        for selected, disabled, symbol in negative_cases():
            run_negative_case(cargo, host, root_lock, parent, selected, disabled, symbol)
            print(f"facade negative {','.join(selected) or 'none'}: {symbol} gated as expected", flush=True)
        selected = tuple(sorted(EXPECTED_FEATURES))
        run_identity_case(cargo, host, root_lock, parent, selected)
        print("facade identity all-features: compatibility passed", flush=True)
    if (ROOT / "Cargo.lock").read_bytes() != root_lock:
        raise RuntimeError("repository Cargo.lock changed during facade feature checks")
    print("facade feature isolation, negative gating, and identity checks passed", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
