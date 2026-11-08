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

For feature handlers with work and finalization, implement `PhasedJobHandler`
and register `PhasedJob::new(handler, reserve)?`. Work receives `JobWork`;
finalization receives `JobFinalization` with the complete work outcome. Both
can borrow the handler and the same native `JobExecution` progress/checkpoint
services across awaits. The adapter owns sequencing and retains both outcomes;
a failed phase must be mapped to a native `JobFailure` through `failed`.
Handlers may explicitly override `settle_late` to settle a successful work value
returned after its shortened deadline. Only the adapter can issue
`LateJobFinalization`; cancellation, missing values, errors and panics cannot
obtain it. Its consuming `settle` method proposes a callback under the original
finalization budget. Recovery requires successful callback return, clean
destruction and live native authority. The default declines recovery, preserving
strict phase behavior. See `PhasedJobHandler::settle_late` for a checkpoint
example. Application writes and subsequent native completion are not atomic.
The native invocation owns the whole driver, with no independently spawned
coordinator. Native deadline, lease loss or task destruction can end it;
no finalization after invocation death is promised. The `PhasedJob` rustdoc and
single-dependency consumer demonstrate registration and checkpoint persistence.

The public `job_phases` function has been removed. The adapter privately derives
both phases from the worker's absolute deadline, with a positive reserve for
finalization. It cancels work after destroying the work future, then runs the
finalizer under the original native deadline. No separately spawned coordinator
or application-owned phase pair exists on this path.

The invocation's exit cancels both phases and all derived contexts. Native
ownership and fencing remain authoritative; a local phase cannot establish that
a remote effect rolled back. Each callback receives native execution services
and its own context. Use `scope.context().child(limit)` for a smaller step budget.

Derivation rejects with a typed `JobPhasesRejection` before any application work:
`Unsupported` for custom `JobExecutionServices` that make no exit claim,
`Ended`, `Exhausted` when no work time remains, or `Reserve` for an invalid
duration. Later operation boundaries reject expired or cancelled work before
invoking their factories. The signal notifies; it does not join detached tasks,
shield cleanup, undo remote effects or convert `OperationError` into `JobFailure`,
which stays an explicit application mapping.

To test a handler without a worker, queue or database, select the `test-support`
feature in `[dev-dependencies]` and drive the registered value through
`test_support::HandlerInvocation::new(budget)?.execute(&handler.into_job_handler(), payload)`.
A handler that reads its job's organization, attempt or lease declares that
context with `HandlerInvocation::for_context(budget, context)`, which rejects a
non-positive run number or attempt as the queue row does. A retried attempt
declares the progress its row retains with `with_progress`, so its writes are
validated against it as the worker validates them against the locked row.
It builds the real `JobExecution`: services that own the invocation's exit, so
`PhasedJob` derives internally, a native deadline `budget` after execution starts, and
in-memory progress persistence with the queue row's merge rule. It dispatches
through `JobHandler::execute_with_services` as the worker does, destroys the
handler future before it ends the invocation, on every exit and when the future
is destroyed, and returns an exhaustive `HandlerExit`: the handler's own
`Result<JobCompletion, JobFailure>` when observed strictly before the deadline,
`TimedOut` otherwise, or `Panicked` with the message; the run also carries the
persisted progress and the contained exit-hook panics. Durable outcome policy,
retries, continuations and dead letters stay in the worker and are not
simulated; there is no lease, so lease loss is not representable. The facade
path is `batter::runledger::test_support` (feature `runledger-test-support`).

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
deadline and cancellation bound the call cooperatively. If interruption wins
before the runner returns local commit acknowledgement, the caller receives
`OperationError::Interrupted` even though PostgreSQL may have committed. When
the work branch completes, `context.run` returns that result without rechecking
the clock or cancellation token. The runner does not yield after acknowledging
commit. Dropping the outer future loses its result and cannot return the runner's
internal classification. An interruption proves neither rollback nor permission
to replay; the application must retain uncertainty about remote effects.

`run_atomic_with` binds SQL and named operations to a concrete consumer error
through `PgFailurePolicy`; its scopes' `sql` methods return `P::Error`. All types
required by its five handlers, including `PgTransactionError`, are reexported
here; the crate-level rustdoc implements a policy using only adapter imports.
The non-policy scopes' `application` methods return `PgScopeError<E>`, also
reexported here. These are the original native types: the
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

Protected database construction uses `batter_sqlx::pool_in(slot, options,
connection, profile)?` followed by `RunledgerDatabase::from_profiled(pool.clone())?`.
Application SQL and Runledger then share one physical pool and one registered
finalizer. Runledger requires exactly one trusted schema, as before. Create the
pool inside protected startup and return it as typed output if the running caller
needs it; native standalone constructors live under `native::postgres::low_level`
and retain caller-owned cleanup. Distinct session authority needs distinct pools.
