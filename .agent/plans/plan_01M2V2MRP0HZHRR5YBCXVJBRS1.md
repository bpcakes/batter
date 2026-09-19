# Expose optional toolkit namespaces with isolated consumer checks

This is the living execution plan for Bead `batter-tmx.2`, following the
completed `batter-tmx.1` extraction. The current Git baseline is
`f4f90c9166ff255a91298c75cc020136f632d758`; the accepted core/facade source
move and its tracker/Jig state are pre-existing work for this plan. No commit,
publication, or deployment is authorized.

## Progress

- [x] Read the B acceptance, current facade/adapters, native feature manifests,
  workspace graph, and the nearest facade/adapter guides.
- [x] Claim `batter-tmx.2` only after A closed and start a new Jig plan.
- [ ] Add the exact optional facade feature/module contract with rustdoc and
  no-default behavior.
- [ ] Add isolated temporary facade consumers for positive identity, graph
  optionality, and specific negative module-gating checks on both toolchains.
- [ ] Route the feature runner through the bounded matrix and synchronized Jig
  input scopes without weakening fail-closed controls.
- [ ] Run focused checks, both full toolchain gates, and current Jig evidence;
  record acceptance and close only B.

## Decisions

### D-01 — Re-export each adapter inside a feature-gated facade module

The facade will keep `batter-core` as its only default normal dependency.
Optional adapter dependencies will be `default-features = false` and exposed
through `batter::axum`, `batter::sqlx`, `batter::runledger`, and
`batter::runlimit`. `runlimit-memory`, `runlimit-postgres`, and
`runlimit-axum` forward only the existing native/adapter features. This keeps
the adapter implementations and native policy ownership in their current
packages while giving agents one cohesive import root.

### D-02 — Test isolation with disposable external workspaces

`scripts/check_facade_features.py` will create a temporary workspace per
feature case, copy the repository lock, reconcile only the temporary root, and
run later checks with `--locked`. It will compare every selected registry/git
dependency tuple against the repository resolution, inspect normal dependency
reachability, use a separate target directory, and propagate the invoking
toolchain explicitly. Identity consumers may add direct adapter/native
dependencies in a separate temporary fixture; facade-only graph checks will not.

### D-03 — Keep the matrix bounded and fail closed

The feature runner is a separate bounded process in `scripts/test_matrix.py`.
The existing direct Runlimit eight-combination gate remains separate. Matrix
control tests will assert batch sizes, ordering, early stop, and explicit
feature inventory so a new facade feature cannot silently escape validation.

## Verification

Focused verification will cover every declared feature and documented union,
facade/core and direct/ facade type compatibility, disabled Axum/SQLx/Runledger/
Runlimit/fixture/HTTP imports, and the `axum + runlimit` versus
`runlimit-axum` distinction. Then run `bash scripts/verify.sh`,
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`, rustdoc, both direct and
facade feature runners, and fresh Jig `api:test`/contract/file-budget evidence.
No live database or hosted platform claim is made by B.

## Outcomes & Retrospective

Not complete. Record the exact matrix, toolchains, receipts, lock/source drift
checks, and any unverified external prerequisites after implementation.
