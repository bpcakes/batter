# batter-axum

Axum 0.8 integration for the `batter` Tokio foundation. Import its API from
`batter_axum`. This separate dependency provides request readiness/deadline
policy, operation context extensions, sanitized infrastructure failure rendering,
HTTP observations, and liveness/readiness handlers.

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
match its own policy. Built-in probes retain default severity; the runnable
example explicitly selects INFO for lifecycle readiness failures while starting
or draining. Their status remains 503 and outcome remains `server_error`, so
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
`/work`, and `/fail`. `BATTER_BIND` and `BATTER_REQUEST_TIMEOUT_MS` configure its
listener and deadline. SIGINT/SIGTERM initiate shutdown on Unix.

Version 0.1.0; Rust 1.94 minimum; publishing disabled. MIT licensed.
