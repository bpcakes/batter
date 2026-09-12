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
- [x] Implement native loop initialization acknowledgements and deterministic tests.
  Native sibling exposes `startup_observer`, typed snapshots and
  `wait_initialized() -> Result<(), RuntimeStartupStopped>`. All four enabled loops
  acknowledge after local initialization; shutdown and loop destruction stop the
  shared state. Seven focused tests pass, including closed-pool initialization,
  disabled loops, waiter cancellation, invalid config and never-polled destruction.
- [x] Implement native shutdown reporting including owned observer settlement.
  `run_until_shutdown_report` uses validated `RuntimeShutdownBudget`, a first-stop
  timestamp shared by every request path, and independently retained shared joins
  for worker jobs, running/terminal observers, reaped observers and terminal hooks.
  Reports retain later failures and unjoined work after wrapper abortion. The
  PostgreSQL integration test confirms a blocked success callback remains unjoined
  after worker abortion while the actual job remains durably successful.
- [x] Implement Batter managed components and phase-budget composition.
  `register_managed` validates inert factories, owns initialization acknowledgement,
  drains on native stop and retains settlement outside the abortable direct waiter.
  Frozen managed records gate cleanup; observers retain late reports without
  retroactive cleanup. First-stop state anchors every process phase. The native
  stop callback receives that timestamp; native `requested` and
  `request_shutdown_since` supply event-driven observation and clock propagation.
  Fourteen public contract tests and all 281 foundation tests pass on Rust 1.98.1.
  Real optional-adapter composition remains the later integration acceptance.
- [x] Implement owned finite commands and exercise cancellation/failure finalization.
  `command::Command` owns finite work, downward cancellation and independently
  driven registered cleanup. `within` reserves cleanup inside one absolute total.
  Reports retain work, late interruption, destruction panic and cleanup outcomes.
  The UDP example now consumes this owner. Sixteen command contracts, seven scoped
  ownership tests and six example tests pass on both Rust 1.98.1 and 1.94.0. The
  foundation test suite (298 tests including examples), 22 doctests, all-target
  Clippy and warning-denied rustdoc pass on 1.98.1; all six executable command
  modes returned their expected statuses. Full workspace/two-toolchain/live gates
  remain pending until native adapter/reference cutover is implemented.
- [x] Tighten the native/adapter construction boundary to consume an owned inert
  launch value. Native preparation owns validated configuration; the adapter accepts
  no live supervisor or closure. Native/adapter inertness tests and compile-fail
  controls pass; reference callers now transfer preparation.
- [x] Separate first failure cause from a deadline that can only tighten; propagate
  native stop timestamps into the parent and wake already-running phase waits.
  Idempotent stop callbacks exchange the earliest clock and retain their last
  acknowledged timestamp so a concurrent earlier update cannot be swallowed.
  Tests cover active native grace/abort waits, process phase wakeups, delayed
  native observation, and another update arriving during propagation.
- [x] Finish supported native adapter and reference production witness removal.
  Native lifecycle and production-root cases now pass: actual HTTP liveness,
  withheld readiness across health intervals, no durable control jobs and awaited
  SIGTERM cleanup. Full final acceptance and fresh-agent evaluation remain below.
- [x] Implement tested offline retirement of legacy controls; retain migrations/history.
  Native disable, quiescence/identity/session checks and owned finite commands are
  implemented. Seven live cases cover old catalog behavior, history, late/prepared
  enqueue, hidden sessions, wrong identity, commit rejection/readback cancellation
  and actual lost COMMIT acknowledgement. A separate required library probe covers
  physical-session replacement. Native cancellation retains SQLx and rollback causes.
- [x] Update contracts, source references, examples, input scopes and validation.
  Existing exhaustive crate/example/script patterns cover new runtime inputs;
  sibling identities accompany forced Jig execution. The consumer archive is a
  separately executed historical workspace under docs/evidence.
- [x] Run two-toolchain matrices, rustdoc, HTTP smokes, native tests and live reference suite.
  Root full matrices, all five HTTP smokes and 58+1 reference cases pass on both
  exact toolchains. Native 481 library tests, 22 doctests, five PostgreSQL integration
  cases and ten supervisor integration cases pass on both; strict Clippy passes on native pinned 1.94.0. Supplementary
  1.98.1 native Clippy reproduces the same 18 baseline diagnostics, documented
  without claiming a pass or altering unrelated public error representations.
