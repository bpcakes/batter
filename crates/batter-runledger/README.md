# batter-runledger

An optional adapter for agent consumers of native Runledger. Pass the native
builder's `prepare()` result to `register`. Preparation owns validated configuration
and starts no tasks; registration accepts no live supervisor or application factory.
The process starts native work after accepting ownership and validating its name.
The executable rustdoc shows the complete registration path.

Native loop initialization acknowledges local state. Dependency health, durable
execution proof and application readiness approval are separate. Production startup
does not enqueue a control job. The native driver and its settlement report survive
direct waiter abortion; Batter uses that evidence before dependency finalization.
`NativeReport` preserves all original native outcomes and unresolved descendants.

Native graceful and abort/join allowances come from the process budget's drain and
cancellation phases. The adapter exchanges the earliest native/parent stop timestamp;
earlier discoveries shorten active phase waits without replacing the first native
cause. Native stop observation drains peers promptly. No caller-owned termination gate,
independent driver, failure side channel or nested cleanup stack is needed.

This unpublished Unix-only adapter consumes Runledger at Git revision
`d57ec6be61e9f00ccce373b19ca356cafe98f206`, pinned in the workspace and Cargo.lock.
No sibling checkout is required. The foundation has no dependency on this adapter.
