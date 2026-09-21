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

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This unpublished Unix-only adapter uses coordinated sibling Runledger packages
from `../runledger` while these feature branches are reviewed. CI checks out
`7da3ce5d086023ddd71d9b0e50a97d26954535a9`. Runledger depends
only on `batter-sqlx`, which depends on `batter-core`; neither depends on the
facade or this integration. Publishing and replacement with immutable released
package identities remain separate decisions.

`verify_schema(&database)` acquires and owns a read-only repeatable-read transaction.
Runledger qualifies and checks authoritative objects, returning a
`SchemaCompatibilitySnapshot` after rollback. It never borrows caller session
state. The snapshot records one compatible observation, not future validity.
