# Constrain startup registration without duplicating lifecycle ownership


This living ExecPlan implements Bead `batter-lp2.1` under epic `batter-lp2` and
follows `.agent/PLANS.md`. Beads owns scope, acceptance, priority and dependencies;
this file owns implementation sequence, decisions and execution evidence. Keep
Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective
current. Preparing this plan does not authorize implementation or commits.

## Purpose / Big Picture


An application agent should register components and dependency cleanup during
startup without receiving methods that extract cleanup or start a replacement
process. After this change it can call `Startup::scoped`, register native HTTP
and Runledger work, and await the existing owned startup driver. A real request
must succeed, shutdown must join its components before releasing resources, and
attempts to extract or replace the supervisor through the new scope must fail
compilation. Existing public startup and adapter calls remain source-compatible.

## Progress


- [x] (2026-09-12 13:25Z) Prepared this plan from the owning Bead, current source,
  package guides and preserved API experiments; implementation has not started.
- [x] (2026-09-12 14:07Z) Recorded baseline `39c1b6e` and claimed `batter-lp2.1`.
- [x] (2026-09-12 14:23Z) Added sealed registration authority and positive/negative public fixtures.
- [x] (2026-09-12 14:23Z) Added protected startup and stable error envelope over the shared driver.
- [x] (2026-09-12 14:23Z) Added source-compatible Axum and Runledger entrypoints with real integration tests.
- [x] (2026-09-12 14:29Z) Completed failure, dispatch, compatibility and contract documentation acceptance.
- [x] (2026-09-12 14:36Z) Passed both full toolchain verifiers, ten HTTP smokes and Jig required gates.

## Surprises & Discoveries


The existing `StartupScope::supervisor()` can extract and drop cleanup; an isolated
baseline subject then obtained an empty successful cleanup report without running
its hook. This is a documented escape, not a contradiction of the old contract.
The new scope must remove that operation rather than add another warning.

Replacing an adapter's concrete `&mut Supervisor` parameter with a generic trait
parameter breaks some existing dereference-coercing wrappers. The experiments
observed E0277 for that change. Preserve the old signatures and add new names.

A temporary registration view cannot lend a cleanup reservation beyond the
temporary's lifetime. The prototype rejected that expression with E0716. Direct
`scope.reserve_cleanup(...)` is the canonical path for acquisition across await.

The startup driver explicitly catches polling and then separately destroys the
completed initializer future. Mapping its error through an ordinary async await
wrapper can destroy the inner future before retaining its returned error. Use a
poll-level mapping, as the native report adapter already does.

These are historical Linux/Rust 1.98.1 observations, recorded with source in
`docs/evidence/agent-startup-apis-2026-09-12/README.md`; they are not this task's
production acceptance. Record new discoveries and actual commands here.

## Decision Log


Decision: expose a concrete borrowed `Registration` and a sealed target trait.
Rationale: the view permits registration but cannot transfer ownership; sealing
limits supported receiver types, not a claim that all public traits are unsafe.
Date/author: 2026-09-12, planning agent, carrying the reviewed Bead decision.

Decision: preserve `Startup::new`, `StartupScope` and original adapter signatures.
Rationale: existing downstream use makes an immediate removal a compatibility
break; the new canonical scope must nonetheless exclude their escape operations.
Date/author: 2026-09-12, planning agent.

Decision: `Startup::scoped` returns a thin public `ScopedStartup<F>` specification.
Rationale: a separate builder gives protected startup a stable error type and lets
the next task add its signal policy without changing legacy startup types. This
is configuration and callback projection, not a second execution driver.
Date/author: 2026-09-12, ExecPlan author, specifying the reviewed constructor.

Decision: introduce `InitializationError<E>` now, including signal-policy variants.
Rationale: Bead `batter-lp2.2` must not change the owner/observer type delivered
here. The existing `SignalRegistrationError` type already exists, so no dependency
on the signal implementation is introduced. No new context accessor is added.
Date/author: 2026-09-12, planning agent.

