# Implicit state machines in Batter

Source snapshot: `316e721bd52b58219d5ef213da9000532a0df520`, inspected on
2026-11-05. Owning audit: `batter-rffr`.

Follow-up: F08 is addressed by the consuming startup handoff in `batter-r4ir`;
see its [implementation and API evidence](startup-handoff-2026-11-05.md). The
inventory below describes the original snapshot, not a list of unresolved defects.
F19's capability/deadline representation is addressed by `batter-u4lj`: a private
`ComponentState` owns either initializing or running authority. The serial run
loop and its outcome handling remain explicit control-flow protocols.

The inventory contains 89 machine families. The largest concentrations are
lifecycle admission and shutdown, native task settlement, database lease
protocols, and outcome retention. Many are private
implementations behind consuming public APIs. Their presence does not establish
a defect or imply that each should become one enum.

This inventory treats a machine as a protocol whose permitted next action depends
on previous actions or retained phase information. It includes machines encoded
by flags, optional owners, counters, collection membership, timestamps, channels,
SQL predicates, and ordered control flow. Related implementations are grouped in
one row. The representations are:

- **Implicit:** phase is inferred from data rather than a dedicated state type.
- **Mixed:** explicit states exist, but additional fields or control flow carry
  essential substates or transition obligations.
- **Protocol:** the program counter and ownership of local values encode the
  sequence; there is no enduring aggregate phase value.

The search covered Rust packages, native Runledger and Runlimit, examples,
external consumers, SQL, scripts, and shared test infrastructure. It followed
candidate mutations and transitions, rather than treating every `Option` or
boolean as a state machine. Immutable configuration, simple diagnostic counters,
ordinary collection iteration, and the compiler-generated machine of every
`async fn` are excluded. Test-specific copies of a production protocol and simple
test synchronization flags are grouped with that protocol rather than counted
individually. This is a source audit, not a formal proof of exhaustive discovery
or runtime correctness. No behavioral tests were executed for the audit.

The most concrete caller-visible issue is
[`StartingSupervisor::wait_with_output`](../../crates/batter-core/src/startup/handoff.rs)
(lines 100–128): the first completed wait consumes a one-shot result, but leaves
the same `&mut self` API callable. Its rustdoc explicitly says a second completed
wait panics. A cancelled pending wait is resumable; a completed wait is not.
The distinction is runtime state that the public type does not express. This was
confirmed from source and rustdoc, not by executing a new reproduction.

`OperationPhases` is another caller-visible sequencing obligation: its two
contexts encode deadline and cancellation relationships, but both accessors are
available immediately and repeatedly. The documented work → finalization order
and explicit awaiting remain caller-owned (F25). Naming the contexts does not
make this a type-state transition.

Three particularly clear internal representations are the periodic driver's
`pending`/`running` pair, `FixtureRun`'s `driver`/`result` pair, and OTLP's
`Option<Attempt>` plus `phase` and optional `outcome`. Their legal combinations
depend on the owning code. The TUI's requested/in-flight/applied generations are
another substantial implicit protocol. These are assessment candidates, not
demonstrated externally reachable failures. Lifecycle's orthogonal flags should
not be mechanically flattened into one enormous enum.

