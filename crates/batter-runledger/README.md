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

`RunledgerTransaction::begin(&mut PgSession)` is the protected SQLx composition
path. It owns Batter's opaque transaction and supplies `view()` for Runledger's
sealed transaction execution boundary. The view retains the actual native
transaction borrow; arbitrary downstream executors are rejected. It exposes
borrow-scoped application SQL execution plus
consuming commit/rollback. Application writes, direct enqueue and durable enqueue intents can
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
`c541dad69fcb6c03b39541084538681b2d710a32`, pinned in the workspace and Cargo.lock.
No sibling checkout is required. The foundation has no dependency on this adapter.

`verify_schema(&mut PgSession)` bridges the native read-only compatibility check
inside the caller's consuming lease operation. It neither exposes a connection
nor substitutes a local schema verifier. Runledger owns the exact schema policy;
Batter owns the session's physical identity and disposition.