## Outcomes & Retrospective


Implemented one constrained registration capability and projected protected scope
through the existing coordinator. Runtime controls cover a real channel request,
Axum socket request and prepared Runledger settlement; rejection, ownership loss,
LIFO cleanup and returned-error/destructor panic cases pass. Exact legacy adapter
function pointers and `DerefMut` callers compile. Both supported toolchains, all
ten HTTP smokes and required Jig gates pass. No dependency graph changed; macOS,
hosted CI and live PostgreSQL were not executed for this delivery.

## Context and Orientation


The checkout is a virtual Rust workspace, not a root Cargo package. At preparation
HEAD is `39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`. Current production sources match
the API research baseline; later documentation and tracker edits are preserved.
Resolve current paths again if independent facade work `batter-tmx` has landed.
That work is not a prerequisite: keep one foundation type identity in either layout.

`crates/batter/src/lifecycle.rs` owns `Supervisor`, ordinary `register`, managed
`register_managed`, name validation and cleanup reservations. A component is an
owned task whose exit the process observes. Managed registration additionally
retains a native runtime's complete settlement, meaning observation of all native
owned descendants before deciding whether dependency cleanup may proceed.

`crates/batter/src/startup.rs` owns the inert startup specification and callback
scope. `startup/driver.rs` launches the coordinator and monitor, drives initializer
polling/destruction, publishes an observer result and transfers the running owner.
`startup/report.rs` owns concrete causes and redacted report formatting. A waiter
borrows the result channel; cancelling it does not cancel its owner. Dropping the
owner requests drain while the live runtime continues owned cleanup.

`crates/batter/src/cleanup.rs` provides `CleanupSlot`: a validated exclusive
reservation whose consuming `register` cannot reject the name after acquisition.
Hooks run in reverse registration order and are explicitly awaited. Reservation
alone is not ownership of an acquired resource, and dropping an unstarted startup
specification does not invoke cleanup.

`crates/batter-axum/src/serving.rs` registers a bound listener/router.
`crates/batter-runledger/src/lib.rs` translates owned native preparation into
managed initialization, stop clocks and retained reports. Their native protocols
must remain unchanged. Read root AGENTS.md, agent-map.md and these three packages'
AGENTS.md before editing. Generic test support remains a leaf; the foundation
must not depend on adapters, even to obtain their composition fixtures.

## Interfaces and Dependencies


Add `crates/batter/src/registration.rs` and export it from `src/lib.rs`. Its public
view has a private `&mut Supervisor` field, no public constructor, and no conversion
or dereference to the supervisor. A reborrow is a shorter exclusive borrow of
the same registration authority, not another owner. Define the target shape as:

    pub trait RegistrationTarget: private::Sealed {
        fn registration(&mut self) -> Registration<'_>;
    }

Implement the sealed target for `Supervisor`, `ProtectedStartupScope`, and
`Registration<'_>` only. The sealing module is private. The view exposes
`register`, `register_managed`, and `reserve_cleanup`, forwarding their existing
argument types, bounds and results without changing native semantics. In
particular, ordinary futures return `Result<(), BoxError>`; managed factories
receive `ManagedShutdownBudget` and return `ManagedComponent<R>` with
`R: ManagedSettlement`. Do not invent a cleanup-target trait.

Expose these new public types through `batter::startup`:

    pub struct ProtectedStartupScope { /* private owner and stage */ }
    pub struct ScopedStartup<F> { /* inert specification only */ }

    #[non_exhaustive]
    pub enum InitializationError<E> {
        Application(E),
        Signals(batter::lifecycle::SignalRegistrationError),
        SignalPolicyAlreadySelected,
    }

