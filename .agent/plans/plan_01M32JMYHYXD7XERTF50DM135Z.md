# Import native Runledger into the Batter workspace

Owning Bead: batter-biqv.1. PINNED_BASE: b5beb2f7d9bb2ef0b8e1af6b2da1ef9c698a51fb.
Upstream master snapshot: 46b5cd085d011e597de9552dfebbed4c19416453.

## Purpose
A single checkout and Cargo graph builds, tests and changes Batter and Runledger together, without a sibling source checkout or coordinated Git source pin. Preserve the five native packages and their ownership.

## Progress
- Clean migration worktree created from Batter origin/master. The unfinished API branch remains separate.
- Latest remote Runledger master fetched into /tmp/batter-runledger-upstream and pinned above.
- Import and graph/asset integration complete. Both verify.sh toolchains, ten HTTP smoke runs, static package checks and required Jig gates pass. Native cumulative review pending.

## Surprises & Discoveries
Runledger contains five packages including TUI and its own Docker/PostgreSQL 18 test support. Its native DB tests run by default. Docker is available locally. Beads defaults to the main worktree: all migration mutations explicitly set BEADS_DIR to this worktree's .beads; its local database was copied consistently from the main tracker.

## Decision Log
Place the retained native tree under runledger/ to preserve package-relative migrations, SQLx caches and support paths. Remove the nested virtual workspace manifest and let each package inherit the root workspace. Preserve version 0.12.0 and upstream package lint policy explicitly. Keep SQLx cache and migration synchronization tests; replace foundation Git attestation with Cargo graph/source identity regression checks. The root workspace keeps optional adapters and native package responsibilities. All packages remain unpublished and Unix-only. Existing SQL migration files are copied byte for byte.

## Outcomes & Retrospective
Native packages now share the local Cargo graph. Validation passes on macOS arm64 with Rust 1.94.0 and 1.98.1 and PostgreSQL 18 Docker tests. Required api:test receipt: receipt_01M32M8V2FMEDEYQM2KWCJQ5W5. Native review remains pending. No runtime, persisted schema or public API redesign belongs in this import.

## Context and orientation
Root Cargo.toml currently pins three Runledger packages and patches batter-sqlx. scripts/check_facade_features.py constructs external manifests; .jig.toml and .agent/jig-contract.json enumerate exhaustive check inputs. Native runledger-postgres/build.rs currently rejects changed foundation source. Native packages use PostgreSQL 18 and testcontainers.

## Plan of work
Import native packages, caches/migrations, license and operational documents from the recorded snapshot; preserve provenance for omitted historical planning/tracker/release machinery. Wire the five packages into root Cargo, preserve native lint/features and regenerate Cargo.lock with Cargo. Add graph tests for local source/type identity and acyclic dependency boundaries. Expand required Jig scopes and CI prerequisites to cover native code/assets and Docker tests. Update current root/source-map/adapter contracts and status.

## Concrete steps
Use git ls-files from the pinned upstream to copy selected tracked assets. Remove cross-repository build attestation and its obsolete negative tests; keep migration consistency checks and add local graph negative controls. Adapt independent facade consumer manifests to path dependencies without patches. Run cargo metadata/check, migration byte comparison, native tests, both verify.sh toolchains, HTTP smokes and Jig gates. Commit coherent implementation before codex review --base PINNED_BASE; all repair reviews retain that base.

## Validation and acceptance
All five packages resolve locally with one batter-core and batter-sqlx identity; no Runledger -> Batter facade edge. Native tests, optional feature matrix, migrated package strict linting, rustdoc and HTTP smoke matrix pass on the two supported toolchains. Native database tests execute against PostgreSQL 18 through existing testcontainers support. Linux evidence remains limited to later actual hosted execution. Preserve migration/cache bytes from upstream unless a separately justified forward-only change is required. Fresh-agent usability evaluation remains unexecuted.

## Idempotence and recovery
No databases are migrated as part of source import outside disposable tests; no source repository is archived or published. Keep all imported changes on feat/runledger-workspace. Stop on architectural escalation, repeated findings, unavailable full cumulative review or unexpected worktree changes. Print and remove the external findings file on early exit. Never rewrite commits or discard the original unfinished API work.

## Interfaces and dependencies
runledger-core owns durable types; runledger-postgres owns persistence and consumes batter-sqlx; runledger-runtime owns workers and supervision; runledger-test-support owns its native disposable tests; runledger-tui remains a distinct binary. batter-runledger remains the optional adapter. Root native consumer/source-copy tooling is completed by dependent Bead batter-biqv.2.
