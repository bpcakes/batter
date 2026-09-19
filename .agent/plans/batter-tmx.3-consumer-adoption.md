# Adopt cohesive Batter imports in runnable consumers

This is the living execution plan for Bead `batter-tmx.3`, following the
completed core extraction and facade feature work. The current dirty worktree
belongs to the same epic; preserve its source, tracker and append-only Jig
state. No commit, publication or deployment is authorized.

## Goal

Move the real runnable application examples and executable consumers behind the
`batter` facade. Keep adapter implementations, direct adapter tests, native
libraries, SQLx TLS/runtime choices, application policy and external database
provisioning in their current owners.

## Work areas

1. Move `batter-axum`'s `http_service` example and support into
   `crates/batter/examples/http_service`, with `required-features = ["axum"]`.
2. Move SQLx `owned_pool` and `verification` examples into the facade with
   `required-features = ["sqlx"]`.
3. Move Runlimit `shape` to facade `quota_service`, with
   `required-features = ["runlimit-memory", "runlimit-axum"]`; preserve its
   native admission and HTTP 200/429 assertions.
4. Move `examples/postgres-lifecycle` and `examples/reference-service` onto
   facade features. The reference consumer uses `axum`, `sqlx`, `runledger`,
   `sqlx-test-support`; its direct adapter imports remain only where a direct
   interoperability test requires them.
5. Update manifests, explicit targets, CI, smoke/live commands, package guides,
   usage docs, source links, and the Jig archive fixture. Keep adapter-owned
   integration/lifetime tests direct and keep the facade foundation examples.

## Invariants and risks

- No adapter or core package gains a dependency on the facade, including dev
  dependencies.
- Native SQLx, Runledger, Runlimit, Axum, serde and harness dependencies retain
  their existing pins and ownership. No new runtime, storage, provisioning,
  migration, quota or protocol behavior is introduced.
- Required features gate runnable targets; default normal-library and external
  consumer graphs remain isolated even when facade dev-dependencies are built.
- SQLx live/reference cases remain explicitly selected and external-prerequisite
  dependent. Ignored compilation is not live evidence.
- Preserve Unix-only scope, test-support/process relative paths, HTTP smoke
  protocol, quota assertions, diagnostics and multi-migration inputs.

## Validation

Run focused target/build checks first, then `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, both facade feature/example
checks, HTTP and quota smokes on both toolchains, affected SQLx/reference live
checks when approved prerequisites exist, and fresh Jig `work check`, evidence,
gates and finish. Record exact commands, platform, lock hash and live limits in
`docs/validation.md`; update the status row and owning Bead before closing C.

## Progress

- [x] Read C acceptance, package guides, current manifests and consumer roots.
- [x] Claim C after B closed and start this task-local plan/Jig session.
- [x] Relocate runnable examples and configure facade targets/features.
- [x] Migrate lifecycle/reference consumers and retain direct interoperability
  tests only where justified.
- [x] Update docs, CI, smoke/live/archive commands and source paths.
- [x] Run focused checks, both toolchains, current Jig evidence, and close only C.

## Outcomes & retrospective

Moved the Axum HTTP, SQLx pool/verification, and Runlimit quota demonstrations
under `crates/batter/examples` with explicit facade feature gates. Migrated the
PostgreSQL lifecycle and reference consumers to facade imports while retaining
native integration ownership and direct adapter suites. Rust 1.98.1 and 1.94.0
verification, the 14/7/1 facade consumer matrix, ten HTTP smoke profiles, both
quota assertions, moved SQLx example builds, offline consumer suites, and
format/contract/integration checks passed. Live PostgreSQL scripts stopped at
their required missing endpoint prerequisites. Final Jig target validation used
receipt `receipt_01M2V7B4M8KB1FKB75PG4KRQNQ`; the follow-up policy freshness
refresh used receipt `receipt_01M2V7C17RN6G7BQMPXF7WSWPP`.