The protected scope exposes exactly `stage(&mut self, &'static str) ->
Result<(), RegistrationError>`, `reserve_cleanup(&mut self, &'static str) ->
Result<CleanupSlot<'_>, RegistrationError>`, and `registration(&mut self) ->
Registration<'_>`. It has no supervisor, handle, context, start, extraction,
replacement, `Deref`, `AsMut`, or `BorrowMut` escape. Root-captured handles and
contexts remain outside this restriction; arbitrary application spawning is
not prevented by this interface.

Put `scoped` in `impl Startup<()>` so ordinary `Startup::scoped(...)` calls do
not require a turbofish for an otherwise uninferable receiver type:

    pub fn scoped<F, E>(
        supervisor: Supervisor,
        context: OperationContext,
        cleanup: CleanupBudget,
        initialize: F,
    ) -> ScopedStartup<F>
    where
        F: for<'a> FnOnce(&'a mut ProtectedStartupScope) -> StartupFuture<'a, E>;

The existing `StartupFuture<'a, E>` is a boxed, pinned, Send future returning
`Result<(), E>`. The higher-ranked callback bound means the returned future may
borrow the scope for that invocation but cannot retain it after initialization.
`ScopedStartup::start<E>` requires the same `Send + 'static` callback and
`E: Send + Sync + 'static` bounds as legacy start and returns
`StartingSupervisor<InitializationError<E>>`. Preserve
`without_readiness_approval(self) -> Self` on the protected builder. No signal
builder method is delivered here; that belongs to `batter-lp2.2`.

New adapter signatures are `register_http_in<T: RegistrationTarget + ?Sized>`
with `target: &mut T`, then the same name/listener/router arguments as legacy
`register_http`; and `batter_runledger::register_in<T: RegistrationTarget +
?Sized>` with `target: &mut T`, name, `OperationContext`, and native
`PreparedSupervisor`. Both return the existing `Result<(), RegistrationError>`.
Old entrypoints retain exact non-generic signatures and delegate to the same
private registration body. Do not wrap or duplicate native runtime ownership.

## Plan of Work


### Milestone 1: constrain registration and establish public compilation controls


Add the view, sealed trait and root export. Forward to current Supervisor methods
so invalid or duplicate component names are still rejected before factory calls,
including both ordinary and managed names. Component names and cleanup names
remain separate namespaces. Add foundation-owned `tests/registration.rs` and
small supporting fixtures under `tests/registration/` if necessary.
Implement the trait for Supervisor and Registration in this milestone; add its
ProtectedStartupScope implementation with that scope in milestone 2.

Positive subjects must compile real calls through a Supervisor, a retained view,
and successive reborrows. Pair negative subjects with positive baselines sharing
imports, dependencies and toolchain. Reject extraction, starting/replacing the
supervisor, conversion to raw mutable ownership, forging the sealed trait and
moving the borrowed view into a static spawned task. Assert diagnostics at the
attempted operation; missing imports or arbitrary compilation failure do not count.
Use existing report-usage fixture conventions or a test-owned temporary consumer
with Cargo, not a new workspace package. Keep generated locks Cargo-owned.

Run `cargo test -p batter --test registration --locked` and the foundation
doctests. The negative subjects should fail for method/type/lifetime/sealing
reasons while their positive baselines pass. They will not compile against the
old public API; that is distinct from proving the runtime ownership defect.

### Milestone 2: project protected startup through the existing coordinator


Represent the owning scope once: a private protected scope can hold the actual
Supervisor/stage, while legacy StartupScope privately wraps it and preserves
its old methods. Adapt old private field access in startup/driver.rs without
changing ownership transitions. The new builder stores configuration and adapts
its callback into this same coordinator; do not copy start, monitor, cleanup,
publication or handoff logic. A private shared configuration struct is acceptable.

Map each returned E into `InitializationError::Application` in a Future::poll
implementation around the boxed callback future, before its destruction. A safe
wrapper around `StartupFuture` can poll its pinned box without unsafe projection.
The coordinator must retain the mapped result before explicitly dropping that
wrapper, so a completed future's destructor panic coexists with the returned E.
Keep the existing scoped tracing dispatch around polling and full destruction.

