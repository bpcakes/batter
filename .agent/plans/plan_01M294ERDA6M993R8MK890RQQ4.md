# Own component and command settlement for agent consumers

This living ExecPlan follows `.agent/PLANS.md`. Owning delivery task: `batter-gi4`.
The user authorized the full implementation, all working changes, and comprehensive
all-reviewer review/fix cycles, including additional cycles after a round limit.
No commit, push, deployment or publication is authorized.

## Purpose

An application agent registers native components and runs finite commands without
building termination gates, independent cleanup drivers, or failure side channels.
The reference acknowledges local runtime initialization and observes dependency
health separately; production startup no longer creates durable control jobs.
Native task supervision stays in the native runtime. Batter owns integration
lifetime, registration, reports and cleanup policy.

## Progress

- [x] Inspected current Batter baseline `46c4881` and existing staged worker repairs;
  native sibling checkout is clean at `50620137e36aab2333213fa8d8e51a095484e6eb`.
- [x] Confirmed agent-only guidance and recurrence assessment are now in root guidance.
- [ ] Implement native loop initialization acknowledgements and deterministic tests.
- [ ] Implement native shutdown reporting including owned observer settlement.
- [ ] Implement Batter managed components and phase-budget composition.
- [ ] Implement owned finite commands and exercise cancellation/failure finalization.
- [ ] Add supported native adapter and remove reference production witness machinery.
- [ ] Implement tested offline retirement of legacy controls; retain migrations/history.
- [ ] Update contracts, source references, examples, input scopes and validation.
- [ ] Run two-toolchain matrices, rustdoc, HTTP smokes, native tests and live reference suite.
- [ ] Run fresh-agent integration/modification acceptance against independent scenarios.
- [ ] Run comprehensive all-reviewer repair cycles to verified convergence.

## Surprises & Discoveries

Native `build` starts loops, but exposes no acknowledgement. The current native
source is identical to the pinned version for supervisor, worker and task group.
Native observer shutdown can log incomplete abortion without returning that fact.
The existing hosting Bead explicitly prescribed the example-owned termination
gate; delivery contracts must change with implementation, not preserve that debt.
The current Batter process admission requires a running ready service and is not
an owned finite-command abstraction.

## Decision Log

2026-09-11: Initialize components inside managed registration, after validation,
instead of constructing a live host before registration. Reuse existing lifecycle
readiness and owned-driver mechanics; independently retain native settlement in
the coordinator so wrapper abortion cannot destroy its only observer.

2026-09-11: Native initialization means local loop configuration/state is ready
before normal processing. Database health and durable execution are separate
observations. Never use a mutating first scheduler/reaper pass as initialization.
Application approval remains withheld until the actual business handler exists.

2026-09-11: Modify `/home/aa/Documents/runledger` in place (clean baseline) and
consume its uncommitted native packages through explicit sibling path overrides.
Never edit Cargo's cache. Record external source identity with verification and
review both repositories; no receipt may be reused after external source changes.
Keep the same native package graph/type identity and Cargo-generated lockfiles.

2026-09-11: Legacy controls retire only after old producers and their database
sessions/transactions have stopped. Free advisory locks alone are insufficient.
Retain applied migrations, terminal history and the harmless epoch sequence.

## Outcomes & Retrospective

Implementation is in progress. No new behavior or validation claimed yet.

## Context and Orientation

Native source lives in `../runledger/runledger-runtime/src`: `supervisor.rs`,
`task_group.rs`, `worker.rs`, `scheduler.rs`, `intent_promoter.rs`, `reaper.rs`,
and the worker/reaper observer modules. The source owns all native task handles.
New initialization and settlement types belong there and never depend on Batter.

Batter `crates/batter/src/lifecycle.rs` and `lifecycle/` own critical task startup,
readiness, shutdown phases and conservative finalizer skipping. `startup/driver.rs`
already retains initialization/cleanup independently of waiters. `cleanup.rs`
owns reserved LIFO finalizers. Reuse these mechanisms for a finite command owner.

