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
`NativeReport` preserves all original native outcomes and unresolved descendants.
The exact `register(&mut Supervisor, ...)` signature remains available for
lower-level consumers; both names enter the same native ownership path.

`RunledgerTransaction::begin(&mut PgSession)` is the protected SQLx composition
path. It owns Batter's opaque transaction, implements Runledger's
`PgTransactionExecutor`, and exposes only borrow-scoped SQL execution plus
consuming commit/rollback. Application writes and direct Runledger enqueue can
therefore share one transaction without making the physical connection or native
transaction replaceable. An unfinished transaction cannot authorize pool return:
dropping or forgetting the wrapper causes the enclosing lease to retire its
connection.

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This unpublished Unix-only adapter consumes Runledger at Git revision
`638ee3480f69962597147f5d7bd52822267560b7`, pinned in the workspace and Cargo.lock.
No sibling checkout is required. The foundation has no dependency on this adapter.
