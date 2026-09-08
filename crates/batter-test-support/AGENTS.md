# Generic test-support guide

## Purpose

Provide finite scripted results and error-preserving result combination for
tests. Keep the crate independent of the runtime foundation and its adapters.

## Key entrypoints

- `src/lib.rs`: `Script`, `ScriptError`, `TestFailure`, and `finish`.
- `tests/support.rs`: exhaustion, call counts, and dual-failure contracts.

## Edit here for X

Put generic synchronous test helpers here. Application/database fixtures belong
in their owning example or application test targets. PostgreSQL provisioning
remains in the external harness; no optional database integration belongs in
this leaf crate.

## Invariants

Do not repeat exhausted scripted results or manufacture success. Preserve body
and cleanup errors. `finish` only combines already observed results; it cannot
guarantee cleanup runs. Do not add a foundation/adapter dependency: foundation
tests already depend on this crate, and the direction must remain acyclic.

## Common commands

Run from the workspace root:

```sh
cargo test -p batter-test-support --locked
cargo clippy -p batter-test-support --all-targets --locked -- -D warnings
scripts/jig check test
```
