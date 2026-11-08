# Flags catalog guide

## Purpose

Own the flag and kill-switch catalog grammar, the three declaration kinds and
their relationships, validated snapshots, clock-free evaluation, pins, the
evaluation event and, with `serde`, the manifest. Depend on no Batter crate and
no async runtime: loading, persistence and lifecycle belong to `batter-flags`
and `batter-flags-postgres` ([ADR-016](../../docs/adr/016-typed-flags-control-plane.md)).

## Key entrypoints

- `src/builder.rs`, `builder/`: `CatalogBuilder`, declarations and their
  single-assignment policy, relationships in `builder/relations.rs`.
- `src/catalog.rs`, `src/declaration.rs`, `src/digest.rs`: the built catalog,
  per-declaration policy and its digests.
- `src/candidate.rs`, `src/snapshot.rs`, `snapshot/relations.rs`: candidates,
  per-row validation and the invariant holds a fresh and a stale view make.
- `src/invariants.rs`: the one invariant judgement, also replayed over a
  manifest by `manifest/holds.rs`.
- `src/view.rs`, `view/decide.rs`: evaluation into layered `Decision`s and the
  one kill-switch function every surface reads a switch through.
- `src/published.rs`, `src/authority.rs`: the one immutable record a loader
  publishes and the transitions it may take.
- `src/pin.rs`, `pin/`: pinned decisions and their replay.
- `src/low_level.rs`: the disclosed, weaker publication path.

## Edit here for X

A new declaration policy goes on the draft in `builder/draft.rs` and into the
digest and manifest together; a new hold cause goes into `HoldCause`, its name
and the closed-vocabulary test in `tests/snapshots.rs`. Change how a switch is
read in `snapshot/relations.rs`'s override and entry stages only, so views and
the invariant judgement cannot drift apart.

## Invariants

Every public enum is exhaustive, errors included; a new variant is a recorded
hard cut. Evaluation reads no clock and does no I/O. Only a structurally
corrupt candidate is rejected whole; every other row is published or held on
its own. Holding never releases an engaged switch. The invariant judgement
visits relations in canonical key order, so declaration order changes nothing.
Debug never prints a setting's value. Without `serde` the normal closure holds
no serialization library.

## Common commands

```sh
cargo test -p batter-flags-core --locked
cargo test -p batter-flags-core --no-default-features --locked
cargo clippy -p batter-flags-core --all-targets --all-features --locked -- -D warnings
```
