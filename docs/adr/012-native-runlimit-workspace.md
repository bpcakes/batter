# ADR-012: Native Runlimit in the shared workspace

Status: accepted for source migration epic `batter-isdr`.

## Context

The user authorized moving latest Runlimit master into Batter in a separate PR,
following the Runledger workspace migration. Batter already selects exactly
`12e035dac504a1d348c2058ee7ade8e61f2e7974`; this import needs no API or schema upgrade.

## Decision

Import all five native packages under `runlimit/` as root workspace members.
Preserve native versions (0.3.0, PostgreSQL 0.3.1), dual licenses, strict lint policy,
features, tests, source algorithms and immutable SQL. Root Cargo owns the generated
lock. All packages are unpublished. No nested workspace or consumer patch is needed.

Native packages own validated policies, keys, decisions, bounded memory storage,
replica-safe PostgreSQL persistence/migrations, HTTP fields and caller-controlled
Axum middleware. They do not depend on Batter. `batter-runlimit` separately owns
operational quota/attempt execution and protected authenticated HTTP assembly.
Default facade adoption remains free of native backend dependencies. Application
policy and credential meaning remain application-owned.

## Enforcement and consequences

Graph checks require one local unpublished identity per native package and reject
reverse dependencies or runtime/transport/database dependencies from native core.
Original SQL and license digests guard import integrity; future migrations must be
forward-only. Source archives/mutation fixtures retain the native root. Verification
preserves default/all-feature tests and lint, the release fail-closed regression,
and explicit PostgreSQL tests against a disposable database. CI keeps PostgreSQL 16;
actual local/hosted results remain separately recorded evidence.

Task `batter-isdr.2` proves Git-free exported-source native/facade consumption.
No new public Rust API is introduced: the invalid-state review retains existing
native and adapter boundaries without claiming stronger remote-effect guarantees.
Fresh-agent usability assessment remains proposed and unexecuted. Registry release,
upstream repository archival and unrelated API-hardening work are outside this PR.
See [provenance](../../runlimit/IMPORT.md).

## Facade reachability amendment

On 2026-09-30, `batter-0jsf` made all five native packages reachable through
`batter-runlimit`: `native` (`runlimit-core`) and `postgres` already existed,
`memory` was added, and the two transport packages are reached through
`native_transport::{http, axum}` behind the new `native-http` and `native-axum`
features. `native_transport` is named apart from the adapter's own `http` module
because the native layer is caller-controlled and is not the protected
quota-before-body assembly; neither feature selects `axum`, `batter-axum` or that
module. The namespaces are the native packages themselves, so a consumer needs one
`batter` dependency and cannot hold two source identities. Nothing depends on the
facade in the other direction and no ownership moves: policy validation, key
derivation, algorithms, storage and immutable migrations stay native. These
packages' public APIs are now consumer-facing through two paths, so a rename or
removal changes the facade's surface too.

## Publication amendment

On 2026-09-22, `batter-ddc` prepared all five native packages as coordinated
version 0.4.0 crates restricted to crates.io and updated their repository metadata
to this shared source repository. This semver-minor release signals the breaking
pre-1.0 API changes recorded in the changelog. Internal path dependencies carry
registry versions; native ownership, immutable migrations, algorithms, and the
prohibition on Batter dependencies remain unchanged. Artifact validation and an
explicit upload record are still required before claiming the release exists on
the registry.
