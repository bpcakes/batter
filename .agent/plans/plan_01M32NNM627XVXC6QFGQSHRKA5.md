# Verify standalone native consumers and shared-workspace maintenance

Owning Bead: batter-biqv.2. PINNED_BASE: f0dbc3fe5161dde53e70a4b02e18d3b149dbd729. Depends on the reviewed batter-biqv.1 import.

## Purpose
Prove that a source copy without Git metadata can build and run direct native Runledger alongside the Batter facade, and restore native SQLx metadata maintenance in the shared workspace.

## Progress
Complete. Both full Rust 1.98.1 and 1.94.0 verification matrices and all ten HTTP profiles passed. Standalone source consumers and the serial captured worker test passed on both toolchains. Eleven tooling controls pass with the optional live PostgreSQL 18.6 database-ahead regression enabled; a real SQLx refresh and offline compilation also pass. Native review round three found no issues in the full fixed range f0dbc3fe5161dde53e70a4b02e18d3b149dbd729..0d6817c76b9c73d05ea98feb60340e2f1f5b9754. Final committed-tree Jig api:test receipt_01M32TA70HQP73V8P27K0K82PB is fresh and passing.

## Surprises & Discoveries
Overlapping toolchain matrix reruns hit the native six-second retention-fence assertion (7.85 seconds). The unchanged assertion passed in isolation; both sequential matrix reruns passed. No timing limit was relaxed.

The import review repaired the two existing mutation copy inventories and the ZIP source allowlist. Actual extracted archives already compile all workspace targets on both Rust versions. This task adds a durable independent consumer/worker proof and maintained developer commands, not registry publication.

## Decision Log
Use the source archive eligibility policy for disposable source copies, preserving SQL and query caches while excluding Git, environments and build outputs. Standalone manifests use only paths into the copy, with no patch table. Seed temporary locks from Cargo.lock and reject external dependency drift. Reuse the native producer/worker example test for executed persistence and shutdown behavior. Regenerate native query metadata in a disposable workspace; require SQLx CLI 0.9.0, PostgreSQL 18 and a fully applied canonical migration set before preparation. Only synchronize original caches/migration copies after successful preparation and offline compilation. Never apply migrations to the configured database from the refresh command.

## Outcomes & Retrospective
Delivered Git-free native/facade consumer execution, source snippet validation and native SQLx maintenance. Review identified three tooling gaps: SQLx info omits extra database migrations, standalone execution loses directory-based toolchain selection, and uncaptured serial output interrupts libtest result lines. All were fixed at their source and validated; none recurred, and cumulative round three was clean. No native runtime API or persisted schema redesign was needed. Local evidence is macOS arm64 with PostgreSQL 18.6; hosted CI is not claimed.

## Context and orientation
scripts/package.py defines exported source eligibility. scripts/parallel_process.py owns bounded child execution. scripts/check_runledger_workspace.py enforces local identities and native assets; scripts/test_matrix.py owns required Python/Rust verification. Native queries and caches live under runledger/. The native README retains four compiled quick-start snippets and points to this task for adapted refresh tooling.

## Plan of work
Add reusable source-copy/command helpers, an external consumer identity executable, and execution of the native worker round-trip test from the copied workspace. Add a refresh command using a scratch copy, explicit PostgreSQL/migration preconditions, and failure-preserving source synchronization. Restore quick-start snippet checking. Add independent negative controls for leaked local paths, omitted artifacts, dependency drift, migration-state rejection and failed preparation. Integrate them into ordinary verification and update both Jig input inventories for any newly read source. Update current native developer guidance, status and Bead evidence.

## Concrete steps
Select and claim the ready task, commit ceremony separately, record fixed full HEAD, then start this Jig plan. Implement scripts/tests/docs. Execute consumer checks on Rust 1.98.1 and 1.94.0, exercise refresh against an owned disposable PostgreSQL 18 database using a disposable source copy, run required relevant verification and HTTP smokes, and inspect fresh Jig receipts. Commit before native codex review --base PINNED_BASE. Fix findings in new commits and always review the full task range. No changes after clean review except task closeout metadata.

## Validation and acceptance
No Git metadata, sibling checkout or consumer patch is needed. Direct native and facade types compile as one identity; the producer/worker transaction and shutdown test executes from the source copy. Invalid local source identities and dependency drift are rejected. Migration-state checks reject pending, unknown, empty and checksum-drift output. Preparation failure leaves the original source assets intact. Actual prepared caches compile offline in the copied workspace. Existing native migrations remain byte-identical. Record local platform/toolchain evidence only; no hosted or registry claims without execution.

## Idempotence and recovery
Temporary source copies and owned test databases are removed after checks. The refresh command does not create, migrate or delete application databases. Keep findings in the existing external workflow file. Stop and summarize/remove it on architectural escalation or review non-convergence. Never rewrite commits, discard unrelated changes, or modify the original unfinished API checkout.

## Interfaces and dependencies
This task changes developer tooling and evidence, not Rust public interfaces. Native runtime, database ownership, facade optionality and unpublished package boundaries remain governed by ADR-011.
