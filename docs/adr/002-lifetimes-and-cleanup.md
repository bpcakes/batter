# ADR-002: Explicit lifetimes and conservative asynchronous cleanup

Status: accepted for the MVP source. Date: 2026-09-07.

Context: lexical ownership does not await asynchronous finalizers or prevent a
dropped JoinHandle from leaving background work. Arbitrary Rust future abortion
is not preemption, and server wrappers may own hidden asynchronous descendants.

Decision: distinguish operation, process, and durable lifetimes. Use child
cancellation contexts for operations and JoinSet for directly registered critical
process tasks. Separate stop-admission/drain from forced cancellation. Observe
all direct exits, including early Ok and panic. Preserve errors and unjoined names.

Finalizers are explicit owned factories, run in LIFO order with shared/per-hook
budgets and bounded abort observation. They do not run from Drop. After process
panic, requested abort, or unjoined direct tasks, skip dependent finalizers
conservatively and return an unsuccessful report. Stopped is coordinator state,
not proof that all asynchronous work in the process ended.

Consequences: some resources will not be explicitly finalized on forced failure;
operators receive an honest failure and must terminate/recover the process. The
alternative of declaring abort-plus-wrapper-join "safe shutdown" is rejected.
Callers must drive lifecycle/finalizer futures; no SIGKILL cleanup guarantee,
async-RAII claim, or Effect-style interruption mask is implied.

Amendment, 2026-09-08: bounded finite tasks share process ownership without
becoming critical long-lived loops. Root admission linearizes with drain;
active scope descendants remain capacity-bound until forced cancellation.
Task-level errors trigger drain; ordinary domain denial remains a value. Successes
are counted rather than retained indefinitely. Startup readiness requires actual
component acknowledgements as well as application approval and a running driver.

`start` owns a coordinator and completion monitor independently of result waiters.
Last-owner drop requests shutdown, and waiter cancellation cannot cancel cleanup.
Direct `run_until`/`close` remain cancellation-fragile. The runtime must stay alive;
the conservative hidden-descendant rule is unchanged.

Cancellation hardening, 2026-09-08: `run_until` arms its emergency guard at future
construction, the point of ownership transfer, while startup remains first-poll
lazy. Shutdown harvests ready joins at phase boundaries and distinguishes
unobserved results from unfinished tasks before requesting abort. A delayed
observation of successful termination must not suppress dependent finalizers.

State ownership hardening, 2026-09-09: a private lifecycle state module owns all
readiness/admission mutations. Every readiness transition and snapshot publication
requires the same admission mutex; Stopped is irreversible. The published atomic
snapshot preserves readiness reads while enqueue holds that mutex. Explicit
readiness and cancellation notifications occur after releasing it.

Supervisor construction now owns synchronous abandonment signaling and transfers
it into `run_until`. Dropping an unstarted owner withdraws readiness, signals drain
and cancellation and wakes readiness waiters before dropping application captures.
Abandonment reports Draining, invokes no finalizer, and creates no completion
report. This does not extend cleanup guarantees or change observer ownership.

Completion observer boundary, 2026-09-09: observers can be obtained only from
`RunningSupervisor` after `start`. The owned driver creates the completion channel
and its monitor owns the sender. Remove `ShutdownHandle::observer`: standalone
control handles, abandoned startup and caller-owned drivers have no completion
publisher. This deliberate source API change makes those invalid waits
unrepresentable without inventing a shutdown report or coordinator error.
Observer cloning and waiter cancellation still confer no process ownership.
