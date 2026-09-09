# Verify the native reference dependency graph

Owning Bead: batter-4t6. Maintain this plan under `.agent/PLANS.md`.
Baseline: `e518b6b52a88a460edfb6f7c41107e71932b6c70` (initially clean).

## Purpose

Provide executable evidence that application SQLx transactions, Runledger jobs
and external PostgreSQL test leases compose on the supported Rust versions.
The new unpublished `batter-example-reference-service` package contains minimal
compatibility probes; subsequent Beads own the actual reference business command.

## Progress

- [x] Claim the ready Bead and inspect local and published upstream sources.
- [x] Verify the candidate Git revisions are available on the upstream remotes.
- [x] Add the package and native type probes; resolve a Cargo-generated lockfile.
- [x] Add four ignored live migration, enqueue, startup witness and lease ownership probes.
- [x] Execute all four live PostgreSQL probes on Rust 1.98.1 and 1.94.0.
- [x] Record the API manifest and contract/status/reference/validation changes.
- [x] Complete both Rust verification matrices and five HTTP smoke profiles.
- [x] Complete final Jig work gates and record receipt identities.
- [x] Audit all acceptance criteria against source, graph, logs and fresh receipts.

Delivery closure is recorded in the owning Bead and Jig session state.

## Surprises & Discoveries

The published Runledger 0.12.0 uses SQLx 0.8.6 and Rust 1.88. Its Git candidate
at `0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4` uses SQLx 0.9.0 and Rust 1.94.
Both advertise the same package version, so registry version equality cannot
establish source equivalence. `cargo info` and the downloaded registry manifest
establish this difference. Both candidate revisions match remote HEAD.

Initial probes exposed a 16-character harness namespace limit and enum status
readback requirements; the older-schema setup also required SQLx 0.9's explicit
history-table arguments. Final four-case live runs passed on PostgreSQL 18.6.
The matrix's exact initial-batch test caught the added runner-control command;
it now verifies all three commands, retaining both runtime configurations.

Jig recopy refused existing customized managed paths without changing files.
A disposable detached worktree generated the contract; only the intended input
arrays were imported, with semantic equality to generated output verified.

## Decision Log

Pin Runledger's three participating crates to the full Git revision above and
the harness to `3d525e6fc5745ce2e2437c7997de5cccdecff4ac`. This preserves the
inspected APIs and native SQLx 0.9 type identity without absolute path dependencies.
Use external-only harness mode, with an explicitly selected disposable PostgreSQL
18 endpoint and cleanup-on-start disabled. Provisioning stays with the external
harness/operator; no database provisioning implementation belongs in Batter.

Use the existing Python Unix process owner for the explicit live runner's
preflight, compile inventory and execution bounds. Add three offline controls
that reject incomplete/filtered results and unsuitable endpoints. Keep the native
SQLx-only factory probe in the existing lifecycle example so future adapters can
verify those types independently of Runledger/harness compilation.

The upgrade fixture initializes the exact pre-index migration prefix on a
disposable connection through SQLx Migrate::apply; all real upgrades invoke
the supported cutover-aware API. No upstream migration source is copied.
Lease coverage proves awaited/deferred/Drop and never-polled cleanup ownership;
polled-waiter cancellation remains explicitly source-inspected as permitted by
the owning Bead's record requirement, with broader cases owned by batter-kjl.

## Outcomes & Retrospective

The immutable Git graph compiled and all four live cases passed on both Rust
versions. Each full matrix passed 455 Rust test/doctest executions with four
intentionally ignored live cases, plus 25 Python runner controls. All five HTTP
profiles passed. All five Jig targets passed in run_01M23N5PHMDW9GBPY27H9FT1XF,
with api:test receipt receipt_01M23N6K8WMJVQN882T48SNTH0 and then-fresh work evidence.
After recording these results, the final receipt refresh is retained in Jig's
append-only evidence rather than editing this plan again.
Scope is
Linux compatibility evidence; macOS, hosted execution and broader adapters remain
unverified/unimplemented as described in docs/reference-compatibility.md.

