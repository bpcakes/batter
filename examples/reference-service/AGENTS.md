# Reference compatibility package

## Purpose

Own executable native SQLx, Runledger and external harness compatibility probes
for batter-4t6 and reusable fixture acceptance for batter-4jz. This unpublished Unix-only package will host later reference
composition; it is not a reusable database framework.

## Key entrypoints

- `src/lib.rs` proves native SQLx type identity.
- `tests/reference_live.rs` owns explicitly ignored live probes.
- `../../scripts/test_reference_live.sh` selects and verifies live execution.

## Edit here for X

Keep integration probes and application migrations here. Provisioning stays in
postgres-test-harness, supervision in Runledger, and generic test support remains
independent. Update `../../docs/reference-compatibility.md` with executed evidence.

## Invariants

Use one native SQLx graph and explicit application-owned transactions. Close pools
before consuming leases; explicitly drain deferred cleanup. Startup witnesses
prove only their observed job path. Ordinary workspace tests never require a live
database. Explicit live invocation fails when prerequisites are missing.

## Common commands

From the root, run `cargo check -p batter-example-reference-service --all-targets
--all-features --locked` and `bash scripts/test_reference_live.sh`. Follow root
two-toolchain, HTTP smoke and Jig verification requirements.
