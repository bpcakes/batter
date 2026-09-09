# batter

An operational foundation for native Tokio backends: process ownership, explicit
cleanup, deadlines, cancellation, retries, concurrency admission, and tracing.
Application futures and errors stay concrete. HTTP support is a separate
`batter-axum` dependency; this crate has no Axum or SQLx dependency.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

```rust
use batter::operation::OperationContext;
use std::time::Duration;

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let context = OperationContext::new(Duration::from_secs(2))?;
    let count = context.run("example.read", |_| async {
        Ok::<_, std::io::Error>(42)
    }).await?;
    assert_eq!(count, 42);
    Ok(())
}
```

The deadline can stop awaiting a future; it cannot establish rollback of an
external effect. Retry requires explicit replay authorization. Supervision owns
registered tasks and admitted finite work, not their arbitrary detached children.
Cleanup requires explicit driving, and its guarantees require a live runtime.
Automatic observations omit error contents; applications choose their subscriber.

Adapter authors can wrap a future with `batter::telemetry::with_current_dispatch`
to retain the dispatcher captured at the call through polling and destruction.
Keep observations and nested spans inside that future; it does not capture or
enter the current span. The helper does not spawn
work, add `Send` or `'static` bounds, or make cancellation run asynchronous cleanup.

From the workspace root:

```sh
cargo test -p batter --locked
cargo run -p batter --example worker
cargo run -p batter --example process_owned
cargo run -p batter --example operation_budget
```

Version 0.1.0; Rust 1.94 minimum; publishing disabled. This is an early foundation,
not a production-validated release. MIT licensed.

`health` provides an ordinary supervised monitor with explicit timing policy and
cloneable read-only observations. Readers do no probes; stale or stopped writers
are unready. See the [usage example](../../docs/usage.md#sample-health-independently-of-http-traffic).
