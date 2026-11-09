# API assessments: Startup, service completion, maintenance and panic reporting

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to specific changes. They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md).

## Startup handoff assessment (`batter-r4ir`)

The F08 inventory found that `StartingSupervisor::wait(&mut self)` and
`wait_with_output(&mut self)` permit a second completed handoff call, which
panics. This is local library-owned single-use authority, not application policy
or a remote-effect ambiguity. `finish(self)` and `finish_with_output(self)` now
consume that authority into one native future. Rust rejects another handoff call
on the moved owner; unit output alone exposes the unit convenience method.
No borrowed compatibility shim remains because it would preserve the defect.

Cancellation and resumption use a borrow of the same pinned future. Dropping the
future, including before first poll, requests drain just as dropping the original
owner does. The existing coordinator still owns initialization and cleanup;
observers still cannot prolong running ownership or receive typed output. No new
task, channel, state enum, or allocation is added. Polling an already completed
future remains the standard Rust `Future` caller obligation, not a second Batter
handoff operation. Regression scenarios cover pending-borrow cancellation,
unpolled and pending owner abandonment, queued output destruction, and retained
cleanup failures; compile-fail examples cover repeated handoffs and skipped typed
output. Fresh-agent usability evaluation remains unexecuted.

## Bounded metrics setup assessment (`batter-6vn`)

The first metrics API exposed `describe()` beside an application-installed
recorder. Calling it before installation compiled and silently lost every unit
and description, and a recorder built on another `metrics` major version
compiled and silently received nothing. The canonical path is now
`telemetry::metrics::install(recorder)`: it requires a recorder implementing
the re-exported `metrics` 0.24 `Recorder` trait, installs it, and only then
publishes the catalog, so both misuses are rejected at compile time or cannot
be written. A compile-fail rustdoc rejects a non-recorder value, and an
installation test checks descriptions and recording. Recording itself needs no
consumer calls: operations, retries, admission, tasks, cleanup and shutdown
record through library-owned guards with closed label vocabularies.

Installation also owns the catalog target: a local recorder scope cannot redirect
descriptions away from the accepted global recorder. A private forwarding
recorder holds a once-filled shared reference, preserving the upstream `Sync`
bound without adding `Send`. The accepted recorder is retained for the process
lifetime; a rejected recorder is returned intact. Setup fills the reference
before invoking recorder code, and concurrent registrations wait only for that
fill. The cost is one shared slot and a slot read per forwarded recorder call.
This keeps installation on the caller's thread and adds no consumer sequencing
requirement. The installation regression covers local scope dispatch and
rejected-recorder ownership.

An exporter that installs itself globally remains a lower-level escape hatch
that bypasses both checks; the module documentation names that obligation.
On that lower-level recorder path, flushing belongs to the application root
after it awaits supervisor completion; the protected OTLP path below owns it, and the recorder's own buffering, aggregation and faults cannot be
proved by local types. Fresh-agent usability evaluation is proposed and
unexecuted.

## Protected diagnostic completion assessment (`batter-i3ny`)

The reference application duplicated owner/observer monitoring and coordinated
startup failure cleanup, running settlement and final export. Its outer task
could discard an already-known service result if diagnostic finalization panicked.
The supported path now consumes `ScopedStartup` plus an inert prepared diagnostic
adapter through `service::start`. Consumers cannot pass an arbitrary future,
construct diagnostic completion observations, clone the owner, or call a separate
public final flush. Core retains the native result outside the diagnostic task,
then releases the completion observation and joins diagnostics independently.
Unix listeners remain installed before start returns. Read-only observers survive
owner loss; dropped waiters do not initiate shutdown.
The dynamically checked Tokio runtime precondition precedes diagnostic
installation, so an invalid call cannot consume the process-global recorder slot.

The lower-level `Diagnostics` trait allows custom adapter implementations. Its
synchronous install, phase ordering, I/O bounds and termination obligations are
explicit; custom trait forwarding can violate them and is not equivalent to the
supported OTLP path. An opaque witness describes retained reports, including
incomplete coverage, not cessation of unsupervised or remote activity. Application
exit policy, configuration sources and deployment restrictions stay at the root.
The catalog's kinds, units, descriptions and validation now have one core owner;
the opt-in adapter guards complete-key capacity before native bridge allocation.

