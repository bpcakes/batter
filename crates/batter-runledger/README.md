# batter-runledger

An optional adapter for agent consumers of native Runledger. Pass the native
builder's `prepare()` result to `register_in` from `Startup::scoped`. Preparation owns validated configuration
and starts no tasks; registration accepts no live supervisor or application factory.
The process starts native work after accepting ownership and validating its name.
The executable rustdoc shows the complete registration path.

Native loop initialization acknowledges local state. Dependency health, durable
execution proof and application readiness approval are separate. Production startup
does not enqueue a control job. The native driver and its settlement report survive
direct waiter abortion; Batter uses that evidence before dependency finalization.
`NativeReport` owns the native consuming `RuntimeSettlement` classification.
Its `native()` accessor preserves all original outcomes and unresolved descendants;
`settlement()` borrows the unforgeable classification. Only `Clean` reports
success, and `Unsettled` never authorizes dependency cleanup.
The exact `register(&mut Supervisor, ...)` signature remains available for
lower-level consumers; both names enter the same native ownership path.

Inside a `JobExecutionHandler`, `job_phases(execution, reserve)` derives Batter
`OperationPhases` from the native invocation. Both phases share one root whose
deadline is exactly the worker's `JobExecution::deadline()`, converted to Tokio's
view of the same monotonic clock and never rebuilt from the remaining budget.
Work ends `reserve` earlier, by the same checked arithmetic as
`reserve_finalization`; finalization keeps the invocation deadline, and cancelling
or exhausting work leaves it running. `Duration::ZERO` selects no reserve
(`split_finalization`): then any result or final-state write after work expiry is
classified `job.timeout_exceeded` by the worker. A positive reserve is application
policy and must cover the final-state work done after work expires. The worker
persists the outcome after the handler returns, outside that budget.

The invocation's own exit cancels both phases and every context derived from
them: success, continuation, failure, panic, timeout, lease loss, failed lease
maintenance and task abort, but not a graceful stop that lets it finish. The
native invocation owns the only cancellation authority, so no guard, forwarding
task or paired call exists to forget, and handlers cannot cancel the invocation.
The worker destroys the handler future before it ends the invocation, so work
awaited inside the handler is dropped rather than seeing the cancellation, and
nothing computed afterwards is stored; the exit reaches spawned work that holds
a phase or a child derived from it. Run final-state work under `finalization()`
when it uses Batter boundaries such as admission, retries, children or spawned
tasks; a plain write awaited in the handler is bounded by the native deadline
either way. Limit one step inside work with `phases.work().child(limit)`, which
ends at the earlier of that limit and the work deadline and stays linked to the
invocation's exit; a new root built inside a handler is never cancelled by it.
The facade's `batter::runledger` module documentation shows a complete handler
using facade paths.
Derivation rejects with a typed `JobPhasesRejection` before any application work:
`Unsupported` for custom `JobExecutionServices` that make no exit claim,
`Ended`, `Exhausted` when no work time remains, or `Reserve` for an invalid
duration. Later operation boundaries reject expired or cancelled work before
invoking their factories. The signal notifies; it does not join detached tasks,
shield cleanup, undo remote effects or convert `OperationError` into `JobFailure`,
which stays an explicit application mapping.

Declare `PgSessionProfile` and construct `RunledgerDatabase`; an arbitrary native
pool cannot establish serving authority after reset. The profile names login and
effective roles, the authoritative schema, timeouts and optional tenant settings.
Ordinary APIs and workers use `database.pool()`; schema and atomic APIs take
`&database`. Mandatory hooks and owned scopes establish the same policy. Custom
schemas and SET ROLE are supported; fallback schemas are rejected so missing
tables cannot resolve elsewhere. Qualify application objects outside the declared
schema. Provisioning and grants remain application-owned.

`run_atomic(&database, async |scope| ...)` reexports Runledger's protected runner,
built on `batter-sqlx`. The initial `PgIntentScope` supports application SQL and
`record_required_job_enqueue_intent`. Known conflicts are typed rejections, not
successful handoffs requiring a later status check. Consume it with `scope.queue()` to enter
`PgQueueScope` for enqueue operations; intent recording is then unavailable.
There is no transaction view, owner extraction, separate completion call or legacy
bridge. Outputs leave the runner only after acknowledged commit; rejected bodies
only after acknowledged rollback. `PgAtomicUncertainty` retains the domain
result/error and cause. Caught terminal failures retain the first database poison
cause; abandoned operations are classified separately. `PgScopeFailure` cannot
contain an ordinary application rejection.
Every completion retires the session, and acquisition resets inherited state.
The runner has no `_in` variant. Bound the whole call with the caller's
context, `context.run("records.create", |_| run_atomic(&database, ..))`, so the
deadline and cancellation reach the transaction; an abandoned call is
classified as described above.

`run_atomic_with` binds SQL and named operations to a concrete consumer error
through `PgFailurePolicy`. All types required by its five handlers, including
`PgTransactionError` and the `PgScopeError` that `scope.application` returns, are
reexported here; the crate-level rustdoc implements a
policy using only adapter imports. These are the original native types: the
reexports do not add constructors or relax transaction ownership and poisoning.

The `native` module reexports the native packages themselves —
`native::core`, `native::postgres`, `native::runtime`, and `native::test_support`
behind the opt-in `test-support` feature — so one `batter` dependency reaches
worker preparation, the job catalog, durable intents and the migrators. They are
deliberately low-level: `register_in` remains the protected registration path, and
the module rustdoc states what a caller that builds a live native supervisor takes
on instead. Sync job definitions during owned startup, before `register_in`,
with `native::runtime::catalog::JobCatalog::sync_definitions`: a standard
supervisor promotes recorded intents only for registered, enabled definitions.

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This version 0.0.1 Unix-only adapter targets crates.io and consumes the native
Runledger 0.13.0 packages in this workspace. They were imported from master
`46b5cd085d011e597de9552dfebbed4c19416453`, including PR #22. Local package paths
select the same SQLx foundation as the facade without dependency patches or a
sibling checkout. See [the compatibility manifest](../../docs/reference-compatibility.md#git-consumers).
Runledger persistence depends on `batter-sqlx`, which depends on `batter-core`;
neither depends on the facade or this adapter. Publishing remains a separate decision.

`verify_schema(&database)` acquires and owns a read-only repeatable-read transaction.
Runledger qualifies and checks authoritative objects, returning a
`SchemaCompatibilitySnapshot` after rollback. It never borrows caller session
state. The snapshot records one compatible observation, not future validity.
