# OTLP adapter guide

## Purpose

Own bounded metrics export through Batter's protected service completion.
Native aggregation and protobuf encoding remain owned by OpenTelemetry.
Unix only; Linux/macOS execution claims require actual recorded checks.

## Key entrypoints

- `src/lib.rs`: inert preparation, canonical diagnostic installation, serial export and closure.
- `src/config.rs`: validated explicit endpoint, identity and timing.
- `src/pipeline/guard.rs`: bounded recorder using core's shared catalog schema.
- `src/pipeline/transport.rs`: bounded HTTP and typed response evidence.
- `src/diagnostics.rs`: redacted report types.
- `src/pipeline/tests.rs`, its child modules and `tests/service.rs`: native collector and public composition acceptance.

## Edit here for X

Change transport/SDK integration here, lifecycle ownership and catalog schema in
batter-core, configuration sources and deployment policy in the application.
Shared collector test source lives at `crates/batter-otlp/tests/support/collector.rs`.

## Invariants

Depend on core, never the facade. Preparation installs/spawns/connects nothing.
Use empty explicit resource attributes, reject ambient OTEL_* before builders,
and do not mutate environment. Installation is process-global and explicit.
Only the opaque core completion observation can release canonical final export.
There is no public arbitrary-future runner or separate flush/close pairing.
Reject before bridge allocation, keep the full-key cap, response/request limits,
one serial owner, cumulative snapshots, no queue/retry, and typed failure evidence.
Coalesce missed intervals in constant time. Preserve native service outcomes on
any diagnostic failure. Closure runs once on normal finalization; do not claim
panics, non-yielding SDK calls or runtime death satisfy that guarantee.

## Common commands

- `cargo test -p batter-otlp --locked`
- `cargo clippy -p batter-otlp --all-targets --locked -- -D warnings`
- `bash scripts/verify.sh`