Independent source review found a public timing hazard: nanosecond intervals could
make iterative missed-tick catch-up block finalization. Catch-up now uses constant-
time arithmetic with a saturating count. Behavioral regressions cover phase
panics, retained startup/cleanup errors, early owner loss and signal installation;
SDK/collector tests and a separate worker composition exercise adapter extraction.
Fresh-agent consumer implementation/modification evaluation remains unexecuted.

## Periodic maintenance assessment (`batter-s6vu`)

A read-only two-consumer survey recorded hand-written drain-aware maintenance
loops in two independently maintained downstream services, repeated maintenance
and progress supervision code, and a closeout review that found one cleanup loop
had been omitted entirely — an omission no type prevented. Those quantities are
historical survey evidence, not a current measurement, and the later
`batter-qjxi` audit withdraws the earlier claim that either service was
deployed. Repeated consumer instructions to coordinate cadence, drain
observation, per-run budgets and error retention are exactly the signal this ADR
names, so the review is on the API shape rather than on more documentation.

The canonical path now makes these states unrepresentable rather than merely
documented. `PeriodicPolicy` is an opaque validated value, so a zero or
unrepresentable interval, run budget or initialization allowance is rejected
before registration instead of panicking at the first tick as the pinned Tokio
`interval` would. The stopping class is a required constructor parameter with
exactly two library-owned variants, so a registration cannot silently default to
the wrong drain behaviour, and there is no application-managed token, join
protocol, dependency graph or third shutdown timer to assemble incorrectly.
Batter owns one directly polled serial future per component, so overlapping
invocations, a detached per-run task and a second scheduler are not reachable
from the protected path. The schedule itself realigns every overdue tick,
including short intervals inside Tokio's five-ms `Skip` tolerance; callers do
not have to select a minimum interval or discard catch-up work. This repairs an
implementation gap against the existing protected contract without narrowing
the accepted timing domain or adding another caller obligation.
Each run receives a library-created `OperationContext`,
so a caller cannot forget a per-run deadline, derive a child that outlives the
run, or reuse an expired protected-startup initializer context. `PeriodicFailure`
converts a propagated application error into the recoverable variant and offers
no conversion into `Fatal`, so an ordinary error cannot implicitly become a
process drain while `?` still works; this is deliberately the mirror image of
`Fatal<E>` for finite tasks, where recurrence does not exist and the safe
default is therefore to refuse to compile. The supervisor retains each
component's bounded history independently of its runner and publishes it in
`ShutdownReport::periodic`, so forgetting to join or poll an observer cannot
lose the evidence, and `PeriodicReader` is a read-only projection that cannot
run, stop, cancel or clear anything.

What the API does not claim matters as much. Registration authorizes recurrence
only: it is not evidence of remote rollback, idempotence, a successful renewal,
fencing, or the absence of a timed-out effect, and lease-loss, freshness and
durable witness transitions stay application- or native-owned. The admission
guarantee is that no run is admitted after its applicable stopping point;
Batter deliberately does not claim that invoking an already admitted callback is
atomic with stop, because a cancellation preflight cannot close that race, and
the limit is stated rather than designed away. A support component that
initialized and then returns success before its own stopping point would be an
early exit, and the periodic API makes that unreachable by construction — the
only representable early end is an explicit escalation, which is retained as a
component failure. Pending or uncooperative native settlement is never treated
as a stopped proof, so conservative cleanup skipping is preserved rather than
replaced by "support stopped, call it clean". Adding `ShutdownReport::periodic`
is a source-compatibility hard cut, recorded in the compatibility notes, because
the report is constructed only by the coordinator and consumers should match it
with `..`. Fresh agent implementation and modification evaluations remain
proposed and unexecuted for this API.

## Typed startup initializer assessment (`batter-syah`)

A read-only two-consumer survey recorded every protected initializer written as
`Box::pin(async move { ... })`, a bound address passed out of startup through a
oneshot channel and `expect`, a separate address announcement in the reference
root, anyhow errors converted to `std::io::Error` six times, and adapter newtypes
that make boxed initializer errors implement `std::error::Error`. Carrying values
out by a side channel and adapting errors so that `?` works are the repeated
caller obligations this ADR names. That survey is historical evidence, not a
current measurement.

