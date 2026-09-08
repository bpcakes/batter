# batter-axum

Axum 0.8 integration for the `batter` Tokio foundation. Import its API from
`batter_axum`. This separate dependency provides request readiness/deadline
policy, operation context extensions, sanitized infrastructure failure rendering,
HTTP observations, and liveness/readiness handlers.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

Use `RequestPolicy` with Axum's `middleware::from_fn_with_state(policy, request_scope)`.
Put business routes behind the policy and merge probe routes separately. The
policy retains its existing combined readiness and deadline behavior; it is not
a collection of independently configured middleware layers.

`with_failure_renderer` accepts application-controlled response mapping with a
snapshot of request parts. Establish trusted metadata outside the boundary.
Handlers should use the application's renderer for their own failures.

The deadline ends at response construction. Streaming bodies, WebSockets, client
disconnect lifetime, authentication, proxy trust, and request body limits need
application policy. A joined server wrapper does not prove detached connection
tasks have stopped. Automatic observations omit raw error/request contents;
the application installs its tracing subscriber.

From the workspace root:

```sh
cargo test -p batter-axum --locked
cargo run -p batter-axum --example http_service
```

The example binds `127.0.0.1:3000` by default and provides `/live`, `/ready`,
`/work`, and `/fail`. `BATTER_BIND` and `BATTER_REQUEST_TIMEOUT_MS` configure its
listener and deadline. SIGINT/SIGTERM initiate shutdown on Unix.

Version 0.1.0; Rust 1.94 minimum; publishing disabled. MIT licensed.