Implement fixed redacted Debug/Display for the envelope without requiring E to
implement either formatting trait. Implement `Error::source` conditionally where
the concrete E supports it, exposing original application or signal causes for
trusted inspection. Do not add variants to existing public StartupCause or
StartupError enums. Signals variants exist but are not constructed by this task.

Add `tests/protected_startup.rs` alongside existing startup tests. Require concrete
failure plus two LIFO cleanup errors, marker return plus destructor panic, factory
panic, prior-hook cleanup when the owner is lost before first poll, owner loss
during initialization, abandoned borrowed wait then successful resumption, and
retained pointer-identical observer results. Preserve stage validation and
readiness withholding. An unstarted specification must invoke no application work.

Check in a public typed owner/observer fixture with
`StartingSupervisor<InitializationError<Concrete>>` and
`StartupObserver<InitializationError<Concrete>>`, including an E that is Send/Sync
but not Error. Compile it now. The signal task later preserves and reruns this
fixture; this task's completion does not wait for that future delivery.

### Milestone 3: native adapters use registration authority without source breakage


Move each adapter's existing body behind one private function accepting the view;
add its `_in` entrypoint and retain the old function as a forwarding compatibility
wrapper. For Runledger preserve the budget validation before native start, inert
PreparedSupervisor input, actual initialization observer, earliest stop timestamp,
complete settlement and poll-mapped NativeReport. A closure hiding a live runtime
must remain rejected. Axum still accepts the same bound TcpListener and Router;
ConnectInfo and arbitrary detached descendants remain outside that helper's promise.

Add old function-pointer assignments and old Deref/DerefMut wrapper calls for both
adapters, as well as direct protected-scope and retained-view calls. In Axum's
operational serving tests, perform a real loopback HTTP request and inspect its
response; then require component exit before dependent finalization and a complete
successful shutdown report. Use independent events/resource weak references, not
only the helper's own return value.

In `crates/batter-runledger/tests/lifecycle.rs`, add a protected counterpart of
`native_local_initialization_requires_no_database_or_durable_witness`. Its closed
lazy pool and disabled scheduler/reaper permit actual native local initialization
and complete loop settlement without PostgreSQL. Assert native acknowledgement,
the retained NativeReport and cooperative stop. Real database-backed combined
reference acceptance belongs to task .4, not this milestone.

### Milestone 4: failure contracts, public guidance and final acceptance


Retain existing managed pending-settlement and unjoined-descendant cases: they
must still skip dependency cleanup rather than equate an aborted waiter with
settled native work. Reuse scoped_owned_tasks and scoped_dispatch tests to cover
destructor events and nested span destruction under the captured subscriber.
Never install a global subscriber or panic hook to make tests quiet.

Write complete rustdoc compositions with explicit module paths, concrete error
handling and correct `check_shutdown` use; it returns unit on success, not a
report. Add a runnable foundation-owned protected channel-service example under
`crates/batter/examples/protected_startup.rs`; one request maps 21 to 42, startup
acknowledges, drain joins the service, and one dependent resource closes once.
Do not add SQLx or adapter dependencies to the foundation example.

Update owning public docs and `docs/guarantees.md`, `docs/status.md`,
`docs/testing.md`, `docs/validation.md` and crate-guide navigation. The canonical
new scope is protected; the old scope remains an explicitly lower-level path.
Do not migrate all existing application roots here: that is task .4. Update
`.jig.toml` and `.agent/jig-contract.json` together for any newly introduced
source or fixture root outside existing exhaustive globs; check complete input
ownership even when an existing glob already covers the files.

## Concrete Steps


Run commands from `/home/aa/Documents/batter`, or the actual workspace root on
another machine. Before editing, inspect `git status --short`, `br show
batter-lp2.1 --json`, and the current guides. Start a task-specific Jig plan with
`scripts/jig work start --title 'Implement batter-lp2.1' --body-file
.agent/plans/batter-lp2.1-constrained-startup.md --json`; record its returned ID.
Claim this delivery Bead only when implementation is actually authorized.

