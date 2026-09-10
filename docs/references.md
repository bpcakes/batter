# Primary references and verification boundaries

Initially reviewed on 2026-09-07; dated follow-up checks appear below.
URLs pointing at `latest` may change;
verify the resolved Cargo.lock and pinned documentation when implementing or
upgrading adapters. These sources explain ecosystem semantics. They do not
validate Batter's source or prove any of its tests pass.

## SQLx PostgreSQL disposition, reviewed 2026-09-09

For `batter-7r3.2`, inspected the Cargo registry sources for `sqlx-core` and
`sqlx-postgres` **0.9.0**, the exact workspace lock resolution, and these primary
versioned sources. This is independent of the larger durable-runtime/harness
compatibility graph.

- [PoolConnection 0.9.0 API](https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolConnection.html)
  and [pool implementation](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/pool/connection.rs):
  `detach` removes pool accounting and allows replacement; `close` retains its
  permit while awaiting closure. Ordinary Drop spawns return work, whose ping
  precedes slot release. `close_on_drop` uses a bounded asynchronous close path,
  not the synchronous capacity-release contract selected here. Nonzero minimum
  connections can spawn replacement work after detachment.
- [PostgreSQL connection implementation](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/connection/mod.rs):
  `ping` writes Sync and awaits readiness. Graceful close sends Terminate; the
  adapter instead detaches and drops client ownership without claiming a remote
  acknowledgement. Live evidence, not source inspection, establishes observed
  residual locked sessions and their later disappearance.
- [Native transaction implementation](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/transaction.rs):
  commit/rollback await the native transaction manager; dropping an open
  transaction starts rollback without awaiting acknowledgement. The adapter
  therefore requires explicit completion before ordinary pool return.
- [Native error variants](https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/error.rs):
  native pool timeout differs from operation interruption. Database and transport
  failures retain their original causes. Neither the native enum nor this
  adapter's diagnostic categories establish safe replay of an uncertain commit.