The canonical path now removes them. `Startup::protected` takes an owned async
closure, and the library boxes its future, so a caller cannot box incorrectly or
need a channel: the closure's `Ok` value is the typed output. It travels only in
the one-shot handoff. `StartingSupervisor::wait_with_output` returns it with the
running or unapproved owner, observers receive only the supervisor observer, and
every failure variant carries none, so a failed initializer cannot yield output.
Output returned in the same poll that observes drain, cancellation or deadline,
or followed by a destruction panic or a failed final check, is destroyed before
cleanup instead of being delivered. `wait` exists only for unit output, so a
typed value cannot be skipped by an ordinary call, and `service::start` accepts
only unit output rather than silently dropping a value it does not yet deliver.
The output must be `Send + 'static`, so it cannot carry a scope borrow, a
cleanup reservation or a registration view out of startup. A non-`Send` future is
rejected when the specification is built. Stable Rust cannot name an
`AsyncFnOnce` future, so `Initializer` states that bound through a sealed
higher-ranked helper; generic code cannot derive `AsyncFnOnce` from an `FnOnce`
bound, so that helper, not `AsyncFnOnce`, is what `start` and `service::start`
require. Existing boxed closures therefore keep their exact `start` turbofish and
generic bounds.

Errors stay concrete. Rust coherence forbids implementing `std::error::Error` for
`InitializationError<BoxError>` or `InitializationError<anyhow::Error>` beside
the generic `E: Error` implementation, because an upstream crate may add `Error`
for either. `ApplicationError` is therefore a local opt-in type that deliberately
does not implement `Error`. That makes its `From<impl Into<BoxError>>` conversion
coherent and lets `InitializationError<ApplicationError>` implement `Error`
directly, with the retained box as its source. The box is kept unchanged, so a
concrete cause stays downcastable. A raw `BoxError` or `anyhow::Error` initializer
error remains a compile error at the `?` boundary, not a silent loss. anyhow's own
conversion wraps its head error; that limit is documented and pinned by a facade
test rather than hidden.

What the API does not claim: typed output is application data, not a readiness or
liveness witness. Rejected output is destroyed on the coordinator task; an
unclaimed handoff is destroyed wherever the owner or its queued value drops. If both
the future's and the output's destructors panic, the earlier payload is retained.
The later one is released only when it is a known string payload; an opaque
payload is intentionally leaked, following the native containment above, because
its own destructor could panic before cleanup runs. Delivery through `ServiceOwner` belongs to
`batter-m9rp`. `Startup::scoped` and `StartupFuture` remain the lower-level boxed
form, which the PostgreSQL lifecycle and reference roots still use. Fresh-agent
implementation and modification evaluations remain proposed and unexecuted for
this API.

## Finite command signal and owned completion assessment (`batter-od4b`)

A read-only survey of the two downstream consumers in
[ADR-014](../../adr/014-reuse-after-two-consumers.md), on 2026-09-27, found both
hand-installing SIGTERM and SIGINT listeners around `Command` and cancelling the
retained `RunningCommand`, although protected startup already owned that wiring
through `with_unix_signals`. Four call sites, three in one consumer and one in
the other, then used `expect` to reach a value after `check_command` returned the
shared success report, and one consumer bounded its output by `Clone` to copy a
result out of that report. Rebuilt signal wiring, a panic path after a successful
check and a bound on a value only the caller receives are the repeated caller
obligations this ADR names. The same consumer's three signal-cancellable stdout
publishers are the stated non-goal: output publication stays application policy
until a second concrete need appears. The survey is historical evidence, not a
current measurement.

The canonical path now has a sole owner whose consuming `finish` moves the value.
`start_with_unix_signals` and `start_without_signals` return `CommandOwner`, which
has no observer and no borrowed waiter, so nothing else holds the report. `finish`
takes `self`, applies the `CommandReport::is_success` contract through one
destructuring that names every report field, and returns `CommandSuccess<T>`, an
opaque witness: its private fields make success unforgeable, and `into_value`
needs neither `expect` nor `Clone`. A failure stays the existing
`CommandFailure::Report`, where a completed value is reachable only by reference,
so a value from an unsuccessful command cannot pass as checked output. Because the
value is never shared, `T` and `E` need only `Send + 'static`; a `Sync` bound is
unnecessary on the owned path. The command owns its signals: listeners are
installed before the start returns, polled only by the coordinator while work can
be cancelled, mapped to the existing `Interrupted(Cancelled)` on the command's own
context and dropped before cleanup, so no caller wires `select!` around a second
handle. Two explicitly named starts make a root without signals a visible choice.
Whether a call site is the executable root cannot be expressed locally, so the
library cannot choose: one neutral owned start would silently omit signals at
roots, and one signalled start would install process-wide dispositions from
helpers and tests.