During milestones run:

    cargo test -p batter --test registration --test protected_startup --locked
    cargo test -p batter --test startup --test startup_composition --locked
    cargo test -p batter --test managed_components --test scoped_owned_tasks --test scoped_dispatch --locked
    cargo test -p batter-axum --test operational --locked
    cargo test -p batter-runledger --locked
    cargo test -p batter --doc --locked
    cargo test -p batter-axum --doc --locked
    cargo test -p batter-runledger --doc --locked
    cargo run -p batter --example protected_startup --locked

The new named targets exist only after their milestone adds them. A zero-test
filter, ignored case or compilation-only invocation cannot satisfy a runtime
assertion. Record exact counts from the delivered tests, not invented counts now.

Final production-code verification is:

    RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

For each toolchain rebuild the HTTP example, then run all five modes against
that rebuilt binary, retaining the selected RUSTUP_TOOLCHAIN in the environment:

    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Read `scripts/jig work evidence --plan-id <implementation-plan-id>` and `work
gates` first, then obtain final passing `scripts/jig work check --plan-id
<implementation-plan-id>` evidence. Reuse valid Rust receipts only with unchanged
inputs, commands, toolchain, environment and prerequisites and no unresolved later
failure. Tracker-only edits may require cheap policy refresh, not automatic Rust
reruns. If collection hits its limit, read evidence/gates again with
`--freshness-timeout-ms 30000`; that does not run checks. Finish the implementation
Jig plan and close the Bead only after all actual acceptance is satisfied.

## Validation and Acceptance


Acceptance means both old and new clients compile with the intended types, real
HTTP/channel/native work runs and joins before cleanup, and every named failure
retains its original cause alongside all cleanup/destruction outcomes. The
protected scope's negative compilation subjects must have passing positive
controls. Direct reservations must survive native acquisition awaits; a temporary
view lifetime failure is not acceptable in the canonical example. No wrapper
abort may create false native settlement or false clean reports.

All public APIs need rustdoc and a complete compiling example. Default formatting
must exclude private marker contents even when trusted error inspection recovers
them. Both supported Rust toolchains are required; Unix means Linux and macOS,
not Windows. Label platform execution individually. No runtime-death, non-yielding
task, arbitrary spawned-descendant, async Drop or hosted CI guarantee is added.

## Idempotence and Recovery


Tests and temporary consumer builds must use unique task-owned directories and
bounded processes. Never extract the historical experiment archive over the
worktree or copy its old foundation wholesale. Compare its small relevant code
only if helpful. No database provisioning or durable migration is required here.
On test failure retain the report and release held gates before teardown; do not
erase failures by retrying silently. Revert only identified task-owned edits if
needed, preserving unrelated changes and append-only `.agent/state` records.

If a repaired example repeats a confirmed lifecycle defect or breaks a coupled
phase, stop dependent repair long enough to apply ADR-010's design assessment:
separate implementation mistake, library gap and native/application complexity,
record evidence on this Bead and report it. Continue only in authorized scope.
Do not use another caller instruction as proof that an ownership gap disappeared.

## Artifacts and Notes


Keep source revision, toolchain versions, exact commands, test counts, negative
diagnostics, runtime observations and final Jig receipt IDs in this file and the
owning Bead. A useful expected runtime observation is request 21 -> response 42,
component exited -> dependent resource released once -> successful report; this
is expected evidence, not an executed transcript. Legacy error-enum exhaustiveness
and adapter coercion fixtures must remain in the repository for later tasks.

Revision note, 2026-09-12: initial execution plan created from the reviewed delivery
contract. It specifies the thin ScopedStartup builder, concrete interface shape,
milestone test targets, native no-database acceptance and safe completion workflow
without changing the owning Bead's delivery scope.
