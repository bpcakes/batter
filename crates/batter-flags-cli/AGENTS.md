# Flags operator command guide

## Purpose

Own the `batter-flags` executable root: argument validation before any
connection, one end-to-end timeout, the audited write path with its refusals,
and one report per exit. Store semantics belong to `batter-flags-postgres`;
this crate adds no SQL. It stays `publish = false` until the release-train
record (`batter-jkw6.23`).

## Key entrypoints

- `src/main.rs`: the root; installs the redacting panic hook first and makes
  the one read of the process environment, `BATTER_FLAGS_DATABASE_URL`,
  captured once and redacted everywhere (`src/capture.rs`).
- `src/plan.rs`: every argument validated into a `Plan` before connecting.
- `src/commands.rs`, `src/writes.rs`, `src/fleet.rs`: the commands, the write
  outcomes and refusals, and the fleet and rollout views.
- `src/output.rs`, `src/exit.rs`: the JSON or text report and exit codes.
- `src/tests.rs`: watchdog-bounded command settlement with outstanding blocking
  work, through the same runtime owner as the executable root.

## Edit here for X

A new store refusal or outcome needs a report arm in `src/writes.rs`, with its
`refusal` code and an exit code, and a live case in `tests/live_writes.rs`.
A new flag on writes goes on `Guard` in `src/commands.rs`.

## Invariants

Exit codes are 0 ok, 1 failed with nothing written, 2 usage, 3 precondition, 4
commit outcome unknown and 101 panic; under `--output json` every exit except
the argument parser's own and a panic writes exactly one document whose field
names are a public contract. No operation is retried. A signal during the
preflight or a read ends the command with `interrupted`; during a write, one
before COMMIT rolls back and one during COMMIT waits for the outcome. An
interrupted post-release fleet read preserves that outcome with a warning. The command never prints a
connection string or a stored value it did not ask for.
After command settlement, release the runtime without waiting for blocking
work before returning the report; a cancelled DNS future can leave a resolver
thread running. Release it on unwind too, then resume the original panic without
claiming settlement. Process exit, not runtime release, ends that thread.

## Common commands

```sh
cargo test -p batter-flags-cli --locked
BATTER_FLAGS_POSTGRES_TEST_DATABASE_URL=postgres://... \
  cargo test -p batter-flags-cli -- --ignored
```