The foundation machines are below. Source references identify the owning
implementation; line numbers are for the snapshot above.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| F01 | Readiness and admission | Mixed. `Readiness` combines with `driver_started`, `ready_requested`, `pending_startups`, `forced`, `failed`, `support_closed`, and `finite_active`. Registration and acknowledgements permit readiness; drain closes root admission; support stops later; forced cancellation closes remaining admission. One mutex owns transitions. | [lifecycle/state.rs](../../crates/batter-core/src/lifecycle/state.rs), line 42 |
| F02 | Shutdown escalation | Protocol. Start tasks → observe stop cause → drain → cancel → harvest → abort unfinished work → reap → freeze evidence → run or skip cleanup → publish stopped. Elapsed phase allowances depend on the earliest retained stop time. | [lifecycle.rs](../../crates/batter-core/src/lifecycle.rs), line 618 |
| F03 | Direct-task accounting | Implicit. Join-set and metadata membership distinguish owned, finished-but-unobserved, observed, abort-requested, and unjoined tasks. `unsafe_ordinary_exit` is a sticky restriction on support stopping and cleanup. | [lifecycle/tasks.rs](../../crates/batter-core/src/lifecycle/tasks.rs), line 45 |
| F04 | Supervisor and driver ownership | Implicit internally. `ownership: Option<SupervisorOwnership>` transfers once into the driver. Emergency cancellation is armed before first poll and executes on destruction. Public unapproved/approved ownership is explicitly typed. | [lifecycle.rs](../../crates/batter-core/src/lifecycle.rs), line 238; [caller_owned.rs](../../crates/batter-core/src/lifecycle/caller_owned.rs), line 9 |
| F05 | Finite-task admission and lifetime | Implicit. Admission count, queue membership, `ProcessStart`'s optional one-shot sender/lease, and `ActiveTask.active` represent accepted → queued → started → finished. Descendant admission requires a still-active ancestor. | [lifecycle/process.rs](../../crates/batter-core/src/lifecycle/process.rs), lines 316, 411; [state.rs](../../crates/batter-core/src/lifecycle/state.rs), line 308 |
| F06 | Managed-component coordination | Mixed. Optional initialization future/start token, `settled`, explicit initialization outcome, optional settlement, and `finished` combine initialization, acknowledgement, early stop, stop propagation, settlement, and publication. Stop-clock acknowledgement may subsequently tighten. | [managed/drive.rs](../../crates/batter-core/src/lifecycle/managed/drive.rs), line 61; [managed/report.rs](../../crates/batter-core/src/lifecycle/managed/report.rs), line 153 |
| F07 | Startup execution and failure cleanup | Protocol with explicit initial callback/failure variants. Install selected signals → invoke initializer → destroy initializer → recheck boundary → transfer signals and supervisor, or destroy output and drive failure cleanup. | [startup/driver.rs](../../crates/batter-core/src/startup/driver.rs), lines 111, 234 |
| F08 | Startup owner handoff | Implicit, public temporal obligation. `handle: Some` plus pending receiver → one completed handoff and `handle: None`; pending waiter cancellation preserves the owner. Repeating a completed wait panics. | [startup/handoff.rs](../../crates/batter-core/src/startup/handoff.rs), line 89 |
| F09 | Signal-policy selection | Implicit. `selected: None` → selected once → sticky invalid repeated selection. Repeated configuration is rejected when startup consumes the selection. | [startup/signals.rs](../../crates/batter-core/src/startup/signals.rs), line 2 |
| F10 | Installed Unix signal latch | Implicit. `received: false` → true on either listener becoming ready; subsequent waits stay ready, and later registration retains the observed stop request. | [lifecycle/unix.rs](../../crates/batter-core/src/lifecycle/unix.rs), line 94 |
| F11 | Retained completion publication | Implicit. Watch channel `None` → `Some(outcome)`; channel closure without an outcome is a distinct failure. Shared by lifecycle, startup, command, and service observers; waiter cancellation does not own execution. | [completion.rs](../../crates/batter-core/src/completion.rs), line 3; [lifecycle/driver.rs](../../crates/batter-core/src/lifecycle/driver.rs), line 215 |
| F12 | Finite-command execution | Protocol. Preflight → work polling → returned/interrupted/panicked work → separately observed destruction → remove signal listeners → cancel work scope → owned cleanup → retained report. `interruption_after_work` is separate from the work result. | [command/driver.rs](../../crates/batter-core/src/command/driver.rs), lines 140, 172 |
| F13 | Service and diagnostic completion | Protocol. Install diagnostics → start service → retain service result → publish coverage witness → await diagnostics → publish combined completion. Diagnostic failure must not overwrite service evidence. | [service/driver.rs](../../crates/batter-core/src/service/driver.rs), line 115 |
| F14 | Operation boundary | Protocol. Preflight → invoke factory in child scope → select work/cancellation/deadline → resolve retained outcome → finish observation → destroy scope and cancel descendants. Unpolled and refused factories remain uninvoked. | [operation.rs](../../crates/batter-core/src/operation.rs), line 428 |
| F15 | Retry execution | Mixed. Attempt counter, retained last error, replay/classification decisions, backoff clock, and explicit attempt results drive attempt → classify → stop or wait → next attempt. Initial attempt consumes the same total budget. | [retry.rs](../../crates/batter-core/src/retry.rs), line 647 |
| F16 | Blocking-call delivery | Implicit. `BlockingCall.work: Some` → taken/run or destroyed unstarted; `Delivered.outcome: Some` → claimed or destroyed unobserved. The permit remains owned through closure destruction, independently of waiter lifetime. | [admission/blocking.rs](../../crates/batter-core/src/admission/blocking.rs), lines 335, 413 |
| F17 | Cleanup stack and individual finalizers | Mixed. Stack membership → popped/running → joined or timed out → abort/reap → recorded or unjoined; an unjoined hook prevents dependent cleanup. `PendingCleanupObservation.observed` distinguishes recorded from abandoned execution. | [cleanup.rs](../../crates/batter-core/src/cleanup.rs), lines 323, 434 |
| F18 | Health publication and freshness | Mixed. `alive` plus optional last probe and its age represent unobserved, fresh healthy/unhealthy, stale, and writer stopped. Explicit snapshot enums are projections of those retained facts. | [health/observation.rs](../../crates/batter-core/src/health/observation.rs), lines 160, 253 |
| F19 | Periodic initialization and execution | Mixed. `pending: Option<ComponentStartup>` and `running: Option<RunningComponent>` encode initializing versus acknowledged. The loop alternates waiting and one active run; first success may acknowledge; drain, expiry, and fatal failure select terminal paths. | [periodic/driver.rs](../../crates/batter-core/src/periodic/driver.rs), lines 95, 273 |
| F20 | Periodic cadence | Implicit. `next: Some(due)` → delivered tick and next future cadence point; arithmetic exhaustion produces `None`, an interruptible permanently parked schedule. | [periodic/schedule.rs](../../crates/batter-core/src/periodic/schedule.rs), line 6 |
| F21 | Periodic evidence retention | Mixed. First/last failure slots encode no sample → first sample → subsequent sample/replacement. Counters saturate, acknowledgement latches, and completion is published then reconciled with join evidence. | [periodic/history.rs](../../crates/batter-core/src/periodic/history.rs), lines 231, 245 |
| F22 | Observation completion guards | Implicit or mixed. `Terminal.armed` permits exactly one finish/drop record for retry, attempts, bulkhead admission, and shutdown; `Observation.outcome` starts at `Dropped` and changes on completion. Cleanup has its own observation guard in F17. | [metrics/terminal.rs](../../crates/batter-core/src/telemetry/metrics/terminal.rs), line 31; [telemetry.rs](../../crates/batter-core/src/telemetry.rs), line 119 |
| F23 | Recorder and metric-name publication | Implicit. Recorder installation has publication-before-slot-fill and ready phases using `OnceLock`. Name slots progress empty → name published → hash published; zero hash permits a string-comparison fallback during publication. | [metrics/installation.rs](../../crates/batter-core/src/telemetry/metrics/installation.rs), line 6; [metrics/names.rs](../../crates/batter-core/src/telemetry/metrics/names.rs), line 87 |
| F24 | Scoped destruction ownership | Implicit. `ScopedDispatch.inner` and `ScopedOutput.value` are present until destruction or transfer. Taking/clearing them prevents duplicate disposal and preserves the captured tracing dispatch through destruction. | [scoped_dispatch.rs](../../crates/batter-core/src/scoped_dispatch.rs), lines 13, 67 |
| F25 | Caller-owned work/finalization sequencing | Protocol, public temporal obligation. Reserve deadline → execute work → observe result → explicitly await finalization. `OperationPhases` exposes both sibling contexts concurrently; `cancel_work` latches work cancellation but does not transition into a distinct finalization type or execute cleanup. Deadline relationships are enforced; this sequencing remains in consumer control flow. | [operation.rs](../../crates/batter-core/src/operation.rs), lines 73, 88, 94, 177 |

