# batter-flags

Typed flags and kill switches for Batter applications. An application declares
every flag and kill switch once, in one catalog, and evaluates them only
through typed handles: one view per HTTP request, job invocation or periodic
run. Declared mode resolves values once at startup from the application's
existing settings source and changes them by restart. The optional control
plane, selected with `postgres`, loads and refreshes an audited PostgreSQL
snapshot that a registered component keeps fresh.

No default features. `postgres` re-exports the store as `postgres` and
exposes `control_plane`, the protected registration; `axum` exposes `http` for
route-group gates on the Batter Axum adapter; `runledger` exposes `jobs` for
job-handler gates and pinned decisions. `core` is always the catalog crate
(`batter-flags-core`) itself, so handles, decisions and views keep one type
identity through the direct and facade paths.

```sh
cargo test -p batter-flags --no-default-features --locked
cargo test -p batter-flags --all-features --locked
```

Experimental. This crate is the scaffold of the flags delivery: its module and
feature shape is settled, and the items arrive with the declared-mode and
control-plane tasks. The catalog grammar and evaluation live in
[`batter-flags-core`](../batter-flags-core/README.md), the store in
[`batter-flags-postgres`](../batter-flags-postgres/README.md) and the operator
command in [`batter-flags-cli`](../batter-flags-cli/README.md).