The reference `examples/reference-service/src/worker.rs` and `worker/` currently
own a production durable control protocol, independent driver, termination gate
and nested cleanup. `runtime.rs` must become a consumer of supported integration
instead. Existing live tests in `tests/support/hosted_*` contain reusable failure
scenarios; preserve coverage of ownership while retiring obsolete probe semantics.

## Plan of Work

First add a native startup observer created with the supervisor. Each enabled loop
acknowledges after validation and local state construction and before processing;
disabled loops are excluded. Record exits and shutdown under a coherent state
owner so late acknowledgements cannot revive stopped startup. Add tests with
empty registries/lazy pools proving initialization does not require database work.

Next add a native report-returning drive method. Preserve named first and later
errors, requested aborts and unresolved task names, including internal observers.
Existing Result-returning methods remain compatible. Ordinary job failures keep
their durable meaning and do not become process failure merely to simplify reports.

Then extend Batter registration with a managed factory whose lifecycle result
includes termination evidence. The coordinator retains its independent settlement
observer and owns dependency-finalizer eligibility. Validate names before factory
invocation. Preserve no-work-before-start, failure retention and tracing destruction.
Use the same bounded owner/waiter distinction for finite commands, returning typed
work results plus complete cleanup observations even after cancelled waiters.

Add an optional native adapter that owns the translation and budget requirements.
Migrate the reference root onto it, with local runtime acknowledgement, explicit
application approval and separate fresh database health. Move durable execution
proof to isolated live acceptance. Add an offline legacy retirement entrypoint
using native cancellation after verified old-writer absence, preserving history.

Finally synchronize public docs, tests, tracker and verification input policy;
test real consumers, then run the comprehensive repair loop over the full working
scope of each repository. Treat confirmed recurrence as a design assessment,
not permission to weaken assertions or add endless caller obligations.

## Concrete Steps and Validation

Run focused native `cargo test -p runledger-runtime --lib`, Clippy and rustdoc
from `../runledger`, obeying its guide. From Batter run focused new tests while
implementing, then `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`. Build the HTTP example on each
toolchain and run all five `scripts/smoke_http.py` modes from `docs/testing.md`.
Run the exact reference live inventory against two task-owned PostgreSQL 18
clusters, recording identities and keeping provisioning outside library code.
Finish with `scripts/jig work check --plan-id <new-plan-id>` and inspect evidence
and gates; maintain `.jig.toml` and `.agent/jig-contract.json` together.

Independent scenarios must prove owner/waiter cancellation distinctions, factory
inertness, startup failure, registration rejection, initialization timeout,
simultaneous work and cleanup errors, native failure followed by settlement error,
unjoined descendants preventing cleanup, readiness monotonicity, exhausted budgets,
and legacy-row retirement without domain-job mutation. Clock tests do not prove
PostgreSQL orderings: retain controlled real-database interleavings where required.

Fresh agents receive only public consumer guidance and behavioral assignments,
not the implementation's private rationale. Evaluate initial integration and a
subsequent budget/failure modification. Record actual results and remedy discovered
consumer gaps before declaring usability acceptance.

## Idempotence and Recovery

Preserve the pre-existing index and unrelated working files. No automatic staging.
Keep task-local progress here and Beads as delivery authority. New database state
changes are forward-only. Live experiments use only isolated disposable databases.
If a review repeats a confirmed invariant failure, record its cause/owner and
repair the relevant boundary within the user's already-authorized design scope.

## Interfaces and Dependencies

Native initialization observation reports enabled loop initialization or stopped
startup. Native settlement separates cause, observed exits, abortion and unresolved
descendants. Batter managed registration consumes inert native factories and retains
their settlement authority; finite command ownership exposes a borrowed wait and
explicit cancellation without relinquishing finalization. Exact signatures are
recorded as implemented; no public API may imply observation of arbitrary spawned
descendants or remote effects. Preserve native Rust, concrete errors and optional
adoption packages throughout.