HTTP and outbound transport have the following additional machines. Public
request/response outcome enums do not encode all of the live transport phases.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| H01 | HTTP middleware ownership and admission | Implicit/protocol. Private request-extension markers establish operational and observation ownership; admission installs scope and responder metadata before handler execution. Nested middleware checks existing ownership. Protected boundary assembly fixes ordering. | [correlation.rs](../../crates/batter-axum/src/correlation.rs); [low_level.rs](../../crates/batter-axum/src/low_level.rs), line 260; [observation.rs](../../crates/batter-axum/src/observation.rs), line 69 |
| H02 | HTTP response observation | Implicit. `status: None` → response status, with optional level; destruction records dropped versus completed/error response. This boundary ends at the response, not at completion of a streaming body. | [observation.rs](../../crates/batter-axum/src/observation.rs), line 89 |
| H03 | Quota observation authority | Mixed. Shared `writer_taken` permits one irreversible claim; explicit recorder types encode unstarted → started → terminal facts. Dropping started ownership retains unresolved consumption. | [quota_observation.rs](../../crates/batter-axum/src/quota_observation.rs), line 91 |
| H04 | Authentication and quota-before-handler | Mixed. Authenticate → build checks → quota started → quota decided/failed → handler or rejection. The HTTP adapter's `unstarted`/`started` options rely on the quota callback ordering. | [http.rs](../../crates/batter-runlimit/src/http.rs), line 493; [quota.rs](../../crates/batter-runlimit/src/quota.rs), line 284 |
| H05 | Request-draft validity | Implicit. Body absent/present combines with a sticky first error. A second body or forbidden header poisons preparation. `prepare` consumes the draft into an explicit send-once owner or error. | [request.rs](../../crates/batter-outbound-http/src/request.rs), line 43 |
| H06 | Outbound exchange phases | Protocol/mixed. Preflight → DNS/TCP/TLS/HTTP setup → immediate handoff gate → request submitted → headers → bounded body → exhaustive exchange. `driver: Some` → `None` prevents polling a finished connection while buffered response data remains usable. Stopping before versus after handoff changes the evidence. | [send.rs](../../crates/batter-outbound-http/src/send.rs), lines 37, 150, 217; [transport.rs](../../crates/batter-outbound-http/src/transport.rs), line 21 |
| H07 | Address fallback | Protocol. Preferred family alone → delayed or immediate alternate → at most two family candidates → winner with loser dropped; within each family, failures/time slices advance the address index. | [transport/dial.rs](../../crates/batter-outbound-http/src/transport/dial.rs), line 8 |

