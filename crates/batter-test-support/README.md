# batter-test-support

Small synchronous utilities for Rust tests. `Script<T, E>` consumes a finite
sequence of expected results and records call counts. `finish` combines a body
result with an already completed cleanup result, preserving both failures in
`TestFailure`.

This crate follows the workspace's [Unix-only platform policy](../../README.md#platform-support).
Windows is unsupported and not planned.

```rust
use batter_test_support::{finish, TestFailure};

let result = finish::<(), _, _>(Err("body"), Err("cleanup"));
assert!(matches!(result, Err(TestFailure::Both { .. })));
```

These utilities do not depend on `batter`, Axum, SQLx, or a database harness.
Add them as a development dependency. They do not drive teardown after a panic
or cancellation: the test must await cleanup before combining results.

From the workspace root, run `cargo test -p batter-test-support --locked`.
Version 0.1.0; Rust 1.94 minimum; publishing disabled. MIT licensed.
