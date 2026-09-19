# Extract `batter-core` behind the `batter` facade

This is a living implementation plan for Bead `batter-tmx.1`, the first
delivery task of the closed umbrella `batter-tmx`. The exact source baseline is
`f4f90c9166ff255a91298c75cc020136f632d758`. The worktree already contains a
tracker-only modification to `.beads/issues.jsonl`; it belongs to the current
Beads session and must be preserved. No commit, publication, or deployment is
authorized.

## Progress

- [x] Read the root and nearest package guides, the Bead acceptance, current
  manifests, workspace metadata, and relevant runner/source references.
- [x] Confirmed the current graph has six library packages and two executable
  example packages, with all four adapters depending on `batter` today.
- [x] Claimed `batter-tmx.1` in Beads.
- [ ] Create `batter-core`, move the foundation implementation and tests, and
  reduce `batter` to a compatibility facade plus its existing public examples.
- [ ] Move all adapter foundation imports and manifests to `batter-core` while
  preserving direct adapter behavior and the Runlimit → Axum optional edge.
- [ ] Route runner, CI, Jig input scopes, documentation, and source maps to the
  new ownership boundary.
- [ ] Verify the focused extraction contracts and the complete required
  two-toolchain, rustdoc, HTTP-smoke, and Jig evidence.
- [ ] Record the completed acceptance in the Bead and close only the child;
  leave the closed umbrella untouched.

## Surprises & Discoveries

- The current checkout is exactly the refreshed epic baseline, but the tracker
  export is dirty from Beads updates. Source files are clean at the start.
- The current workspace contains `batter`, Axum, SQLx, Runledger, Runlimit,
  generic test support, and two example packages. The new core must make seven
  libraries without adding a consumer package.
- `batter-runlimit` is already implemented with independent `memory`, `postgres`,
  and `axum` features. Task A must only redirect its foundation dependency and
  preserve its optional Axum adapter edge; feature forwarding belongs to Task B.
- The current scripts and CI use `-p batter` and `crates/batter/src/...` for
  foundation checks. These references need an ownership-aware split: core tests
  move to `batter-core`, while the public examples and facade doctest remain on
  `batter`.

## Decision Log

### D-01 — Extract by moving the single implementation, then re-export it

- Status: accepted by the Bead design.
- Choice: move the existing foundation source and foundation tests into
  `crates/batter-core`; make `crates/batter` a small facade that re-exports
  `batter_core` and owns only the existing public examples.
- Why: adapters can depend downward on one implementation while applications
  retain the `batter::...` paths and concrete type identity. Copying source or
  adding wrappers would create two type identities and a cycle.
- Revisit when: the current source graph reveals a public API or test target
  that cannot be moved without adding a new runtime or dependency edge.

### D-02 — Keep the adapter test graph direct

- Status: accepted by the Bead design.
- Choice: change normal adapter dependencies and adapter-owned tests/doctests to
  `batter-core`; do not add facade dev-dependencies to make old imports compile.
- Why: `batter-core` and every adapter must have no normal, build, or dev edge
  back to the facade. Direct adapter adoption remains a supported path.
- Revisit when: a test proves a public facade-only contract rather than an
  adapter/core contract; that test belongs in the facade or a later consumer
  fixture.

### D-03 — Preserve existing runtime and tracing behavior byte-for-behavior

- Status: accepted.
- Choice: perform mechanical package/import/manifest moves only, retain module
  names, private dispatch/drop implementation, `batter` tracing targets, native
  dependency pins, Unix-only behavior, and all failure assertions.
- Why: this task changes package ownership and public routing, not operational
  semantics. Any behavior change is a separate defect and must not be hidden in
  the extraction.

## Context and orientation

The foundation entrypoint is currently `crates/batter/src/lib.rs`; its nested
modules own lifecycle, startup, health, readiness, cleanup, operation, retry,
admission, settings, telemetry, completion, dispatch, and public error/type
aliases. Its unit/integration tests and private Unix process machinery are
package-owned. The four adapter manifests currently depend on `batter`:

- `crates/batter-axum/Cargo.toml` and all HTTP tests/examples;
- `crates/batter-sqlx/Cargo.toml`, owned-pool/verification examples, and SQLx
  tests;
- `crates/batter-runledger/Cargo.toml` and lifecycle tests;
- `crates/batter-runlimit/Cargo.toml`, quota/HTTP tests, and `examples/shape.rs`.

The generic `batter-test-support` crate remains a leaf. The private
`test-support/process/` sources stay with the consuming foundation and Axum
tests. The two executable example packages remain outside the facade in Task A;
Task C will later adopt facade features in real consumers.

## Execution graph

### T-01 — Establish the core package and compatibility facade

- Outcome: the full foundation implementation has one package owner at
  `crates/batter-core`, while existing `batter::...` imports resolve through
  actual re-exports.
- Changes: add `crates/batter-core/Cargo.toml`, README, `src/`, tests, and any
  package-local support moved from `crates/batter`; reduce `crates/batter` to a
  facade manifest/lib and retain the existing foundation examples exercising
  public facade paths. Update the virtual workspace and generated lockfile via
  Cargo.
- Depends on: none.
- Verify: `cargo metadata --locked` reports seven libraries and two executable
  example packages; isolated core tests and no-default facade checks compile;
  facade doctests can use every legacy root/module path and concrete errors.
- Recovery: keep the source move and manifest edits in the working tree; if a
  target cannot move, restore only that target's ownership with `apply_patch`
  after inspecting the compiler error. Never use destructive Git resets.
- Done when: no foundation implementation remains duplicated, the facade has
  no runtime implementation, and the old public paths/type identities compile.

### T-02 — Redirect adapters and adapter-owned consumers to the core

