# Usage patterns

Source examples are part of the intended compile/test matrix; no example has yet
been compiled in this package's authoring environment. The Rust snippets below
illustrate the existing APIs, not future APIs disguised as working code.

## Fresh futures and explicit replay

```rust
use batter::{
    operation::OperationContext,
    retry::{self, ReplaySafety, RetryDecision, RetryPolicy},
};
use std::time::Duration;

async fn read_from_provider() -> Result<u64, std::io::Error> {
    Ok(42)
}

async fn example() -> Result<u64, Box<dyn std::error::Error>> {
    let context = OperationContext::new(Duration::from_secs(3))?;
    let policy = RetryPolicy::new(
        3, Duration::from_millis(50), Duration::from_millis(400),
    )?;
    let result = retry::execute(
        &context,
        "provider.read",
        ReplaySafety::Idempotent,
        &policy,
        |_attempt| read_from_provider(),
        |error| match error.kind() {
            std::io::ErrorKind::ConnectionRefused => RetryDecision::Retry,
            _ => RetryDecision::Stop,
        },
    ).await?;
    Ok(result)
}
```

This example authorizes replay for a read; it is not a classifier to copy onto
arbitrary writes. For provider Retry-After, parse and validate the remote value
in that provider's adapter, then return RetryDecision::RetryAfter. Do not classify
by matching error-message text. A database commit or external mutation needs an
explicit idempotency/uncertainty design, not just this helper.

## Bound concurrent work separately

```rust
use batter::{admission::{Admission, Bulkhead}, operation::OperationContext};
use std::time::Duration;

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let bulkhead = Bulkhead::new(8)?; // Construct once and share clones.
    let context = OperationContext::new(Duration::from_secs(1))?;
    let _permit = bulkhead.enter(&context, Admission::Wait).await?;
    context.run("dependency.read", |_scope| async {
        Ok::<_, std::io::Error>(())
    }).await?;
    Ok(())
}
```

Place the permit around the resource you actually intend to limit. Holding it
across backoff limits whole logical operations but consumes capacity while idle.
Acquiring per attempt limits active dependency calls but creates a different
queueing policy. Neither choice consumes a user admission quota automatically.
The application must choose and test the intended accounting.

## Supervising a service

The [worker example](../examples/worker.rs) shows inert registration and observed
signal handling. The [HTTP example](../examples/http_service.rs) shows Axum's
native server future and separate probe/router state. Do not add a bare spawn
inside a registered component merely to make borrowing convenient; use composed
futures or an explicitly owned JoinSet and await its shutdown.

Every registered task is critical. A one-shot warmup belongs in startup, not in
the critical task set. A maintenance loop needs an intentional missed-tick,
error, and cancellation policy. A Runledger worker should be hosted through its
upstream supervisor rather than flattened into Batter-managed internal loops.

Call each component's `shutdown.mark_started()` after its actual initialization.
The application calls `handle.mark_ready()` after its own startup checks; the
driver publishes Ready only once all components acknowledge. Prefer
`let running = supervisor.start();` and `running.wait().await` for process
ownership. `running.shutdown().await` is cancel-safe as a waiter; the separately
driven cleanup still requires the runtime to remain alive.

## Finite process-owned work

The [process-owned example](../examples/process_owned.rs) configures a finite
capacity, transfers a dependency permit into admitted work, drops its result
receipt, and observes completion through shutdown. Rejection is immediate, not
a new queue of waiters. Do not capture arbitrarily large request bodies merely
because task count is bounded.

`ProcessHandle::try_spawn` admits roots only while Ready. A live `ProcessScope`
may admit bounded descendants during drain; forced cancellation stops both.
Scope cloning does not keep an already-finished ancestor active. Handle Full
explicitly when a parent and child compete for the same capacity.

`Err(E)` is a process task failure and starts drain. Ordinary user/business
rejection is `Ok(Err(denial))` so it remains a successful finite completion.
The typed receipt and shutdown report share the original failure cause; a lost
receipt is not authority to cancel the work. Successful tasks increment a
counter rather than accumulating one report record per operation forever.

## Reserve work and finalization budgets

The [operation-budget example](../examples/operation_budget.rs) uses
`reserve_finalization` before work, runs under `phases.work()`, then explicitly
awaits `phases.finalization()` and retains both outcomes. The latter has the
original total deadline, not a fresh timeout. Work and finalization have sibling
tokens: parent cancellation reaches both, but finishing work does not cancel
finalization. This is budget separation, not an interruption mask or guarantee
that a database/provider effect was rolled back.

The same example supplies a per-execution sample stream to `execute_with_jitter`.
Use a fixed deterministic stream in tests and independently seeded sampling in
production. Provider-directed delays remain a lower bound after jitter. Existing
`execute` callers retain deterministic capped backoff.

## Application HTTP envelopes

`RequestPolicy::with_failure_renderer` receives a `HttpFailure` and a snapshot of
request parts. Use `failure.code()`/`status()` and a trusted private extension to
render your envelope. Install trusted metadata middleware outside the policy so
it is available even for readiness/deadline failures. The [HTTP example](../examples/http_service.rs)
generates a process-local ID rather than trusting an incoming correlation header.
Applications remain responsible for ID uniqueness requirements and trust policy.

The callback controls only middleware-generated failures. Handlers should reuse
the application's renderer for a consistent envelope; health probes have their
own contract. Keep HTTP status/outcome observations separate from whether the
handler successfully constructed a response. An ordinary INFO fmt subscriber
receives completion events without enabling span events or logging raw causes.

## Resource acquisition and partial startup

Register a cleanup factory immediately after acquiring an owned resource. A
resource dependency closes after its dependents, so register it before them.
If later startup fails, call supervisor.take_cleanup().close(budget).await and
retain both the startup failure and the complete cleanup report. The [native
SQLx example](../examples/postgres_lifecycle.rs) demonstrates this shape.

The example's outer CLI error is sanitized; its StartupFailure object has the
original cause plus cleanup report available for a trusted error sink. In a
real application, choose that sink deliberately rather than dropping valuable
diagnostics or printing Debug on a report containing secrets.

Do not wrap an entire transaction/commit in a blanket retry. Continue to use
native SQLx transaction parameters where application writes and Runledger enqueue
must share the transaction. That reference integration is planned, not included.

## Tests that do not lose teardown errors

```rust
use batter_test_support::{finish, TestFailure};

let body: Result<(), &str> = Err("body failed");
// In an async test, actually await cleanup BEFORE combining these results.
let cleanup: Result<(), &str> = Err("cleanup failed");
assert!(matches!(
    finish(body, cleanup),
    Err(TestFailure::Both { body: "body failed", cleanup: "cleanup failed" })
));
```

The helper combines results; it does not magically run teardown after panic or
cancellation. A test that needs that behavior must drive the body in an owned
task, observe its JoinError, and explicitly perform teardown afterward.
