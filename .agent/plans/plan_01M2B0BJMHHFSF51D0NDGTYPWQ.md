# Own Unix signals across startup and running handoff


This living ExecPlan implements Bead `batter-lp2.2` under `batter-lp2`, following
`.agent/PLANS.md`. Beads owns scope and acceptance; this plan owns executable
steps and evidence. Maintain its four living sections throughout implementation.
This is preparation only: no signal implementation or new experiment is executed
by writing the plan, and no commit, publication or deployment is authorized.

## Purpose / Big Picture


An application agent should select Unix shutdown behavior once, then write only
application initialization. It should not install two listeners, race reception
against initialization, request drain, and transfer listeners itself. After this
change `.with_unix_signals("signals")` on protected startup installs listeners
synchronously before `start()` returns and preserves their ownership through
startup failure or normal running supervision. Real TERM and INT sent after that
return, during initialization, at handoff and while running must drive the proper
bounded cleanup without requiring a second application signal protocol.

## Progress


- [x] (2026-09-12 13:25Z) Prepared the plan from the reviewed Bead and current driver.
- [ ] Verify completed predecessor batter-lp2.1 and record its actual revision/receipts.
- [ ] Add inert policy configuration, precedence and private name reservation.
- [ ] Integrate synchronous installation and preflight failures into the single driver.
- [ ] Preserve reception through initialization, destruction and running handoff.
- [ ] Execute real Unix process scenarios and deterministic error/handoff controls.
- [ ] Complete docs, both-toolchain verification, HTTP smokes and final Jig evidence.

## Surprises & Discoveries


The asynchronous-install prototype died from SIGTERM after the owner was returned
but before its coordinator first polled. The synchronous-install counterpart
survived and finalized. Therefore wrapping only the application initializer does
not cover the start-return interval. These are Linux/Rust 1.98.1 prototype results,
not delivery acceptance; their sources are retained in
`docs/evidence/agent-startup-apis-2026-09-12/README.md`.

Existing `install_signals` checks a name but does not reserve it. An initializer
can register another component with that name before listener transfer. Final
transfer then rejects after resources already exist. The new path reserves the
identity before application work and completes it internally without rejection.

The current startup coordinator checks interruption before invoking its callback.
Storing an already-observed installation error only in that callback would lose
the error if owner loss caused the callback to be skipped. A completed preflight
failure must enter the driver as data, with unused captures destroyed separately.

Tokio 1.53.1 signal handlers are process-global and remain installed after listener
drop. Notifications may coalesce. Neither type ownership nor listener cleanup
restores the old OS disposition or proves exclusive process-wide signal ownership.

## Decision Log


Decision: policy selection is an inert, opt-in builder method returning Self.
Rationale: construction must do no application work or OS installation; a fallible
consuming builder must not drop a supervisor's already-registered finalizers.
Invalid configuration is reported through started ownership and awaited cleanup.
Date/author: 2026-09-12, reviewed planning decision.

Decision: install synchronously at start, then drive reception and cleanup through
the existing startup coordinator. Rationale: this covers the experimentally missed
interval without introducing an application relay or a second lifecycle driver.
Date/author: 2026-09-12, planning agent.

Decision: previously recorded configuration errors win over later interruption;
otherwise initial interruption is checked before validation or installation.
Rationale: preserve observed causes without starting work for already-stopped
applications. A signal observed during initialization produces existing Draining.
Date/author: 2026-09-12, third planning review refinement.

Decision: component reservation is private and distinct from readiness accounting.
Rationale: only a real critical task may contribute an initialization acknowledgement.
A reserved name is not a placeholder running component or a new public capability.
Date/author: 2026-09-12, planning agent.

## Outcomes & Retrospective


Implementation is blocked on `batter-lp2.1`. Do not implement that task's absent
types here. After its completion this task adds signals without changing its
typed owner/observer API. Consumer migration remains `batter-lp2.4`; the SQLx
constructor in `.3` is independent. All process scenarios below remain proposed.

## Context and Orientation


At preparation the workspace HEAD is `39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`.
The future predecessor will add `startup::ScopedStartup`, `ProtectedStartupScope`
and `InitializationError<E>`, reusing `startup/driver.rs`. Its ExecPlan is
`.agent/plans/batter-lp2.1-constrained-startup.md`; the required interface is
repeated here so implementation does not depend on remembered context.