The SQLx adapter owns the following remote-session protocols. Local state can
restrict calls, but cannot establish whether a lost remote commit took effect.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| Q01 | Checked-out connection disposition | Implicit behind a consuming API. `PgLease.connection: Some` defaults to retirement; successful completion plus pool-return proof takes it for reuse. Every other destruction detaches it. | [lib.rs](../../crates/batter-sqlx/src/lib.rs), lines 136, 479; [session.rs](../../crates/batter-sqlx/src/session.rs), line 52 |
| Q02 | Session and transaction continuity | Protocol. Normalize session → apply/validate profile → begin transaction → establish identity/savepoint → application/native query → revalidate → release/rollback savepoint → commit/rollback. Phase correctness is partly encoded by consuming owners and opaque SQL capabilities, partly by server checks and function order. | [atomic.rs](../../crates/batter-sqlx/src/atomic.rs), lines 101, 163; [atomic/scope.rs](../../crates/batter-sqlx/src/atomic/scope.rs), line 60; [profile.rs](../../crates/batter-sqlx/src/profile.rs) |
| Q03 | Atomic outcome retention across interruption | Implicit. Private mutex slot empty → completed atomic result retained → resolver takes it. The retained result can win over interruption observed after acknowledgement. | [atomic_context.rs](../../crates/batter-sqlx/src/atomic_context.rs), line 82 |
| Q04 | Read-only snapshot lifecycle | Protocol. Acquire/normalize/profile → begin repeatable-read, read-only transaction → capture identity → callback → validate continuity → end transaction and classify. | [snapshot.rs](../../crates/batter-sqlx/src/snapshot.rs), line 92 |
| Q05 | Managed schema installation | Protocol. Hold session advisory lock → migrations → managed-object transaction/ledger lock → retire/create/record → application hook → commit → unlock → prove reuse or retire. Cancellation boundaries differ before and after commit. | [managed/apply.rs](../../crates/batter-sqlx/src/managed/apply.rs), lines 196, 259; [managed/application.rs](../../crates/batter-sqlx/src/managed/application.rs) |
| Q06 | Managed trigger lock acquisition | Mixed. `targets`, `busy`, round count, savepoint, and explicit retry outcomes encode try set → rollback partial locks → wait for one refused relation → retry set → revalidate partition tree → accept or exhaust. | [managed/apply/drops.rs](../../crates/batter-sqlx/src/managed/apply/drops.rs), line 271 |
| Q07 | Managed-object reconciliation | Implicit/mixed. Desired entries, optional observed kind, `touched`, retained identities, and retired sets determine unchanged, replace, remove, collision, and post-drop reinspection paths before ledger publication. | [managed/catalog.rs](../../crates/batter-sqlx/src/managed/catalog.rs), line 275; [managed/apply.rs](../../crates/batter-sqlx/src/managed/apply.rs), line 302 |
| Q08 | Verification snapshot and protected ledger cursor | Protocol. Begin inspection transaction → inspect/lock ledger identity → open cursor under savepoint → validate held relation locks → read bounded history/catalog → evaluate → settle snapshot. Cursor declaration failures have rollback/classification paths. | [verification/executor.rs](../../crates/batter-sqlx/src/verification/executor.rs), line 79; [migration/portal.rs](../../crates/batter-sqlx/src/verification/migration/portal.rs), line 12 |

OTLP adds separate execution, transport, and recorder lifetimes.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| O01 | Export-session lifetime | Protocol. Install resources → serial periodic attempts → service-completion signal → settle current attempt → final snapshot/export → consume resources and close exporter/provider. Installation rejection follows a separate closure path. | [lib.rs](../../crates/batter-otlp/src/lib.rs), lines 94, 306, 332, 371 |
| O02 | Export transport evidence slot | Mixed. `None` means no attempt; `Some(Attempt)` combines `Prepared/Sending/Receiving` with `outcome: None/Some`. `begin` opens the slot, transport updates it, and `finish` takes it. Serial ownership enforces ordering. | [pipeline/transport.rs](../../crates/batter-otlp/src/pipeline/transport.rs), line 43 |
| O03 | Recorder closure and delegated calls | Implicit. `closed` and `active` encode open-idle/open-busy → closing with calls outstanding → closed-idle. Close rejects new calls and waits for active guards to release. | [pipeline/guard.rs](../../crates/batter-otlp/src/pipeline/guard.rs), lines 50, 102 |
| O04 | Metric-series admission | Implicit. Absent key → admitted key; full registry rejects new keys while retaining existing ones. The key set and capacity enforce this monotonic transition under the registration lock. | [pipeline/guard.rs](../../crates/batter-otlp/src/pipeline/guard.rs), line 123 |

