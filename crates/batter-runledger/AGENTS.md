# Native runtime adapter guide

## Purpose

Translate Runledger initialization and complete native settlement into Batter
managed registration. Follow the root Unix-only policy. Native task ownership,
durable job outcomes, registry/catalog policy and database provisioning stay native.

## Key entrypoints

- `src/lib.rs`: inert registration, startup observation, stop propagation and native report.
- `tests/lifecycle.rs`: actual native-supervisor contracts without PostgreSQL.
- Reference service: application schema, handler selection and dependency health.

## Edit here for X

Keep lifecycle translation here. Change native descendant accounting in Runledger,
process cleanup eligibility in Batter, and application policy in the consumer.

## Invariants

Accept only owned native preparation. Start it inside the managed factory after
validation, without awaiting before managed transfer. Never use a durable startup
witness. Native stop must drain peers before full settlement, and use the parent's
original timestamp. Preserve the original native report and conservative dependency
cleanup classification. Do not install tracing subscribers or print error contents.

## Common commands

`cargo test -p batter-runledger --locked` and
`cargo clippy -p batter-runledger --all-targets --locked -- -D warnings`.
The native dependency uses an immutable Git revision recorded in the workspace
and Cargo.lock. Update both the root and archived consumer graph when changing
that revision. Final acceptance includes root two-toolchain/live gates.
