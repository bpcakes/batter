# Operating an application built with Batter

This is a runbook for the intended contract, not a claim that the examples have
been load-tested or deployed. Validate the package and your application first.

The project targets Unix systems; Windows is unsupported and not planned.
Example shutdown listeners register SIGINT and SIGTERM directly. See
[platform scope](adr/007-unix-platform-scope.md) and [validation](validation.md).

## Startup

Parse and validate configuration without logging secrets. Acquire dependencies,
register finalizers, run migrations/schema checks through the application's
normal path, and perform readiness checks. Set readiness only when dependencies
and required initialization are complete. Each critical component calls
`mark_started` after initialization. The application's `mark_ready` arms Ready,
which is published only once the driver runs and every component acknowledges.

Registration is inert. Use ordinary awaited startup work before the supervisor.
On failure, explicitly drive all registered cleanup and retain the primary
failure plus cleanup results. The default HTTP example binds loopback only;
changing exposure, TLS, authentication, and proxy trust is an application decision.

All operational durations are capped at one year, including validated combined
phase budgets; zero is rejected except for drain/cancel phase durations. These
are misconfiguration guards, not useful production budget defaults. Choose
service budgets from actual latency/capacity requirements.

## Normal shutdown

Withdraw readiness and notify stop-admission/new-claim boundaries first. Let
already admitted work finish while pools/clients remain available. Escalate to
cooperative cancellation, then request abort and observe its completion within
the remaining configured phase. Inspect the complete ShutdownReport.

The example's allowance is 10 seconds drain, 2 seconds cancellation, 1 second
abort observation, 5 seconds cleanup work, and up to 1 additional second final
cleanup abort observation. Those are illustrative configuration values, not
measured latency expectations. Set any container/process-manager termination
grace above your chosen sum plus scheduling/export overhead.

Batter never calls process::exit or terminates the runtime for the caller. A
binary should surface unsuccessful shutdown as a nonzero exit, not "healthy"
termination. After panic/abort/unjoined direct tasks, finalizers are skipped
conservatively. A process supervisor should terminate/restart the process; do
not recycle it as a clean instance with possibly live hidden work.

Use `Supervisor::start` and retain its `RunningSupervisor` while the service is
intended to run. `shutdown` requests drain; cancelling that waiter does not stop
cleanup. Obtain an observer with `running.observer()` after `start` when another
owner may disappear; a shutdown control handle cannot create one. Dropping the last
owner requests drain automatically; keep the runtime alive to receive the report.

No shutdown contract can promise arbitrary async cleanup after SIGKILL, host
crash, or nonpreemptible code. A hard watchdog and durable reconciliation belong
outside this library. Do not use an outer timeout to discard the lifecycle
future and then report that cleanup ran.

## Telemetry and diagnostics

Install the application's tracing subscriber/exporters once in the binary.
Batter does not set global state. Current fields are static operation/component
names, outcome categories, elapsed milliseconds, attempt number, and delay.
Operation completion emits INFO for success/cancellation and WARN for returned
failure, deadline, or dropped execution, without requiring span-close logging.
HTTP observations separately record normalized method, matched route (or
`<unmatched>`), status, HTTP outcome, and latency. An HTTP 500 is a server error
even if constructing the response completed normally. Automatic fields omit
raw paths, query strings, headers, and error messages. Cleanup has its own span
and emits visible success/failure/skipped observations. Task failures emit WARN.
Scoped subscribers remain attached through future destruction as well as polling,
including task abort. No extra application-side cancellation wrapper is needed.
Each skipped finalizer produces one warning, including the first hook skipped
when the shared work budget is exhausted.
No metrics backend, OpenTelemetry propagation, or exporter shutdown adapter is
included. Implement those as explicit optional integrations.

Task/cleanup reports retain original errors. Use Display for aggregate counts;
Debug or source inspection may expose secrets. Route detailed diagnostics to a
trusted sink with redaction. The standard Rust panic hook can independently
print panic payloads to stderr; Batter does not replace it. Never put credentials
in panic messages and do not claim library span hygiene sanitizes the whole process.

Keep user/account/job/request IDs out of metric labels. Trace correlation and
metric cardinality are different concerns. Persist only validated/versioned
correlation metadata across durable job boundaries; never serialize execution
cancellation or monotonic deadlines.

## Capacity and retries

Coordinate pool limits, admitted requests, Runledger worker concurrency,
maintenance work, and test harness budgets explicitly. The Bulkhead waiter queue
is not a memory limit. Prefer reject-at-capacity or an independently bounded
queue when overload must not accumulate waiters.

For finite process work, configure `with_process_capacity` and transfer any
dependency permit into its factory. A dropped result receipt must not free that
permit while work continues. Root admission closes at drain, while active scope
descendants consume the same capacity until forced cancellation. Expected
business denials belong in task values; task-level Err means process failure.

Rate limits, in-flight concurrency, and retries are separate budgets. Set one
retry owner, authorize replay, and preserve provider delays. For desynchronized
backoff use `execute_with_jitter` with independently seeded samples; replay a
fixed stream in tests. The default `execute` still uses deterministic backoff.
No timeout result proves that a mutation can
safely be repeated.

Reserve finalization time before starting work. Drive the shortened work context
then explicitly await the sibling finalization context, retaining both outcomes.
Do not wrap the pair in the work deadline or reuse a token cancelled when work
finished. The reserve neither ignores parent cancellation nor extends total time.

## Deployment prerequisites

Production and publication evidence is tracked in [Beads](roadmap.md).
Local library validation is not application-specific operational evidence or
production certification. Deployment and publication remain separate owner
decisions.

## Loading example settings

The HTTP example keeps loopback bind `127.0.0.1:3000`, request budget 2000 ms and
Bulkhead capacity 32. Set `BATTER_BIND`, `BATTER_REQUEST_TIMEOUT_MS` and
`BATTER_BULKHEAD_CAPACITY` explicitly to change them. `RUST_LOG` keeps the existing
`batter=info,http_service=info` default only when absent. Invalid or non-Unicode
values fail before resource acquisition, with static configuration diagnostics.

`BATTER_ENV_FILE` selects one optional literal dotenv file, limited to 64 KiB.
There is no default `.env` search. The selected file must exist and be readable;
unknown/duplicate/malformed entries fail even when environment values would
replace them. Environment overrides valid file structure; explicit in-memory
overrides are available to root callers/tests. The file cannot select another
file. Quote values containing spaces or `#`; dollar and backslash are literal.

For example, write these fake, local settings to a temporary file:

```text
BATTER_BIND=127.0.0.1:3000
BATTER_REQUEST_TIMEOUT_MS=5
BATTER_BULKHEAD_CAPACITY=1
RUST_LOG='info,batter=warn'
```

Run `BATTER_ENV_FILE=/path/to/file BATTER_REQUEST_TIMEOUT_MS=100 cargo run -p
batter-axum --example http_service --locked`. The environment selects a 100 ms
response budget rather than the file's 5 ms budget; the demonstration `/work`
operation takes 25 ms. The selected capacity applies to actual concurrent work.
Response streaming remains outside this deadline.

The reference package's validated constructors have a larger application schema,
listed in its [README](../examples/reference-service/README.md). It has no serving
command yet. Serving configuration requires an explicit database password and
worker ID; Setup allows externally provisioned passwordless local TCP fixtures.
Remove native PG* variables from the process launch environment when using this
root, select sslmode explicitly, and keep the environment unchanged thereafter.
Native connection option Debug/URL output and source-chain inspection may reveal
credentials. Automatic root formatting does not make those exposures redacted.