- [x] Run fresh-agent integration/modification acceptance against independent scenarios.
  A context-free agent passed five prewritten integration oracles on its first
  compile, then two prewritten total-budget oracles plus all originals without
  repair. Public guidance/signatures only; archive under docs/evidence/batter-gi4.
- [x] Run comprehensive all-reviewer repair cycles to verified convergence.
  Batter: three repair rounds and four complete review passes. Native: five repair
  rounds and six complete review passes. Claude, Codex and Cursor participated;
  Cursor used xhigh effort. All verified medium-or-higher findings are fixed;
  rejected claims and remaining low findings have explicit dispositions.
  Final Jig verify profile passes all five targets, including api:test, with
  unchanged paired source identities. Tracker and policy-only closeout follows.

## Surprises & Discoveries

The first complete native cutover live run exposed two problems in the unjoined
callback scenario. Direct Notify wakeup could delay the fixture controller behind
the intentionally non-yielding callback; an independent timer/atomic observation
now proves the callback is still held at stop and report boundaries. That stronger
scenario exposed an actual native teardown deadlock: the registry's shared-join
waker owned the registry itself. GDB confirmed last-owner destruction under
futures-util's notifier lock, recursively locking it through Shared::drop. A
weak-owner regression failed before the fix without hanging; notification now
retains only a separate wake signal. The scenario, all 233 native runtime library
tests, and the complete reference suite pass after repair. No cleanup assertion
was relaxed. The initial missing entry was the cleanup skipped list, not a missing
managed record; the diagnostic investigation corrected that interpretation.

Additional draft research on 2026-09-12 confirmed that restricted PostgreSQL roles
can see another session while its backend type is NULL; the draft's client-only
predicate misses it. A disposable PostgreSQL 18.6 experiment also demonstrated a
prepared transaction retaining its definition lock and later committing after
its client exited. Native cancellation stringifies begin/commit errors and drops
a secondary rollback error; draft readback can additionally lose its captured
primary error when the command is cancelled. These require changes at their actual
owners. SQLx pool size one does not prevent physical connection replacement.
See the final research-closure section in `docs/references.md` for executed evidence,
failed exploratory controls, source references and bounded decisions. The new
retirement draft initially failed compilation because its return annotation used
two result generics where the public inert command takes a factory generic.
Correcting that annotation to return an opaque factory restored the reference
all-target compile and package formatting checks on Rust 1.98.1; operational
retirement acceptance and its identified fixes remain outstanding.

The 2026-09-12 follow-up in `docs/references.md` found two remaining draft API
obligations: a factory can capture an already-running supervisor, and a late
parent stop can be ignored by the native first-clock latch. It also identifies
native definition disable as the legacy producer admission boundary and adds
prepared transactions to retirement preconditions. The task-owned PostgreSQL
18.6 server has two-phase transactions disabled, so its existing live evidence
does not cover a prepared enqueue. These findings require implementation/tests;
source research is not a passing acceptance result.

Further research closure is recorded in `docs/references.md` under
"Agent-consumer lifecycle redesign: research closure (2026-09-11)". Native running
observer Drop paths abort and release join handles, so a report-returning outer
supervisor alone is insufficient: descendant observation must survive loop-future
destruction. Native stop-handle requests and the driver's external shutdown future
are separate paths today; the new first-stop deadline must cover both. Loop-local
initialization does not imply a global barrier before processing or prove durable
execution. These are implementation constraints, not newly executed guarantees.

Native `build` starts loops, but exposes no acknowledgement. The current native
source is identical to the pinned version for supervisor, worker and task group.
Native observer shutdown can log incomplete abortion without returning that fact.
The existing hosting Bead explicitly prescribed the example-owned termination
gate; delivery contracts must change with implementation, not preserve that debt.
The current Batter process admission requires a running ready service and is not
an owned finite-command abstraction.

## Decision Log

