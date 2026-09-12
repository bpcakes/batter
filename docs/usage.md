# Usage patterns

## Canonical agent consumer path

For services, start from the executable
[`Startup` rustdoc](../crates/batter/src/startup.rs) or the
[HTTP composition example](../crates/batter-axum/examples/http_service.rs).
These demonstrate owned initialization and transfer to a running driver;
await its report and inspect every retained task or cleanup failure. Compose
native Rust/Tokio futures through these boundaries, use validated settings,
and keep application errors concrete.
Repeated instructions that a consumer must manually rebuild these protocols
indicate integration debt and should prompt a design review.

For finite work, use [`Command`](../crates/batter/src/command.rs) and the
[finite-command example](#finite-commands-and-owned-cleanup). The command owner
retains registered finalizers after callback errors, unwinding or work cancellation.
Borrowed waiter cancellation changes no ownership; owner drop requests cancellation
while finalization continues on the live runtime. Use `check_command` to interpret
the complete work/cleanup result.

The lower-level APIs remain available where an application needs a different
ownership boundary, but their caller obligations are explicit: direct
`run_until` and `CleanupStack::close` driving can be abandoned by cancellation,
and joining a wrapper cannot prove detached descendants stopped. Treat these
escape hatches as deliberately weaker contracts, not alternate spellings of
the owned-driver path. Agent-only consumption also does not add a DSL, hide
application policy, or guarantee remote effects.

When proposing a change, test independent failure scenarios and ask a fresh
agent to implement or modify a consumer from the documented path. Mark that
evaluation proposed and unexecuted until this repository records the run.

If example repairs repeatedly fail the same invariant or break a coupled
lifecycle phase, the implementing agent must initiate the
[consumer/API assessment](adr/010-agent-only-consumption.md#recurring-example-review-defects).
Determine whether the remedy belongs in the consumer, Batter or a supported
adapter, or the application/upstream protocol before continuing dependent repairs.

Source examples belong to their owning packages and are part of the workspace
compile/test matrix. See [validation](validation.md) for executed commands and
remaining gaps. The Rust snippets below illustrate existing APIs.

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

[`Startup`](../crates/batter/src/startup.rs) has an executable rustdoc example
that constructs its supervisor and budgets, reserves resource cleanup, acquires
a native capacity permit, and starts a channel request service. It waits for
readiness, checks an actual reply, and awaits shutdown before checking that the
permit was released. Run it with `cargo test -p batter --doc --locked startup::Startup`.

Successful initialization transfers ownership to a running supervisor. It does
not mean a finite command has completed. A `Supervisor::new` with no critical
components and no finite-work capacity reports `EmptySupervisor`; successful
resource cleanup does not make that shutdown report successful. Configure
genuine service work, or use the finite-command composition below.

The [worker example](../crates/batter/examples/worker.rs) shows inert registration and observed
signal handling. The [HTTP example](../crates/batter-axum/examples/http_service.rs) shows Axum's
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

An owned-driver `Ok` contains `SharedShutdownReport`; call `is_success()` and
handle retained task/cleanup failures before treating shutdown as successful.
The wrapper cheaply clones the same completed report, dereferences to
`ShutdownReport`, and can be retained as an application error through `BoxError`.
Its `must_use` warning catches a bare `running.wait().await?;` or
`running.wait().await.unwrap();`, but cannot require inspection after binding
or explicit disposal. See its rustdoc example for awaited shutdown and failure
propagation followed by an explicit process exit boundary. A `main` returning
`Result<(), BoxError>` uses Rust's `Termination` implementation, which prints an
error's **Debug** representation, including retained task and cleanup errors.
Keep rich errors inside `run`/`stop` and return `ExitCode` from `main`, as in the
[SQLx example](../examples/postgres-lifecycle/src/main.rs). Choose a trusted sink
before that boundary; only application-selected sanitized output belongs on the
default CLI path. Changing a report's Display does not sanitize Debug or arbitrary
errors propagated through `?`.

Migration from the earlier owned-driver API: replace explicit
`Arc<ShutdownReport>` return annotations with `SharedShutdownReport` and
`Arc::clone(&report)` with `report.clone()`. Borrow `&*report` where a
`&ShutdownReport` is required. The wrapper does not expose its internal Arc;
identity checks can compare borrowed reports with `std::ptr::eq`. Method and
field access continue through dereferencing. Coordinator failures still use
`Arc<JoinError>`.
For a standalone count summary, format the borrowed `ShutdownReport` (for
example, `(*report).to_string()`). Formatting the wrapper itself gives
`owned shutdown report`; an error-chain renderer obtains the summary from its
concrete report source, so the summary appears once.

## Finite process-owned work

The [process-owned example](../crates/batter/examples/process_owned.rs) configures a finite
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

`Supervisor::with_process_capacity` can supervise finite jobs without any
critical components. It still has a running/readiness phase, explicit submission,
and explicit shutdown; completing a successful job does not shut down the
supervisor. Its initializer is not a standalone acquisition/work/finalization
owner. Do not enable unused capacity or register a dummy service just to avoid
`EmptySupervisor` in a setup command.

## Finite commands and owned cleanup

Use `Command::new(work_context, cleanup_budget, callback).start()` for a finite
operation. The callback receives a `CommandScope`: reserve a cleanup name before
native acquisition and register the acquired resource immediately afterward,
before another await. Returning an error with `?` still transfers control to
owned finalization. The scope exposes a work context for nested native operations;
its cancellation cannot travel upward to the parent context.

The [finite-command example](../crates/batter/examples/finite_command.rs) binds a
native loopback UDP socket, sends and receives one message, and releases the
socket through registered cleanup. It uses the library report and owner. Socket
close is synchronous; a dependency's async close/flush belongs inside its finalizer.

```sh
cargo run -p batter --example finite_command --locked
cargo run -p batter --example finite_command --locked -- --fail-work
cargo run -p batter --example finite_command --locked -- --fail-cleanup
cargo run -p batter --example finite_command --locked -- --fail-both
cargo run -p batter --example finite_command --locked -- --cancel
cargo run -p batter --example finite_command --locked -- --deadline
```

The default succeeds; each injected failure/interruption exits unsuccessfully.
`--cancel` and `--deadline` interrupt after resource registration and still await
successful cleanup. `CommandReport` retains the concrete work result, independent
future-destruction panic and all cleanup observations. A cleanup coordinator
failure cannot replace the work result. `check_command(command.wait().await)`
returns the report only when the complete command contract succeeds; failure
retains the same report. Automatic report formatting omits cause/value contents.
The example prints only outcome facts at an explicit `ExitCode` boundary.

`RunningCommand::cancel` interrupts work cooperatively, without a service drain
phase. Cancelling a borrowed `wait()` has no effect; dropping the owner requests
work cancellation while its coordinator finishes registered cleanup. A Unix signal
handler can call `cancel` and await the retained report. Observers can await after
owner drop. Work completion cancels its children before cleanup, while finalizers
use an independent budget and must not capture work cancellation tokens.

`Command::new` starts the cleanup allowance after work stops. For an enclosing
absolute total, use `Command::within(total_context, cleanup_budget, callback)`:
it reserves the complete cleanup work plus abort-observation allowance before
invoking work. Scheduling delay consumes the total, so expired cleanup is reported
as skipped rather than extending the deadline. A value returned by a synchronous
final poll after cancellation/deadline is preserved, with a separate interruption
that prevents success. Runtime/process death and non-yielding threads cannot be
repaired by ownership. Arbitrary spawned descendants and remote database/provider
effects retain their actual contracts; this API does not prove they stopped.

## Reserve work and finalization budgets

The [operation-budget example](../crates/batter/examples/operation_budget.rs) uses
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

Import `RequestPolicy`, `HttpFailure`, `request_admission` and `observe_http`
from `batter_axum`. Apply admission to business routes and observation to the
complete router after merging probes and fallback. Keep trusted identity outside
observation. Routes added after the observer layer bypass it; assemble first.
`request_scope` remains the combined compatibility middleware. Replace it with
`request_admission` when adding outer observation to avoid two HTTP events.
Select the separate `batter-axum` dependency; the foundation has no HTTP feature.
`RequestPolicy::with_failure_renderer` receives a `HttpFailure` and a snapshot of
request parts. Use `failure.code()`/`status()` and a trusted private extension to
render your envelope. Install trusted metadata middleware outside the policy so
it is available even for readiness/deadline failures. The [HTTP example](../crates/batter-axum/examples/http_service.rs)
uses `operational_http` to generate a UUID and replace incoming header/Tower/
adapter identities. This opt-in wrapper replaces the outer observer/identity
pair; it emits one HTTP completion with an event-local ID even when INFO spans
are disabled. Extract `Extension<CorrelationId>` for explicit metadata propagation.
Applications remain responsible for durable uniqueness requirements and trust policy.
The example selects `with_infrastructure_json()` and uses
`render_infrastructure_failure` in handlers; legacy Problem JSON and custom
rendering remain compatible. Readiness uses `ReadinessPolicy` with a read-only
HealthReader, and owned startup calls `register_http` after binding the native
listener. These helpers do not own domain errors, body streaming or authentication.

The callback controls only middleware-generated failures. Handlers should reuse
the application's renderer for a consistent envelope; health probes have their
own contract. Keep HTTP status/outcome observations separate from whether the
handler successfully constructed a response. With default severity, an ordinary
INFO fmt subscriber receives completion events without enabling span events or
logging raw causes.

To select severity for a specific response, import `HttpObservationLevel` and
return `(axum::Extension(HttpObservationLevel(tracing::Level::INFO)), response)`.
Set this in a handler, failure renderer or middleware inside observation. This
can make an expected readiness failure informational while preserving 503,
`http_outcome="server_error"`, sanitized fields and one completion event. Both
observers honor it; `RequestPolicy` needs no severity configuration. Unannotated
5xx and dropped futures remain WARN, other unannotated responses remain INFO.
The span stays INFO. If selecting DEBUG/TRACE, enable those levels on the
subscriber. Status/outcome-based alert rules still need application-owned probe
filtering; lowering severity does not reclassify an HTTP result.

## Adapter tracing context

Use `batter::telemetry::with_current_dispatch(future)` when an adapter must keep
its caller's subscriber during both polling and full future destruction. Capture
occurs at the helper call, including for a never-polled future; call it inside an
async entrypoint to capture at that entrypoint's first poll. Borrowed/non-Send
futures remain supported. The helper does not capture or enter the current span:
instrument the inner future when span context is needed, then wrap the whole
instrumented future so its span is destroyed under the saved dispatcher.

This does not spawn a task, allocate a wrapper on the heap, install a subscriber,
or shield asynchronous cleanup from cancellation. Ordinary operation users get
dispatch preservation from `OperationContext::run` without adding this wrapper.

## Explicit settings sources

```rust
use batter::settings::{SettingsSource, SecretString, bounded_u64};

fn example() -> Result<(), batter::settings::SettingsError> {
    let source = SettingsSource::from_pairs([("LIMIT".into(), "4".into())])?;
    let limit = bounded_u64(source.required("LIMIT")?, "LIMIT", 1, 16)?;
    assert_eq!(limit, 4);
    assert_eq!(SecretString::new("credential").to_string(), "[REDACTED]");
    Ok(())
}
```

Applications own names, defaults, source order and required-secret policy.
Nothing here reads the process environment or searches for a file. The HTTP
example accepts `BATTER_ENV_FILE` and configured bind/timeout/capacity values;
the reference package exposes validated native pool and worker constructors.
See [operations](operations.md#loading-example-settings) and the
[reference schema](../examples/reference-service/README.md#typed-settings-and-native-constructors).

## Resource acquisition and partial startup

Register a cleanup factory immediately after acquiring an owned resource. A
resource dependency closes after its dependents, so register it before them.
If later startup fails, call supervisor.take_cleanup().close(budget).await and
retain both the startup failure and the complete cleanup report. The [native
SQLx example](../examples/postgres-lifecycle/src/main.rs) demonstrates this shape.

An extracted stack may be closed after dropping the supervisor, but that drop
cancels the supervisor's operation tokens. Cleanup hooks must not use those tokens
to cancel teardown. Await the resource's native close operation directly, or use
an independent `OperationContext::new` for cleanup; the stack's `CleanupBudget`
still applies. The [extracted-cleanup test](../crates/batter/tests/lifecycle_state.rs)
demonstrates independent teardown after the owner is dropped.

The example's startup completion path returns `StartupFailure` inside `BoxError`
rather than rebuilding an `io::Error` from text. Its error source remains the
original startup cause, and its owned `CleanupReport` retains each cleanup outcome
and error. Its shutdown completion path similarly retains the owned
`ShutdownReport`, including task and cleanup errors. The outer `ProcessFailure`
keeps either typed failure, or an earlier concrete startup error, as its source
while exposing fixed `Display` and `Debug` text. The explicit `ExitCode` handler
prints only that known wrapper's `Display`. A trusted sink can traverse or
downcast the retained source chain. Choose that sink
deliberately instead of dropping diagnostics or printing the reports' derived
`Debug` output.

Do not wrap an entire transaction/commit in a blanket retry. Continue to use
native SQLx transaction parameters where application writes and Runledger enqueue
must share the transaction. The
[reference delivery command](../examples/reference-service/README.md#atomic-delivery-command)
shows the implemented boundary: validate before acquisition; pass one operation
budget through `PgLease` acquisition and transaction work; return the lease only
after acknowledged commit/rollback; and reconcile an uncertain result by the
original authenticated owner and idempotency key. An absent reconciliation row
while a database session may still settle is not proof of rollback.

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

## Sample health independently of HTTP traffic

```rust
use batter::{
    health::{HealthMonitor, HealthPolicy},
    lifecycle::{Readiness, Supervisor},
};
use std::{io, time::Duration};

fn register_health(supervisor: &mut Supervisor) -> Result<batter::health::HealthReader<io::Error>, batter::BoxError> {
    let policy = HealthPolicy::new(
        Duration::from_secs(1), // whole probe, including acquisition
        Duration::from_secs(2), // delay after completion/destruction
        Duration::from_secs(4), // maximum observation age
        Duration::from_secs(1), // scheduling margin
    )?;
    let monitor = HealthMonitor::new(policy, || async {
        // Replace with the complete native dependency probe.
        Ok::<_, io::Error>(())
    });
    let reader = monitor.reader();
    supervisor.register("dependency.health", move |shutdown| async move {
        monitor.run(shutdown).await;
        Ok(())
    })?;
    Ok(reader)
}
```

For each readiness request, evaluate
`handle.readiness() == Readiness::Ready && reader.is_healthy()`.
This does no dependency I/O. To inspect why it is unready, call `reader.snapshot()`
and inspect `status()` and `last_probe()`; original errors require deliberate
trusted access through `ProbeOutcome::Failed`. Do not cache a healthy snapshot
as permanent approval. Readers report expired success as Stale and writer loss
as Stopped. Recovered probes can restore health without restarting the process.

The [HTTP example](../crates/batter-axum/examples/http_service.rs) runs a simulated
probe every completion-plus-delay interval and combines its reader with lifecycle
state. Its real loopback tests preserve process-phase/telemetry behavior, and a
controlled router test exercises unknown, failure, recovery, staleness and writer
loss without extra probes. Actual dependency work must remain in the supplied
future; dropping that future does not establish remote cancellation or cleanup
of unregistered resources. Health does not automatically change business-route
admission policy or process readiness approval.
