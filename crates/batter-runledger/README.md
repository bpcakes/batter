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

`RunledgerTransaction::begin(&pool)` starts the protected transaction path.
It reexports Runledger's concrete owner, built on `batter-sqlx`. Use consuming
`application` scopes for SQLx writes and `enqueue_job` or
`record_job_enqueue_intent` for domain operations. There is no `view()`, native
resource extraction, or legacy bridge. Every returned owner has revalidated
transaction continuity; unfinished, cancelled and failed owners retire their
connection. Commit and rollback consume it and return explicit acknowledgement.

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This unpublished Unix-only adapter uses coordinated sibling Runledger packages
from `../runledger` while these feature branches are reviewed. CI checks out
`4a973a37427ec5da123d82dd924520fc3b5e455d`. Runledger depends
only on `batter-sqlx`, which depends on `batter-core`; neither depends on the
facade or this integration. Publishing and replacement with immutable released
package identities remain separate decisions.

`verify_schema(&pool)` acquires and owns a read-only repeatable-read transaction.
Runledger qualifies and checks authoritative objects, returning a
`SchemaCompatibilitySnapshot` after rollback. It never borrows caller session
state. The snapshot records one compatible observation, not future validity.