The closure audit confirmed all seven acceptance bullets: the single SQLx graph
and public identities compile on both toolchains; native transactional probes
prove rollback, owner identity, canonical conflicts and isolation; fresh and old
initialized schema paths exercise the supported migration APIs; a submitted job's
handler ID and persisted success provide the scoped startup witness while its
driver observes failure; upstream shutdown/lease limits are recorded with explicit
inspected-only boundaries; all required witness/lease probes executed; and the
unpublished Unix/MSRV package plus exact ignored-case runner participate in the
ordinary matrix. New API rustdoc/example, contract/status updates, five HTTP
smokes and current Jig evidence are present. Source and diagnostics remain scoped;
the two upstream local repositories are still clean. The test server contains
zero remaining disposable databases. No unsupported contract requires a Bead
acceptance revision.

## Context and implementation

The virtual workspace currently contains three libraries and one native SQLx
example. Add `examples/reference-service` as an ordinary member with Rust 1.94,
Unix-only policy, `publish = false`, inherited lints and a nearest AGENTS.md.
Keep the existing SELECT-1 example intact. The foundation and generic test support
must remain independent of Runledger and the harness.

Add a small library module proving equality of native Pool, Connection and
Transaction types at compile time, with a no-run rustdoc example. Keep transaction
parameters visible. Put live cases in `tests/reference_live.rs` and subordinate
test modules. Every live case is named and ignored by default; ordinary all-feature
checks must compile them and report them unexecuted. Add
`scripts/test_reference_live.sh` to preflight the endpoint and test inventory,
run `cargo test -p batter-example-reference-service --test reference_live --locked
-- --ignored`, and reject zero executed cases or missing prerequisites.

Migration probes start from an empty leased database, call
`migrate_after_idempotency_cutover`, add an application migration through SQLx,
then rerun migration and `ensure_schema_compatible_after_idempotency_cutover`.
Read migration history back independently. Do not call deprecated migration APIs
or bypass cutover checks with Runledger's raw migrator.

Enqueue probes use an application-owned READ COMMITTED transaction, registered
job definition and owner-scoped key. Assert Inserted then Existing with the same
job ID, rejection for changed canonical payload and independent owner identity.
Rollback and independent database readback must prove the native transaction seam.

The startup witness synchronizes definitions and schema, then observes a controlled
job through its registered handler. Drive the upstream supervisor continuously so
an unexpected exit cannot masquerade as startup success. Assert persisted job
completion and await upstream shutdown before closing the pool. Document that this
only witnesses the selected job path; it establishes no universal loop readiness.
Read actual supervisor/task-group and worker code for in-flight claim ordering,
transitive join, first-error visibility and extra abort-cleanup allowance.

Lease probes observe database presence independently, close pools before awaited
cleanup/defer/Drop, explicitly drain deferred cleanup, and prove database removal.
Test transfer when a cleanup waiter is cancelled at an observable boundary if
the public API supports a controlled witness. Keep prerequisite failures visible
and preserve body and teardown failures. Budget all native pools and administrative
connections, and document PostgreSQL major 18, CREATE/DROP authority and local
non-TLS endpoint restrictions.

## Validation and recovery

Run from the repository root: `cargo metadata --locked --format-version 1`,
`cargo tree -p batter-example-reference-service --duplicates`, and
`cargo check -p batter-example-reference-service --all-targets --all-features --locked`
on Rust 1.98.1 and 1.94.0. Run the explicit live runner, recording case inventory,
PostgreSQL version, platform, lock hash and outcomes. Failed assertions require
fixing the probe or recording a real upstream incompatibility; never weaken the
requested contract to close this Bead.

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
Build `cargo build -p batter-axum --example http_service --locked`; execute
`python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in
default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
`--warn-filter --deadline` modes. Inspect `scripts/jig work evidence` and
`scripts/jig work gates`, then run the applicable `scripts/jig work check` for
this plan, requiring a successful final api:test receipt. Record evidence in
`docs/validation.md`; update `docs/reference-compatibility.md`, references,
integration contracts and implemented status. Flush tracker changes through br.

Use fresh disposable leases for retries and await cleanup before fixture teardown.
Do not alter upstream repositories or existing databases. Keep material failures
and remaining prerequisites recorded here and in the owning Bead. No commit,
publication or deployment is authorized.