- Outcome: all four adapters compile against `batter-core` without a dependency
  cycle and retain their current native behavior and direct package commands.
- Changes: update workspace dependency tables, adapter manifests, source
  imports, doctests, tests, and Task A-owned examples (`http_service`, SQLx
  `owned_pool`/`verification`, and Runlimit `quota_service`/`shape.rs`) to use
  `batter_core` for foundation APIs. Preserve direct adapter imports and the
  Runlimit optional `batter-axum` edge.
- Depends on: T-01.
- Verify: package metadata has no normal/build/dev edge from core or any
  adapter to `batter`; focused Axum, SQLx offline, Runledger, Runlimit feature
  tests and existing example build commands pass; direct Runlimit feature
  isolation still passes its eight combinations.
- Recovery: revert only the import/dependency change that introduced a cycle or
  type mismatch, then re-run the focused package graph check. Keep native source
  pins unchanged.
- Done when: adapter tests/doctests use the core identity, all current adapter
  commands remain available, and no facade dev dependency is needed.

### T-03 — Re-route verification, mutation, CI, and exhaustive input scopes

- Outcome: required automation exercises the moved core implementation and the
  facade compatibility surface with fail-closed controls.
- Changes: update `scripts/test_matrix.py`, `scripts/check_scheduling_mutation.py`,
  `scripts/test_parallel_process.py` as needed, CI package/target selections,
  `scripts/test_jig_integration.py`, `.jig.toml`, `.agent/jig-contract.json`,
  file-budget/path checks, archive fixtures, and relative fixture paths. Keep
  protocol identities `batter-fixture-v1` and `batter-scheduling-v1` unchanged.
- Depends on: T-01, T-02.
- Verify: scheduling mutation still changes the intended core admission file
  and rejects its mutant; subprocess matrices remain complete and fail closed;
  contract scopes include every moved/new source; core and facade test routing
  runs under normal discovery.
- Recovery: restore the previous runner logic only if it is the active source
  of truth, then adapt the package/path selectors with explicit coverage checks.
  Historical validation records remain immutable.
- Done when: runner and CI references point at the correct package/path, Jig
  exhaustive scopes remain synchronized, and no test inventory is silently
  dropped.

### T-04 — Update current contracts and status

- Outcome: current documentation tells agents where the implementation,
  facade, adapters, examples, and verification controls live.
- Changes: update ADR-006, architecture, README/status/source maps, relevant
  package guides, testing/guarantees/integration links, and validation evidence
  only for commands actually executed. Use generic scenario names and preserve
  historical evidence wording.
- Depends on: T-01, T-02, T-03.
- Verify: links and package counts point to existing paths; docs name seven
  libraries and the downward dependency direction; `rg` finds no stale current
  ownership claims outside immutable historical evidence.
- Recovery: retain the source-of-truth code and adjust only stale current docs;
  do not rewrite historical validation records to imply new execution.
- Done when: the root/nearest guides, status, architecture, and validation
  describe the delivered ownership boundary accurately.

### T-05 — Run the complete acceptance matrix

- Outcome: the extraction is proven on both supported Rust toolchains within
  the repository's Unix scope.
- Changes: no production changes; run and record evidence.
- Depends on: T-01, T-02, T-03, T-04.
- Verify: focused core/facade/adapter checks; `bash scripts/verify.sh`;
  `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; rustdoc/doctests; both
  toolchain HTTP smoke builds and all five HTTP smoke modes; Runlimit feature
  isolation; scheduling mutation; applicable SQLx/reference preflight limits;
  `scripts/jig work check`, evidence, gates, and final `api:test` receipt.
  Do not claim macOS, live PostgreSQL, hosted CI, publication, or deployment
  without executed evidence.
- Recovery: classify failures by package/path and repair the implementation or
  runner before repeating the narrowest falsifying check. Preserve all output
  and do not relax semantic tests.
- Done when: every Bead acceptance item has direct current evidence, the final
  Jig receipt is fresh for the current plan/input set, and the child can be
  closed without reopening the umbrella.

## Validation and acceptance

The minimum acceptance evidence is the Bead's seven-point list: seven-library
workspace and one core identity; isolated core/facade checks; moved failure,
dispatch, cancellation, scheduling, non-yielding and must-use doctest targets;
strict scheduling mutation; all four adapters and direct Runlimit matrix;
current documentation; and two-toolchain/rustdoc/HTTP/Jig evidence. Compilation
of an example or an ignored live test is not live runtime evidence. The normal
foundation default graph must remain small and the generic support crate must
not acquire SQLx or adapter dependencies.

## Idempotence and recovery

All moves are source-controlled and can be inspected with `git diff` and
`cargo metadata`; repeatable edits use `apply_patch`. Cargo owns `Cargo.lock`.
Do not reset the worktree or overwrite unrelated dirty tracker/Jig state. If a
move is partially complete, finish the package graph first, then repair imports
and test routing from compiler errors and focused `rg` searches. If the current
source differs from this plan, update this plan's discovery/decision sections
before changing the architectural scope.

## Interfaces and dependencies

The final Task A normal direction is:

`batter` → `batter-core` → native Tokio/tracing/error dependencies

and each adapter → `batter-core`, with `batter-runlimit` → optional
`batter-axum` preserved. The facade's optional adapter feature wiring is owned
by `batter-tmx.2`; Task A must leave a minimal facade that can receive it.
`batter-test-support` remains independent and may be a core test dependency but
never depends on core or an adapter.

## Outcomes & Retrospective

Not complete. Update this section only after the source move, dependency graph,
automation, documentation, and required evidence have all been inspected. End
with the exact commands, limits, receipts, and any unverified platform/live
claims rather than a general success statement.