Native Runledger contains additional machines independent of the Batter facade.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| R01 | Invocation exit and waiter registration | Implicit behind separate owner/observer types. `ended` latches; hooks/waiters accumulate until end drains them. Each waiter's optional ID encodes unregistered → registered/replaced → removed or completed. | [core/jobs/invocation.rs](../../runledger/runledger-core/src/jobs/invocation.rs), lines 19, 215; [runtime/worker/invocation.rs](../../runledger/runledger-runtime/src/worker/invocation.rs), line 23 |
| R02 | Loop-start acknowledgement | Mixed. Explicit `RuntimeStartup` contains a pending-name collection; `LoopStartup.name: Some/None` permits acknowledgement once. Dropping a loop token stops initialization even after acknowledgement. | [runtime/startup.rs](../../runledger/runledger-runtime/src/startup.rs), lines 61, 94 |
| R03 | Shutdown cause and clock arbitration | Implicit/mixed. Optional first request, retained signal error/panic, watch boolean, and signal `initiating` flag distinguish unrequested, winning signal, later signal evidence, and tightened stop time. First cause is retained while the clock can move earlier. | [runtime/shutdown.rs](../../runledger/runledger-runtime/src/shutdown.rs), line 14; [shutdown_signal.rs](../../runledger/runledger-runtime/src/shutdown_signal.rs), line 96 |
| R04 | Signal-future lifetime | Implicit internally, explicit submission types. Optional raw future and one-use output mapper encode unsubmitted → supervised/pollable → trigger published → separately destroyed. Poll and destructor panics have different retention paths. | [shutdown_signal/owner.rs](../../runledger/runledger-runtime/src/shutdown_signal/owner.rs), lines 21, 27 |
| R05 | Supervisor loop settlement | Mixed. `tasks`, `pending`, `records`, optional shutdown-budget decision, and per-loop `aborted` encode running → graceful wait → abort wait → observed/unjoined or owner interrupted. Runtime-task `started` separately latches first poll. | [runtime/task_group.rs](../../runledger/runledger-runtime/src/task_group.rs), line 15; [task_group/report.rs](../../runledger/runledger-runtime/src/task_group/report.rs), line 201 |
| R06 | Descendant registry and wake queue | Mixed. Entry membership, `aborted`, explicit abort mode/kind, and `EntryWake.queued` govern register-before-start → active → notified/queued → polled → observed/removed. Escalation also applies to newly arriving descendants. | [runtime/settlement.rs](../../runledger/runledger-runtime/src/settlement.rs), lines 112, 210, 241, 424 |
| R07 | Settlement report delivery | Implicit. `SettlementOwner.sender: Some` means report owed; taking it transfers an `UnobservedReport(Some(report))`; observation takes the report, while abandonment emits its diagnostic. Owner destruction synthesizes interrupted evidence. | [supervisor/driver.rs](../../runledger/runledger-runtime/src/supervisor/driver.rs), lines 95, 133 |
| R08 | Callback destruction guard | Implicit. Optional callback future is polled while present, then taken before destruction on ready/panic/drop. This prevents a second disposal and retains a secondary destructor panic independently. | [runtime/callback.rs](../../runledger/runledger-runtime/src/callback.rs), line 21 |
| R09 | Claimed-job execution and heartbeat | Protocol/implicit. Claimed → mark running → handler plus at most one pending heartbeat → handler result/timeout/lease-maintenance loss → end invocation → persist outcome → notify. Pending-heartbeat collection emptiness controls new heartbeat admission. | [worker/execution.rs](../../runledger/runledger-runtime/src/worker/execution.rs), lines 84, 290, 310 |
| R10 | Lease-loss latch | Implicit. `lease_lost: false` → true after a fenced progress write fails with owner mismatch; subsequent writes refuse and the handler driver is notified. Deadline expiry is a separate boundary. | [worker/execution_services.rs](../../runledger/runledger-runtime/src/worker/execution_services.rs), line 17 |
| R11 | Running-before-terminal observers | Implicit. Optional running-observer handle is retained, drained, transferred into terminal notification, or aborted on drop. Join guard's `completed` distinguishes observed completion from abandonment. | [worker/observers.rs](../../runledger/runledger-runtime/src/worker/observers.rs), lines 495, 573, 615 |
| R12 | Reaper hook admission during shutdown | Implicit. `shutdown_observed` latches once; a bounded post-shutdown admission budget is granted once and decremented. In-flight membership and metadata track completion/skipping. | [reaper/terminal_hooks.rs](../../runledger/runledger-runtime/src/reaper/terminal_hooks.rs), line 192 |

