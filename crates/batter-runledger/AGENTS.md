# Native runtime adapter guide

## Purpose

Translate Runledger initialization and complete native settlement into Batter
managed registration, and reexport Runledger's consuming transaction owner. Follow the root Unix-only policy. Native task
ownership, durable job outcomes, registry/catalog policy and database provisioning stay native.

## Key entrypoints

- `src/lib.rs`: inert registration, startup observation, stop propagation and native report.
- `src/lib.rs`: opaque `RunledgerTransaction` composition without native connection exposure.
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
NativeReport owns RuntimeSettlement and derives cleanup authority from its
unforgeable variants; never restore caller-writable report fields or reclassify
borrowed evidence. Schema verification owns its read-only transaction and returns
snapshot evidence; never restore borrowed session/transaction views.
`RunledgerTransaction` uses consuming application and domain operations backed by
Batter's SQLx foundation. No raw owner extraction or legacy bridge is supported.

## Common commands

`cargo test -p batter-runledger --locked` and
`cargo clippy -p batter-runledger --all-targets --locked -- -D warnings`.
The coordinated native dependency uses sibling paths recorded in the workspace
and Cargo.lock. Validate both feature branches together; external source state is
an explicit prerequisite for receipt reuse. Publication is not authorized by this
local cutover. Final acceptance includes root two-toolchain/live gates.
