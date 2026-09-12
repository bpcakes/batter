# batter-axum

Axum 0.8 integration for the `batter` Tokio foundation. Import its API from
`batter_axum`. This separate dependency provides request readiness/deadline
policy, operation context extensions, sanitized infrastructure failure rendering,
HTTP observations, trusted server correlation, dependency readiness and supervised
native serving.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

Use `RequestPolicy` with
`middleware::from_fn_with_state(policy, request_admission)` on guarded business
routes. Merge unguarded probes and fallback, then apply
`middleware::from_fn(observe_http)` to the assembled router. Install trusted
server request identity outside observation, and response rejection/status
transformation inside it. Probes, fallback and rejected requests then receive
HTTP status/outcome/latency events without acquiring an execution policy.

`observe_http` requires no lifecycle state and adds no deadline or operation
context. `RequestPolicy` keeps readiness and deadlines combined. The existing
`request_scope` remains the combined observation/admission compatibility entry
point. When adding outer observation, replace inner `request_scope` with
`request_admission`: each installed observer emits its own event, with no
automatic deduplication.

HTTP completion events default to WARN for 5xx responses and INFO otherwise.
Applications can select a different level by returning
`(axum::Extension(HttpObservationLevel(tracing::Level::INFO)), response)` or
inserting that value into `response.extensions_mut()`. Both observers honor the
response extension without changing status, outcome, fields or event count.
Request extensions and client headers do not configure severity. A dropped future
with no response retains WARN. The span remains INFO; subscriber filtering still
controls delivery of events at all levels.

Each completion event carries its own normalized method, route template, optional
status, outcome and latency. WARN/ERROR events retain these fields when INFO spans
are disabled. Application correlation carried by spans still requires those spans
to be enabled. The observer retains its HTTP span or the available application
span at first poll through execution and destruction. Later ambient request spans
cannot replace the completion event's parent. HTTP fields are only recorded into
the HTTP span, never the inherited application span. Per-layer filters can still
hide context in an individual sink. Formatters that show both span and event
fields may display the HTTP fields twice on the same completion line; there is
still one event.

Set overrides in handlers, failure renderers or middleware inside observation.
Middleware changing a response must retain, replace or remove the override to
match its own policy. Status-only probes retain default severity.
`ReadinessPolicy` selects INFO for Starting/Draining and WARN for Stopped and
unhealthy dependencies while Ready. Their status remains 503 and outcome remains `server_error`, so
status/outcome alerts still need application-owned probe filtering.

Axum's `Router::layer` runs after routing and covers only routes/fallback already
assembled when applied. Routes appended later bypass it. A service wrapper
outside routing records `<unmatched>` because the matched route template is not
yet available. The [runnable example](examples/http_service.rs) and
[`observe_http` rustdoc](src/lib.rs) demonstrate the supported composition.

`with_failure_renderer` accepts application-controlled response mapping with a
snapshot of request parts. Establish trusted metadata outside the boundary.
Handlers should use the application's renderer for their own failures.

The deadline ends at response construction. Streaming bodies, WebSockets, client
disconnect lifetime, authentication, proxy trust, and request body limits need
application policy. A joined server wrapper does not prove detached connection
tasks have stopped. Automatic observations omit raw error/request contents;
the application installs its tracing subscriber. A polled response future dropped
before completion emits `dropped` without a status, under its first-poll subscriber.
A never-polled future emits nothing. Dropping a body after response construction
does not produce another HTTP event.

Handler panics propagate through the middleware. An unwind before a response
emits one WARN `dropped` event without a status and cancels admitted operation
context. Batter does not convert the panic to a 500 or log its payload. Rust's
default panic hook remains separate and may print the payload.

From the workspace root:

```sh
cargo test -p batter-axum --locked
cargo run -p batter-axum --example http_service
```

The example binds `127.0.0.1:3000` by default and provides `/live`, `/ready`,
`/work`, and `/fail`. `BATTER_BIND`, `BATTER_REQUEST_TIMEOUT_MS` and
`BATTER_BULKHEAD_CAPACITY` configure its listener, deadline and concurrency.
`BATTER_ENV_FILE` may name one literal dotenv file; there is no default `.env`
search. SIGINT/SIGTERM initiate shutdown on Unix.

Version 0.1.0; Rust 1.94 minimum; publishing disabled. MIT licensed.

The HTTP composition example combines lifecycle readiness with a cached reader
from the foundation health monitor. Its simulated probe runs independently of
HTTP traffic; application probe and timing policy remain explicit.

## Opt-in operational composition

Use `operational_http` instead of outer `observe_http` to generate a UUID, replace
incoming x-request-id and Tower/adapter identity extensions, observe once, and
replace the response ID header. Extract `Extension<CorrelationId>` and propagate
its string explicitly to nested application metadata. It carries no authority.
The HTTP completion event has its own `request_id` even with all INFO spans
disabled, including on drop under another dispatch. Native nested tracing still
requires enabled spans; no tenant/principal or inbound trace context is inferred.

`RequestPolicy::with_infrastructure_json()` explicitly selects application/json
with `{code, message, request_id}`, fixed sanitized messages and no-store. The
request_id is a generated string or null if no typed correlation was installed;
headers are never fallback identity. Handlers may call
`render_infrastructure_failure(failure, Some(&id))` for the same infrastructure
envelope. Default Problem JSON and `with_failure_renderer` remain available;
the last renderer selection wins. Domain error mappings remain application-owned.

Mount `dependency_readiness::<E>` with `ReadinessPolicy::new(handle, health)`
outside admission. Its response has an empty body and 200 only when lifecycle is
Ready and the latest read-only health snapshot is healthy. `ReadinessReason`
retains Starting/Draining/Stopped and each unready dependency status in response
extensions. `with_level` explicitly changes severity only. Decisions do no probe
I/O and are point-in-time observations, not atomic with future drain.

Inside protected `Startup::scoped` composition, bind a native `TcpListener`, assemble the
`Router`, then call `register_http_in(scope, "http", listener, router)`. It acknowledges when
the registered task runs and delegates graceful drain to Axum. Registration
failure and abandoned startup release the listener. Native accept errors are
retried by Axum. Streaming bodies can outlive response deadlines and direct
wrapper abortion; dependent cleanup remains conservatively skipped on forced
abort. `register_http(&mut Supervisor, ...)` remains source-compatible for the
lower-level path. The [operational tests](tests/operational.rs) exercise that limit with a
real socket and explicitly release the outstanding body afterward.
