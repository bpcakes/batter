# Native runtime adapter guide

## Purpose

Translate Runledger initialization and complete native settlement into Batter
managed registration, reexport Runledger's phase-scoped atomic runner, and make
the native Runledger packages reachable through one facade dependency. Follow the
root Unix-only policy. Native task
ownership, durable job outcomes, registry/catalog policy and database provisioning stay native.

## Key entrypoints

- `src/lib.rs`: inert registration, startup observation, stop propagation and native report.
- `src/lib.rs`: opaque `run_atomic` composition without native connection exposure.
- `src/lib.rs`: the `native` module reexporting `runledger-core`, `runledger-postgres`,
  `runledger-runtime` and, behind `test-support`, `runledger-test-support`.
- `src/phases.rs`: `job_phases`, the one bridge from a native `JobExecution` to
  Batter `OperationPhases`, and its typed `JobPhasesRejection`.
- `tests/lifecycle.rs`: actual native-supervisor contracts without PostgreSQL.
- `tests/job_phases.rs`: paused-time bridge arithmetic, rejection, phase,
  observer and isolation contracts through the native invocation owner.
- Reference service: application schema, handler selection and dependency health.

## Edit here for X

Keep lifecycle translation here. Change native descendant accounting in Runledger,
process cleanup eligibility in Batter, and application policy in the consumer.

## Invariants

Accept only owned native preparation. Start it inside the managed factory after
validation, without awaiting before managed transfer. Never use a durable startup
witness. Native stop must drain peers before full settlement, and use the parent's
original timestamp. Preserve the original native report and conservative dependency
cleanup classification. Do not install tracing subscribers or print error contents.
NativeReport owns RuntimeSettlement and derives cleanup authority from its
unforgeable variants; never restore caller-writable report fields or reclassify
borrowed evidence. Schema verification owns its read-only transaction and returns
snapshot evidence; never restore borrowed session/transaction views.
`run_atomic` owns disposition and consumes the intent phase before queue operations.
It uses Batter's SQLx foundation and only releases outputs after acknowledgement.
No raw owner extraction or legacy bridge is supported.
`job_phases` takes the whole `JobExecution` and an explicit reserve. Derive the
root from the absolute native deadline, never from the remaining budget; reuse
core `reserve_finalization`/`split_finalization` rather than new arithmetic.
Link cancellation only through the native invocation's exit hook, which owns
the root's authority: no guard, forwarding task or paired call, and no public
core constructor from a token. Reject services without an exit claim instead
of treating them as never ending. Keep `OperationError` to `JobFailure`
mapping in applications.
The `native` module reexports the native packages themselves, never wrappers or
copies, so facade and direct paths keep one type identity. Reachability is not
ownership: keep `register_in` the protected path, keep the module documentation's
caller obligations current when native lifecycle APIs change, and never let a
native namespace become the documented way to run a worker. `native::test_support`
stays behind its own opt-in feature and out of the default graph; Docker
provisioning and teardown remain Runledger's. Adding a namespace never authorizes
changing a native runtime, storage, policy or lifecycle. `runledger-tui` is
binary-only and has no namespace.

## Common commands

`cargo test -p batter-runledger --locked` and
`cargo clippy -p batter-runledger --all-targets --locked -- -D warnings`.
The native packages live under `runledger/` in the shared workspace. Run
`scripts/check_runledger_workspace.py` to check actual Cargo source identity and
ownership direction. No source pin, sibling checkout or foundation patch remains.
Validate both feature branches together. `python3 scripts/check_single_facade_consumer.py`
executes the single-dependency consumer, and `python3 scripts/check_facade_features.py`
proves each namespace's reachability, isolation and identity. Publication is governed by the active
`batter-ddc` release task. Final acceptance includes root pinned-toolchain/live
gates locally and MSRV verification in CI.