2026-09-12: Shared-join notification must not own the registry containing the join.
Use an independent Arc wake signal, retaining native supervision/report ownership
elsewhere. Keep the adversarial callback held until bounded report publication,
then observe its actual completion. Native cancellation also adds an explicit
RollbackFailure variant; local exhaustive matches are updated. This is a source
API addition requiring coordinated downstream handling before any native release,
not a claim that all external exhaustive matches remain source-compatible.

2026-09-12: Retirement must reject unknown target-database activity and check
prepared transactions before disable as well as before completion. Pin the
verified mutating session and stop mutation on replacement; perform any subsequent
read-only reconciliation against a separately verified connection. Publish the
primary command failure before that optional reconciliation, so cancellation
cannot erase it. Fix native cancellation's error-source and rollback retention
upstream; do not compensate with an application error channel or generic journal.
An absent definition is not a disabled tombstone. Keep deployment revocation
external. Correct the draft's public factory return signature; one compiler-caught
annotation error alone does not justify another foundation redesign.

2026-09-12: Implemented clock exchange through the idempotent native stop control:
`request_shutdown_since` returns its earliest timestamp; managed stop callbacks
return it to the process and accept subsequent tightening. The independent native
stop future remains an event, with no second timestamp protocol. Phase waits
subscribe before observing state and re-read their absolute deadline on changes.
Retain the callback's acknowledgement, not a later shared snapshot, to avoid
losing an update that arrived concurrently during callback execution.

2026-09-12: Follow-up research selects an owned inert native preparation value for
canonical adapter registration. Keep native build compatibility, but application
agents must not supply arbitrary live-supervisor factories on the protected path.
Keep first cause immutable while permitting earlier stop times to tighten active
native/parent phase deadlines. Add both ordering and active-wait regressions.

2026-09-12: Offline legacy retirement disables the exact job definition through
the native update API before native cancellation. The inspected old additive
catalog sync preserves disable. Require session/transaction absence, including
prepared transactions; keep deployment revocation external and never infer it
from a boolean, role name or database snapshot. Preserve terminal/domain state
and migrations. Native disable does not authorize arbitrary deployment actions.

2026-09-12: Pinned Jig cannot observe sibling inputs through parent-directory
globs. During path-patched development, final Rust evidence must be force-executed
with both source identities recorded before/after and source writers stopped.
Root-only receipt freshness cannot justify reuse. No separate Jig feature or
runtime vendoring is added to this task.

2026-09-12: Finite commands retain concrete T/E through a shared report; callbacks
borrow an exclusive reservation-only scope. No takeable cleanup stack or process
admission is exposed. Work cancellation and normal completion cancel descendants
without reaching the supplied parent; finalization uses a separate owner/budget.
Optional total reservation includes cleanup abort observation. A final poll's
returned value/error survives a late interruption instead of being overwritten.
Cleanup coordinator failure also cannot discard the work result. Runtime death
remains explicitly outside the completion guarantee and is covered by a negative
observer test; a previously published report survives it.

2026-09-12: Managed factories are synchronous adapter boundaries and return Err
only when native construction was rejected before spawning. Asynchronous resource
acquisition uses owned Startup. Init failures after transfer still await native
settlement. Classification runs after retaining the original report; a classifier
or future destructor panic cannot erase it. The managed owner outlives its direct
waiter, and the process freezes pending native evidence when its absolute report
deadline expires. It never retrospectively executes skipped cleanup.

2026-09-12: Parent drain/cancel are the native graceful/abort allowances; parent
abort/reap remains a final observation reserve. The stop callback receives the
original parent timestamp and native requests preserve their first cause/clock.
Native failure notification is event-driven and independent of settlement. No
application termination gate, report channel or budget summation is required.

2026-09-11: Native shared joins retain the original `Arc<JoinError>` for both the
loop-local caller and supervisor registry. Forced admission closes before abort;
new native futures wait for registered admission before their first application
poll. Report harvesting polls joins without Tokio cooperative-budget deferral,
never application futures. The old native Result methods remain compatible but
are not the complete-report path consumed by the upcoming adapter.

