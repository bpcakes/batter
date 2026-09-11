# Complete probe preparation ownership and shutdown reporting

Owning Bead: `batter-2zw`. Maintain this plan under `.agent/PLANS.md`. Work from
the existing uncommitted worker-host snapshot; preserve it and do not commit.

## Purpose / Big Picture

Startup interruption must leave an independent owner settling the control session
and native worker. Completion must retain release uncertainty and original failures,
and shutdown signals must cover every awaited initialization stage. The dedicated
control session remains outside the configured pool capacity and is documented.

## Progress

- [x] (2026-09-11) Research exact Runledger 0f464b4, SQLx 0.9 and PostgreSQL 18 semantics.
- [x] (2026-09-11) Implement owned preparation, complete observation and typed release results.
- [x] (2026-09-11) Repair root signals, error aggregation, shared budgets and reconciliation.
- [x] (2026-09-11) Add transition/fault regressions; both full toolchain matrices, all ten HTTP smokes and both 54-entry live runners passed.
- [x] (2026-09-11) Align contracts, evidence and tracker. Stationary Jig run passed all five targets; final metadata policy refresh closes the plan.

## Surprises & Discoveries

Runledger build already spawns loops. Restoring NotStarted after successful build
is false. A blocked preparation query also reproduced SQLx return-to-pool ping retaining capacity after cancellation. Native preparation helpers now use a temporary one-slot close-on-return pool; witness reads use PgLease. The internal-only parent-pool API is deliberately avoided. SQLx close sends Terminate and closes transport without proving backend
exit. cancel_job sets LEASED directly to CANCELED, but an already-terminal race
returns job.invalid_state_transition. A dropped acquisition future can lose a lock
acknowledgement, so ownership must precede that query.

## Decision Log

Keep native supervision and conservative transitive-termination gating. Retain
preparation and native failures independently of waiter delivery, and let cleanup
observe settlement before deciding whether to close or skip dependencies. Validate
clock arithmetic before acquisition/spawn. A bounded release may report uncertainty;
it never fabricates server confirmation. Pool max continues to describe pooled
capacity plus a separately owned control session in normal operation.

## Outcomes & Retrospective

Implementation and both toolchain verification matrices are complete. Both strict 54-entry runners and all ten HTTP process smokes passed. The first complete live run passed all 52 database cases but correctly rejected two filtered offline entries; full execution now includes those controls with zero filtered/ignored cases. Initial Jig receipts were invalidated by concurrent documentation edits. The stationary rerun passed all five targets, including api:test receipt_01M28GP546TXHQ6Z3E5BXD90C9. Final metadata edits reuse unchanged Rust evidence and refresh repository policy checks. Both task-owned containers were removed; Cargo.lock and the pre-existing uncommitted work are preserved.

## Context and Orientation

`examples/reference-service/src/worker.rs` hosts native loops; `worker/probe.rs`
prepares the durable generation-bound witness. `worker/cleanup.rs` owns dependency
finalizers. `runtime.rs` composes startup and signals. Foundation signal handoff is
in `crates/batter/src/lifecycle/unix.rs`. The native driver and preparation waiter
must have separate lifetimes; a retained owner means cancellation of a caller only
requests stop while another task continues explicit settlement on a live runtime.

## Plan of Work

First retain the complete preparation operation before lease acquisition, with
cancellation requests and join observation. Keep a separate preparation-pending
fact so NotStarted cannot bypass an acquired session's settlement. Every native
build must transfer synchronously into its driver; failures before it leave no
native work. Preserve generation checks in enqueue, dispatch and witness readback.
Reconciliation cancels pending/leased controls and accepts a verified terminal
state after the specific native race error; other errors remain failures.

Return typed unlock and close outcomes, with a shared release deadline and retained
errors even when a later step times out. Keep native cooperative-stop proof separate
from release success. Derive the worker allowance from ten seconds native shutdown,
one second abort draining, two seconds release and scheduling margin. Use the same
allowance for parent work reservation and process grace; include dependency cleanup
in the outer cleanup allowance. Retain late cleanup reports on startup errors.

Install Unix sources before the first initializer await and poll throughout startup.
Remember completed signal reception during registration. Test actual child signals,
overflow without side effects, caller cancellation while preparing, release failures,
terminal races and composed shutdown limits. Add source files under existing src
and tests input scopes; edit both Jig contracts if an uncovered root is needed.

## Concrete Steps

From `/home/aa/Documents/batter`, claim the Bead and start a Jig plan using this
file. Run focused package tests and Clippy during development. Then execute:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service

Run the HTTP smoke also with --signal SIGINT, --deadline, --warn-filter and
--warn-filter --deadline; repeat the build and five modes on 1.94.0. Provision
two task-owned disposable PostgreSQL 18 containers with SCRAM and autovacuum
naptime=1s, export their local endpoints for the existing reference runner, and
run `bash scripts/test_reference_live.sh` on each toolchain. Remove only those
containers afterward. Record versions, inventory and outcomes in validation.
Run Jig work check/evidence/gates/finish for the new plan, retaining a final api:test
receipt and reusing fresh checks where their environment and inputs still match.

## Validation and Acceptance

Cancellation before native build settles the session and cleanup returns; an
immediate successor acquires after confirmed unlock. Overflow creates no worker
and cannot hang cleanup. Driver/release failures coexist in retained reports.
Signals at acquisition, schema and preparation enter owned cleanup. A completed
received() followed by register() drains without a second signal. Live leased
cancellation and concurrent terminal transitions converge without weakening native
errors. Both complete toolchain matrices, HTTP smokes and live inventories pass.

## Idempotence and Recovery

No migrations, dependency revisions or payload formats change. Existing durable
controls retain their generation checks. Keep unknown native termination unproven
and skip dependent cleanup. Never reset others' work, edit generated lock entries,
or delete unrelated databases/containers. Repeat failed checks after repairs and
record their initial failures as well as final passes.

## Interfaces and Dependencies

Keep prepare_probe_worker source-compatible where practical. Add documented typed
release outcomes and retained startup aggregation as needed. No new queue, generic
supervisor, database harness or foundation dependency on Runledger/SQLx is introduced.

Revision 2026-09-11: include temporary preparation-pool disposition and the 54-entry full runner (52 live plus two offline signal entries), based on executed cancellation failures.