Persisted Runledger state has explicit status enums, but several important
substates and transition obligations live in other columns and SQL predicates.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| D01 | Job lease and attempt execution | Mixed. `JobStatus` alone omits owner, expiry, claim origin, run/attempt identity, persisted execution-start marker, heartbeat, and resume state. Claim, start, renew, release-unstarted, retry, continuation, reaping, and replay update coupled facts. | [queue/claim.rs](../../runledger/runledger-postgres/src/jobs/queue/claim.rs), line 156; [queue/lifecycle](../../runledger/runledger-postgres/src/jobs/queue/lifecycle); [queue/reaper.rs](../../runledger/runledger-postgres/src/jobs/queue/reaper.rs), line 589; [queue/advance.rs](../../runledger/runledger-postgres/src/jobs/queue/advance.rs), line 10 |
| D02 | Execution-resource exclusion | Implicit persisted ownership. Claim-table membership plus lease identity/expiry encode available → claimed → renewed or retained while cancellation remains live → released/expired. Claims are acquired/released alongside job lifecycle transitions. | [queue/claim.rs](../../runledger/runledger-postgres/src/jobs/queue/claim.rs), line 315; [queue/lifecycle/heartbeat.rs](../../runledger/runledger-postgres/src/jobs/queue/lifecycle/heartbeat.rs); [queue/release.rs](../../runledger/runledger-postgres/src/jobs/queue/release.rs) |
| D03 | Enqueue-intent promotion and retention | Mixed. Explicit intent status combines with promotion-attempt count, next-attempt time, promoted job identity, and error/timestamp fields. Promotion savepoint → acknowledge/conflict/defer; retention takes its own fence and job locks before deletion. | [queue/intents.rs](../../runledger/runledger-postgres/src/jobs/queue/intents.rs), lines 921, 1151, 1209, 1336 |
| D04 | Workflow dependency release and terminal propagation | Mixed. Step/run status combines with pending/unsatisfied dependency counts, job linkage, release/start/finish times, and the propagation queue. Terminal prerequisite updates may release dependents or cancel them, then recompute the run. | [workflows/runtime/terminal.rs](../../runledger/runledger-postgres/src/jobs/workflows/runtime/terminal.rs), lines 450, 555; [runtime/run_status.rs](../../runledger/runledger-postgres/src/jobs/workflows/runtime/run_status.rs), line 35; [workflows/release.rs](../../runledger/runledger-postgres/src/jobs/workflows/release.rs) |
| D05 | Workflow active-key ownership | Implicit. Claim row absent → owned with `release_pending=false` → deferred release while canceled jobs retain live leases → row deleted after quiescence. Terminal run status alone cannot authorize release. | [workflows/active_claims.rs](../../runledger/runledger-postgres/src/jobs/workflows/active_claims.rs), lines 60, 85, 157 |
| D06 | Schedule eligibility and fire clock | Implicit/mixed. `is_active`, enabled definition, `next_fire_at`, and `last_fired_at` determine disabled/not due/due/claimed/fired. Row locks and transactional enqueue coordinate advancing the clock; runtime catch-up policy determines the next fire. | [schedules/claim.rs](../../runledger/runledger-postgres/src/jobs/schedules/claim.rs), lines 28, 91; [schedule_definition_guard.rs](../../runledger/runledger-postgres/src/jobs/schedule_definition_guard.rs), line 99; [runtime/scheduler.rs](../../runledger/runledger-runtime/src/scheduler.rs) |
| D07 | Workflow result listener | Mixed. Initial probe → optional listener connection/subscription → second probe → notification or polling backstop → final deadline probe. `listener: Option<PgListener>` and absolute next-poll time carry live substates. | [workflows/handles.rs](../../runledger/runledger-postgres/src/jobs/workflows/handles.rs), lines 127, 204 |
| D08 | Online migration and index repair | Mixed/protocol. Explicit index classification is combined with recorded/pending history, migration-lock ownership, optional invalid OID, and saved session timeouts. Inspect → lock/repair → lift timeouts → concurrent build → validate → restore → release lock. | [migrations/online.rs](../../runledger/runledger-postgres/src/migrations/online.rs), lines 135, 347; [online/repair.rs](../../runledger/runledger-postgres/src/migrations/online/repair.rs); [online/session.rs](../../runledger/runledger-postgres/src/migrations/online/session.rs), line 15; [online/lock.rs](../../runledger/runledger-postgres/src/migrations/online/lock.rs), line 31 |

Quota state is mostly encoded numerically or by lease presence, rather than by
a named machine enum.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| L01 | Fixed-window quota counters | Implicit. Missing/expired entry → active counter → partially or fully consumed → expired/reclaimed. Usage, expiry, capacity slots, and observed time control admission. Memory and PostgreSQL implement the same policy with different storage protocols. | [memory/store.rs](../../runlimit/runlimit-memory/src/store.rs), line 24; [postgres/admission.rs](../../runlimit/runlimit-postgres/src/admission.rs) |
| L02 | GCRA quota debt | Implicit. Optional theoretical-arrival time and wall/monotonic clock encode replenished versus indebted quota; allowed evaluation produces a provisional new arrival time that storage commits after batch preflight. | [core/gcra.rs](../../runlimit/runlimit-core/src/gcra.rs), line 46; [memory/gcra.rs](../../runlimit/runlimit-memory/src/gcra.rs); [postgres/gcra.rs](../../runlimit/runlimit-postgres/src/gcra.rs) |
| L03 | Memory shard expiry and poison recovery | Implicit. Entries and expiration index must transition together; time never regresses. Mutex poisoning closes the shard until explicit recovery rebuilds it and clears poison. Attempt-store cleanup also uses a wrapping scan cursor. | [memory/shards.rs](../../runlimit/runlimit-memory/src/shards.rs), lines 62, 119, 147, 371; [memory/attempts.rs](../../runlimit/runlimit-memory/src/attempts.rs), line 253 |
| L04 | Memory attempt lease and backoff | Implicit. Lease absent/present, receipt generation, failures, last failure, and retry time encode eligible → leased → success/removal or failure/backoff; expired lease counts as failure; quiet period resets failures. Stale receipts cannot complete a newer lease. | [memory/attempts.rs](../../runlimit/runlimit-memory/src/attempts.rs), lines 20, 109, 168, 216 |
| L05 | PostgreSQL attempt lease and transactional completion | Implicit with explicit receipt/claim types. Admission creates token/expiry; claim rotates token and binds transaction identity; completion checks the appropriate fence, clears lease, and sets backoff. Quiet expiry permits cleanup. | [postgres/attempts.rs](../../runlimit/runlimit-postgres/src/attempts.rs), lines 217, 411, 441; [attempt_complete.sql](../../runlimit/runlimit-postgres/src/attempt_complete.sql); [attempt_cleanup.sql](../../runlimit/runlimit-postgres/src/attempt_cleanup.sql) |
| L06 | PostgreSQL limiter transaction and connection disposition | Protocol/implicit. Acquire guarded connection → set bounds → serialize keys/capacity → evaluate → persist → commit/rollback → reusable or close. `ConnectionCancellationGuard.armed` defaults to closing and is disarmed only for reusable outcomes. | [postgres/protocol.rs](../../runlimit/runlimit-postgres/src/protocol.rs); [postgres/lib.rs](../../runlimit/runlimit-postgres/src/lib.rs), line 508 |
| L07 | Protected outcome-aware attempt runner | Protocol. Admit receipt → verify → claim under transaction → application work → finish native attempt in that transaction → acknowledge transaction → release typed output. Returned error phase does not itself drive this sequence; the runner does. | [batter-runlimit/attempts.rs](../../crates/batter-runlimit/src/attempts.rs), line 312 |

