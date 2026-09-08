# Foundation guide

## Purpose

Own native Tokio operational contracts. Read the root guide and
[guarantees](../../docs/guarantees.md) before changing failure behavior.

Follow the root [Unix-only platform policy](../../AGENTS.md#platform-scope).
Windows support and non-Unix fallbacks are out of scope.

## Key entrypoints

- `src/lib.rs` defines the public modules and boundary error aliases.
- `src/lifecycle.rs` and `src/lifecycle/` own process tasks and shutdown reports.
- `src/operation.rs`, `src/retry.rs`, and `src/admission.rs` bound application work.
- `src/cleanup.rs` drives explicit LIFO finalizers.
- `src/telemetry.rs` exposes observations and the adapter dispatch seam;
  `src/scoped_dispatch.rs` owns its private pin/drop implementation.

## Edit here for X

Keep runtime-independent application policy out of this crate. Change HTTP
translation in `batter-axum`; change SQLx composition in the example package.
Put foundation failure tests in `tests` and core usage examples in `examples`.
Update the root contracts, implemented status, owning Bead, and validation for behavior changes.

## Invariants

Preserve child deadline clamping, downward cancellation, inert factories,
explicit replay permission, bounded process admission, and retained failures.
Stop admission before cancellation. Harvest ready tasks before escalation;
do not equate aborted wrappers with stopped detached work. Keep conservative
cleanup skipping after uncertain termination. Never print cause contents or
install a global subscriber/panic hook. Protected dispatch covers full future
destruction and nested spans. The foundation cannot depend on its adapters;
generic test support remains a leaf dependency used by tests.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter --locked
cargo test -p batter --doc --locked
cargo clippy -p batter --all-targets --locked -- -D warnings
scripts/jig check test
```
