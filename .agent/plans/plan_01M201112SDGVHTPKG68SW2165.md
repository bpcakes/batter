# Separate the foundation, Axum adapter, and SQLx example

This living ExecPlan follows `.agent/PLANS.md`. Maintain its progress, discoveries, decisions and outcomes during implementation.

## Purpose

Consumers can select the operational foundation and Axum adapter independently. SQLx belongs to the existing runnable PostgreSQL example instead of the foundation's dependencies. The user explicitly excluded importing postgres-test-harness; leave that sibling repository and downstream applications unchanged.

## Progress

- [x] 2026-09-08: Inspected source dependencies, guides, scripts and repository state; Jig doctor reports ready.
- [x] 2026-09-08: Moved core and SQLx example into a virtual workspace with three libraries and one unpublished example package.
- [x] 2026-09-08: Extracted Axum while preserving readiness, cancellation, error rendering and tracing destruction behavior; added four regressions and a doctest.
- [x] 2026-09-08: Updated guides, contracts, examples, tooling, archive coverage and CI paths.
- [x] 2026-09-08: Both Rust targets passed 106 core / 127 workspace tests and two doctests; three HTTP smoke modes and runnable examples passed. Recorded evidence in docs/validation.md.
- [x] 2026-09-08: Final frozen-worktree Jig profile and required final test command passed; delivery issue closed. Work-plan closure is the final bookkeeping command.

## Context and Orientation

The Git baseline is `5c77593c6700de9b2e8d3cbc1d2acf4bbbb0b71d`. Existing uncommitted documentation, Beads configuration and append-only Jig records belong to earlier work; preserve them. The old root package owns `src/`, `tests/` and five examples. `crates/batter-test-support` contains only generic scripted outcomes and test error combination. The `axum` feature exposes `batter::http`; `postgres-example` only compiles a SQLx composition. No downstream Batter consumers exist.

The target is a virtual root Cargo manifest (workspace without a root package), `crates/batter`, `crates/batter-axum`, unchanged `crates/batter-test-support`, and `examples/postgres-lifecycle`. Each package has explicit version, Rust minimum and publishing policy. Preserve Rust 1.94 and publishing disabled. Cargo must generate the new workspace lock entries while preserving existing resolved versions where possible.

## Milestones and Plan of Work

First move core source/tests into `crates/batter`, with operation/worker/process examples inside that crate. Move SQLx composition to `examples/postgres-lifecycle/src/main.rs`, naming its package `batter-example-postgres-lifecycle` and binary `postgres_lifecycle`. Signal and budget setup remains local to each example owner so packages are self-contained. Share dependency declarations with minimal features; each member requests its own features. Core normal dependencies exclude Axum, Serde, SQLx and the harness.

Next move HTTP source into `crates/batter-axum/src/lib.rs`, plus HTTP tests and the service example. Split HTTP portions of mixed telemetry/scoped-dispatch tests without weakening assertions. Expose `batter::telemetry::with_current_dispatch`, returning an opaque future backed by the existing private pin-projected wrapper, retaining the current subscriber during polling and full destruction. Document capture timing and borrowed/non-Send compatibility, with tests and rustdoc. Keep adapter validation semantically identical without exposing the general private validation module. Existing `RequestPolicy` continues combining shutdown readiness and budgets; decomposing that API is separate work.

Then update READMEs, ownership guides, architecture, ADRs, integrations, roadmap/status and source links. Keep historical validation commands as evidence and append new outcomes separately. Update verification, smoke commands, source archive tests, Jig crate roots and generated contract where required. Preserve semantic tests, particularly destruction-time span coverage. No speculative adapters or external harness import.

## Validation and Acceptance

From the root run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Check core compilation separately, then workspace tests/examples/doctests/Clippy/rustdoc with locked dependencies. Build `cargo build -p batter-axum --example http_service --locked`; run `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` normally, with `--signal SIGINT`, and with `--deadline`. Probes, sanitized errors, request IDs, telemetry and clean shutdown must remain successful. Run core `process_owned` and `operation_budget` examples. Compile the SQLx package; this alone does not prove live database behavior.

Run static package checks and relevant Python tooling tests. Inspect `cargo tree -p batter --edges normal` to exclude transport/database dependencies, and workspace metadata for exactly four intended packages. Run `scripts/jig work check`, inspect evidence/gates and finish after required checks pass. Finish backend verification with `scripts/jig check test`. Keep command logs in ignored `.agent/tmp` and append versions, lock hash, outcomes and limitations to `docs/validation.md`.

## Interfaces and Dependencies

Normal direction is `batter-axum -> batter`, with no reverse dependency. The SQLx example uses core plus native SQLx. Test support stays independent and core uses it as a dev-dependency. The Axum import becomes `batter_axum`; core remains `batter`. The harness stays external.

## Idempotence and Recovery

Check source/destination before repeating moves; never overwrite unrelated edits. Do not reset, commit, publish or edit sibling repositories. Repair failures without relaxing semantic assertions. No persisted state or PostgreSQL protocol changes occur. Re-run failed checks after targeted fixes and record failures and final outcomes.

## Surprises & Discoveries

HTTP uses private validation and destruction-aware tracing, requiring a narrow public tracing capability. The user excluded the harness before any harness mutation.

The exact lifecycle move needed stable Git index metadata for Jig to inherit its unchanged 48-line legacy debt. Intent-to-add is explicitly unsupported by this gate; the unchanged rename alone is staged. A check during parallel documentation edits correctly rejected worktree drift. No thresholds were relaxed. All external dependency versions and checksums remained identical after Cargo generated the new local package entries.

## Decision Log

2026-09-08: Preserve runtime semantics, combined HTTP policy and compiler minimums. Leave the harness external per the user's correction. Do not commit or publish.

## Outcomes & Retrospective

The four-package structure and public tracing seam are implemented. Both compiler matrices, all original tests plus four regressions, both doctests, three HTTP smoke modes, standalone SQLx compilation, core example execution, seven Python tooling tests and package guide checks passed. The final frozen-worktree profile and required final test command passed, and the delivery issue is closed. No harness or consumer changes, commit, or publication. Validation evidence is in docs/validation.md; local logs are under .agent/tmp/workspace-refactor.

Final revision, 2026-09-08: recorded completed verification and the Git rename requirement for unchanged file-budget debt. The refactor preserves core and HTTP runtime behavior and removes transport/database dependencies from the core package.
