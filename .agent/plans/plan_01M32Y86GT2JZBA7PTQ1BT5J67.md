# Prove exported Runlimit consumption
Owning Bead: batter-isdr.2; epic batter-isdr. The task begins only after the native import passes its pinned review. Fixed PINNED_BASE: 0d9b40e1e41b3fe61b195a5a219eaee44e28be61.

## Progress
Implemented. Git-free original native and facade quota consumers execute on Rust 1.98.1 and 1.94.0. Three independent consumer controls, existing native tool controls and bounded matrix controls pass. Actual extracted ZIP execution passes on both compilers. The updated full minimum-toolchain verify.sh and all five default Jig targets pass; api:test receipt receipt_01M32Z8QVS4YHN26TFEZJTTQN6. Native review remains.

## Surprises & Discoveries
The original native smoke is an executable with runtime assertions and an explicit RUNLIMIT_KEY_SECRET input. It needs no database server. Existing Runledger source-copy tooling already excludes Git, environment files and outputs and validates local identities and locked external sources; reuse it while preserving its existing defaults.

## Decision Log
Use a sibling standalone Cargo workspace whose dependencies point only into a Git-free exported source tree. Seed its lock from the root and reject dependency drift after Cargo prunes the temporary lock. Execute the retained upstream smoke source without rewriting its assertions, then execute an additional facade/direct-adapter/native identity and quota outcome consumer. Select the compiler explicitly from the source workspace or RUSTUP_TOOLCHAIN. Supply a public fixture key. Do not provision PostgreSQL or imply remote-effect verification from this consumer.

## Outcomes & Retrospective
Source-copy consumer passes on both toolchains. Native runtime, SQL, manifests and root lock are unchanged from the reviewed import. No hosted execution claim.

## Context and orientation
scripts/runledger_source.py owns generic source copying and graph validation; scripts/check_runledger_consumer.py owns toolchain selection and the existing consumer. runlimit/smoke/native_consumer.rs contains the original upstream smoke. scripts/test_matrix.py owns bounded batches of at most four checks, and scripts/test_parallel_process.py verifies execution/failure inventory. .jig.toml and .agent/jig-contract.json own exhaustive Rust verification inputs.

## Plan of work
Add scripts/check_runlimit_consumer.py with a separate standalone manifest containing all five native packages, the facade and direct adapter. Run the exact imported smoke as a named binary and a second binary proving native/facade type identity and allowed/denied quota behavior. Validate copied source graph, unchanged external dependency versions and original lock preservation. Add independent negative controls for omitted/remote/duplicate/outside identities, changed external dependencies and missing execution evidence. Add the checker and controls to bounded verification batches. Include the executed smoke source in both exhaustive input declarations. Replace pending smoke guidance with executable commands in native/root documentation.

## Concrete steps
Run selected compiler checks with Rust 1.98.1 and 1.94.0, controls, formatting/static checks, full required Jig gates, and actual source ZIP extraction followed by consumer execution. Reuse task1 evidence for unchanged native runtime, database and HTTP source. Preserve two-toolchain verification claims only when the changed verification runner passes on each. Commit initial implementation before native codex review --base PINNED_BASE. Append every finding to the external workflow findings file, investigate and repair in new commits, and review the entire cumulative task diff again against the same base. Never amend or advance the base while the task is unfinished.

## Validation and acceptance
The native smoke executes its original assertions; the additional consumer proves quota-before-work and denial omission of work. All five native package identities and both facade/adapter/core identities resolve once from copied sources, without Git metadata, upstream native Git dependencies or consumer patches. The selected toolchain survives leaving the repository. Missing or failed execution and graph drift are rejected. New fixture inputs invalidate verification receipts. No database migrations, public Rust APIs or native semantics change.

## Idempotence and recovery
Use owned temporary directories outside the repository and remove them after checks. Never touch the original dirty hardening worktree. Keep external findings outside commits; print their summary and remove the findings file before any early exit. Stop on architectural escalation or recurring findings per user workflow.

## Interfaces and dependencies
Depends on completed batter-isdr.1. Source exports contain native SQL, licenses and existing Runledger assets. Task closure and epic completion are tracker metadata after clean review; PR publication remains the explicitly requested deliverable, stacked on the unmerged Runledger PR.
