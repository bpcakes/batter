# Flags adapter guide

## Purpose

Own declared settings resolution, the process reader, protected flags
surfaces, the exact-role grant inventory and, with `postgres`, the control-plane
registration. Catalog grammar, evaluation and the invariant judgement belong to
`batter-flags-core`; the store and its SQL to `batter-flags-postgres`; periodic
timing, lifecycle and failure retention to `batter-core`. Flags has no
view-returning method; the only public live read is `low_level::capture_now`.

## Key entrypoints

- `src/declared.rs` resolves captured settings into process flags.
- `src/reader.rs` owns capture and publication; both remain crate-private.
- `src/flags/gate.rs` validates switches and binds each reading to one view.
- `src/http.rs` owns protected route-group views and switch gates with `axum`.
- `src/jobs.rs` owns native job-handler views and gates with `runledger`.
- `src/periodic.rs` owns gated registration, its restricted policy and skips.
- `src/test.rs` supplies public fixtures; `src/low_level.rs` discloses live reads.
- `src/control_plane.rs` is `register_in` and `RegisteredFlags`; `control_plane/`
  holds the refresh (`refresh.rs`), presence and its heartbeat report
  (`presence.rs`), status and readiness (`status.rs`), timing and load policy
  (`timing.rs`), the LISTEN component (`listen.rs`) and the crate-private load
  source with its test fake (`source.rs`).
- `src/last_good.rs` and `last_good/` own the last-good file, format `BFSN` v1
  ([`docs/snapshot-file-v1.md`](docs/snapshot-file-v1.md)).
- `src/grants.rs` and `grants/inventory.rs` list the privileges each
  `FlagsOperation` needs, checked against the store's statements.

## Edit here for X

Change periodic admission and skip evidence in `src/periodic.rs`, with paused
clock contracts in `src/periodic/tests.rs`. Do not add another scheduler or
change core's lifecycle to implement a flags surface. Change what a load does in
`control_plane/refresh.rs` and what a heartbeat reports in
`control_plane/presence.rs`, with paused-time contracts over the scripted source
in `control_plane/tests/`. When a store statement gains a column, update
`grants/inventory.rs`; the live grant case runs the statement under exactly
those privileges.

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
injection private to tests; public fixtures use `TestFlags`. Registration
checks the pool's cleanup evidence against the same borrowed target before
registering anything. Presence reservation consumes its preparation and retains
that target through component registration, lending only fresh reborrows. Persistence selects work and enters
its writing phase, numbered by selection, under one lock; the selected owner
lives in the blocking task, records its outcome and releases admission in one
critical section, and a later destruction releases only its own selection.
`RegisteredFlags::last_good` reads that state; never keep a failure apart from
the publication it failed.
Registration validates every input before registering anything, and refuses a base an
engaged governing switch would leave breaking an invariant. The writer owns
core's `Publisher`; the refresh publishes and records its verdict in one
critical section of the shared status state, and `status()` reads under it.
Questions only a live reader can answer belong to its `LiveRecord`, never to
`Reader`, so declared and test modes answer nothing with a stand-in. A heartbeat takes
its version, freshness and held keys from one published record. Persisting the
last-good file never changes a run's outcome or narrows readiness.

## Common commands

```sh
cargo test -p batter-flags --all-features --locked
cargo clippy -p batter-flags --all-targets --all-features --locked -- -D warnings
BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL=postgres://... cargo test -p batter-flags \
  --features postgres --test control_plane_live --test listen_live \
  --test invariants_live -- --ignored
```

Full backend verification follows the root guide and the active delivery
assignment's verification ownership.