Protected startup is constructed with `Startup::scoped(supervisor, context,
cleanup, initialize)`. Its scope provides stage, direct cleanup reservation and
registration authority, not mutable supervisor or context access. Start returns
`StartingSupervisor<InitializationError<E>>`. The non-exhaustive envelope has
Application(E), Signals(SignalRegistrationError), and SignalPolicyAlreadySelected.
Legacy `Startup::new` and existing public error enums remain unchanged.

`crates/batter/src/lifecycle/unix.rs` owns Tokio's native TERM/INT listeners,
InstalledSignals, `received`, and old explicit registration helpers. Its received
boolean remembers a consumed notification when listeners are later registered.
`lifecycle.rs` validates both ordinary and managed component names and increments
real component accounting. Cleanup slots use a separate namespace.

`startup/driver.rs` owns the coordinator, observer publication, running handoff,
initializer destruction and failure cleanup. `lifecycle/state.rs` owns readiness,
drain and the earliest stop time. Drain stops admission before forced cancellation;
later signals must not reset budgets or overwrite the first cause. A borrowed
waiter is not the owner; dropping the owner requests drain but leaves cleanup
driven on a live runtime. No runtime-death or arbitrary async Drop guarantee exists.

Read root and foundation AGENTS.md plus `test-support/README.md`. Shared
`test-support/process/` sources are private std-only Unix machinery compiled
through each owning suite, not the generic batter-test-support crate. Keep
scenario logic beside foundation tests, and preserve relative include paths.

## Interfaces and Dependencies


