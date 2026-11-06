# Flags adapter guide

## Purpose

Own declared settings resolution, the process reader and protected flags
surfaces. Catalog grammar and evaluation belong to `batter-flags-core`;
periodic timing, lifecycle and failure retention belong to `batter-core`.

## Key entrypoints

- `src/declared.rs` resolves captured settings into process flags.
- `src/reader.rs` owns capture and publication; both remain crate-private.
- `src/flags/gate.rs` validates switches and binds each reading to one view.
- `src/http.rs` owns protected route-group views and switch gates with `axum`.
- `src/jobs.rs` owns native job-handler views and gates with `runledger`.
- `src/periodic.rs` owns gated registration, its restricted policy and skips.
- `src/test.rs` supplies public fixtures; `src/low_level.rs` discloses live reads.

## Edit here for X

Change periodic admission and skip evidence in `src/periodic.rs`, with paused
clock contracts in `src/periodic/tests.rs`. Do not add another scheduler or
change core's lifecycle to implement a flags surface.

## Invariants

Follow the root guide's Explicit state machines rule, including internal code.
Validate handles before registration. Capture once and exhaustively match
`GateReading`. Periodic work runs only on `Open` with that same view; awaiting
publication and tripped readings skip. HTTP may admit released default decisions
while awaiting publication under `ServeDefaults`; aggregation must preserve any
withholding decision. Jobs defer awaiting publication and apply their native
policy to tripped readings. Immediate startup and stop-at-drain are fixed by
`GatedPeriodicPolicy`. Never gate support work needed during native settlement.
No protected surface accepts a caller-supplied view. Keep control-plane writer
injection private to tests; public fixtures use `TestFlags`.

## Common commands

```sh
cargo test -p batter-flags --locked
cargo clippy -p batter-flags --all-targets --locked -- -D warnings
```

Full backend verification follows the root guide and the active delivery
assignment's verification ownership.
