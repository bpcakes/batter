# Flags evaluation 2 oracle (control plane)

Frozen before launch on 2026-11-08 (`batter-jkw6.24`). The agent never sees
this file. Each item is scored met, partly met or not met, with the consumer's
file and line as evidence.

1. The consumer's manifest declares exactly one Batter dependency, `batter`,
   with `flags-postgres`, `flags-axum` and `flags-runledger`; no direct
   `batter-flags*`, `batter-core`, `batter-axum` or `batter-runledger` entry.
2. `RegisteredFlags` comes from `batter::flags::control_plane::register_in`
   consuming the `Resolved` that `declared::resolve` produced from the root's
   `SettingsSource`.
3. The new kill switch declares a `SafeState` and a `StaleRule`.
4. The route group is gated through `flags.http_gate` (or `http_gate_any`)
   installed with `GroupPolicy::gated`, and its handlers read `AdmittedFlags`,
   not a captured or native view.
5. The job type is a `GatedHandler` built with `gated_direct` or
   `gated_workflow_step`, with an explicit tripped policy and an awaiting
   recheck.
6. The release flag is pinned with `view.pin` into the application's own row in
   the same `run_atomic` that enqueues the job, and resolved in the handler with
   `JobFlags::resolve_pinned`, matching every `PinnedResolution` arm with no
   wildcard.
7. A retry after the operator command exits 4 reads the key's revision
   (`batter-flags history` or `show`) and re-runs with `--if-revision`, never a
   blind `--yes` replay.
8. Nothing mutates the process environment: no `set_var` or `remove_var`; tests
   inject a `SettingsSource`.
9. The root installs `redacting_panic_hook` as the first statement of `main`.
10. `cargo check` of the consumer succeeds with `SQLX_OFFLINE=true`.