Add only this protected public configuration method:

    impl<F> ScopedStartup<F> {
        pub fn with_unix_signals(self, name: &'static str) -> Self;
    }

The returned specification is must-use, like readiness selection. Selecting twice
records SignalPolicyAlreadySelected, even if names are equal. It is not last-wins
and is not a Duplicate component error. Do not add this policy to legacy startup
or change its generic error type. Preserve the predecessor's no-policy typed
owner/observer fixture unchanged and compile it with both policy selections.

Use existing `SignalRegistrationError::Registration(RegistrationError)` for
invalid/occupied names and `Install(io::Error)` for recoverable native failures,
inside InitializationError::Signals. A pre-callback failure uses stage `startup`.
Do not manufacture an application E or require E: From<SignalRegistrationError>.
Fixed formatting must retain original IO for trusted inspection without printing
it. Runtime absence or missing Tokio signal support is a documented native panic
precondition, not an IO failure or new cleanup guarantee.

Add private signal-policy/state code in `crates/batter/src/startup/signals.rs`
and private component-reservation operations in lifecycle.rs or a small owned
submodule. Keep implementation modules within the file-budget policy. A reserved
entry owns only the component name; both ordinary and managed duplicate checks
consult it. Only completing that private reservation with the actual signal task
increments acknowledgement accounting. Do not borrow Supervisor across the
application callback or expose the reservation token publicly.

The native versions remain Tokio 1.53.1 and the current lockfile; no dependency
upgrade or Windows path belongs here. Verify the resolved native source before
changing integration assumptions and record it in docs/references.md.

## Plan of Work


### Milestone 1: inert policy and protected component identity


Confirm `.1` is complete through `br show batter-lp2.1 --json`, inspect its saved
revision/receipts, and run its protected startup and compatibility targets.
If its delivered interface disagrees with the stated contract, report the gap
to the predecessor rather than silently taking over its scope.

Store policy as private configuration on ScopedStartup. Record repeated selection
without constructing listeners or invoking any callback. Dropping any unstarted
specification remains inert with respect to application work and finalization.
Add private name reservation before native installation, consulted by both
Supervisor::register and register_managed. Preserve same-named cleanup hooks as
legal. Internal fulfillment must not repeat fallible validation after startup.

Unit tests should cover empty/invalid names, already occupied ordinary/managed
names, repeated policy, a cleanup hook with the same name, and an initializer
attempting to steal the reserved component name. Independent counters prove no
rejected factory ran and inert captures were released. An unstarted configured
specification must not call the injected installer or initializer.

### Milestone 2: synchronous preflight with independently driven error cleanup


Use this exact start ordering. First retain an already-recorded policy error.
If none exists, check existing drain/cancellation/deadline state and skip all
validation/installation when interrupted. Otherwise validate and reserve the name,
then install TERM and INT synchronously before returning StartingSupervisor.
After any observed preflight error, do not invoke application initialization.
Create the usual owned monitor and cleanup path even on failure.

Extend the shared private driver input to distinguish a callback to execute from
an already-completed failed initialization plus an unused callback capture. This
may be a private enum or shared start argument; do not make a second coordinator.
The failure branch bypasses the driver's initial check that could replace the
observed error. Destroy unused captures under existing tracing/unwind protection,
retain a destruction panic separately, request drain and await prior LIFO hooks.
Owner loss or deadline expiry after start observed the error must not erase it.

Test current-thread start without yielding: pre-register a counted finalizer,
inject an IO marker, call start, retain its observer and drop the owner. Resume
polling and require that same IO cause, no initializer call, one successful hook
and a retained observer report. Repeat with an unused capture whose Drop panics
and with a failed cleanup hook; all causes must coexist. A pre-cancelled no-error
specification must never call the installer; repeated selection followed by
pre-cancellation must preserve the configuration error instead.

### Milestone 3: reception remains owned through every lifecycle transition


Retain native sources outside the initializer future, in the coordinator's owned
startup state. Poll reception alongside initialization. Preserve existing priority
for drain, context cancellation and deadline; then select signal reception before
polling a simultaneously-ready application future. A selected signal requests
drain and reports Draining. If an application poll has actually returned Err,
retain that E through destructor/final checks even if that poll requested drain.
Do not claim an error from an application future that was never polled.

Destroy the initializer before dependency cleanup and keep signal state through
that destruction. On successful initialization, fulfill the reserved component
with the same sources and retained reception state before the final handoff
check. A consumed notification must request drain without a second signal. If
startup fails, no running transfer is needed; drop the sources with their owned
state while retaining the original report. Listener drop does not restore OS
dispositions. The same existing cleanup driver still owns all hook execution.

The real signal component marks started only after actual listener installation
and driver execution. Preserve signal-only supervisor behavior and existing
EmptySupervisor semantics; a finite command must not gain a dummy component.
Readiness approval and native component acknowledgements remain independent.
A pending OS event can be observed after the initializer begins; assert no Ready
after observed drain, not an impossible atomic fence at kernel signal arrival.

Preserve original explicit helpers and InstalledSignals signatures as lower-level
compatibility paths. Refactor shared native construction/reception privately,
without turning the old public install/check behavior into an undocumented new
reservation contract or requiring caller changes.

### Milestone 4: deterministic failures and real Unix subprocess acceptance


Add `crates/batter/tests/startup_signals.rs` with suite-owned child dispatch and
phase assertions under `tests/startup_signals/`. Use the existing private process
owner for bounded launch, capture, signal delivery and reaping. Require a fresh
process for every OS disposition scenario. Default-termination controls require
default TERM/INT dispositions; fail fixture preflight if the launcher inherited
ignored signals rather than treating a surviving control as evidence. Do not
alter the application's inherited signal policy or add non-Unix fallbacks.

For the start-return case, enter a signal-enabled current-thread runtime, register
a counted cleanup, call start, flush `start-returned`, and block on a parent pipe
before yielding the runtime. The parent sends TERM or INT, releases the gate and
then bounds completion. Require survival, one awaited hook and a drain report.
An asynchronous-install negative subject using the same gate must terminate by
the sent signal. Check Unix ExitStatusExt's signal, not merely a nonzero exit.

In separate fresh children, test an unstarted configured specification and a
started specification without policy. TERM/INT must retain default termination
behavior. These are successful negative controls, not library failure cases.
Inside started opted-in children, acknowledge a registered resource before
holding initialization. Send each signal and require no Ready, one finalizer and
Draining. Repeat after dropping only a borrowed waiter and after dropping the
owner while retaining only its observer. Keep the runtime alive for cleanup.

Use private deterministic seams beside unit tests for exact final-poll/handoff
ordering. A gate must prove the notification was consumed and retained before
transfer; a nearby sleep does not prove it. Also use a real running channel
component that answers a request, then joins after either OS signal before its
resource finalizer. Include readiness deliberately withheld by application policy.

Inject failure before any listener and after the first real listener succeeds but
the second fails. Preserve original IO plus cleanup/destruction errors. The latter
case validates partial installation without promising rollback. Test final-poll
application Err plus drain, final-poll success plus drain, and destructor-triggered
drain. Hold cleanup, send repeated signals, then release or exhaust its budget;
assert unchanged original stop time, exactly one hook and truthful completion.

Keep a late-reservation negative subject that lets the initializer claim the
name; it must fail the protected name-reservation oracle. Do not modify the real
acceptance tests to make a negative subject pass. Source-bind both subjects and
oracles. Internal deterministic seams are private, never production test flags.

### Milestone 5: public guidance and complete verification


Add a small compiling signal-enabled protected service example or extend the
predecessor's channel example with explicit opt-in policy. Show application
initialization without install/select/transfer code. Document the synchronous
start boundary, runtime precondition, persistent OS handling, error precedence,
owned cleanup and old helper obligations. Do not migrate the reference/SQLx/HTTP
roots here; that remains `.4`.

Update guarantees, testing, implemented status, validation and primary-source
references, plus foundation guide navigation where needed. Check exhaustive
Jig inputs for all new fixtures/helpers; edit .jig.toml and
.agent/jig-contract.json together if extending their scopes. Keep fmt, contract
and file-budget checks as required siblings, not dependencies of Rust execution.

## Concrete Steps


Work from `/home/aa/Documents/batter`, or the actual workspace root. Record git
status, baseline, predecessor revision and actual rustc/cargo versions. Start
`scripts/jig work start --title 'Implement batter-lp2.2' --body-file
.agent/plans/batter-lp2.2-startup-signals.md --json` and save its returned plan ID.
Run focused commands after the named new target exists:

    cargo test -p batter --test protected_startup --test registration --locked
    cargo test -p batter --test startup_signals --locked
    cargo test -p batter --lib --locked startup
    cargo test -p batter --test startup --test startup_composition --test shutdown_causes --locked
    cargo test -p batter --test scoped_owned_tasks --test scoped_dispatch --locked
    cargo test -p batter --doc --locked

Require per-scenario success, child reaping and absence of watchdog timeout.
The two signals run independently at every required real-process boundary.
Do not count a zero-test filter, ignored scenario or no-run compile as execution.
Then execute:

    RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

On each toolchain rebuild and run the HTTP modes below, setting RUSTUP_TOOLCHAIN
for the build and runner so the tested executable identity is unambiguous:

    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Finally inspect Jig work evidence/gates for the implementation plan, run missing
required gates with `scripts/jig work check --plan-id <id>`, and read them back.
Reuse valid receipts only with unchanged inputs, command/configuration, toolchain,
environment and prerequisites. Use `--freshness-timeout-ms 30000` to inspect
collection-limited evidence, not to rerun checks. Close the implementation plan
and owning Bead only when no required acceptance remains.

## Validation and Acceptance


The supported public initializer contains no signal protocol; real Unix signals
drive cleanup across the start-return, acquired-resource, handoff and running
phases. Configuration or IO failures invoke no application factory, retain their
original cause despite later owner loss, and await prior finalizers. Reservation
prevents ordinary and managed name theft without merging cleanup namespaces or
fabricating readiness. Completed application errors, destruction panic and
cleanup failures remain independently inspectable and redacted by default.

Execute changed Unix behavior on Linux and macOS where available and label actual
platform evidence separately. A Linux result does not prove macOS execution or
hosted CI. No forced second-signal exit, restored signal disposition, shortened
cleanup budget, non-yielding-task bound or runtime-death cleanup is promised.
Record unexecuted platform checks rather than inventing their outcome.

Implementation note, 2026-09-12: the selected API and private reservation are
implemented. Focused signal and injected-error tests pass, followed by complete
`scripts/verify.sh` runs on Rust 1.98.1 and 1.94.0 and five HTTP smoke profiles
per toolchain. Linux x86_64 is the only new signal execution environment; a
fresh child covers first-listener success followed by injected second-listener
failure. macOS and hosted CI remain unexecuted.

## Idempotence and Recovery


Signal tests run only in their own child processes. The parent owns timeout,
termination and reaping, and must retain output when failure itself occurs during
cleanup. Never send process-group signals to the workspace or reuse an unverified
PID. Do not run disposition controls in a process that previously installed
Tokio listeners. Release test gates after assertion failure and retain incomplete
reports; cleanup retries must not rewrite an earlier timeout as success.

No durable database state or provisioning changes belong here. Preserve unrelated
worktree edits and append-only Jig state. Revert only task-owned changes if a
milestone must be backed out. Repeated confirmed example defects trigger the
ADR-010 assessment before dependent repairs: separate implementation errors,
library gaps and upstream/application complexity, record evidence and report the
concern. This task does not authorize unrelated API or native-runtime redesign.

## Artifacts and Notes


Retain each real-signal command, phase acknowledgements, exit signal/status,
typed cause, cleanup records, source/lock identity, toolchain and platform. Mark
injected failure versus actual partial native installation distinctly. Expected
output such as `start-returned` followed by one cleanup event and Draining is a
protocol specification, not an executed transcript. Keep typed `.1` fixtures and
negative controls available to later migrations.

Revision note, 2026-09-12: initial task-local ExecPlan specifies private policy
state, start ordering, completed-error transfer, name accounting and deterministic
process validation while retaining the reviewed public API and delivery boundary.