This is not a hard cut. No `CommandCause` or `CommandFailure` variant and no
`CommandReport` field was added, so existing exhaustive matches still compile,
and `start`, `RunningCommand`, `CommandObserver` and `check_command` keep their
signatures. Rejected: `Arc::try_unwrap` on the shared report, which fails while
any observer clone survives and so needs a fallback that restores the panic or
the clone; a borrowed `finish(&mut self)`, which leaves a finished owner that can
be finished again; `IntoFuture`, which makes `owner.await` hide that dropping the
future cancels work; a `wait*` name, because every `wait` in this repository is a
non-owning waiter whose cancellation has no effect, while dropping `finish`
cancels work; and a type-state `with_unix_signals` wrapper before one start, which
adds a type and still lets a root omit it silently.

Against the review questions: construction has nothing new to reject, since a
command stays inert and an already-interrupted context is recorded rather than
given listeners; no install, select, cancel, check or extraction ordering remains
with the caller; authority is a consuming owner and an opaque success witness,
not a cloneable shared report plus `expect`; and library-owned execution is the
only ordinary path to the value by move. Compile-fail rustdoc enforces it:
constructing `CommandSuccess` fails with E0451, a second `finish` with E0382,
moving the value out of `CommandFailure::Report` with E0507 and calling
`observer` on `CommandOwner` with E0599, while a discarded owner is rejected
under `deny(unused_must_use)`. What remains is application policy, upstream
behaviour or a deliberate local limit, kept explicit. An installation
`io::Error` returns to the root after the factory is destroyed uninvoked; Tokio
panics outside a runtime or without its signal driver; an already-interrupted
context installs nothing and records the interruption. After work stops,
SIGTERM and SIGINT are consumed without effect and the replaced disposition is
never restored. That combines Tokio behaviour with the application's policy for
a multi-phase root: dropping listeners before cleanup is a deliberate library
limit, not an upstream limit alone, so a later phase that must stop on a signal
selects its own policy. Whether a signal between two signal-owning commands
cancels the second depends on scheduling. Wrapping `finish` in an application
timeout and then leaving the root lets runtime shutdown abandon cleanup; that
root-exit composition is the application's, and the work deadline and cleanup
budget already bound `finish`. Signal identity is not recorded: exit-status
mapping is application policy, and recording it later would be additive.
`RunningCommand` from `start` stays a neutral-looking, unsignalled
shared-observation path for compatibility; its rustdoc and the facade path table
direct a value-receiving root to the owner. A command started inside another
command's work still compiles: an interrupted outer command drops the inner owner
and nothing joins its cleanup. Review caught guidance that recommended
`start_without_signals` for nested commands; the rustdoc, usage and facade table
now direct work to register resources on its own `CommandScope` instead. A
library-owned nested start that registers the inner join as an outer finalizer
would remove this obligation, but no consumer has asked for nesting; it remains
a recorded gap. A helper returning an inert command must spell the higher-ranked
work bound in full, as the example's helper does; a nameable bound would be
additive if consumers repeat it.

