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

`run_atomic(&pool, async |scope| ...)` reexports Runledger's protected runner,
built on `batter-sqlx`. The initial `PgIntentScope` supports application SQL and
`record_job_enqueue_intent`. Consume it with `scope.queue()` to enter
`PgQueueScope` for enqueue operations; intent recording is then unavailable.
There is no transaction view, owner extraction, separate completion call or legacy
bridge. Outputs leave the runner only after acknowledged commit; rejected bodies
only after acknowledged rollback. Uncertainty retains the domain result/error.
Every completion retires the session, and acquisition resets inherited state.

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This unpublished Unix-only adapter uses coordinated sibling Runledger packages
from `../runledger` while these feature branches are reviewed. CI checks out
`70e55a521f61edd85059d57ad1031be8e4be060b`. Runledger depends
only on `batter-sqlx`, which depends on `batter-core`; neither depends on the
facade or this integration. Publishing and replacement with immutable released
package identities remain separate decisions.

`verify_schema(&pool)` acquires and owns a read-only repeatable-read transaction.
Runledger qualifies and checks authoritative objects, returning a
`SchemaCompatibilitySnapshot` after rollback. It never borrows caller session
state. The snapshot records one compatible observation, not future validity.
