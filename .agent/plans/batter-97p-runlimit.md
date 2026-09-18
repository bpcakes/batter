# Protected native quota execution (batter-97p)

## Purpose and scope

Implement the user-approved, compiled prototype as an optional `batter-runlimit`
adapter, directly in the library. The reference service is a later adopter.
Preserve native Runlimit policies, atomic batches, typed failures and quota
consumption truth. HTTP owns async authentication ordering, direct-peer handoff,
quota-before-body execution and exactly one retained completion observation.
No PostgreSQL provisioning, migration/maintenance lifecycle, reference-service
changes, facade extraction, retry engine, publication or commits belong here.

## Progress

- [x] Inspect clean baseline `de7bd2d`, prototype and current native source pin.
- [x] Implement adapter, optional dependencies and constrained observation seam.
- [x] Port failure scenarios, runnable consumer and independent feature checks.
- [x] Update contracts, scope/dependent tracker records and validation evidence.
- [x] Run focused checks, both verification matrices, five HTTP smoke profiles
  on each toolchain, and final Jig gates; close the owning Bead on success.

## Surprises & Discoveries

The prototype at `/tmp/batter-runlimit-api.ha92wP` passed 24 runtime/shape tests,
two compile-fail doctests and a runnable example on macOS arm64 Rust 1.98.1 and
1.94.0. Its native sequential-check counterexample partially charged a denied
request; native atomic batching retained the other allowance. Its observation
mutation failed the timeout retention regression. These are prototype results,
not verification of the production implementation.

The production port adds single-take observation ownership and standalone quota
factory-destruction dispatch coverage. Focused checks pass 28 adapter cases,
one positive and two compile-fail doctests; all eight isolated feature graphs
compile with expected disabled-HTTP failures. The initial full verification
stopped at the test runner's four-phase assertion after adding a fifth feature
phase. Updated its exact phase inventory and failure-stop tests; both full scripts
subsequently passed, with all eight isolated features and all ten rebuilt HTTP
smoke profiles. Focused review added concrete HTTP error-retention assertions,
closing its one coverage gap without changing runtime code. The first Jig
receipt was rejected because these assertions changed during execution;
frozen final gates passed with `api:test` receipt
`receipt_01M2RPCD74VHKMEMRS0KJ77S7J` and no unresolved gate. The final
documentation/tracker refresh reuses unchanged Rust receipts.

## Decision Log

- User explicitly authorized direct adapter delivery, superseding the
  example-first/two-consumer prerequisite for this integration.
- Depend on Runlimit Git `b2e61516f5a540fe4bc1e3d90fa476a00d0a5946` (core/memory
  0.3.0, postgres 0.3.1, native SQLx 0.9.0); use its atomic Limiter API rather
  than stacking single-check HTTP layers. Memory and PostgreSQL error bridges
  preserve known consumption without changing storage or transaction handling.
- Default adapter graph includes only foundation and native core. `memory`,
  `postgres`, `axum` features are independent. Existing facade tasks stay open.
- Quota observation is opt-in. Existing operational_http gets no new per-request
  allocation. A dedicated middleware selects the shared observer with a fresh
  private record; the quota adapter takes the sole writer before calling auth or
  handlers. No callback runs from the observation destructor.
- Lifecycle/deadline precedes authentication, then quota, then body/handler.
  Unavailable service therefore precedes authentication rejection on this new path.
  Existing reference and Axum APIs preserve their behavior.

## Outcomes & Retrospective

Implementation, both full verification scripts, ten HTTP smokes and focused
read-only review and final frozen Jig gates are complete. PostgreSQL
runtime, Linux/hosted execution and independent agent
usability remain unverified. The downstream client task records that reference
adoption needs an explicit application-scope decision before quota wire claims.

## Context and execution graph

T1: new `crates/batter-runlimit` plus opt-in observation in `batter-axum`.
T2 depends on T1: native memory and scripted-failure tests, native loopback serving,
compile-fail boundaries, minimal independent feature graphs and runnable example.
T3 depends on T2: contracts/status/references and both complete Rust matrices,
HTTP smokes and required Jig evidence. Independent focused review may examine
HTTP trust/observation separately from execution cancellation while validation runs.

The current root is a virtual workspace and adapters depend on `batter` directly.
Do not create a dependency cycle by prematurely adding facade imports. Use native
OperationContext children with the same deadline for both phases. Preserve
admitted quota separately from whether work actually started or completed.

## Validation and recovery

Run focused `cargo test -p batter-runlimit --all-features --locked`, then
`bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
On each toolchain rebuild `cargo build -p batter-axum --example http_service
--locked` and invoke `python3 scripts/smoke_http.py --binary
target/debug/examples/http_service` in default, SIGINT, deadline, WARN and
WARN+deadline modes. Run Jig work evidence/gates before final work check and
reuse only receipts for unchanged commands, inputs and environment.

All changes are additive library/source changes. No database mutation or rollout
occurs. Preserve preexisting and concurrent work; any failed check stays recorded
until the cause is resolved. Cargo alone updates Cargo.lock. Existing exhaustive
`crates/*/src`, tests and examples globs cover the new package; keep any new
feature-runner inputs covered in both Jig contract representations.