- [PostgreSQL 18 activity observation](https://www.postgresql.org/docs/18/monitoring-stats.html#MONITORING-PG-STAT-ACTIVITY-VIEW)
  and [advisory locks](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS):
  session locks support explicit unlock; own-session activity is visible to its
  role. Test observers use independent autocommit queries to avoid retaining an
  activity snapshot inside a transaction. PID observations distinguish local
  capacity release from backend termination; they do not imply remote cancellation.

The package retains Rust 1.94 and SQLx runtime-tokio/PostgreSQL features; consumers
select TLS. No upstream external package or checksum changed in the Cargo-generated
lockfile. Linux execution and unverified platforms are recorded in [validation](validation.md).

## Exit boundaries and subprocess controls: 2026-09-09

Rechecked Rust 1.98.1's
[`Termination` implementation for `Result`](https://doc.rust-lang.org/src/std/process.rs.html#2718-2727):
an error returned from `main` is formatted through Debug. Sanitizing a report's
Display does not sanitize this path or an arbitrary retained domain error.
The shared-report example now includes the outer `ExitCode` boundary and fixed
output. Internal error propagation and concrete source identity are unchanged.

The [rustdoc attribute contract](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#attributes)
only requires a `compile_fail` example to fail compilation. The
[`expect` attribute](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-expect-attribute)
supports a stronger companion check: require the precise `unused_must_use`
diagnostic and deny unfulfilled expectations. That check now covers raw cleanup
and shutdown reports as well as shared owned reports.

Python's [subprocess documentation](https://docs.python.org/3.12/library/subprocess.html#subprocess.Popen.communicate)
distinguishes output collection, timeout, killing and waiting; a timeout from
`communicate` does not itself kill a child. Rust's
[`Child` documentation](https://doc.rust-lang.org/std/process/struct.Child.html)
likewise states that dropping the handle does not wait for the child. The SQLx
smoke and configuration tests therefore share the existing Unix process owner
in `scripts/scheduling_process.py`, including bounded output, process-group
cleanup and direct-child reaping. A readiness phase selects when to signal and
starts a separate shutdown deadline. The local Python version is 3.12.3.
These deadlines cannot preempt process creation, OS scheduling, or destruction
of the watchdog parent. Synthetic protocol tests do not establish database
cleanup; live SQLx checks remain separately selected.

## Database lifecycle exit coverage: 2026-09-09

SQLx remains locked to 0.9.0. Its locally cached `sqlx-core/src/pool/mod.rs`
documents that awaiting `Pool::close` closes idle connections and waits for
checked-out connections, while later acquisition returns `Error::PoolClosed`.
The versioned [Pool documentation](https://docs.rs/sqlx/0.9.0/sqlx/struct.Pool.html#method.close)
could not be retrieved; these semantics were checked in the resolved source.
The live tests use PostgreSQL's documented
[`22012` division-by-zero SQLSTATE](https://www.postgresql.org/docs/18/errcodes-appendix.html),
rather than matching a localized server message, to verify concrete cause
retention. Test provisioning is external and no schema changes are made.

The shared shutdown wrapper retains the concrete report through
[`Error::source`](https://doc.rust-lang.org/std/error/trait.Error.html#method.source)
and displays distinct owner context, avoiding a duplicate report summary when
a diagnostic sink walks that chain. Neither the library nor the example exit
handler automatically prints that chain.

## Shared shutdown report lint: 2026-09-09

The [Rust must_use reference](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute)
defines diagnostics for discarded values of an attributed type; wrapping that
type does not generally propagate the attribute. The owned driver now returns
an attributed `SharedShutdownReport` instead of a raw `Arc<ShutdownReport>`.
Binding or explicitly discarding the wrapper still bypasses the warning.
The [lint expectation reference](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-expect-attribute)
supports controls that fail if the expected lint is absent. Repository tests
use `expect(unused_must_use)` with `deny(unfulfilled_lint_expectations)` to check
the actual owned-driver expressions, alongside compile-fail doctests. Executed
Rust 1.98.1 and 1.94.0 evidence is recorded in [validation](validation.md).

## Error handling corrections: 2026-09-09

Rust 1.98.1's [Result termination implementation](https://doc.rust-lang.org/src/std/process.rs.html#2718-2727)
prints an Err through Debug. A non-Unicode DATABASE_URL therefore exposes its
original OsString when propagated directly out of main. The SQLx example now
returns ExitCode through an explicit sanitized handler, retaining the failure
object until that boundary. Rust's default panic hook is unaffected.

Checked the resolved thiserror 2.0.20 implementation (`src/aserror.rs` and
`thiserror-impl/src/expand.rs`) against the [source attribute contract](https://docs.rs/thiserror/2.0.20/thiserror/).
Marking Arc<E> as the source exposes that wrapper; its delegated source skips E
itself. The receipt now implements [Error::source](https://doc.rust-lang.org/std/error/trait.Error.html#method.source)
manually to return E, matching the existing heterogeneous report wrapper.

The locally cached tracing-subscriber 0.3.23 `filter/env/mod.rs` and
`filter/env/builder.rs` distinguish environment and parse failures in FromEnvError.
The example uses `EnvFilter::try_new` after explicit environment handling, and
preserves both kinds of failure while redacting Debug/Display. Only NotPresent
selects the default. The versioned docs.rs EnvFilter page could not be retrieved;
these details were checked in the resolved source, not inferred from latest docs.
No dependency or lockfile changes were needed.

## Owned completion observers: 2026-09-09

Rechecked Cargo.lock, cached Tokio 1.53.1 source and its pinned documentation.
[`watch::Receiver::changed`](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Receiver.html#method.changed)
returns a receive error when all senders have been dropped and the current value
has been seen. Keeping a control handle alive previously retained a sender even
when no driver could publish; dropping that handle exposed the observer panic.
The completion channel now exists only inside `Supervisor::start`, with its
sender transferred to the owned monitor before returning `RunningSupervisor`.

[`tokio::spawn`](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn.html) never polls
the spawned future synchronously. The current-thread observer regression drops
the last driver owner without awaiting after `start`, then observes cleanup and
the retained report. This proves the pre-first-poll ownership case without a
scheduler timing assumption; the runtime must still remain alive.

The same version's cached runtime source and
[`Runtime` shutdown documentation](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Runtime.html#shutdown)
confirm that spawned tasks need not run to completion when the runtime shuts
down. The observer runtime-loss regression enters a current-thread runtime
without driving it, then drops that runtime and observes the closed sender on a
second runtime. A separate control publishes a report first and retains it after
runtime destruction. These cases document the existing panic boundary, without
claiming that runtime destruction can preempt non-yielding work or run finalizers.

## Terminal process admission: 2026-09-09

The [Rust destructor reference](https://doc.rust-lang.org/reference/destructors.html#destructors.operation)
specifies struct field destruction in declaration order. The supervisor and its
private caller-owned driver wrapper put the abandonment guard before application
captures/the inner future. The wrapper uses the existing pin-project-lite 0.2.17
dependency for safe projection, with no new allocation or dependency. This avoids
relying on async capture destruction order for abandonment-before-capture signaling.

Cargo.lock resolves Tokio 1.53.1. Its locally cached source and rustdoc for
[`mpsc::Sender::is_closed`](https://docs.rs/tokio/1.53.1/tokio/sync/mpsc/struct.Sender.html#method.is_closed)
confirm that receiver drop or explicit receiver closure closes the channel.
Process admission checks this before startup errors so a retained handle cannot
mistake a dropped unstarted supervisor for one that may still start. The enqueue
operation retains its own closed-channel error handling for concurrent closure;
the preliminary check is not a reservation or a guarantee that the receiver stays alive.

The same cached Tokio version's `sync/mpsc/chan.rs` calls the receiver waker
from its send path. Readiness therefore retains an atomic read snapshot while
enqueue holds admission. Its sole writer is private and takes the same mutex
guard as state transitions; explicit notification/cancellation runs after release.
Abandoned supervisors now signal lifecycle closure explicitly as well as dropping
the queue. The closed-channel check remains a defensive admission condition.

## Scheduling process ownership: 2026-09-08

The [Python 3.12 subprocess contract](https://docs.python.org/3.12/library/subprocess.html)
distinguishes child waiting from pipe communication, documents partial output on
timeout, and says a Popen context exit waits for its child. Inspection of the
installed Python 3.12.3 source and the [matching CPython source](https://github.com/python/cpython/blob/v3.12.3/Lib/subprocess.py)
confirmed the unbounded context-exit wait and the ECHILD path that can substitute
return code zero when child status is unavailable. The scheduling process owner
therefore uses bounded polling, preserves output as result data and requires the
default SIGCHLD disposition without installing a handler.

Python's [Unix process-group operations](https://docs.python.org/3.12/library/os.html#os.killpg)
and `Popen(start_new_session=True)` support a separately owned group. The
[POSIX read contract](https://pubs.opengroup.org/onlinepubs/009604599/functions/read.html)
requires all pipe writers to close before EOF; reaping one child is insufficient.
Group termination cannot cover a descendant that starts another session. The
runner records incomplete EOF after its cleanup allowance instead of discarding
checkpoints or claiming that escaped work stopped. These source checks do not
establish an OS scheduling or process-creation latency guarantee.

Follow-up checks of [Linux wait semantics](https://man7.org/linux/man-pages/man2/wait.2.html)
and Apple's [wait documentation](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/wait.2.html)
confirm that waiting reclaims a terminated child's resources. CPython's `poll()`
performs that wait; cached `returncode` is therefore also an ownership boundary.
The runner defers polling while either output pipe remains open, sends any needed
group signal before reaping, and forbids signalling a cached reaped child. This
keeps the numeric group identifier reserved through cleanup without another wait
API. Python documents that process creation cannot always be interrupted by a
timeout, so control tests include explicit startup overhead instead of treating
their observation and cleanup budgets as the complete outer elapsed-time bound.

The fixture-ownership correction rechecked the same Python 3.12 subprocess
contract and [pipe descriptor API](https://docs.python.org/3.12/library/os.html#os.pipe).
Popen accepts an existing file descriptor for stdout. The escaped-pipe control
therefore shares a test-created pipe between two directly owned children in
separate sessions, retaining the outside-group writer's handle until bounded
termination and reaping. This exercises incomplete EOF without relying on an
orphan's stale PID file. The unjoined fixture's watchdog now budgets startup plus
the complete case allowance before its hang-observation allowance; a two-second
startup regression checks that required report evidence survives.

The [Cargo resolver contract](https://doc.rust-lang.org/cargo/reference/resolver.html#lock-file)
prioritizes compatible lockfile entries; `--locked` refuses changes. Omission of
resolver fallback was not a demonstrated cause of failure in the already compatible
locked graph. The mutation copy now retains and records the repository Cargo
configuration for fidelity. No dependency refresh accompanies this correction.

## SIGINT ownership in process tools: 2026-09-08

Research preceded the correction. Python's [3.12 signal guidance](https://docs.python.org/3.12/library/signal.html#note-on-signal-handlers-and-exceptions)
explains that exceptions raised by signal handlers can appear between arbitrary
instructions, including resource acquisition and context-manager entry. It
recommends non-raising SIGINT handling for orderly shutdown. The [signal API](https://docs.python.org/3.12/library/signal.html#signal.signal)
allows handler installation only from the main interpreter's main thread and
returns the previous handler. The initial correction checked default handler
ownership before launch. The inherited-policy correction below distinguishes
standard signal dispositions from competing handlers.

The installed Python 3.12.3 `subprocess.py` was inspected alongside the
[versioned source](https://github.com/python/cpython/blob/v3.12.3/Lib/subprocess.py).
Its KeyboardInterrupt waiting behavior assumes an interactive child may also
have received the interrupt; that does not establish ownership of a child in
another session. Its `poll()` remains the explicit child-status/reaping operation.
The existing bounded selector polling can observe a non-raising stop request
without an extra signal thread or wakeup pipe. Cleanup deliberately does not use
that stop request and retains its one absolute deadline. Explicitly raised callback
exceptions still follow ordinary cleanup; no general asynchronous-exception safety
or hard process-creation/scheduling latency guarantee is claimed.

## Inherited SIGINT policy in scheduling tools: 2026-09-08

Research preceded implementation. The [Python 3.12 signal API](https://docs.python.org/3.12/library/signal.html#signal.getsignal)
distinguishes ignored and default dispositions from callable or unknown native
handlers. Inspection of [CPython 3.12.3 signal initialization](https://github.com/python/cpython/blob/v3.12.3/Modules/signalmodule.c)
confirmed that Python installs its raising SIGINT handler only over SIG_DFL,
preserving inherited SIG_IGN. The [subprocess contract](https://docs.python.org/3.12/library/subprocess.html#subprocess.Popen)
lists the signals reset by `restore_signals`; SIGINT is not among them.
The [GNU Bash signal rules](https://www.gnu.org/s/bash/manual/html_node/Signals.html)
document ignored SIGINT for asynchronous commands without job control. Local
`/bin/sh` and Cargo background-launch probes reproduced this inherited disposition.
The POSIX shell page returned HTTP 403 during this check; its contents were not
used as retrieved evidence.

The resulting tool policy accepts standard SIGINT dispositions. SIG_IGN stays
ignored, including across child execution; Python-default and SIG_DFL use the
existing non-raising stop recorder. The exact prior disposition is restored after
resource release. Main-thread and default-SIGCHLD requirements remain, and custom
or unknown SIGINT handlers still fail before launch. Real background-shell tests
verify both successful child launch and ignored signal delivery; this is distinct
from tests that explicitly choose an active SIGINT handler.

## Closure and partial capture setup: 2026-09-08

Research preceded this correction. Python's [3.12 selector contract](https://docs.python.org/3.12/library/selectors.html#selectors.BaseSelector.close)
requires explicit close to release the underlying resources. The installed
[CPython 3.12.3 selector implementation](https://github.com/python/cpython/blob/v3.12.3/Lib/selectors.py)
exposes the underlying epoll/kqueue descriptor through `fileno()`. A regression
retains that selector object and checks descriptor closure after either stream
setup or registration raises, so collection cannot conceal a missing close.
Only Linux/Python 3.12.3 execution is established here.

Rechecked the pinned [Tokio 1.53.1 task cancellation contract](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.53.1/tokio/src/task/mod.rs)
and [paused clock implementation](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.53.1/tokio/src/time/clock.rs).
An abort request can race normal completion. Paused time advances idle timers,
including while a task awaits a channel; it can exercise cancellation expiry
without a wall-clock sleep. The closure oracle therefore accepts permitted
completion/termination while checking exact report outcomes and conservative
cleanup after every abort request. Post-closure admission remains forbidden.

## Python probe optimization: 2026-09-08

The [Python 3.12 assert reference](https://docs.python.org/3.12/reference/simple_stmts.html#the-assert-statement)
states that optimized compilation omits assertion statements. The
[command-line/environment reference](https://docs.python.org/3.12/using/cmdline.html#envvar-PYTHONOPTIMIZE)
defines `PYTHONOPTIMIZE` as enabling optimization like `-O` (or repeated `-O`
for integer levels). Checked against the 3.12 documentation for the locally
installed Python 3.12.3. The parent-death probe therefore uses explicit failure
branches, with real-process positive and negative controls under optimization;
executed results are recorded in [validation](validation.md).

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

### Service and finite-command examples: 2026-09-10

Rechecked Cargo.lock and the cached Tokio 1.53.1 source for the native primitives
used by `batter-7r3.7`; no version or feature change was needed.

- [Semaphore::acquire_owned](https://docs.rs/tokio/1.53.1/tokio/sync/struct.Semaphore.html#method.acquire_owned)
  consumes an `Arc` and yields an owned permit. Its native Drop returns capacity;
  the startup example keeps that permit in a finalizer until the service joins.
- [UdpSocket](https://docs.rs/tokio/1.53.1/tokio/net/struct.UdpSocket.html)
  supports native bind/send/receive and shared `Arc` ownership through `&self`
  methods. The finite command retains a socket owner for explicit cleanup;
  closing that native socket requires no async close method. This is a local
  resource-lifetime example, not a database/session termination analogue.

Execution and error/cleanup controls belong in [validation](validation.md).

## Independent HTTP observation reviewed: 2026-09-08

Resolved and checked Axum 0.8.9 in Cargo.lock and the local Cargo source.
The current primary documentation also identifies 0.8.9:

- [Router::layer source documentation](https://docs.rs/crate/axum/latest/source/src/docs/routing/layer.md):
  middleware covers previously assembled routes/fallback and runs after routing.
  Routes added afterward are not covered.
- [Router::route_layer source documentation](https://docs.rs/crate/axum/latest/source/src/docs/routing/route_layer.md):
  middleware runs on matching routes, allowing unmatched fallback to retain its
  response instead of receiving an admission rejection.
- `axum-0.8.9/src/routing/path_router.rs`, `call_with_state`, inserts MatchedPath
  for a matched non-fallback route before invoking its endpoint. Inspection and
  the local [placement tests](../crates/batter-axum/tests/observation/placement.rs)
  establish that a service wrapper outside routing has no matched template at
  observer entry. It records `<unmatched>` without substituting a raw URI.

These semantics determine the assembled-router observer placement. Public
`observe_http` and `request_admission` are additive; `request_scope` remains a
combined wrapper and nested observers are not deduplicated. No upstream version
or Cargo.lock change is required. Destruction and event-count evidence belongs
in [validation](validation.md), not in upstream documentation claims.

## HTTP observation severity reviewed: 2026-09-09

Resolved Axum 0.8.9 and tracing 0.1.44 remain unchanged in Cargo.lock.
[Axum Extension response documentation](https://docs.rs/axum/latest/axum/struct.Extension.html#as-response)
identifies 0.8.9 and describes response extensions as application-to-middleware
metadata. The local `axum-0.8.9/src/extension.rs` implements IntoResponseParts by
inserting T into the response extensions; its Layer implementation instead
inserts into request extensions. Batter uses only the response-side value.

[Tower HTTP failure classification](https://docs.rs/tower-http/latest/tower_http/classify/struct.ServerErrorsAsFailures.html)
and [DefaultOnFailure::level](https://docs.rs/tower-http/latest/tower_http/trace/struct.DefaultOnFailure.html#method.level),
identifying 0.7.1 when reviewed, provide a comparison: 5xx responses are failures,
while event severity is configurable (default ERROR). Tower HTTP is not a
dependency of Batter; this comparison does not claim equivalent lifetimes.

[OpenTelemetry HTTP span status](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#status)
(semantic conventions 1.44.0 when reviewed) generally recommends Error for 5xx
and permits more precise classification with additional request context.
[Log severity](https://opentelemetry.io/docs/specs/otel/logs/data-model/#field-severitynumber)
is a separate field; these specifications do not prescribe WARN for a readiness
503. Batter retains its existing HTTP status-class outcome and makes only event
severity explicit. This is not a claim of OpenTelemetry instrumentation.

[Kubernetes probes](https://kubernetes.io/docs/concepts/workloads/pods/probes/)
describe readiness for initialization, maintenance and temporary unavailability;
readiness failure controls traffic and need not require restarting the process.
The observer cannot infer whether a particular failure is expected. Application
policy supplies `HttpObservationLevel` on the completed response. The runnable
example treats Starting/Draining readiness responses as INFO, leaving Stopped at
the normal 503/WARN default. HTTP status/outcome alerts still need probe policy.

[EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
selects enabled events by level and other directives; it does not rewrite event
severity. The resolved tracing-subscriber 0.3.23 test fixture captures all five
selected levels; the default INFO subscriber remains the ordinary test baseline.

## HTTP event fields and span filtering: 2026-09-09

Rechecked Cargo.lock and installed sources: tracing 0.1.44, tracing-core 0.1.36,
tracing-subscriber 0.3.23 and Axum 0.8.9 are unchanged. The versioned
[Span documentation](https://docs.rs/tracing/0.1.44/tracing/struct.Span.html#method.new_disabled)
says recording on a disabled span does not notify the subscriber. Therefore an
enabled completion event cannot rely on a separately filtered span for its fields.
Local tracing-core 0.1.36 `event.rs` exposes event fields through `Event::record`;
`field.rs` implements `Value for Option<T>` by recording only `Some` values. The
HTTP event uses that behavior to omit status when no response exists. Local Axum
0.8.9 `extract/matched_path.rs` stores the cloneable route template in `Arc<str>`,
so retaining `MatchedPath` does not retain the raw request or add a copied path.
Direct event-visitor regressions exercise these semantics without span formatting.

## Filtered process-task and cleanup context: 2026-09-09

For `batter-cpb`, rechecked resolved tracing 0.1.44 and tracing-subscriber 0.3.23
against Cargo.lock and their installed sources. The versioned
[`Instrument` documentation](https://docs.rs/tracing/0.1.44/tracing/trait.Instrument.html#method.instrument)
explicitly describes loss of the current parent when spawning with a disabled
span, and recommends `Span::or_current` before the handoff. `instrument.rs`
enters the retained span on polling and inner-future destruction. The existing
Batter dispatch wrapper remains outside instrumentation to protect complete
future/span destruction. No dependency or public API changes are needed.

The finite admission path selects the fallback before taking its mutex:
[`Span::current`](https://docs.rs/tracing/0.1.44/tracing/struct.Span.html#method.current)
queries the application subscriber and must not execute under the admission
lock. Mixed-target-filter regressions cover all three task boundaries on both
current-thread and multi-thread Tokio runtimes; this does not guarantee delivery
through every subscriber layer or exporter.

The follow-up `batter-cpb.1` test implements the native `Subscriber` callbacks
and delegates span bookkeeping to `tracing_subscriber::Registry`. Its test-only
direct dependency on the already resolved tracing-core 0.1.36 names
`tracing_core::span::Current`, the return type of `Subscriber::current_span`
which tracing 0.1.44 does not re-export. Cargo adds only that dev-dependency edge
to the lockfile; no resolved package version or production dependency changes.

## HTTP context ownership and panic policy: 2026-09-09

Before implementation, rechecked the unchanged resolved Axum 0.8.9, Tower 0.5.3,
tracing 0.1.44, tracing-core 0.1.36 and tracing-subscriber 0.3.23 sources and
primary documentation:

- [`Span::or_current`](https://docs.rs/tracing/0.1.44/tracing/struct.Span.html#method.or_current)
  explicitly supports retaining the current span when a child span is disabled.
  `Span::current` clones its span handle and subscriber. Batter selects this
  context at first poll, not from whichever request is current at completion or
  destruction. The HTTP field span remains separate to avoid recording into the
  inherited application's fields.
- [`Event::new_child_of`](https://docs.rs/tracing-core/0.1.36/tracing_core/struct.Event.html#method.new_child_of)
  and local `event.rs` distinguish an explicit absent parent (`Parent::Root`)
  from contextual parenting. Thus using a disabled HTTP span as an explicit
  parent loses an enabled application parent; removing the explicit parent or
  finding a fallback at Drop can instead adopt another request's identity.
- [`Instrument`](https://docs.rs/tracing/0.1.44/tracing/trait.Instrument.html)
  and local `instrument.rs` enter the retained span during polling and inner
  destruction. Batter's existing dispatcher wrapper additionally protects full
  destruction. No second pin/drop mechanism or foundation API change is needed.
- [`EnvFilter`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.EnvFilter.html)
  supports target directives and per-layer filtering. Local `layer/context.rs`
  resolves explicit event parents through the layer's span visibility. Retaining
  globally available context does not force every sink to display it. Event
  fields and HTTP span fields remain intentionally duplicated for their distinct
  uses; flattening/exporter conventions remain application-owned.
- [Tower HTTP 0.7.0 panic middleware](https://docs.rs/tower-http/0.7.0/tower_http/catch_panic/index.html)
  provides opt-in conversion of handler panics to responses. It was inspected,
  not added as a dependency. Batter keeps Axum/Tower's existing propagation:
  an unwind before a response emits a sanitized dropped observation and cancels
  admitted context, without inventing an HTTP status. The default panic hook can
  still print the payload, and aborting panics are outside unwinding guarantees.

The consumer composition and related metadata/exporter tasks confirm that trusted
identity and sink policy belong to applications. Native `tracing::Level` and all
five explicit response levels remain appropriate; no severity floor, duplicate
level type, panic catcher or ambient identity abstraction is introduced. Tests
exercise both independent observation and the retained combined entry point.
The broader connection/streaming work stays with its owning transport task.

The follow-up composition tests rechecked resolved Axum 0.8.9's local
`routing/path_router.rs` and `routing/method_routing.rs`: `Router::route_layer`
maps each matched path endpoint through `layer`, including its method fallback.
This differs from `MethodRouter::route_layer`, which wraps only registered methods.
The tested router therefore rejects a matched unsupported method through admission
while unavailable, and returns 405 while Ready. No package version changed.

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

## Non-yielding subprocess evidence: 2026-09-08

Checked the resolved Tokio 1.53.1 registry source and primary documentation
before implementing the test fixtures. No dependency version changed.

- [Tokio 1.53.1 Runtime shutdown](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Runtime.html#shutdown):
  spawned async work stops only once it yields; runtime Drop can wait indefinitely.
  `shutdown_timeout` can release the caller while leaving work running, so the
  fixture uses ordinary Drop and lets the OS parent observe/kill the stuck child.
- [Tokio 1.53.1 timeout](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html):
  timeouts cannot interrupt a future that fails to yield during execution.
- [Tokio 1.53.1 scheduling](https://docs.rs/tokio/1.53.1/tokio/runtime/#multi-threaded-runtime-behavior-at-the-time-of-writing):
  a worker's LIFO wake slot cannot be stolen by another worker. The resolved
  `src/runtime/mod.rs` documents this detail. An initial fixture stalled after
  requesting drain from the blocking task, consistent with that wake-slot
  behavior. The final fixture requests drain from an OS thread after receiving
  explicit task-entry acknowledgement.
- [Rust Child ownership](https://doc.rust-lang.org/std/process/struct.Child.html):
  dropping Child does not kill or wait. `kill` alone does not reap; the parent
  explicitly waits, and keeps the same ownership guard active through errors
  and unwinding. The lifecycle fixtures create no descendant OS processes;
  the separate parent-death probe creates an owner process and adopts its child.

## Subprocess review follow-up: 2026-09-08

The scheduling escalation correction also checked the resolved Tokio 1.53.1
`src/time/clock.rs` and primary versioned documentation. [Paused time](https://docs.rs/tokio/1.53.1/tokio/time/fn.pause.html)
requires the current-thread runtime, leaves `std::time::Instant` unchanged, and
automatically advances when no runnable work remains. [Time advancement](https://docs.rs/tokio/1.53.1/tokio/time/fn.advance.html)
does not ensure every expired timer has been processed before returning; the
regression uses ordinary Tokio sleep under paused time to order the completion
deadline after the shutdown deadlines. A separate real thread sleep demonstrates
that wall-clock delay does not consume a paused phase allowance. These are
controlled test conditions, not a multi-threaded scheduler latency guarantee.
The pinned [Tokio cancellation documentation](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.53.1/tokio/src/task/mod.rs)
also allows a task to complete normally after an abort request if it does not
yield again. The live oracle therefore preserves successful completion counts
while still requiring conservative cleanup after any recorded abort request.

The review's CI cancellation question was researched before the follow-up fix.
The checked-in Rust workflow uses GitHub-hosted Ubuntu runners and does not
pin the runner program version. No hosted execution was observed here.

- [GitHub cancellation reference](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-cancellation)
  describes SIGINT, then SIGTERM, then process-tree termination when the step
  entry process remains alive. This is not an unconditional parent-death link.
- [Runner v2.337.0 process handling](https://github.com/actions/runner/blob/397b032cbf865e9c3ddfab89d533ec19325e1273/src/Runner.Sdk/ProcessInvoker.cs)
  sends Unix signals to one PID; `NixKillProcessTree` calls `Process.Kill()`
  without its tree argument. The inspected release was published 2026-08-26.
  [Job finalization](https://github.com/actions/runner/blob/397b032cbf865e9c3ddfab89d533ec19325e1273/src/Runner.Worker/JobExtension.cs)
  separately scans inherited `RUNNER_TRACKING_ID` values and kills matching
  orphan processes. The inference for this harness is to provide its own
  containment when the owner dies, rather than depend on finalization running.
- [Rust anonymous pipes](https://doc.rust-lang.org/std/io/fn.pipe.html), stable
  since 1.87: reads observe EOF after all writers close; writers can block when
  buffers fill. The parent retains the launch writer and concurrently drains
  output. Dropping Command's retained writers is necessary before awaiting EOF.
- [Rust process exit](https://doc.rust-lang.org/std/process/fn.exit.html) ends
  the process without unwinding Rust stacks. The fixture's parent-disconnect
  and emergency-deadline exits deliberately provide no finalization evidence.
- [Rust Child::kill](https://doc.rust-lang.org/std/process/struct.Child.html#method.kill)
  may return success for an already-exited process. The harness retains actual
  wait status and separately records its kill request. The Unix-only harness
  checks SIGKILL, rejecting numeric exit codes as evidence of that signal.
- [Linux child subreapers](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html)
  adopt orphaned descendants and can wait for their termination. Only the
  isolated Python regression process enables this setting; it allows the test
  to prove reaping without assuming PID 1's behavior or changing Cargo's state.

## Subprocess evidence clocks and platform research: 2026-09-08

Research preceded the evidence/policy refactor. No dependency version changed.

- [Rust 1.98.1 thread sleep](https://doc.rust-lang.org/std/thread/fn.sleep.html)
  promises a minimum duration, with possible scheduling overshoot. A sleep near
  a startup deadline is therefore an unsuitable way to test deadline arithmetic.
  The real blocked-task observation still uses an OS sleep because elapsed wall
  time is itself the behavior being demonstrated.
- [Rust 1.94 Unix process implementation](https://github.com/rust-lang/rust/blob/1.94.0/library/std/src/sys/process/unix/unix.rs#L918)
  checks cached reaped status before signalling and before later wait/try_wait
  calls. This path covers Linux and macOS; a later kill call cannot replace a
  cached natural exit with SIGKILL. This source fact is distinct from executing
  the harness on either platform.
- [GitHub runner labels](https://github.com/actions/runner-images#available-images)
  map `macos-latest` to macOS 26 ARM64 at this research date.
  Its [software inventory](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md#rust-tools)
  lists Rustup 1.29.0. The review claim that the job necessarily lacked rustup
  was incorrect. The existing install command needs no replacement action.
- Read-only inspection of [hosted baseline run 34209833620](https://github.com/bpcakes/batter/actions/runs/34209833620)
  found successful Linux jobs for Rust 1.94.0, 1.98.1 and stable at
  `e5f2f04b2dbb349d08085caf177f662fcbc89812`, with no macOS job. It cannot validate
  this working-tree diff. Execution of the current source on the user-provided
  macOS host is recorded separately in [validation](validation.md).

The resulting design separates bounded diagnostic capture and timestamped
protocol evidence from pure fixed/after-event deadline decisions. The reader
records timestamps and the watchdog samples evidence/time under one mutex.
This gives a consistent parent observation; it does not claim the child emitted
a line at that exact instant or guarantee scheduling during OS suspension.

## Synchronization review research: 2026-09-08

Research preceded this follow-up. The latest read-only GitHub run query still
reports successful runs at `e5f2f04b2dbb349d08085caf177f662fcbc89812`, not this
working tree. Prior execution plans record intentional focused macOS CI and
the removal of environment-based launch authority. Neither requires changing
platform policy or removing its regression control.

- [Rust Child::try_wait](https://doc.rust-lang.org/std/process/struct.Child.html#method.try_wait)
  reports process status; it does not join this harness's independent pipe
  reader. Therefore the harness must join capture before rejecting final
  startup evidence after an observed exit.
- [Python signal behavior](https://docs.python.org/3/library/signal.html#signal.alarm)
  schedules SIGALRM, while Python handlers themselves run later in the main
  interpreter thread. The probe deliberately keeps a hard default alarm rather
  than introducing asynchronously raised exceptions. It can bypass `finally`;
  that fallback is process containment, not cleanup evidence. Python versions
  exercised here are recorded in validation; no Python dependency was added.
- [Python subprocess pipes](https://docs.python.org/3/library/subprocess.html#subprocess.Popen.wait)
  can deadlock waits if output fills an undrained pipe. Both overflow controls
  require natural child exit after flooding output, independently of rejecting
  truncated evidence. Event and byte limits keep distinct diagnostics.
- [GitHub manual workflow execution](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)
  requires `workflow_dispatch` and selects a branch. This Rust workflow has only
  push/pull-request triggers; local uncommitted source cannot be dispatched as
  hosted evidence. Existing authorized SSH access allows full macOS host checks
  without a commit or push.

No finite scheduling margin proves progress on a suspended or starved OS.
The two-second gap between the maximum normal wait and the emergency fallback
is an explicit ordering constraint, with host execution evidence and fail-closed
status checks; it is not a hosted-runner performance claim.

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


## Integration readiness source inspection: 2026-09-09

Historical planning inspection (before the executable selection below): these
were clean local repository candidates, not claims
that released crate archives match those checkouts. Batter has not compiled or
executed these integrations. Selection and executable compatibility remain open
in Beads; recheck the eventual source and generated Cargo graph.

- Runledger candidate
  [`0f464b4f`](https://github.com/bpcakes/runledger/tree/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4)
  has core/postgres/runtime manifests at 0.12.0, SQLx 0.9 and Rust 1.94.
  Its [supervisor](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/supervisor.rs)
  builder spawns loops without a ready receiver; Drop requests shutdown and
  detaches. Join returns the first failure; timeout shutdown allows additional
  abort cleanup of up to the smaller of its timeout and one second. The
  [worker](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker.rs)
  may finish an in-flight claim after stop is requested. These limits require
  a scoped application startup witness and an honest outer shutdown budget.
- Runledger's
  [enqueue implementation](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/enqueue.rs)
  exposes transaction-taking enqueue with outcomes; keyed submission requires
  READ COMMITTED and compares canonical initial fields. Pool-owned enqueue
  wrappers commit their own transaction. The
  [migration implementation](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/migrations.rs)
  supplies post-idempotency-cutover migration and compatibility entrypoints,
  including shared SQLx-history handling. Application migration composition must
  be tested on repeated startup as well as an empty database.
- postgres-test-harness candidate
  [`3d525e6f`](https://github.com/bpcakes/postgres-test-harness/tree/3d525e6fc5745ce2e2437c7997de5cccdecff4ac)
  has manifest version 0.2.0. Its
  [lease and harness implementation](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs)
  consumes a lease for awaited cleanup or explicit defer; Drop queues fallback
  cleanup. Deferral requires closed application connections, and external-mode
  shutdown does nothing, including no deferred drain. Its
  [admin checks](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs)
  require major version 18 and UUIDv7 support. External mode requires suitable
  CREATE/DROP DATABASE authority and the documented local, non-TLS endpoint.
- The resolved local SQLx 0.9.0 `sqlx-core/src/pool/mod.rs` defines
  `Pool::close` as a future returning `()`. Dropping that future does not prove
  close completed. Native close timeout is not a returned SQLx close error.
  [PoolConnection::detach](https://docs.rs/sqlx-core/0.9.0/sqlx_core/pool/struct.PoolConnection.html#method.detach)
  removes a connection from pool accounting; pool closure cannot establish
  termination of detached server sessions.
- Runlimit candidate
  [`b3aaee2b`](https://github.com/bpcakes/runlimit/tree/b3aaee2b3b659a47648336bc633896075cdf5f29)
  has workspace/core/axum version 0.3.0 and postgres package version 0.3.1,
  using SQLx 0.9.0 and Rust 1.94. It is a source-inspection candidate only;
  policy/backend selection remains part of the future admission integration.

## Executable reference selection: 2026-09-09

`batter-4t6` subsequently fetched the inspected revisions through Cargo and
compiled public native type probes. The [compatibility manifest](reference-compatibility.md)
and [validation](validation.md) supersede the preceding planning-only status for
the selected graph and tested paths. Local checkouts remained unchanged.

- [Runledger manifest at the selected revision](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/Cargo.toml)
  requires SQLx 0.9.0 and Rust 1.94. In contrast, `cargo info
  runledger-postgres@0.12.0` and the downloaded crates.io manifest establish that
  the [published 0.12.0 archive](https://docs.rs/crate/runledger-postgres/0.12.0/source/Cargo.toml)
  requires SQLx 0.8.6 and Rust 1.88. Full Git pins are intentional.
- [Migration entrypoints and raw-migrator restrictions](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/migrations.rs)
  govern fresh/upgrade startup, filtered shared history, cutover validation and
  lock/session disposition. The upgrade fixture only reads the bundled migration
  metadata/SQL to initialize the older schema using a disposable connection;
  the actual upgrade calls the supported entrypoint.
- [Canonical transactional enqueue](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/enqueue.rs)
  checks READ COMMITTED and stored initial request fields. The live probe reads
  back the snapshot and asserts the exact conflict/isolation codes.
- [Worker loop ownership](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker.rs)
  checks shutdown before claiming, then dispatches an already returned claim;
  cooperative drain awaits job and terminal-observer tasks.
  [Task-group shutdown](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/task_group.rs)
  retains the first error and logs later failures; abort drain adds at most
  `min(timeout, 1 second)` under cooperative scheduling.
- [Harness lease/cleanup contract](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs)
  and [connection-budget configuration](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/config.rs)
  distinguish awaited cleanup, queue acceptance, Drop fallback, per-lease permits
  and the external shutdown no-op. Live cases verify successful cleanup/defer/Drop
  ownership; cancelled-polled-waiter failure delivery remains source-inspected.
- The resolved SQLx 0.9.0 `sqlx-core/src/migrate/migrate.rs` API takes the history
  table name in `ensure_migrations_table` and `apply`; the initial fixture compile
  caught the older one-argument assumption. The disposable upgrade setup uses
  `_sqlx_migrations` explicitly.

## Concurrent local test runners: 2026-09-09

The installed Python is 3.14.7. Its local `subprocess.Popen` signature and the
[Python 3.14 subprocess contract](https://docs.python.org/3.14/library/subprocess.html#subprocess.Popen)
were rechecked for argument-array execution, session creation, signal restoration,
pipe observation and return codes. `poll()` can reap an exited child; retain the
leader until output EOF or the process-group termination decision, and never
signal a reaped numeric identity. Process creation remains outside hard timeout
preemption. The new parallel runner reuses the existing bounded capture/settlement
helpers instead of adding another pipe reader.

The [Python 3.14 signal API](https://docs.python.org/3.14/library/signal.html#signal.signal)
requires main-thread handler installation. Non-raising scoped SIGINT/SIGTERM
handlers request cancellation while cleanup keeps its original allowance; inherited
ignored signals are preserved. Separate shard processes isolate the controls'
signal changes and mocks. The [unittest result API](https://docs.python.org/3.14/library/unittest.html#unittest.TestResult)
supplies `startTest` and success/skip results. Each shard records actually started
test IDs and the parent compares them with its complete assigned discovery, so a
zero exit alone cannot supply coverage evidence. No fixture deadline was changed.

## Owned startup native semantics (2026-09-09)

The locked Tokio version remains 1.53.1. Its [one-shot receiver contract](https://docs.rs/tokio/1.53.1/tokio/sync/oneshot/struct.Receiver.html#cancel-safety)
confirms borrowed receiver waits are cancellation-safe. The running handoff uses
that receiver while observers carry no service ownership. The [Rust unwind API](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)
catches unwinding panics, leaves the hook in place, and cannot catch aborting
panics. Initializer construction, polling and destruction have separate catch
boundaries so an application failure is retained alongside a destructor panic.

Tokio 1.53.1's [Unix signal source](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/tokio/src/signal/unix.rs)
was inspected in the downloaded Cargo registry because the web page could not
be fetched. `signal` installs listeners immediately; Tokio's process-wide handler
is not restored when listeners are dropped. The helper registers consumption as
a critical component, so signals are consumed once that driver starts. Linux
process smoke evidence is recorded in validation; no new macOS run is claimed.

## Owned dependency health semantics (2026-09-09)

Locked Tokio remains 1.53.1. Its [select contract](https://docs.rs/tokio/1.53.1/tokio/macro.select.html)
explains same-task concurrent polling and biased branch order. Health puts drain
and cancellation before deadline and work, keeps factory invocation inside the
work future, and destroys it before publishing. Its [timeout contract](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html)
notes that inner work is polled first and non-yielding work can overrun. The health
monitor instead selects explicitly and rechecks elapsed time on completion, so
late success is unready without claiming preemption. Timed-out results do not
preserve a concurrently returned late application value.

The exact downloaded Tokio 1.53.1 `src/time/sleep.rs` was inspected after the
versioned `sleep_until` web page could not be fetched. Sleep uses an absolute
instant and has no work while pending; drop cancels it. Completion-to-next-probe
sleep creates no interval catch-up behavior. Native timer granularity and scheduler
stalls remain relevant despite policy validation.

The [standard mutex contract](https://doc.rust-lang.org/std/sync/struct.Mutex.html)
provides exclusive publication of outcome, timestamp and writer liveness. Locks
are short and private; application error destruction occurs after unlocking.
This is synchronous observation, not a lock-free or wait-free guarantee. Tests
exercise readers while a replaced error's destructor runs, and paused-clock
checks keep the writer unscheduled while its cached success expires.

## PostgreSQL fixture readiness audit (2026-09-09)

For `batter-4jz`, inspected the Cargo-selected postgres-test-harness checkout and
verified its Git HEAD is `3d525e6fc5745ce2e2437c7997de5cccdecff4ac`. The versioned
web source could not be fetched; these observations come from that exact local
upstream source, not a newer release. No live test was executed for this audit.

- [Fingerprint inputs](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/fingerprint.rs):
  `FingerprintBuilder` frames labels and content in insertion order;
  `TemplateSpec` accepts the resulting identity. It does not check that a
  caller's initializer matches its declared inputs.
- [Template and cleanup ownership](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs):
  cloned harnesses single-flight template initialization with a weak cache;
  retaining the template handle retains its shared-lock session. Deferred drain
  covers cleanup accepted before the barrier begins. External shutdown is a
  no-op; lease Drop queues destructive cleanup.
- [Connection limits](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/config.rs):
  per-database permits model downstream pools and standalone connections.
  Owner, template-coordination and lifecycle-administration sessions are
  additional usage. Independent harness instances do not establish a combined
  server-wide connection bound.

The implementation handoff and executable acceptance remain in the owning Bead.
These inspected contracts do not establish reusable fixture implementation,
cancel-safe teardown, or new platform/database execution evidence.


## Fixture ownership review corrections (2026-09-09)

Research before this correction inspected the same Cargo-selected harness Git
revision `3d525e6fc5745ce2e2437c7997de5cccdecff4ac`, SQLx 0.9.0 and Tokio 1.53.1.
No upstream version or provisioning implementation was changed.

- Upstream [`cleanup.rs`](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/cleanup.rs)
  starts independent cleanup workers. Omitting drain loses completion/error
  observation; it does not establish that accepted cleanup never executes.
- The pinned [`harness.rs`](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs)
  retains cached template databases beyond handle destruction and external-mode
  shutdown. Stable content/revision identities therefore replace random revisions.
  The upstream tests supply the catalog-lock technique used to make actual
  consuming lease cleanup fail; recovery uses public owner-aware stale cleanup.
- Native [SQLx pool close](https://docs.rs/sqlx/0.9.0/sqlx/struct.Pool.html#method.close)
  waits for tracked checkouts and returns unit. A retained clone can observe
  closure; detached connections are outside that completion evidence. Pools are
  registered before the runner polls acquisition/`after_connect`.
- [Tokio JoinHandle](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinHandle.html)
  detaches work on handle Drop, and borrowing its wait supports cancellation
  without discarding the task result. The fixture driver retains resources outside
  the body task and returns native join failures separately. Runtime death remains
  outside its guarantee; no panic hook is installed.

The public error report retains all branches explicitly. `Error::source` provides
one cause, so callers inspect the typed report fields for simultaneous failures.
Suite-level catalog/initializer pools have their own one-connection bounds;
per-lease declarations and independent harnesses do not impose a server-wide cap.


## Fixture acquisition ownership loop, round 1 (2026-09-09)

Research preceded this correction and used the same pinned harness revision,
SQLx 0.9.0 and Tokio 1.53.1. The versioned harness web source was available this
round and matched the Cargo checkout at
`3d525e6fc5745ce2e2437c7997de5cccdecff4ac`.

The [Tokio spawn_blocking contract](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html)
states that started blocking work cannot be aborted. Pinned harness
[`create_test_database` and template preparation](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs)
use the `run_blocking` bridge. The same source documents that abandoned template
preparation can leave initializing databases outside deferred drain. Returned
initializer errors instead await `initialization.abort()` and retain independent
initializer/abort errors. The first new panic regression exposed that distinction;
an owned initializer task now returns its join failure through that abort path.

The [harness server implementation](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/server.rs)
and public harness source establish shared clone admission, untimed permit waits,
external shutdown as a no-op, and owned-container shutdown as a shared server
operation. The adapter therefore joins its own producers but leaves server
shutdown to the caller. Other owners must release native admission capacity;
observation cancellation cannot be described as released ownership.

[PostgreSQL 18 LOCK](https://www.postgresql.org/docs/18/sql-lock.html) permits
ACCESS EXCLUSIVE with MAINTAIN, UPDATE, DELETE or TRUNCATE; SELECT alone is
insufficient. The [privilege inquiry functions](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-INFO-ACCESS-TABLE)
accept a comma-separated list and return true if any listed privilege is held.
Reference preflight checks this actual catalog privilege before any fixtures.

The downloaded SQLx 0.9.0 `pool/options.rs` was inspected with its
[native PoolOptions contract](https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolOptions.html).
A lazy pool can be retained before explicit acquisition; minimum-connection work
may run in a native background task. Both manual and owned fixture paths retain
the pool first, check one connection and leave minimum maintenance to SQLx. Pool
close remains an awaited tracked-connection boundary, not detached-session proof.


## Fixture report observation loop, round 2 (2026-09-09)

Research preceded the second correction. The [Rust must-use reference](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute)
describes expression-level diagnostics, not mandatory semantic inspection.
Executed compile-fail controls confirmed a bare reference does not inherit its
report's warning. A must-use borrowed view now preserves the warning after a
successful resumable wait; explicit discard remains possible.

The [Tokio 1.53.1 JoinHandle contract](https://docs.rs/tokio/1.53.1/tokio/task/struct.JoinHandle.html)
detaches tasks on handle Drop. Inspection of the current producer registry found
no public abort handle or internal abort path: body cancellation abandons delivery
only, while the driver joins the registered producer. Hypothetical future producer
abortion and runtime destruction are outside the completion contract. No
abort-on-drop wrapper was added; abort would not prove native creation stopped.

The same pinned [harness template implementation](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs)
awaits abort only on returned initializer errors. The manual adapter path delegates
this behavior directly; its panic/cancellation limitation is now explicit. The
owned path converts initializer panic into returned error, but still requires
initializer-owned pools to be closed and operations joined. Abort with live
initializer connections remains unverified. Shared-server shutdown remains
caller-owned; removing an external-server no-op did not remove resource cleanup.

The watchdog question was checked against executed serial case durations
(5.54 and 5.61 seconds): 180 seconds supplies over thirtyfold measured margin.
Inventory compilation has a separate 300-second bound. No evidence justified
raising the live watchdog; stalled work must still fail. Warm-cache initialization
counts deliberately permit zero; fresh-cluster evidence establishes cold startup,
and repeated same-input suites independently prove persisted reuse.