2026-09-11: Best-effort observer/hook panic and timeout handling can return normally.
Retain each such cause during settlement and count earlier interruptions without
an unbounded historical error collection. Both invalidate a cooperative-cleanup
claim; ordinary business job outcomes remain durable and these best-effort failures
do not initiate normal-process shutdown. This explicit conservative distinction is
covered by a callback test and documented in the native downstream guide.

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

Final review outcome (2026-09-12): converged at the configured medium threshold.
The final Batter parent fingerprint matches all reviewed inputs at
`44cbd1eaf8173dc497a6b7a73ea4a93061e9d4a6d125a9d7490c2a58a808faa5`;
native independently matches
`696e4278939fa626943809e3433afa07ad446328fb804a3fa70293e65d075559`.
Trusted HEAD excludes `.agent` from Batter review; native has no exclusions.
All three selected providers completed each final scope. The original real index
was preserved using a temporary review index; convergence applies to working files.
The complete finding/recurrence ledger is `/tmp/batter-gi4-review-ledger.json` and
the final handoff is `/tmp/batter-gi4-review-handoff.md`.

Final direct acceptance: all sixteen paired matrix commands pass on exact Rust
1.94.0 and 1.98.1, including HTTP smoke modes and all 58 reference entries plus the
separate session replacement probe. Seven unchanged consumer oracles pass. Native
481 library tests, 22 doctests, five PostgreSQL cases and ten supervisor cases pass
on both versions; strict native Clippy passes on 1.94.0. Its 18 supplementary
1.98.1 Clippy baseline diagnostics remain explicit. An earlier unchanged fixture
hit PostgreSQL's forced-drop statement timeout; isolated and full reruns passed
without test changes, and the final fresh matrix passes completely.

The revisions move ownership and failure accounting into native/shared library
boundaries. They also remove the obsolete settings builder shortcut, restore the
original independent redaction regression, retain callback destruction and
transaction-phase failures, and serialize descendant collection while preserving
bounded finished-result harvesting without quadratic ordinary completion scans.
A value that could be produced by another application poll is not a returned
result; cancellation retains values actually returned, without polling more work
after observed interruption. Native absence updates return Option and have actual
repeated-absence CLI evidence. Root-only receipts never authenticate sibling code.

Remaining low findings include conservative pre-start cleanup refusal, top-level
Requested labeling for managed failures, one stale reference README sentence,
public native report extensibility and planned-abort diagnostics. They do not
invalidate the configured convergence result and are not claimed repaired.
Provider integration remains the separate open `batter-8q8.2`; application readiness
stays unapproved. The old witness repair tasks are superseded by removal of their
production protocol, not evidence that their original repairs were implemented.
Publication/pinning of the explicitly authorized sibling patch remains outside
this uncommitted task. Final Jig verification passed all five targets on the pinned 1.98.1 toolchain;
`/tmp/batter-gi4-final-jig-verify.log` and its paired source guard retain the result.
Tracker-only closure refreshes policy gates without repeating current Rust receipts.

Earlier entries below are chronological implementation evidence; their pending
statements and smaller counts describe the then-current intermediate state.


2026-09-12 full acceptance progress: `scripts/verify.sh` passes on Rust 1.98.1
and exact 1.94.0. All five default-toolchain HTTP smokes and the expanded
58-entry live reference runner plus separate library session probe pass.
`/tmp/batter-gi4-source-guard.py` records both repositories' code-input identities
before/after each smoke/live command and the minimum-toolchain matrix; they stayed
unchanged. Tracker, Markdown and docs are explicitly excluded from this code
identity. This is additional external-input evidence, not a replacement for Jig.
The fresh consumer exercise lives outside both repositories and receives public
guidance plus callback interfaces; its behavioral oracles predate delegation.
No result is claimed until compilation and those oracles execute.

2026-09-12 retirement/cutover evidence: seven retirement probes plus the separate
physical replacement probe pass on Linux Rust 1.98.1/PostgreSQL 18.6. The complete
reference runner passes 57 target cases and its separate library probe, including
an isolated repeat with no concurrent native test producers. Native runtime:
233/233 library tests pass on Rust 1.94.1. Native cancellation: three new regressions
and the existing scope test pass; the rollback source unit test passes. Both
affected repositories' all-target Clippy passes, as do six Python inventory
controls, six reference doctests and twelve native PostgreSQL doctests. Full exact
two-toolchain/Jig/source-identity/fresh-agent/all-reviewer acceptance is still open.
The exact Rust 1.94.0 reference run also passes all 57 entries and the separate
library probe, plus ordinary targets, six doctests and all-target Clippy.
The reference preserves domain rows; an additional full-row domain-job preservation
assertion also passed in its focused live case after the complete suite run.

