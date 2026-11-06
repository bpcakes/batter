# Consuming startup handoff

Owning task: `batter-r4ir`. This implements F08 from the
[implicit-state inventory](implicit-state-machines-2026-11-05.md).

The former borrowed methods left a completed `StartingSupervisor` callable,
and a second completed wait panicked. The replacement methods consume the owner
into one future. Resumable waiting borrows that retained, pinned future; dropping
the future requests drain. The coordinator, channel, result types, observer and
cleanup implementation remain the same.

The [ADR-010 assessment](../adr/010-agent-only-consumption.md#startup-handoff-assessment-batter-r4ir)
records the invalid-state review. There is no deprecated borrowed shim because
it would retain the repeated-handoff defect. The [changelog](../../CHANGELOG.md)
has before/after examples for unit output, typed output and resumable waiting.

## Public API surface check

Compared the parent source snapshot
`316e721bd52b58219d5ef213da9000532a0df520` with this change's working tree using
the existing [surface extraction method](public-api-diff-2026-10-09/README.md).
Both JSON files were generated with Cargo 1.101.0-nightly
`29c5daa1a` (2026-10-07) and rustdoc 1.101.0-nightly `32dba69d6` (2026-10-09):

```sh
cargo +nightly rustdoc -p batter-core --lib --all-features --locked -- \
  -Zunstable-options --output-format json
# Save target/doc/batter_core.json in separate before/after directories.
python3 docs/evidence/public-api-diff-2026-10-09/surface.py \
  /tmp/batter-r4ir-api-before /tmp/batter-r4ir-api-after batter-core
```

```text
## batter-core: 1184 -> 1184 entries; 2 removed, 2 added
- REMOVED method batter_core::startup::StartingSupervisor::wait
- REMOVED method batter_core::startup::StartingSupervisor::wait_with_output
+ ADDED   method batter_core::startup::StartingSupervisor::finish
+ ADDED   method batter_core::startup::StartingSupervisor::finish_with_output
```

The extractor compares names, not signatures. Manual signature comparison:

| Before | After | Output |
| --- | --- | --- |
| `wait(&mut self)` | `finish(self)` | `Result<R, StartupError<E>>` |
| `wait_with_output(&mut self)` | `finish_with_output(mut self)` | `Result<(R, T), StartupError<E>>` |

Both remain async, with the same generic bounds. The facade re-exports the core
module directly, so this also changes `batter::startup::StartingSupervisor`.
No other public item was added or removed. Nightly was used only for this API
check; execution verification uses the repository's pinned Rust 1.98.1.

## Behavioral evidence

Compile-fail rustdocs reject repeating either handoff and requesting the unit
convenience method for non-unit output. The migrated startup, signal, HTTP and
consumer cases retain one future across interrupted borrowed waits.
`tests/typed_startup/handoff.rs` adds scenarios for unpolled abandonment,
pending abandonment with a retained cleanup failure, and typed delivery after
a borrowed timeout. `typed_startup.rs` checks destruction of already-queued
typed output when an unpolled finish future is dropped.

Execution results are recorded on `batter-r4ir`. Fresh-agent usability evaluation
is unexecuted. This change does not claim runtime-death cleanup or remove the
ordinary Rust prohibition on polling a completed future.
