# Primary references and verification boundaries

Initially reviewed on 2026-09-07; dated follow-up checks appear below.
URLs pointing at `latest` may change;
verify the resolved Cargo.lock and pinned documentation when implementing or
upgrading adapters. These sources explain ecosystem semantics. They do not
validate Batter's source or prove any of its tests pass.

## Workspace packaging: reviewed 2026-09-08

The package split in [ADR-006](adr/006-workspace-packages.md) follows Cargo's
documented package/dependency boundaries:

- [Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html): a
  virtual manifest has no root package; members share a lockfile and output
  directory. Package metadata inheritance is opt-in, so package versions and
  Rust minimums can remain explicit.
- [Dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html#features):
  building multiple workspace members unifies their dependency features. Run
  the foundation check separately to verify its isolated dependency selection.
- [Development dependency cycles](https://doc.rust-lang.org/cargo/reference/resolver.html#dev-dependency-cycles):
  Cargo permits some cycles involving tests, but unit-test builds can expose
  incompatible copies of library types. Keep generic test support independent.
- [Rust-version resolution](https://doc.rust-lang.org/cargo/reference/resolver.html#rust-version):
  mixed-minimum workspaces use resolution heuristics, not a guarantee of each
  member's compatibility. Retain and verify the declared Rust 1.94 minimum.

These rules do not supply execution evidence. The workspace does not import the
external PostgreSQL harness or create a new database integration.

## Effect v4

Rechecked on 2026-09-08 for the [analysis reconciliation](effect-v4-reconciliation.md).
The [official RC announcement](https://effect.website/blog/releases/effect/40-rc),
updated August 12, announces the tagged candidate and presumed-final interfaces,
while allowing necessary narrow breaking changes. Q3/Q4 2026 is its stable
release target, not a guarantee. The [rc.112 release](https://github.com/Effect-TS/effect/releases/tag/effect%404.0.0-rc.112)
is dated August 25 and was the latest visible on the inspected
[release listing](https://github.com/Effect-TS/effect/releases). Use the current
Effect-TS/effect repository for this tag, not the older effect-smol repository.

The [beta announcement](https://effect.website/blog/releases/effect/40-beta)
documents the runtime rewrite, coordinated first-party package versions, and
platform/RPC/cluster consolidation into `effect`. Concrete platform, database
driver, and provider packages still exist separately. Modules under `unstable`
may change in minor releases; this is an explicit compatibility policy, not
simply a packaging switch. These statements do not imply a stable v4 release.

Version-pinned migration references:

- [Services](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/services.md):
  Context.Service replaces Context.Tag; construction still has explicit wiring.
- [Fiber references](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/fiberref.md):
  Context.Reference and scoped service provision replace FiberRef conventions;
  this is more than a spelling change.
- [Cause](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/cause.md):
  flat Fail/Die/Interrupt reasons no longer encode sequential versus parallel
  composition. Batter's internal reports are a smaller aggregation contract.
- [Layer memoization](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/layer-memoization.md):
  sharing spans provide calls, with explicit freshness/local overrides. The guide
  still recommends explicit layer composition.
- [Schema migration](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/migration/schema.md)
  documents redesigned codec, refinement, transformation, and issue APIs.
  [Schema](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/packages/effect/src/Schema.ts)
  and [JsonSchema](https://github.com/Effect-TS/effect/blob/effect%404.0.0-rc.112/packages/effect/src/JsonSchema.ts)
  are core modules. The [unstable tree](https://github.com/Effect-TS/effect/tree/effect%404.0.0-rc.112/packages/effect/src/unstable)
  has additional schema facilities, not a wholesale unstable Schema/JsonSchema API.
- [TxRef](https://effect.website/docs/v4/api/effect/TxRef): core STM operations
  coordinate through Effect.tx. Rust mutexes/atomics/channels do not reproduce
  those semantics merely by being native concurrency tools.

No primary benchmark substantiating the supplied blanket claim that streams and
batching are approximately 20 times faster was verified. Do not promote that
claim into a Batter performance comparison or implementation requirement.

The original architectural references remain useful:

- [Scope](https://effect.website/docs/v4/api/effect/Scope): managed finalization;
  Batter deliberately does not claim equivalent interruptibility guarantees.
- [Layer](https://effect.website/docs/v4/api/effect/Layer): construction/resource
  composition, distinct from Tower middleware layers. The page inspected
  identifies the v4 API as 4.0.0-rc.112.
- [ExecutionPlan](https://effect.website/docs/v4/api/effect/ExecutionPlan): the
  inspiration for coherent retry/fallback policy; Batter implements only a small
  bounded-retry subset.
- [TestClock](https://effect.website/docs/v4/api/effect/testing/TestClock): testable
  runtime time; Batter's authored timer tests use Tokio paused time instead.
- [The Effect type](https://effect.website/docs/v4/getting-started/the-effect-type):
  background for the preceding brief's Success/Error/Requirements comparison.

## Dependency refresh: 2026-09-07

Direct requirements were checked against the crates.io API's latest stable,
non-yanked releases, and Cargo refreshed the compatible transitive graph.
Two older transitive patches are required by upstream exact constraints:
Axum 0.8.9 pins `matchit =0.8.4`, and crypto-common 0.1.7 pins
`generic-array =0.14.7`. These are preserved rather than overriding upstream.

| Dependency | Previous requirement | Updated requirement |
| --- | --- | --- |
| [Tokio](https://crates.io/crates/tokio) | 1.48 | 1.53.1 |
| [tokio-util](https://crates.io/crates/tokio-util) | 0.7.16 | 0.7.19 |
| [tracing](https://crates.io/crates/tracing) | 0.1.41 | 0.1.44 |
| [thiserror](https://crates.io/crates/thiserror) | 2.0.17 | 2.0.20 |
| [Axum](https://crates.io/crates/axum) | 0.8.6 | 0.8.9 |
| [Serde](https://crates.io/crates/serde) | 1.0.228 | 1.0.229 |
| [SQLx](https://crates.io/crates/sqlx) | 0.8.6 | 0.9.0 |
| [tracing-subscriber](https://crates.io/crates/tracing-subscriber) | 0.3.20 | 0.3.23 |
| [Tower](https://crates.io/crates/tower) | 0.5.2 | 0.5.3 |

[SQLx 0.9.0 release notes](https://github.com/transact-rs/sqlx/blob/v0.9.0/CHANGELOG.md)
and its registry metadata require Rust 1.94.0. The workspace minimum is raised
to 1.94; the default toolchain is pinned to Rust 1.98.1. SQLx's selected
`runtime-tokio`, `tls-rustls-ring`, and `postgres` features remain available.
The example continues to use native pool acquisition, queries, and explicit
pool closure; transaction and remote-commit guarantees are unchanged.
See [validation](validation.md) for executed checks and remaining limitations.

## Tokio and Tokio-util

- [JoinSet 1.53.1](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinSet.html):
  direct task ownership, ID-aware joins, abort requests, and cancellation-safe joins.
- [JoinError](https://docs.rs/tokio/latest/tokio/task/struct.JoinError.html): named
  panic/cancellation observations at the task boundary.
- [Task cancellation](https://docs.rs/tokio/latest/tokio/task/): abortion requires
  runtime progress; blocking/non-yielding behavior is a separate concern.
- [CancellationToken](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html):
  downward child cancellation and drop guards; not transitive child-task joining.
- [TaskTracker](https://docs.rs/tokio-util/latest/tokio_util/task/task_tracker/struct.TaskTracker.html):
  task lifetime tracking is not an application failure supervisor.
- [Paused time](https://docs.rs/tokio/latest/tokio/time/fn.pause.html): a runtime
  testing facility, not control of database time.

## HTTP, SQL, and observability

- [Axum middleware from_fn_with_state](https://docs.rs/axum/latest/axum/middleware/fn.from_fn_with_state.html).
- [Axum graceful shutdown server](https://docs.rs/axum/latest/axum/serve/struct.WithGracefulShutdown.html).
- [Axum state](https://docs.rs/axum/latest/axum/extract/struct.State.html): application
  dependencies vs request-derived Extension data.
- [SQLx transaction 0.9.0](https://docs.rs/sqlx/0.9.0/sqlx/struct.Transaction.html):
  native commit/rollback/drop contract, not a proof about ambiguous remote commit.
- [Tracing instrumentation](https://docs.rs/tracing/latest/tracing/trait.Instrument.html):
  native async span propagation; Batter does not install a subscriber.

## Boundary conventions reviewed: 2026-09-08

- [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457.html), sections 3.1, 3.2,
  4.2.1, and 5: Problem Details uses `type` as its primary identifier, allows
  extensions, and does not require `detail` or `instance`. `about:blank` carries
  HTTP status semantics; its title should match that status phrase. A separate
  `code` extension is not a standard problem-type identifier. Response data must
  be selected to avoid exposing internals. Batter's existing basic body and tests
  do not establish an application-wide problem-type taxonomy or full conformance.
- [Tower HTTP panic middleware](https://docs.rs/tower-http/latest/tower_http/catch_panic/index.html)
  provides a native adapter candidate. No tower-http version is selected and no
  catcher is implemented in Batter; inspect the chosen release before integrating.
  Rust's [catch_unwind contract](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)
  excludes aborting panics and runs the panic hook before catching an unwind.
  Sanitizing a response does not sanitize hook output or recover arbitrary state.
- [Cargo feature compatibility](https://doc.rust-lang.org/cargo/reference/features.html#semver-compatibility):
  optional APIs still require compatibility decisions. An `unstable` feature
  name is not itself an exemption from a crate's published stability policy.

## Cancellation hardening: 2026-09-08

The resolved registry sources were inspected locally before these fixes:

- Tokio 1.53.1 `task/join_set.rs`: asynchronous join polling may exhaust its
  cooperative budget even when completion is ready. `try_join_next_with_id`
  polls without that budget. `AbortHandle::is_finished` distinguishes finished
  tasks from results still waiting to be collected; abort requests remain
  conservative when task completion races the decision.
- tracing 0.1.44 `instrument.rs`: `WithDispatch` enters its dispatcher for polling,
  not destruction. Instrumented futures do enter their span during destruction,
  which alone does not install the owning dispatcher.
- tracing-subscriber 0.3.23 `registry/sharded.rs`: nested span removal can consult
  the current dispatcher. A regression must destroy the complete inner future
  and its spans under their original dispatcher, not merely reroute one event.
- [pin-project-lite 0.2.17](https://docs.rs/pin-project-lite/0.2.17/pin_project_lite/macro.pin_project.html):
  pinned projection and `PinnedDrop` support an allocation-free private wrapper.
  A pinned `Option<F>` is cleared with safe `Pin::set` under the saved dispatcher.
  This existing transitive package is now a direct dependency; no resolved
  package version changed.

## User-owned upstream libraries

The preceding conversation reviewed these libraries' published READMEs. This
package has not compiled against them and does not assert version compatibility.
Re-read source/documentation through the relevant connected repository tool or
registry before implementing any upstream integration tracked in Beads.

- [Runlimit repository](https://github.com/bpcakes/runlimit) and
  [runlimit-core](https://crates.io/crates/runlimit-core).
- [Runledger repository](https://github.com/bpcakes/runledger) and
  [runledger-runtime](https://crates.io/crates/runledger-runtime).
- [postgres-test-harness repository](https://github.com/bpcakes/postgres-test-harness)
  and [registry entry](https://crates.io/crates/postgres-test-harness).

No upstream source or documentation is vendored. The design brief is an original
summary of the requested architecture and ownership decisions.

## Clippy configuration: 2026-09-08

[Clippy's configuration reference](https://doc.rust-lang.org/clippy/lint_configuration.html)
defines `cognitive-complexity-threshold` and `too-many-lines-threshold`.
Batter sets these to 20 and 100 and explicitly enables the corresponding
`cognitive_complexity` and `too_many_lines` lints in the workspace manifest;
all workspace packages opt into the shared lint settings.


## Tracing callsite test isolation: 2026-09-08

The [upstream callsite source](https://docs.rs/tracing-core/latest/src/tracing_core/callsite.rs.html)
describes process-wide callsite registration and cached subscriber interest.
The resolved local `tracing-core` 0.1.36 source additionally has a `JustOne`
rebuild path using the current thread's default dispatcher. A subscriber-free
thread first registering a cleanup callsite while another thread captures logs
can therefore interfere with that capture; this is the working diagnosis from
source inspection and the reproduced empty-log failure, not a deterministic
upstream race proof. The two observation tests now run in a separate executable.
No library subscriber installation or dependency change was introduced.

## Jig installation policy: 2026-09-08

The upstream [`v0.3.0` configuration documentation](https://github.com/bpcakes/jig-sh/blob/v0.3.0/docs/configuration.md)
and [installer](https://github.com/bpcakes/jig-sh/blob/v0.3.0/templates/project/scripts/install-jig.sh.jinja)
require an immutable hexadecimal `_commit` for remote automatic installation.
The [runtime configuration implementation](https://github.com/bpcakes/jig-sh/blob/v0.3.0/crates/jig/src/context.rs)
limits product-version matching to legacy contracts through version 3; this
repository uses contract 7. Selecting release `v0.3.0` resolves to commit metadata
internally. A version-only 0.3.0 runtime requirement is not a supported replacement
for the source pin. On 2026-09-08, `jig update --vcs-ref v0.3.0` resolved the
official tag to `8629700b92cd9ab8b09f8ff86de4fc1573469c83`, also checked against
the remote Git tag. `update --recopy` preserves this revision; an ordinary
`update` advances to the remote default branch.

Repository-specific workflow/ignore customizations must be reviewed after
regeneration; the full template has no per-file opt-out for all omitted helpers.

## Jig CI cache and comparison sources: 2026-09-08

The pinned [checkout v4.2.2 ref helper](https://github.com/actions/checkout/blob/11bd71901bbe5b1630ceea73d27597364c9af683/src/ref-helper.ts)
fetches branch refs under `refs/remotes/origin/*`; manual feature-branch runs
therefore use `origin/master` as their comparison base. The
[Jig v0.3.0 comparison implementation](https://github.com/bpcakes/jig-sh/blob/v0.3.0/crates/jig/src/git_receipts/comparison.rs)
recognizes all-zero push-before identities as empty-tree comparisons.

The [cache v4.3.0 action](https://github.com/actions/cache/tree/0057852bfaa89a56745cba8c7296529d2fc39830)
supports path globs and explicit keys. The v4.3.0 tag was verified against the
remote before adding it. GitHub's
[cache reference](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)
describes key matching, branch scope, and eviction; a configured cache does not
guarantee a hit. This repository caches `.git/jig-tools/*-runtime` only and
continues to run Jig's normal source/profile compatibility checks after restore.