2026-09-12 refinement evidence: foundation 301 tests and 22 doctests pass on Rust
1.98.1; adapter three runtime tests and three doctests pass on both 1.98.1/1.94.0.
Fifteen managed and twenty-nine process ownership tests pass on both toolchains;
adapter/reference all-target Clippy passes on both. Native focused supervisor,
shutdown and report selections, nine doctests and Clippy pass on 1.94.1, including
real active-abort tightening with an unjoined callback. These are direct Cargo
results, not new Jig receipts or full live/matrix acceptance. See the newest
`docs/validation.md` entry. Consumer guides now describe preparation and clock
exchange; remaining compatibility/status/testing historical rows need final
cutover reconciliation together with executed live evidence.

The foundation phase now has 281 passing tests (14 new managed contracts), 18
doctests, warning-denied rustdoc and all-target Clippy on Linux Rust 1.98.1. The
native additions have 224 library tests, eight doctests and all-target Clippy on
Rust 1.94.1. Full workspace/live/two-toolchain/reviewer acceptance is still pending.
New foundation files live under existing exhaustive `crates/*/src/**` and
`crates/*/tests/**` scopes; no input patterns need changing yet. The upcoming sibling
path override will require external source identity handling before receipt reuse.
Logs: `/tmp/batter-gi4-managed-tests.log`, `/tmp/batter-gi4-native-tests.log`.

Native initialization and settlement are implemented in the sibling checkout;
Batter now consumes those uncommitted packages through root path patches. The
native source identity and final two-repository verification remain pending.
The actual native Supervisor integration
held a terminal callback non-yielding past both shutdown phases, retained it as
unjoined, refused cooperative cleanup and independently read the job as Succeeded.
That diagnostic reported PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2); an earlier native
probe also recorded server_version_num=180006. The complete two-toolchain
Batter/live/review acceptance remains pending. No commits were made.

## Context and Orientation

Native source lives in `../runledger/runledger-runtime/src`: `supervisor.rs`,
`task_group.rs`, `worker.rs`, `scheduler.rs`, `intent_promoter.rs`, `reaper.rs`,
and the worker/reaper observer modules. The source owns all native task handles.
New initialization and settlement types belong there and never depend on Batter.

Batter `crates/batter/src/lifecycle.rs` and `lifecycle/` own critical task startup,
readiness, shutdown phases and conservative finalizer skipping. `startup/driver.rs`
already retains initialization/cleanup independently of waiters. `cleanup.rs`
owns reserved LIFO finalizers. Reuse these mechanisms for a finite command owner.

The pre-existing staged reference `src/worker.rs` and `worker/` own a production
durable control protocol, independent driver, termination gate and nested cleanup.
The working cutover removes them and makes `runtime.rs` consume the optional
adapter. Replacements in `tests/support/native_hosted.rs` and
`native_descendants.rs` must prove the retained ownership contracts; deletion of
the old hosted tests is not itself acceptance evidence. Applied migrations stay.

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
Before stabilizing that adapter, add native `SupervisorBuilder::prepare` returning
an owned, validated, inert launch value and consume it directly at registration.
Retain `build` through the same native preparation/start implementation. Separate
failure cause from the minimum stop/deadline state and observe tightening during
active graceful/abort waits, including native-to-parent propagation.
Migrate the reference root onto it, with local runtime acknowledgement, explicit
application approval and separate fresh database health. Move durable execution
proof to isolated live acceptance. Add an offline legacy retirement entrypoint
using native cancellation after verified old-writer absence, preserving history.
Disable the exact legacy definition through native policy first; prove that the
actual old additive catalog sync preserves it and a delayed or prepared enqueue
cannot escape completed retirement. Deployment restart revocation remains an
external prerequisite, not a claim made by the database helper.

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