Application examples and the TUI retain application-owned protocols.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| A01 | Provider-effect dispatch and reconciliation | Mixed. Explicit effect state combines with acceptance possibility, retained provider identity/payload, generation, retention deadline, retry-not-before, and native attempt budget. Load → plan → admission → durable dispatch fence → provider exchange → fenced outcome write; reconciliation may authorize a later dispatch. | [delivery/worker.rs](../../examples/reference-service/src/delivery/worker.rs), lines 129, 476; [worker/state.rs](../../examples/reference-service/src/delivery/worker/state.rs), lines 92, 238, 385 |
| A02 | Live effect transaction authority | Protocol with a private owner. Ordinary effect writes lock job → validate current lease → lock/update effect → revalidate lease → commit. Confirmation first locks the record, then the job and effect, serializing against record-generation replacement without holding the job lock while waiting for the record. | [worker/state.rs](../../examples/reference-service/src/delivery/worker/state.rs), lines 440, 466, 499, 550 |
| A03 | Retirement session and command | Implicit/protocol. `connected`/`replaced` enforce first physical session versus rejected replacement; command verifies identity/quiescence → disables definition → cancels batches → verifies again. Stage labels do not enforce the order. | [retirement/session.rs](../../examples/reference-service/src/retirement/session.rs), line 20; [retirement/driver.rs](../../examples/reference-service/src/retirement/driver.rs), line 24 |
| A04 | TUI fetch generations | Implicit. Requested, optional in-flight, and optional applied generations encode dirty → fetching → superseded or applicable completion → scheduled refresh. Pausing changes automatic refresh, and input mode can defer starting a fetch. | [terminal.rs](../../runledger/runledger-tui/src/terminal.rs), line 27 |
| A05 | TUI navigation, overlays, and cached data | Mixed. Explicit screen/input enums combine with screen stack, per-screen selections, optional data/errors, help visibility, refresh pause, payload mode/wrap, and quit flag. Input transitions and fetch results update this product state. | [app.rs](../../runledger/runledger-tui/src/app.rs), line 136; [input/keymap.rs](../../runledger/runledger-tui/src/app/input/keymap.rs), line 7 |
| A06 | Terminal ownership | Protocol. Enable raw mode → install guard → enter alternate screen → event/render/fetch loop → shut down fetch workers on normal exit → restore terminal on guard drop. Early errors still drop the terminal guard and worker set, but bypass the explicit worker shutdown await. | [terminal.rs](../../runledger/runledger-tui/src/terminal.rs), lines 15, 116 |

Shared fixtures and verification tooling also contain operational machines.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| T01 | Fixture-run report ownership | Implicit. `driver: Some, result: None` → `driver: None, result: Some`; repeated observation borrows cached evidence, while consuming report extraction takes it. Pending wait cancellation preserves the driver. | [test_support/runner.rs](../../crates/batter-sqlx/src/test_support/runner.rs), line 242 |
| T02 | Fixture acquisition and cleanup | Protocol/mixed. Registered acquisition producers finish before cleanup; pools close → observe server sessions → retain failure and await explicit retry if needed → clean lease → complete/drain. Explicit cleanup-phase values publish progress but control flow enforces ordering. | [test_support/acquisition.rs](../../crates/batter-sqlx/src/test_support/acquisition.rs); [test_support/runner.rs](../../crates/batter-sqlx/src/test_support/runner.rs), line 299; [test_support/sessions.rs](../../crates/batter-sqlx/src/test_support/sessions.rs), line 125 |
| T03 | Verification pause barriers | Implicit. Global slot empty → barrier installed → taken by verifier → reached → released; dropping the handle removes an unused matching barrier or releases an active one. | [test_support/verification.rs](../../crates/batter-sqlx/src/test_support/verification.rs), lines 10, 61 |
| T04 | Native ephemeral database ownership | Implicit. Owned permit/database → explicit teardown and permit release, or drop transfers permit into best-effort background deletion. Runtime availability affects that fallback. | [db_lifecycle.rs](../../runledger/runledger-test-support/src/db_lifecycle.rs), line 101 |
| T05 | Native container startup and reaping | Protocol/implicit. Named/labeled startup attempt → running container or identity-checked failed-start removal. Process reaper's optional stdin represents live parent ownership versus EOF-triggered cleanup. | [container_start.rs](../../runledger/runledger-test-support/src/container_start.rs), line 41; [container_lifecycle.rs](../../runledger/runledger-test-support/src/container_lifecycle.rs), line 92 |
| T06 | Rust child-process watchdog and capture | Implicit/protocol. Launch handshake → live child/capture → event-relative deadline → natural exit or first kill request → reap → join capture → evidence. Optional reader handle prevents repeated join; capture limit latches overflow. Temporary directories separately use optional path ownership for remove/drop. | [process/watchdog.rs](../../test-support/process/watchdog.rs), line 73; [process/capture.rs](../../test-support/process/capture.rs), line 9; [process/evidence.rs](../../test-support/process/evidence.rs), line 20; [temp_dir.rs](../../test-support/temp_dir.rs), line 9 |
| T07 | Python process owner and readiness signaling | Implicit/protocol. Optional child/capture → optional readiness marker and signal → shutdown deadline → kill/reap → close descriptors → outcome. Capture selector membership encodes pipe EOF; head/tail buffers and overflow retain bounded evidence. | [scheduling_process.py](../../scripts/scheduling_process.py), lines 69, 161, 292 |
| T08 | Python parallel batch supervision | Implicit. `_Job.child`, `capture`, `stopping_at`, `watchdog`, and `outcome` encode unstarted/running/stopping/completed. Batch interruption stops launches; first stop time controls grace; finalization retains every job outcome. | [parallel_process.py](../../scripts/parallel_process.py), lines 17, 71 |
| T09 | Scripted dependency playback | Implicit. `Script` consumes a queue of success/error results until exhaustion; mutex poisoning is a separate failure state. Calls count even exhausted/poisoned attempts. This synchronous reusable test double does not itself supply delay or pending behavior. | [batter-test-support/lib.rs](../../crates/batter-test-support/src/lib.rs), line 60 |

