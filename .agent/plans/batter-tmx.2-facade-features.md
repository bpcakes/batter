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
- [x] Add the exact optional facade feature/module contract with rustdoc and
  no-default behavior.
- [x] Add isolated temporary facade consumers for each public feature, the
  distinct bridge unions, one all-feature identity consumer, and focused
  negative module-gating checks on both toolchains.
- [x] Route the feature runner through the bounded matrix and synchronized Jig
  input scopes without weakening fail-closed controls.
- [x] Run focused checks, both full toolchain gates, and current Jig evidence;
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
It checks each public feature, the bridge unions with distinct graph or API
contracts, seven focused missing-module fixtures, and one all-feature identity
consumer. It does not enumerate the Cartesian product: the adapter suites and
the all-feature consumer already cover repeated unions. The existing direct
Runlimit eight-combination gate remains separate. Matrix control tests assert
batch sizes, ordering, early stop, and explicit feature inventory so a new
facade feature cannot silently escape validation.

## Verification

Focused verification covers the empty default graph, each declared individual
feature, the Axum/SQLx and Runlimit unions with distinct graph contracts, one
all-feature facade/core and direct-adapter identity fixture, seven focused
disabled Axum/SQLx/Runledger/Runlimit/fixture/HTTP imports, and the `runlimit`
versus `runlimit-axum` distinction. It does not enumerate redundant feature
products. The separate Runlimit runner still covers all eight native feature
combinations. `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed, including rustdoc,
doctests, direct Runlimit checks and the facade runner. The ten HTTP smoke
profiles passed after rebuilding on both toolchains. Contract and Jig
integration checks passed; fresh work evidence is recorded below. No live
database or hosted platform claim is made by B.

## Outcomes & Retrospective

Completed locally on 2026-09-18 on macOS arm64 (`aarch64-apple-darwin`). The
facade runner uses 14 positive consumers, 7 expected missing-module consumers,
and 1 all-feature identity consumer. Every temporary graph preserved the
repository's source tuples and copied-lock SHA-256
`09c9a990a01b6f80c28de310f17d1c145dcacddc2690b0516fe3dd2697c4f808`; the
repository lock remained unchanged. The runner passed under
`1.98.1-aarch64-apple-darwin` and `1.94.0-aarch64-apple-darwin`.

The full verification commands passed on both toolchains. The rebuilt
`batter-axum` `http_service` binary passed default, SIGINT, deadline,
warn-filter, and warn-filter-plus-deadline smoke modes on each toolchain.
`cargo fmt --all -- --check`, `git diff --check`, `scripts/jig check contract`,
`python3 scripts/test_jig_integration.py`, and the matrix fail-closed controls
passed. Live PostgreSQL, hosted CI, publication, deployment, commit and push
remain unverified or unauthorized. Final Jig receipts and tracker closure are
recorded after the owning work check.