Injected-source unit tests, public owner and runtime-loss tests, unclaimed-report
dispatch tests, the fresh-process `command_signals` suite and the example's
real-signal root test are the [executable evidence](../../testing.md#verification-commands);
outcomes belong to the owning Bead. Fresh-agent evaluations are proposed and
unexecuted: give a fresh agent an ordinary operator CLI task that does not
mention signals and check whether it selects `start_with_unix_signals` at the
root, then ask it to add a failure-path test and check that the test uses
`start_without_signals`.

## Redacting panic hook assessment (`batter-4q78`)

[ADR-014](../../adr/014-reuse-after-two-consumers.md#accepted-an-application-installed-redacting-panic-hook)
found all three surveyed consumers installing a process panic hook, with two
failure classes. Two consumers print every payload twice: they log the
formatted `PanicHookInfo`, whose `Display` includes a string payload, and then
call the default hook, which prints it again. And installation is late or
missing: one consumer installs after configuration loading, some modes of
another return before installing, and one operator binary installs nothing.
[Guarantees](../../guarantees.md) and [operations](../../operations.md) already said
that the default hook prints payloads and asked applications to keep credentials
out of panic messages. Care alone cannot follow that instruction: `unwrap` and
`expect` put the `Debug` of the error into the payload without anyone writing a
message, so a credential inside a native error reaches standard error. Under the
review questions, the output policy is something every root reconstructs
(question 2) that a library-constructed value can own (question 4).

The canonical path is `std::panic::set_hook(batter::redacting_panic_hook())`,
unwrapped, as the first statement of every binary's `main`.
`redacting_panic_hook` returns a boxed `Fn(&PanicHookInfo<'_>) + Send + Sync +
'static`, the exact type `set_hook` takes. The closure reads only
`PanicHookInfo::location` and hands file, line and column to a private renderer
with no parameter through which a payload could pass. It never reads, writes or
forwards the payload, calls no previous or default hook, emits no tracing event,
reads no thread state and never takes the `Stderr` lock; it returns normally, so
unwinding, joins, Tokio's `JoinError` and an uncaught panic's exit status are
unchanged. Std itself formats a `panic!` message's arguments into the payload
before any hook runs, so the guarantee is about what the hook writes, not about
formatting. Every checked-in executable root in this repository that consumes
Batter installs it first, except three named tooling roots whose failures must print
their messages, a live-suite preflight, a provisioning harness and a benchmark;
native Runledger and Runlimit roots cannot depend on the facade and keep their
own policy. The
facade's path table lists formatting or chaining hooks, panicking wrappers and
late or partial installation as not equivalent.

This removes the first failure class, formatting and delegation, for any root
that installs the hook. It does not remove the second: installation, its timing
and its omission stay with each binary's `main`. The example roots, the path
table row and the repository's first-statement check are evidence, not
enforcement. The library cannot own that obligation, for four reasons. Std
cannot report the installed hook without replacing it: `take_hook` reinstalls
the default hook, so any Batter check would itself replace the application's
hook and open a window in which the default hook prints payloads, and the atomic
`update_hook` is unstable. A check would come too late anyway, because the
canonical roots read secret-bearing configuration before their first Batter
entrypoint. An `install()` function would remove only the constructor-call
pairing, which the compiler already flags, while `set_hook` stays a
last-writer-wins replacement that no returned witness could keep true. And a
required hook acknowledgement on `Startup::protected`,
`Command::start_with_unix_signals` or `service::start` would be a hard cut that
proves nothing, because a local type cannot prove a process-global effect.

Against the review questions: the constructor takes no input, so nothing is
constructed that execution later rejects (question 1). Rendering, the single
write, descriptor handling and error suppression are library-owned; the call and
its placement remain with the root (question 2). A discarded hook already warns
through the core `Fn` traits' `#[must_use]`; the function's own `#[must_use]`
only gives that warning an actionable message, and a compile-fail rustdoc
example rejects a discarded hook under `deny(unused_must_use)`. Nothing local
detects a missing, late, wrapped or replaced installation. No witness is
offered (question 3), for the reasons above. The rustdoc of the root-only
entrypoints, `Command::start_with_unix_signals`,
`ScopedStartup::with_unix_signals`, `DeferredScopedStartup::with_unix_signals`
and `service::start`, points to the hook, and the visible `main` of the signalled
command and `ExitCode` examples installs it, which is the strongest lever
available (question 4). Rejected alternatives:

- Installing the hook in a library entrypoint, including the root-only signal
  starts. The repository forbids library hook installation, a silent
  replacement would discard an application's own hook, and those entrypoints run
  after configuration loading.
- An opaque hook type, which cannot implement `Fn` on stable and needs `.into()`
  at every root; an `impl Fn` or function-item return, which needs `Box::new`.
  None adds a guarantee.
- A `tracing` event from inside the hook. A subscriber that panics while
  formatting or reporting its own write failure, as tracing-subscriber does with
  `eprintln!` on a closed pipe, makes std abort the process, and `catch_unwind`
  inside a hook cannot contain it. A deferred, payload-free structured record is
  the follow-up `batter-krpp`.
- The thread name. Its only stable source, `thread::current`, panics once the
  thread's handle has been dropped in its final runtime cleanup, which would make
  std abort.
- The `std::io::Stderr` lock, as the writer or as a fallback when descriptor 2
  cannot be duplicated. A thread that holds that lock inside `eprint!` while
  waiting for a lock the panicking thread owns would block the report
  indefinitely; the default hook never takes the lock. At the descriptor limit
  the hook therefore writes nothing.
- A descriptor duplicated once at construction for that fallback. It would keep
  the original standard error open after a daemonizing root redirects or closes
  it, and would point a later report at a stale destination.
- A Clippy `disallowed-methods` ban on `set_hook` as the ownership proof. The
  canonical roots and fixtures must call it, so each would carry a lint
  suppression that consumers copy.

What remains is kept explicit (question 5):

- Application policy: installing the hook first in every binary and mode,
  keeping it installed and unwrapped, never installing it in library code or in
  code that in-process tests run, whose panic messages libtest prints only
  through the default hook apart from a `#[should_panic(expected = ...)]`
  mismatch, and any structured panic record. Whether a call site
  is an executable root cannot be expressed locally, as for
  `start_with_unix_signals`.
- Upstream behaviour: a later `set_hook` replaces the hook, the default hook is
  installed between `take_hook` and the next `set_hook`, and a wrapper or later
  hook that formats `PanicHookInfo` or the payload or calls the default hook
  prints it again; a hook that only restores state and then calls this one keeps
  the guarantee. Std aborts when a panic occurs while it prepares or runs a hook,
  printing the nested panic's message only when it is a constant string, and
  prints the full payload itself in a child that `std::process::Command` forks,
  where it disables hooks before `pre_exec` closures and `exec`.
  `#[tokio::main]` builds its runtime before the first statement of the body.
  Tokio's `JoinError` formats a string payload. Closing the duplicated descriptor
  releases the process-owned POSIX `fcntl` record locks on the file behind standard
  error.
- Unverifiable: that a process installed the hook before its first panic, and
  that logs, error-source traversal or explicit `PanicPayload::try_inspect`
  disclose nothing. Payloads retained in Batter reports keep their explicit
  field access.
- Low-level escape hatch: `std::panic::set_hook` and `take_hook` stay available
  to every crate. The facade table names the default hook, formatting or
  chaining hooks, panicking wrappers and late or partial installation as not
  equivalent.

The [unit tests](../../../crates/batter-core/src/panic_hook/tests.rs)
(`a_located_panic_is_one_line_in_one_write`,
`a_panic_without_a_location_is_still_reported`,
`a_line_longer_than_the_stack_buffer_is_written_whole_in_pieces`,
`the_longest_line_that_fits_still_uses_one_write`,
`write_failures_are_ignored_without_retrying_or_panicking`) pin the rendered
line, the single write and ignored write errors. The fresh-process
[`panic_hook`](../../../crates/batter-core/tests/panic_hook.rs) target
(`default_hook_control_prints_the_payloads`,
`caught_panics_report_each_location_and_no_payload`,
`an_uncaught_panic_ends_the_process_with_the_same_status`,
`tokio_task_panics_unwind_into_join_errors_without_their_payload`,
`a_spawned_thread_panic_reaches_its_join_with_the_payload_intact`,
`the_report_does_not_wait_for_a_held_stderr_lock`,
`at_the_descriptor_limit_the_hook_writes_nothing`,
`batter_entrypoints_keep_the_application_installed_hook`) panics with a
secret-shaped payload in every child except the entrypoints one, carried as a
constant message, a formatted message, an `unwrap`-`Debug` and an opaque value,
and requires standard error to be exactly one redacted line per recorded
location, or empty at the descriptor limit. Default-hook controls show that the
oracle sees a disclosure. In the lock case another thread holds the `Stderr`
lock from before the panic until after the report, so a hook that took it would
block until the parent's guard killed the child. In-child controls show that the
identity check sees a wrapper and the count sees a hook swapped out around a
panic. The
[ownership scan](../../../crates/batter/tests/panic_hook_ownership.rs)
(`no_batter_library_installs_takes_or_replaces_the_panic_hook`,
`the_line_rule_flags_code_and_keeps_comments`,
`only_binary_roots_and_cfg_test_modules_are_exempt`,
`only_files_of_cfg_test_modules_are_test_only`,
`production_modules_the_reader_cannot_place_exempt_nothing`,
`every_batter_consuming_root_installs_the_redacting_hook_first`,
`the_first_statement_rule_reads_main_bodies`) covers every production source
path of every workspace library, meaning every file except those that a
`#[cfg(test)]` module loads and no production module does, and the binary roots
the inventory classifies, not dependencies, and classifies every checked-in
executable root: roots that consume Batter must install the hook first,
three named tooling roots keep the default hook, and native roots, which name no
Batter crate, are out of scope. The
uncaught case exercises libtest's failure status, which equals std's status for
a panic escaping a real `fn main`; that boundary and aborting panics are
source-inspected only, because no repository test aborts a process and a host
crash reporter can still be invoked. Fresh-agent usability evaluation is
proposed and unexecuted: ask a fresh agent to build an HTTP service from the
facade examples, to add an operator command binary, and to add structured panic
logging to an existing service. It passes when the hook is the first statement
of every binary and mode, no hook chains to the default hook, no log formats
`PanicHookInfo` or the payload, no wrapper code can panic inside the hook, and
no test or shared helper installs it.

## Protected service readiness and typed output (`batter-m9rp`)

Evidence: `service::start` accepted only unit output and its owner exposed only
completion and shutdown. Consumer feedback in `batter-m9rp` records application
channels, readiness timers and shutdown coordination required to adopt OTLP.

`service::start_with_readiness` owns this protocol. One optional absolute
readiness deadline is installed before initialization. The existing lifecycle
transition mutex owns first-readiness history and gates Ready at the deadline;
a service-owned timer requests drain. A late acknowledgement cannot briefly
publish Ready before the timer is polled. Readiness already reached is retained
through later shutdown, while current lifecycle status stays separately visible.

`StartingService::into_ready` consumes the sole output capability and yields
`(ServiceOwner, T)` only after readiness. It cannot be called twice on the same
owner. Observers cannot obtain output, approve readiness or request shutdown.
They may cancel and repeat their waits without altering the deadline or owner.
A failed handoff awaits full completion and retains startup/shutdown, diagnostics
and any coordinator-side rejected-output destruction panic independently. Output
queued after readiness remains under its producer dispatch until claimed; no
Clone bound is imposed on application data. The existing unit-output `start`
remains valid without a deadline, so no hard cut or deprecation shim is needed.

The invalid-state review checked late acknowledgement before timer polling,
waiter cancellation, owner loss before first poll, missing readiness, retained
cleanup/diagnostic failures, and unread versus claimed output destruction. The
HTTP example and real-collector OTLP test use the protected path. Independent
consumer-agent implementation/modification evaluation remains unexecuted.

Readiness is historical evidence, not a promise of continuing availability.
The deadline does not bound shutdown/diagnostics or preempt non-yielding work.
Application approval policy, remote effects and runtime survival remain outside
this local protocol. A typed output transferred to the caller has normal Rust
ownership; holding it cannot be claimed to prove resource cleanup or quiescence.


## Direct VictoriaMetrics metrics export (batter-cw5f)

`prepare_victoriametrics` is an additive, explicit backend profile. It validates
endpoint scheme/path/credential exclusions, service identity and a required
nonempty bounded instance identity before native construction. It returns the
same inert non-cloneable `Prepared` owner, so the service lifecycle still owns
installation, periodic work, final collection and closure. There are no new
paired calls, arbitrary future runners or independently selectable response
relaxations. The profile's missing-Content-Type exception accepts only an empty
HTTP 200; nonempty/untyped and wrong-type bodies, malformed Protobuf, partial
rejection, timeouts and oversized responses retain typed failures.

Local syntax cannot establish that two processes chose distinct IDs, that a
proxy preserves resource labels, or that an acknowledged request reached durable
storage. Those remain deployment policy and remote effects, explicitly stated in
rustdoc. The profile supplies `service.name` and `service.instance.id` itself;
it does not ask consumers to rebuild the SDK resource or coordinate flushing.
The existing `prepare` remains valid for standard collectors and has unchanged
public signatures and strict response semantics. No compatibility shim is
needed: no public name or call shape is removed.

Executed evidence: dual-profile failure matrix and two isolated processes whose
cumulative metrics and final lifecycle observations are queried from pinned
Docker-hosted VictoriaMetrics. See [the evidence record](../victoriametrics-2026-11-09.md).
Fresh independent agent implementation/modification evaluations are unexecuted;
no agent-usability claim follows from these tests alone.

## Export-attempt state representation (batter-kwfa)

Assessment date: 2026-11-09. Source: branch head `04766294`; the associated
tooling repair changes no Rust files. This is an investigated design proposal,
not an implemented state transition or an executed usability evaluation.
It refines O02 in the [existing state-machine audit](../implicit-state-machines-2026-11-05.md).
The implementation scope and acceptance contract are tracked in `batter-fzw1`.

### Constructible invalid states and transitions

`pipeline/transport.rs` represents an attempt as `Option<Attempt>`, with
independent `phase` and `outcome` fields. `Evidence` exposes the same product
within the crate. `Session::attempt` currently supplies the correct serial
ordering, but the representation permits the following internal mistakes:

| Mechanism | Representable mistake | Consequence |
| --- | --- | --- |
| `Evidence { phase, outcome }` | `Prepared` with `Some(Ok(()))` | `classify` accepts acknowledgement even though this phase precedes dispatch. |
| `begin(&self)` | Begin while another attempt remains installed | The previous deadline and evidence are overwritten. |
| `finish(&self)` | Finish without beginning, or finish twice | Missing ownership is projected as ordinary prepared/no-outcome evidence. |
| `enter(Phase)` / `record(Result)` | Regress a phase or replace a terminal result | Neither transition validity nor single completion is enforced by the methods. |
| `deadline()` before asynchronous exchange | Two callbacks obtain the same active deadline before either records completion | Permission to send is observed rather than claimed; the slot alone does not prevent duplicate dispatch. |

These are source-established representability problems. No current canonical
consumer failure was reproduced. `Session::attempt(&mut self)` serializes
production exports and the pinned SDK invokes the callback once. Current tests
exercise timeout classification, response failures and no-request-outside-attempt;
they do not establish that arbitrary future crate-internal callers cannot misuse
the slot.

### Upstream boundary

Rechecked the exact resolved sources: `opentelemetry-otlp` 0.31.1 and
`opentelemetry-http` 0.31.0. The SDK encodes the request, then directly awaits one
`HttpClient::send_bytes(&self, request)` call. Encoding can fail before that call.
The callback has no invocation token or exclusive receiver; the SDK converts
client failures into formatted native errors. Therefore the adapter still needs
a narrowly scoped bridge for deadline authority and typed evidence. See the
[versioned primary-source record](../../references/runtime.md#otlp-attempt-callback-boundary-rechecked-2026-11-09).

### Recommended design

Keep the native exporter, encoding and protected service lifecycle. Put attempt
execution behind one private owner method that borrows its driver exclusively:
`export(&mut self, snapshot, deadline) -> ExportOutcome`. That method owns arming,
the SDK future, timeout/destruction, evidence resolution and disarming. Remove
the crate-wide independently callable `begin`/`finish` pair.

Use one explicit per-attempt observation enum instead of the phase/outcome
cross-product. A suitable conceptual shape is:

```rust,ignore
enum AttemptObservation {
    Encoding,
    AwaitingHeaders,
    ReceivingBody,
    Completed(ExportOutcome),
}
```

The observation is read-only outside its owning module. Enum replacement alone
does not enforce legal transitions: do not retain an unrestricted `enter` or
`record` setter. Give the callback a non-cloneable dispatch capability obtained
by atomically claiming an armed gate. Claiming consumes that authorization;
another callback must fail before network I/O. Model the gate explicitly as
closed or armed, separately from observation of an already claimed attempt.

Inside the transport, consume phase capabilities when advancing from request
preparation to sending to receiving, and consume the final capability when
publishing a result. Only successful bounded body validation can produce the
acknowledgement evidence. Keep constructors and terminal publication private;
the observation enum must not become a new writable crate-wide status field.

Each attempt owns a distinct observation identity. A consumed or dropped guard
seals that identity and removes any unclaimed authorization. An old completion
cannot publish into a later attempt. Synchronous destruction may revoke local
permission and retain interruption evidence; it must not claim asynchronous
cleanup, remote rollback or stopped server-side work.

Retain existing observable outcomes: encoding failure before a callback,
headers/body timeout distinction, precise typed transport failures, and completed
transport evidence winning the outer deadline race. Do not collapse all missing
acknowledgements into a generic transport error. Public types and call shapes
need not change, and neither dependency upgrades nor new runtime tasks are needed.

The SDK trait cannot express our linear ownership statically. Its callback
admission therefore needs a checked atomic transition. The protected local owner
and consuming capabilities can prevent ordinary internal misuse; they cannot
identify an arbitrary detached callback from an altered upstream exporter, whose
request carries no attempt token. The verified inline SDK behavior remains an
explicit boundary that dependency upgrades must recheck.

An enum-only patch would remove invalid field combinations while retaining the
paired-call and duplicate-dispatch protocol. Replacing the SDK exporter with
hand-built OTLP transport would expand ownership into native encoding and closure
semantics. Neither is the recommended correction.

### Required evidence for a later implementation

Prove that duplicate/concurrent callback claims dispatch at most one request,
unarmed/closed calls dispatch none, and an old attempt cannot change a new result.
Exercise cancellation before first poll and during encoding, headers and body;
then start the next attempt and verify it has a fresh deadline and observation.
Retain typed-result/deadline-race controls and the dual-profile response matrix.
Use compile-fail or module-privacy checks for reused consumed capabilities and
fabricated acknowledgement where the design makes those errors static.
Run the full pinned `scripts/verify.sh`, including actual VictoriaMetrics Docker
ingestion/read-back. These additional state-transition controls are proposed and
unexecuted until the Rust implementation exists.