Parser machines use cursor position, nesting depth, and branch-local state.

| ID | Machine | Representation and transitions | Source |
| --- | --- | --- | --- |
| P01 | Managed SQL scanner | Implicit. Cursor moves between ordinary tokens, line comments, nested block comments, quoted identifiers, ordinary/escaped strings, and dollar-quoted bodies. Local scan functions carry each mode; nesting depth handles block comments. | [managed/source.rs](../../crates/batter-sqlx/src/managed/source.rs), lines 47, 130 |
| P02 | Managed SQL statement boundary | Implicit. `parentheses`, `begin`, and `terminated` track statement/body nesting and whether a terminal semicolon has occurred; further tokens after termination reject the source. | [managed/source.rs](../../crates/batter-sqlx/src/managed/source.rs), line 357 |
| P03 | Route-pattern scanner and matching traversal | Mixed. Cursor/escaping rules parse literal segments, captures, and final wildcard into explicit segment variants; paired traversal constructs an overlap witness or rejects incompatibility. | [boundary/pattern.rs](../../crates/batter-axum/src/boundary/pattern.rs), lines 61, 88, 123, 146 |

Several conspicuous state models are already explicit and should not be reported
as wholly implicit:

| Model | Existing explicit representation | Remaining distinction |
| --- | --- | --- |
| Atomic SQL scope | [`ScopeState::Live/InFlight/Poisoned`](../../crates/batter-sqlx/src/atomic_runner.rs), line 81 | Transaction continuity and remote acknowledgement still require Q02/Q03. |
| Startup approval and component acknowledgement | [`UnapprovedDriver`](../../crates/batter-core/src/lifecycle/caller_owned.rs), [`ComponentStartup` and `RunningComponent`](../../crates/batter-core/src/lifecycle/capability.rs) | Internal coordinator and handoff state remain F01/F08. |
| Guarded HTTP assembly | [`GuardedRouter`, `HttpBoundary`, `AssembledHttp`](../../crates/batter-axum/src/boundary.rs) | Request execution still follows H01–H04; builder configuration is not separately counted. |
| Outbound request consumption and evidence | [`PreparedRequest`](../../crates/batter-outbound-http/src/request.rs), [`Exchange`](../../crates/batter-outbound-http/src/exchange.rs) | Live connection and handoff phases are H06/H07. |
| Submission acknowledgement | [`SubmissionPhase` and `ActiveSubmission`](../../examples/reference-service/src/delivery/service/progress.rs), lines 13, 24 | Already explicitly models before transaction, active, and settled acknowledgement. |
| Jobs, workflows, enqueue intents | [`JobStatus`, `JobStage`, workflow statuses](../../runledger/runledger-core/src/jobs/status.rs), [intent states](../../runledger/runledger-postgres/src/jobs/types/enqueue.rs) | Lease/counter/clock/claim substates and cross-row ordering are D01–D06. |
| Online index classification | [`IndexState`](../../runledger/runledger-postgres/src/migrations/online.rs), line 135 | The multi-step migration/repair protocol is D08. |
| TUI screens and text input | [`Screen`, `ActiveInput`](../../runledger/runledger-tui/src/app.rs), lines 27, 81 | Composite UI and asynchronous refresh state remain A04/A05. |

No additional enduring operational machine was identified in the at-rest crypto
leaf, native HTTP header serializers, facade re-exports, or immutable settings,
policy, and grant declarations. Simple format decoding and line validation are
not separately counted as lifecycle protocols. The runnable examples and external
single-facade consumers predominantly compose the machines above; their use does
not create a second library implementation. Test-only fault fixtures may introduce
local scheduling states beyond the shared infrastructure listed here.

This document changes no API or execution behavior. It records source findings,
not a repair backlog; delivery decisions and any later remediation belong in
Beadroll. Public API changes arising from this inventory require the ADR-010
invalid-state review and the repository's compatibility/migration procedure.
