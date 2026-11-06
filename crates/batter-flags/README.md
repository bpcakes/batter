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

Experimental. Declared mode is implemented: `declared::resolve` consumes a
catalog and the root's captured `SettingsSource` once and returns a `Resolved`
with strict literals, ownership, retired and unknown names, required settings
and invariants, a redacted startup report and two revisions; `Resolved::into_flags`
enters declared mode. `test::TestFlags` is the fixture path, and
`low_level::capture_now` is the disclosed, non-canonical live read. The
protected HTTP and job gates and the control-plane registration arrive with
later delivery tasks of the flags epic. The catalog grammar and evaluation live in
[`batter-flags-core`](../batter-flags-core/README.md), the store in
[`batter-flags-postgres`](../batter-flags-postgres/README.md) and the operator
command in [`batter-flags-cli`](../batter-flags-cli/README.md).

`Flags::gate_periodic_in` registers gated maintenance through an existing
`RegistrationTarget`. Construct `periodic::GatedPeriodicPolicy` from the
interval and run budget; startup is always immediate and work stops at drain.
The callback receives an `OperationContext` and the one `SnapshotView` used to
gate that run. Awaiting publication and tripped switches skip without calling
the callback. `GatedPeriodicReader::skipped()` counts both; its `snapshot()`
returns core's periodic evidence, where `succeeded` includes skipped runs.
A publication during work reaches the next run and does not cancel active work.
Work needed for native settlement during drain must register ungated through
core. The method's rustdoc contains an executable registration example.
