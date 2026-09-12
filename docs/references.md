# Primary references and verification boundaries

Initially reviewed on 2026-09-07; dated follow-up checks appear below.
URLs pointing at `latest` may change;
verify the resolved Cargo.lock and pinned documentation when implementing or
upgrading adapters. These sources explain ecosystem semantics. They do not
validate Batter's source or prove any of its tests pass.

## Pushed native source, 2026-09-12

Verified the clean local Runledger checkout and remote HEAD with `git status`,
`git log -1` and `git ls-remote origin HEAD`. Both identify
[`d57ec6be61e9f00ccce373b19ca356cafe98f206`](https://github.com/bpcakes/runledger/commit/d57ec6be61e9f00ccce373b19ca356cafe98f206).
Cargo fetched that Git source for core/postgres/runtime 0.12.0. The root and
archived consumer manifests use the same revision without path patches; Cargo
regenerated their lock entries. The native initialization/settlement and
transaction-error contracts described below now have an immutable source identity.
This source check does not itself establish runtime acceptance; executed checks
and remaining limits are recorded under `batter-vly` in [validation](validation.md).

## HTTP fixture review follow-up: 2026-09-10

The [Cargo dependency inheritance contract](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#inheriting-a-dependency-from-a-workspace)
allows a package's dev-dependency to use the workspace version requirement.
The test-only `http-body` edge now follows that existing repository convention;
resolved versions are unchanged. Against baseline `495e46f`, the lockfile gains
a test-only `http-body` dependency edge for `batter-axum`; moving that existing
edge to workspace inheritance adds no further lockfile change. This is manifest ownership,
not a transport-version change or prerequisite for the planned facade extraction.

[Tokio 1.53.1 timeout](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html)
cancels its inner future on elapsed time but cannot bound a non-yielding poll.
The HTTP fixture therefore uses inner diagnostic timeouts plus its existing
independent process watchdog. Component comparisons use a paused-time bound only
for yielding deadlocks. No general preemption guarantee follows.

Ordinary graceful shutdown still permits either a 503 response or transport
closure before routing. That case records exactly one accepted outcome; the
separately synchronized admission case is what proves a routed request receives
503 without business-handler entry. Requiring both outcomes from a race would be
an unstable test, not stronger evidence. Write-half closure remains explicitly
outside the full-disconnect contract in ADR-008; no new transport claim is made.

## HTTP/1.1 transport ownership, reviewed 2026-09-10

The follow-up harness review checked the [Cargo environment reference](https://doc.rust-lang.org/cargo/reference/environment-variables.html)
and [Rust module reference](https://doc.rust-lang.org/reference/items/modules.html).
`CARGO_MANIFEST_DIR` identifies the consuming package, not the source file's
original package. Source inclusion also compiles nested test modules in that
consumer. Therefore generic process mechanics now live in workspace-private
`test-support/process/`, with fixture assets, scenario policy and self-tests
attached only by their owning foundation suite. This removes both the Linux
asset-path failure and HTTP's dependency on the planned foundation test relocation.

For `batter-u0m`, rechecked Cargo.lock and the downloaded primary Cargo registry
sources: Axum **0.8.9**, Hyper **1.11.1**, hyper-util **0.1.20**, Tokio **1.53.1**,
and http-body **1.1.0**. The test adds a direct development dependency on the
already resolved http-body and enables Tokio io-util; no versions changed.
The browser could not retrieve the pinned docs.rs pages; source inspection used
the exact locally resolved registry packages, not a different online version.

- [Axum serving source](https://docs.rs/axum/0.8.9/src/axum/serve/mod.rs.html):
  `WithGracefulShutdown::run` spawns the graceful-signal future, spawns each
  connection in `handle_connection`, stops accepting on the signal and waits on
  `close_tx.closed()`. Each connection owns a receiver until its future ends.
  The serving wrapper does not retain connection JoinHandles. Aborting it drops
  its wait; it does not join connections. `IntoFuture` returns Ok after run;
  connection errors are observed separately inside the spawned connection task.
- [Hyper HTTP/1 builder](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html):
  the inspected builder defaults keep-alive to true and half-close support to
  false. `half_close(true)` would permit continued response work after a read
  EOF during a request. The fixture uses Axum's default transport and explicitly
  closes both client socket directions; it makes no write-half-only claim.
- [http-body contract](https://docs.rs/http-body/1.1.0/http_body/trait.Body.html):
  body frame polling is separate from constructing a Response. The controlled
  test body acknowledges a pending poll, releases data explicitly, records None
  and records destruction. Native client framing and EOF are checked separately.

These implementation facts motivate the tests; only the executed loopback
evidence establishes the observed behavior in [ADR-008](adr/008-http-transport-ownership.md).

## HTTP/1.1 lifetimes, reviewed 2026-09-10

For `batter-u0m`, rechecked the actual Cargo.lock and locally cached upstream
registry source: **Axum 0.8.9**, **Hyper 1.11.1**, **hyper-util 0.1.20** and
**Tokio 1.53.1**. Versioned docs.rs retrieval failed in this session; these
semantics were verified directly in the exact resolved upstream source rather
than inferred from another version's documentation.

- [Axum serving source](https://docs.rs/axum/0.8.9/src/axum/serve/mod.rs.html):
  `WithGracefulShutdown::run` spawns the signal task, stops accepting after the
  signal, drops its listener, and awaits `close_tx.closed()`. `handle_connection`
  spawns native connection tasks, passes graceful shutdown to their futures and
  drops a completion receiver after they finish. Connection errors do not become
  the outer serving result. Aborting the wrapper is not a join of these tasks.
- [Hyper HTTP/1 builder](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html#method.half_close)
  and resolved `src/server/conn/http1.rs`: the native default is `half_close = false`.
  Supporting a client that closes its write side while awaiting a response is a
  distinct configuration. `src/proto/h1/dispatch.rs` polls reads/keep-alive while
  managing pending response futures/bodies; actual disconnect timing is tested,
  not inferred as a universal property from the builder option.
- [hyper-util connection shutdown](https://docs.rs/hyper-util/0.1.20/hyper_util/server/conn/auto/struct.UpgradeableConnection.html#method.graceful_shutdown)
  and resolved `src/server/conn/auto/mod.rs`: graceful shutdown delegates to the
  selected native protocol connection. The connection future must continue to
  be polled. Axum's connection task, rather than Batter's request context, owns it.
- [Tokio TCP stream](https://docs.rs/tokio/1.53.1/tokio/net/struct.TcpStream.html)
  and resolved `src/net/tcp/stream.rs`: `AsyncWrite::poll_shutdown` shuts down the
  write direction. The fixture separately uses native `Shutdown::Both` and drops
  its client socket for a full local close, retaining reads for the half-close
  control. Local close is not proof of peer receipt or body-message completion.

The fixture reports terminal chunk/Content-Length framing, body destruction,
socket destruction/EOF and direct wrapper outcome separately. See
[ADR-009](adr/009-http-lifetime-observations.md) for the measured ownership decisions.
No dependency or Cargo.lock change was needed.

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
is not restored when listeners are dropped. `install_signals` retains both
sources for cancellation-safe polling during initialization, then transfers them
to the critical component. A Linux real-child unit control sends SIGTERM before
that transfer; process smoke evidence is recorded in validation. No new macOS run
is claimed.

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

## Axum operational defaults: exact native APIs (2026-09-10)

Before implementation of batter-7r3.4, inspected the downloaded primary crate
sources for **Axum 0.8.9** (`src/serve/mod.rs`, `src/routing/mod.rs`) and
**tower-http 0.6.11** (`src/request_id.rs`, `Cargo.toml`). Cargo subsequently
resolved tower-http 0.6.11 with only its request-id feature, UUID 1.26.0 and Axum
0.8.9 in the generated lockfile; it reported tower-http 0.7.1 as available.
This task selects the audited 0.6 API, not a general dependency refresh. No
foundation dependency on HTTP libraries was introduced. `http-body` 1.0.1 is an
adapter test dependency for a controlled native streaming body.

The [Tower HTTP request-ID source](https://docs.rs/crate/tower-http/0.6.11/source/src/request_id.rs)
shows SetRequestId retaining inbound headers and potentially an existing RequestId
extension. MakeRequestUuid calls native UUID v4 generation. The opt-in boundary
therefore uses that generator directly and replaces header/Tower/adapter values;
it does not deploy the preserving setter at an untrusted boundary.

The [Axum serve source](https://docs.rs/crate/axum/0.8.9/source/src/serve/mod.rs)
spawns native connection tasks and the graceful-signal listener. Graceful return
waits on native connection completion; aborting the direct serve future does not
join those tasks. Native accept errors are retried. This grounds the deliberately
limited register_http helper and the executed streaming-abort regression.

The [Axum routing source](https://docs.rs/crate/axum/0.8.9/source/src/routing/mod.rs)
applies layers to already assembled route/fallback services. That determines
operational_http placement, retained MatchedPath templates and 405/fallback
coverage. Versioned docs.rs web requests failed in this environment; exact local
Cargo source inspection, compilation and runtime tests supplied API evidence.

## Filtered operation context: 2026-09-10

Checked tracing 0.1.44 [Span::or_current](https://docs.rs/tracing/0.1.44/tracing/struct.Span.html#method.or_current)
and the selected local source. A disabled span does not preserve its parent as
an explicit event parent. `or_current` selects the enabled span or current parent.
Batter therefore retains an operation's execution context separately from its
diagnostic span, as its HTTP and task boundaries already do. Selection happens
once on first poll; recording fields still targets only the owned span.
The existing dispatch wrapper protects full future destruction.

## Independently filtered request context: 2026-09-10

Checked [EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
and actual selected tracing-subscriber 0.3.23 `filter/directive.rs` and
`filter/env/mod.rs`. Static target directives prefer the most specific matching
target. Span-name directives maintain dynamic scope matching and can enable
events inside that span, including events whose target alone would be filtered.
The adapter uses static target `batter::request` for its INFO context span and
keeps completion events on `batter`. The supported quiet example filter is
`info,batter=warn,batter::request=info`; it retains request context without
re-enabling INFO operation completions. The operation fallback described above
is also necessary: parenting WARN events to disabled operation spans otherwise
discards the retained HTTP parent.

## Native HTTP composition and policy evolution: 2026-09-10

Axum 0.8.9 [ConnectInfo](https://docs.rs/axum/0.8.9/axum/extract/struct.ConnectInfo.html)
and [ServiceExt](https://docs.rs/axum/0.8.9/axum/trait.ServiceExt.html) require the
make-service conversion to supply connection metadata. A plain Router does not
install that extension. `register_http` intentionally retains its narrow Router
contract; its rustdoc demonstrates an application-owned supervised native
`into_make_service_with_connect_info` serve closure.

Cargo's [SemVer guidance](https://doc.rust-lang.org/cargo/reference/semver.html)
classifies adding enum variants and adding `non_exhaustive` to an existing
exhaustive enum as breaking changes. ReadinessReason remains exhaustive by
design: additional states warrant consumer policy review, rather than a new
wildcard fallback that can conceal a readiness/severity decision.

## Fixture failure retention and session observation: 2026-09-10

Rechecked the resolved SQLx 0.9.0 source in `sqlx-core/src/pool/mod.rs` and
`pool/connection.rs`. [Pool::close](https://docs.rs/sqlx/0.9.0/sqlx/struct.Pool.html#method.close)
returns unit and waits for tracked checked-out connections; native
[PoolConnection::detach](https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolConnection.html#method.detach)
removes that accounting. Neither acknowledges the detached backend's exit.
The live negative control uses native detach alongside Batter's actual PgLease
retirement, then observes the blocked backend IDs outside the closing one-slot pool.

The PostgreSQL 18 [statistics documentation](https://www.postgresql.org/docs/18/monitoring-stats.html)
describes pg_stat_activity session rows, visibility of session existence/database
across roles, and transaction-local snapshot caching. The fixture observer uses
fresh auto-commit catalog queries, tests database presence and that its own database
is distinct, and checks absence of all rows for the disposable database. It does
not infer quiescence from elapsed time, pool size or cached PID disappearance.
The caller must select the same server and stop new connection producers; a read
is not a connection-admission fence. Backend IDs are test witnesses, not durable
identities or a mechanism to terminate sessions.

Rechecked postgres-test-harness revision
[`3d525e6fc5745ce2e2437c7997de5cccdecff4ac`](https://github.com/bpcakes/postgres-test-harness/tree/3d525e6fc5745ce2e2437c7997de5cccdecff4ac),
`src/harness.rs` DatabaseLease::cleanup/defer_cleanup/Drop and
`src/cleanup.rs` awaited/deferred failure delivery. Consuming cleanup delivers its
own error; explicit defer hands a separate resource to the queue, whose drain
returns DeferredCleanup with native per-database causes. External shutdown is a
no-op and cannot replace that barrier. Native lease Drop submits fallback deletion;
destroying a fixture driver is therefore not conservative resource retention.
The new real fault test uses separate resources for both error paths and recovers
tagged residuals only after releasing the acknowledged catalog lock.

The unpublished opt-in fixture API changes PoolAcquire's payload from owned
sqlx::Error to Arc<sqlx::Error>, and adds per-database failure collections. This
source compatibility adjustment is explicit: all in-workspace consumers are
updated together, while external source adopters must update field construction
and payload matching. It preserves native identity after the application handles
an error; cloning a formatted message would not. No dependency version changed;
Cargo regenerated the lockfile for the reference test's direct batter dependency.

## Fixture retry and completion review follow-up: 2026-09-10

Rechecked [Tokio 1.53.1 watch Receiver](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Receiver.html#method.changed)
and [Sender::send_replace](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Sender.html#method.send_replace).
`borrow_and_update` marks the current value seen; `changed` immediately consumes
an unseen update. This establishes the intended broadcast/coalescing semantics:
a request received during an active attempt permits another attempt after failure,
without cancelling that active attempt. The first attempt reads the latest pool.
A live two-database control verifies this behavior and retained failure identities.

Inspected the Cargo-resolved postgres-test-harness revision
`3d525e6fc5745ce2e2437c7997de5cccdecff4ac`, `src/admin.rs:860` and
`src/harness.rs:771,815`. The [pinned admin source](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs#L860)
uses `DROP DATABASE IF EXISTS ... WITH (FORCE)`; consuming cleanup awaits queue
completion and lease Drop submits fallback cleanup. The raw admin web fetch failed;
the exact local Cargo checkout supplied this evidence. PostgreSQL 18
[DROP DATABASE](https://www.postgresql.org/docs/18/sql-dropdatabase.html)
documents FORCE attempting connection termination. A failed observer therefore
must retain the lease; returning a pending error retains the run/control rather
than aborting cleanup. No second protective non-FORCE drop is assumed.

The shared observer timeout bounds acquisition and polling together. It means
absence was not established, and does not identify remaining sessions as its
cause. Diagnostic pool capacity is separate from observation capacity in the
normal reference helpers and detached-session proof. Deliberate capacity starvation
is isolated to the explicitly named active-attempt retry fault test.


## Fixture terminal completion and review questions: 2026-09-10

Rechecked the selected SQLx **0.9.0** local Cargo source
`sqlx-core/src/pool/mod.rs:420-442` and `pool/inner.rs`. The versioned
[Pool::close contract](https://docs.rs/sqlx/0.9.0/sqlx/struct.Pool.html#method.close)
requires awaiting tracked checkout closure; dropping the last pool handle does
not supply that completion witness. The web fetch was unavailable, so the exact
Cargo source supplied the evidence. Multiple close calls can resume waiting.
This supports retaining the cached driver outcome and pools while a bounded
administrative close remains pending. The implementation changes only the private
reference completion policy; native leases and adapter ownership remain intact.

Answers to the independent review's open questions:

- **Retention versus failed test/runtime exit:** the lease-retention contract is
  conditional on the cleanup runtime remaining alive. Returning pending preserves
  it; the later assertion panic and runtime destruction cross that boundary.
  [Tokio 1.53.1 Runtime shutdown](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Runtime.html#shutdown)
  drops yielding tasks rather than guaranteeing completion. The pinned harness
  `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` `harness.rs:815` queues fallback
  cleanup on lease Drop; `admin.rs:860` uses FORCE. PostgreSQL 18
  [DROP DATABASE](https://www.postgresql.org/docs/18/sql-dropdatabase.html)
  documents its connection termination behavior. A non-destructive runtime-death
  guarantee would require an upstream lease/provisioning policy change, outside
  this completion helper's contract. Existing live controls cover both boundaries.
- **Missing database:** remain unresolved. An absent name can mean the wrong
  server or external deletion and cannot establish the required witness. Do not
  recreate the name, manufacture successful cleanup, or drop the retained lease
  to finish a report. A correct replacement observer can recover a wrong-target
  error; external deletion has no automatic recovery protocol here.
- **Shared retry control:** intentionally broadcasts across runs, not just a
  single suite. [Tokio 1.53.1 Sender](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Sender.html#method.send_replace)
  and [Receiver](https://docs.rs/tokio/1.53.1/tokio/sync/watch/struct.Receiver.html#method.borrow_and_update)
  confirm latest-value broadcast, coalescing and seen-version behavior. Use a
  fresh control for independently recovered runs; sharing requires coordinated
  observer-pool lifetime. The existing active-attempt test covers two databases
  in one run. Cross-run broadcast and replacement before a first attempt lack
  dedicated execution coverage; no new claim of such coverage is made.

The public retry example now calls out that its final consuming wait is unbounded.
Repeated `wait_for` preserves bounded, resumable observation when a retry fails.
The original defect was a private completion-ownership split: two caller-level
question-mark returns bypassed diagnostic closure on a terminal driver error.
Closing in the owning helper avoids duplicating error-path cleanup in consumers.


The next review's shared-capacity question was confirmed against the same pinned
[harness source](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/harness.rs),
`DatabaseLeaseInner::permit`, and local `server.rs::acquire_database_permit` /
`admission.rs::acquire`. A retained lease owns admission permits; parked runs can
exhaust shared capacity until explicitly recovered. No automatic permit release
is appropriate while the database remains retained. This liveness limit is now
explicit in SessionObserver rustdoc, the adapter README and integration contract.

Git index/untracked state does not alter the review or build scope: both include
all current source inputs, and this work authorizes no commit. Any later commit
must include new module sources together with their declarations; `commit -a`
is not an instructed delivery step. Tracker closure follows the current review
and gates rather than the staged historical metadata.


Final review question, PostgreSQL background workers: the PostgreSQL 18
[activity view](https://www.postgresql.org/docs/18/monitoring-stats.html#MONITORING-PG-STAT-ACTIVITY)
includes autovacuum, parallel and logical-replication workers, plus extension
backend types. PostgreSQL 18's
[CountOtherDBBackends implementation](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/storage/ipc/procarray.c)
signals conflicting autovacuum workers during DROP interlocking. This does not
supply a prior absence witness. Keep the existing all-database-backend check:
a client-only filter would omit application-related background work too. This is
an intentionally conservative policy; maintenance activity can cause a pending
attempt requiring explicit retry. No autovacuum timing guarantee is claimed.
The reference 30-second whole-run limit remains the documented consumer policy;
only the recorded macOS execution establishes its timing evidence, not slower
or hosted runners. Current `br show`/the Beads database is authoritative for
status; an older Git index is a historical staged snapshot, not delivery state.


## Fixture observation coverage follow-up: 2026-09-10

Rechecked SQLx 0.9.0, Tokio 1.53.1 and harness revision
`3d525e6fc5745ce2e2437c7997de5cccdecff4ac` against Cargo.lock. New probes retain
those native APIs. The earlier cross-run/pre-first-attempt coverage limitations
above describe their historical snapshot; dedicated tests now cover both.

PostgreSQL 18 [statistics visibility](https://www.postgresql.org/docs/18/monitoring-stats.html#MONITORING-STATS-VIEWS)
exposes session existence and database identity to ordinary users. The pinned
[18.4 pgstat implementation](https://github.com/postgres/postgres/blob/REL_18_4/src/backend/utils/adt/pgstatfuncs.c)
assigns database identity before restricted activity fields. The test now connects
as an unprivileged, non-inheriting LOGIN role, verifies absence of superuser
and pg_read_all_stats privilege, and witnesses a different user's detached
session. Observation times out and retains the database until that session closes.
Role identifiers consist only of a fixed prefix and generated decimal digits;
role creation/removal is confined to the dedicated disposable primary server.

PostgreSQL 18.4 [backend initialization](https://github.com/postgres/postgres/blob/REL_18_4/src/backend/utils/init/postinit.c)
publishes initial backend status before authentication and database assignment.
The SCRAM protocol test pauses at AuthenticationSASL, finds the new backend with
NULL datname and observes cleanup advancing to CleaningLease. The first test
incorrectly expected full completion while authentication was paused; actual
execution showed DROP waiting on ProcSignalBarrier. The pinned
[DROP implementation](https://github.com/postgres/postgres/blob/REL_18_4/src/backend/commands/dbcommands.c)
confirms its storage-manager process barrier. The corrected oracle requires
pending completion, then closes the startup connection, witnesses backend exit
and awaits the same successful report. This tests a limitation; it adds no
connection fence or arbitrary startup/shutdown guarantee.

The real autovacuum test uses documented PostgreSQL 18
[table storage parameters](https://www.postgresql.org/docs/18/sql-createtable.html#SQL-CREATETABLE-STORAGE-PARAMETERS)
and [vacuum settings](https://www.postgresql.org/docs/18/runtime-config-vacuum.html)
to retain a worker through a 200 ms observation attempt. It witnesses the actual
worker identity, disables subsequent ordinary autovacuum on its own table,
waits for all sessions to exit and retries explicitly. Production observation
continues to include all database backend types. It does not gain a maintenance
completion deadline.

The active-attempt oracle now uses pg_blocking_pids acknowledgement of both
observer connection-initialization queries instead of racing a published phase
against pool selection. Both current-thread and multi-thread Tokio tests preserve
the two ordered native causes and their Arc identities. Pre-first-attempt updates
are sent while a body gate prevents cleanup; only the latest pool can be selected.
Shared-pool tests cover both waiting for both drivers before close and recovering
a second run after the first completion closes their shared native pool.


The coverage review follow-up replaced SET ROLE with an actual SCRAM-authenticated
unprivileged LOGIN role. PostgreSQL's documented permission checks use current_user;
the stronger control also makes session_user unprivileged. A native Tokio child
contains assertion unwinding, while the outer owner awaits pool close and DROP
ROLE before interpreting its JoinError. The injected assertion regression checks
that the role is absent and the native panic identity remains observable. This
is not a runtime-death or general async-drop guarantee.

The PostgreSQL 18 [control-data function](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-PG-CONTROL)
returns a cluster's system_identifier. Live preflight now requires access on both
endpoints and rejects equal identities before fixture execution, including aliases.
SQLx 0.9 Pool::close is resumable and marks closure only when polled; concurrent
join_all polling starts every retained replacement close even if an earlier pool
has a held checkout. Its existing futures-util 0.3.34 dependency is now explicitly
selected by the reference package too; Cargo generated the lockfile change.
The new held-original/replacement regression tests that all closes start before
releasing the original checkout and resuming the same successful report.

The proposed server-wide wait for every NULL-datname client was considered and
not adopted: it couples independent fixture databases to unrelated authentication
attempts and still cannot fence a later connection. The API continues to require
caller-owned producer shutdown and a same-server observer. The startup control
records the narrower witness instead of adding a global admission policy.

The next review found a test scheduling assumption, a known permanent temporary-role
password, and stale ownership descriptions. The held-original close test now joins
the empty driver before timing administrative closure. Active retries now pass
through the actual recoverable completion owner on both Tokio runtimes. Secondary
URL rejection also has a full runner-entry control.

PostgreSQL 18 [CREATE ROLE](https://www.postgresql.org/docs/18/sql-createrole.html)
defines VALID UNTIL as password expiry, not role removal or session termination.
The restricted-login test uses a fresh
[random UUID](https://www.postgresql.org/docs/18/functions-uuid.html) password and
a five-minute deadline from the server clock. PostgreSQL
[format](https://www.postgresql.org/docs/18/functions-string.html#FUNCTIONS-STRING-FORMAT)
quotes its identifier and literals before executing role DDL. The live role probe
checks finite expiry; its outer owner still explicitly drops the role after joining
the assertion-bearing task. Process death may leave an expired role.

The secondary privilege question was checked against PostgreSQL 18.4
[control-data source](https://github.com/postgres/postgres/blob/REL_18_4/src/backend/utils/misc/pg_controldata.c)
and [built-in grants](https://github.com/postgres/postgres/blob/REL_18_4/src/backend/catalog/system_functions.sql),
then executed on the disposable secondary: BEGIN, CREATE ROLE fixture_control_probe
NOLOGIN NOSUPERUSER, SET LOCAL ROLE, SELECT system_identifier IS NOT NULL FROM
pg_catalog.pg_control_system(), ROLLBACK returned true. Public execution is available
by default on that version; an explicit EXECUTE grant is needed only if revoked.
The progress types retain the workspace's existing exhaustive API convention;
adding a phase remains an intentional compatibility decision, not a claim that
future variants are source-compatible.

Another review exposed ambiguous recovery guidance and report counters. SQLx
pool replacement does not close the old pool, so an inside-target observer must
be closed explicitly before a correct observer can witness session absence. The
reference completion owner now exposes a read-only observer_pools slice of its
native pools. The control exercises explicit close and a backend-exit witness
through the same pending ObservedRun without an external pool clone. It does not automatically close arbitrary replaced
pools because they may still serve active attempts or shared runs. Report Display
now separates consuming-cleanup failures, pool failures and observation attempts;
offline and live controls distinguish recovered errors from failed deletion.

PostgreSQL 18 [DROP DATABASE](https://www.postgresql.org/docs/18/sql-dropdatabase.html)
can fail with prepared transactions, active logical replication slots or subscriptions
despite a session-absence witness; native errors remain in the cleanup report.
The [system_user function](https://www.postgresql.org/docs/18/functions-info.html)
reports authentication for the current connection, not a future database selected
by potentially different pg_hba rules. The disposable server returned NULL for a
trusted in-container administrative connection and scram-sha-256:postgres for the
external test endpoint. The actual disposable-database SCRAM control remains the
authentication oracle. The same-server requirement also remains explicit: a name
lookup cannot distinguish an identical database name on a different cluster.
Normal native catalog session-identity visibility is part of that observer contract.

The autovacuum follow-up uses PostgreSQL 18
[VACUUM progress reporting](https://www.postgresql.org/docs/18/progress-reporting.html#VACUUM-PROGRESS-REPORTING):
the view identifies pid, database and relation OID for the vacuum currently running.
The test records its created table's OID, selects an actual autovacuum of that table
with more than half the heap still to scan, and rechecks the same table/worker after
the short observation timeout. It no longer mistakes a transient database visit
or another table's vacuum for its deliberately cost-limited workload. No arbitrary
server scheduling or maintenance-duration guarantee is added.

The startup follow-up now witnesses the target DROP query's actual
ProcSignalBarrier wait and rechecks its backend after the pending interval before
releasing the raw socket. A merely slow DROP cannot satisfy that oracle. The
dedicated serial server remains required for the before/after startup PID window;
forwarded socket ports need not retain their frontend identity. PostgreSQL 18
[vacuum configuration](https://www.postgresql.org/docs/18/runtime-config-vacuum.html)
requires track_counts as well as autovacuum. Preflight now rejects track_counts=off
before inventory. A real invocation with PGOPTIONS='-c track_counts=off' exited1
with the explicit prerequisite message and no fixture execution. The same local
Docker forwarding control measured frontend port 50977 and server-visible port
65048 (172.17.0.1); matching socket.local_addr would reject that supported path.
The raw connection was closed after the measurement.

The identity parser follow-up uses the documented signed bigint output of
[pg_control_system](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-PG-CONTROL-SYSTEM).
PostgreSQL18.4 [initialization source](https://raw.githubusercontent.com/postgres/postgres/REL_18_4/src/backend/access/transam/xlog.c)
places Unix seconds in the upper32 bits of the stored identifier;
[SQL output conversion](https://raw.githubusercontent.com/postgres/postgres/REL_18_4/src/backend/utils/misc/pg_controldata.c)
uses Int64GetDatum. A negative SQL identifier is therefore valid. Preflight now
compares parsed signed64-bit values, with bounded decimal input and range checks.
Python controls cover both extrema, negative identifiers, malformed/oversized
output, and failed prerequisite rows carrying an otherwise valid identifier.

The final coverage audit inspected SQLx0.9.0
[PoolOptions::connect_lazy](https://docs.rs/sqlx-core/0.9.0/src/sqlx_core/pool/options.rs.html)
and [PostgreSQL URL parsing](https://docs.rs/sqlx-postgres/0.9.0/src/sqlx_postgres/options/parse.rs.html).
The lazy error branch is URL/options parsing, before pool construction; it feeds
the same retained pool-failure owner as an acquire error. The harness supplies
its validated administrative URL with an encoded disposable database path, so a
malformed raw URL is not directly injectable through FixtureScope. Parser-specific
option disagreement remains a narrower unisolated failure path; no additional
coverage is claimed for it. No test-only production injection seam was added.

## Native connection graceful acknowledgement: 2026-09-10

For `batter-mhp`, rechecked the resolved Cargo source and the tagged
[Axum 0.8.9 serve implementation](https://github.com/tokio-rs/axum/blob/axum-v0.8.9/axum/src/serve/mod.rs#L386-L415).
The signal task first awaits the supplied future, then closes its watch receiver.
Each separately spawned connection selects that notification and emits
`signal received in task, starting graceful shutdown` immediately before calling
synchronous `conn.as_mut().graceful_shutdown()`, without an intervening await.
Graceful server completion separately awaits connection-completion receivers.

The two HTTP fixture runtimes are current-thread and contain one connection per
case. Their shared helper rejects other runtime flavors and requires the exact
native TRACE target/message. Therefore a test resuming after observing that event
runs after the synchronous call. It records `connection-graceful`, holds work for
a finite pending checkpoint, then releases it. The producer event is named
`graceful-signal-ready` to describe only its own boundary. The trace is a
version-specific compatibility observation, not a stable Axum callback contract;
a missing/filtered/changed event must fail the regression and prompt upstream
source review. No production tracing filter or serving API changes follow.

## Test dispatcher interest cache, 2026-09-10

Resolved tracing 0.1.44, tracing-core 0.1.36 and tracing-subscriber 0.3.23.
[Upstream issue #2874](https://github.com/tokio-rs/tracing/issues/2874) documents
first callsite registration on a thread without a default caching disabled
interest when only one dispatch is registered. The
[tagged callsite implementation](https://github.com/tokio-rs/tracing/blob/tracing-core-0.1.36/tracing-core/src/callsite.rs)
uses the thread default in `Rebuilder::JustOne`; the
[callsite documentation](https://docs.rs/tracing-core/0.1.36/tracing_core/callsite/index.html)
describes cached interest and dispatcher registration. The issue remained open at
inspection. Our deterministic isolated reproduction failed with a bare dispatch
and passed with an inert registration created before the real subscriber.

Private test-support construction uses that workaround. It neither installs a
global default nor changes production tracing behavior, and real subscribers no
longer need indefinite retention. Removal requires passing the isolated
`tracing_dispatch` regression with the actual replacement graph. This finding is
not evidence of a callback running under the foundation admission mutex.

## Fixture phase cancellation and subscriber retention, 2026-09-10

The resolved graph uses Tokio 1.53.1. Its
[timeout contract](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html) cancels by
dropping the owned inner future and cannot preempt non-yielding execution. The
[tagged JoinHandle contract](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/tokio/src/runtime/task/join.rs)
states that dropping a handle detaches its task and that an observed join follows
task destruction. Therefore the fixture puts timeout inside its joined exercise
task and preserves a separately owned running supervisor for teardown. A single
absolute teardown deadline covers report acquisition and resource reconciliation.

The remaining real-dispatch retention vector in filtered telemetry tests was
obsolete after the shared inert-dispatch constructor. Removing it preserves the
single-dispatch workaround while allowing capture storage to be released; a Weak
storage regression allows temporary borrows by concurrent interest-cache rebuilds.
Inspection of the actual
[tracing-core 0.1.36 callsite implementation](https://github.com/tokio-rs/tracing/blob/tracing-core-0.1.36/tracing-core/src/callsite.rs)
confirmed that DefaultCallsite enters the global list before rebuilding interest.
The review's memory-based hypothesis about the opposite registration order does
not describe this macro path. This inspection is not a proof that upstream tracing
has no other concurrency bugs; the existing isolated regression covers the
specific reproduced interest-cache failure.


## Dispatcher bootstrap and terminal report phases, 2026-09-10

Bead `batter-rv8` rechecked the actual tracing 0.1.44, tracing-core 0.1.36,
tracing-subscriber 0.3.23 and Tokio 1.53.1 graph before implementation.
[tracing-core's tagged callsite source](https://github.com/tokio-rs/tracing/blob/tracing-core-0.1.36/tracing-core/src/callsite.rs)
shows that registration rebuilds interest before updating the global maximum;
the single-dispatch path can compute interest without the dispatcher-list lock.
[NoSubscriber](https://github.com/tokio-rs/tracing/blob/tracing-core-0.1.36/tracing-core/src/subscriber.rs)
does not override the default absent maximum-level hint. An inert NoSubscriber
therefore raises the initial global maximum from OFF to TRACE. An unscoped macro
can begin registration then and store stale Never after a later rebuild.

A scheduling hook in a private dependency copy paused DefaultCallsite immediately
before that store: the old sentinel failed, while a registry with
[LevelFilter::OFF](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.LevelFilter.html)
passed. The latter keeps macros disabled until the real subscriber has registered
and rebuilt interest with two dispatchers. The checked-in bootstrap probe asserts
OFF during the real subscriber's registration callback; the earlier first-hit
regression remains. This is evidence for these macro paths with the shared test
constructor, not arbitrary external callsite registration or all upstream races.
[Issue 2874](https://github.com/tokio-rs/tracing/issues/2874) remains open.

[Tokio's tagged Timeout::poll](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/tokio/src/time/timeout.rs)
polls its inner future before checking elapsed time, contrary to the review's
unpolled-reconciliation hypothesis for this version. A near-exhausted teardown
can still time out pending reconciliation; the report remains separately owned.
Moving terminal report waits out of exercise removes the actual success-path
budget overlap. Delayed cleanup and failed event reconciliation are tested through
the real supervisor; the intentional blocked-body report checkpoint stays in exercise.


## Implicit dispatch construction and cancelled waits, 2026-09-10

Before implementing `batter-538`, the resolved tracing 0.1.44 `instrument.rs`
confirmed that [`WithSubscriber::with_subscriber`](https://docs.rs/tracing/0.1.44/tracing/instrument/trait.WithSubscriber.html#method.with_subscriber)
accepts `Into<Dispatch>` and executes `subscriber.into()`. Raw subscriber arguments
therefore bypassed the test bootstrap helper even without a visible `Dispatch::new`.
Already-constructed dispatcher arguments are unaffected.

The resolved Tokio 1.53.1 timeout implementation owns and drops its inner future;
its poll order is recorded in the preceding section. Retaining diagnostic evidence
in private pending-wait destructors works for exercise, disconnect and teardown
cancellation regardless of remaining phase time. These bounds still require
polling/yielding; the Unix process watchdog owns non-yielding containment.

Tokio's [`Notify`](https://github.com/tokio-rs/tokio/blob/tokio-1.53.1/tokio/src/sync/notify.rs)
retains one permit for `notify_one`; a check followed by a single wait was not a
proven lost wakeup here. Event history plus `notify_waiters` and creation of the
notification future before checking history supports multiple event waiters
without consuming another waiter's sole permit. Fixture history remains the
predicate; a notification alone never proves the required event occurred.

## Typed configuration audit: 2026-09-10

Planning evidence for `batter-5pm`, inspected against worktree baseline
`0e47f7dbd5d0c04d878d290c5d8c9181d1162b18`. This section records native
semantics used by the [implementation plan](../.agent/plans/batter-5pm.md);
it is not execution evidence for the planned settings API.

[dotenvy 0.15.7 source reading](https://docs.rs/dotenvy/0.15.7/src/dotenvy/lib.rs.html)
provides exact-path and reader iterators, separate from environment-mutating
loaders and ancestor-search helpers. Its
[substitution parser](https://docs.rs/dotenvy/0.15.7/src/dotenvy/parse.rs.html)
still reads the real process environment before previously parsed file values.
The selected registry `src/errors.rs` also retains the offending line in
`Error::LineParse` and includes it in Debug/Display. Choosing an iterator alone
therefore establishes neither source isolation nor diagnostic redaction. The
planned literal source dialect is an explicit Batter design choice, not a claim
that dotenvy disables interpolation.

SQLx's selected registry package is 0.9.0; its `.cargo_vcs_info.json` names
`003b698e99e024f3621b8043a2426fde5b741171`. The
[connection option source](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-postgres/src/options/mod.rs)
derives Debug over credential fields. `new_without_pgpass` avoids passfile
loading but still initializes fields from PG* variables. Public setters allow
explicit endpoint/password/SSL policy; they do not provide a general
environment-free initializer. The
[URL parser](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-postgres/src/options/parse.rs)
accepts userinfo and query passwords, logs unknown query parameters with their
values, and applies passfile lookup after parsing. Wrapping its returned error
cannot retract an already emitted warning. The plan requires URL validation
before native construction and an explicit reference-root ambient-source policy.
These points were checked in Cargo's selected registry source and the versioned
upstream source; docs.rs SQLx source retrieval was unavailable during this audit.

The selected SQLx core 0.9.0 `src/pool/options.rs` exposes
`get_max_connections`, `get_min_connections`, and `get_acquire_timeout`.
Its documented minimum is internally clamped to maximum. Root validation must
reject an invalid min/max pair if that is the application contract, and native
getter checks must accompany real acquisition tests.

The pinned Runledger
[JobsConfig implementation](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/config.rs)
defaults malformed environment input and clamps several values. Direct
`validate` instead rejects invalid configuration; its lease/retry lower bounds
are 1, whereas environment loading clamps them to 10 seconds/1000 milliseconds.
The inspected `src/supervisor.rs` explicit builder requires a Tokio runtime and
retains typed config; `builder_from_env` introduces additional ambient intent
settings. Root constructors must use the explicit path. The existing reference
worker probe is an executable consumer seam, not an implemented production host.

### Implementation recheck and native limits

During batter-5pm implementation the selected Cargo sources were rechecked at
those same SQLx 0.9.0 and Runledger Git revisions; `Cargo.lock` adds only direct
edges to already selected packages, with no upstream upgrade. The typed builder
and JobsConfig validation remain the constructor path; environment loaders are
not called by reference settings or worker probes.

The SQLx [URL formatter](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-postgres/src/options/parse.rs)
interpolates a setter-provided host without adding IPv6 brackets. Its
[native TCP socket path](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-core/src/net/socket/mod.rs)
passes `(host, port)` to Tokio. The reference keeps bare IPv6 for native connection
and verifies startup/password bytes in a loopback handshake; DNS/IPv4 option URLs
separately prove password round-tripping. No corrected upstream formatter is
claimed. The selected reference feature graph contains no native SQLx TLS backend;
mode setters are inspected/tested, TLS negotiation is unverified.

The already selected [url 2.5.8](https://docs.rs/url/2.5.8/url/struct.Url.html)
provides URL structure parsing. The reference applies its own narrow query-key
policy and validates percent triplets before
[percent-encoding 2.3.2](https://docs.rs/percent-encoding/2.3.2/percent_encoding/struct.PercentDecode.html)'s
strict UTF-8 decode. Query plus-to-space conversion occurs once. Explicit empty
userinfo passwords retain source presence even when Url normalizes its password
accessor to None, so a query password cannot conceal a conflicting source.

## Configuration boundary follow-up research, 2026-09-10

Rechecked the actual selected graph before this correction round: SQLx 0.9.0,
Runledger 0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4 and harness
3d525e6fc5745ce2e2437c7997de5cccdecff4ac; no dependency update is involved.

- [SQLx 0.9 PgConnectOptions](https://docs.rs/sqlx/0.9.0/sqlx/postgres/struct.PgConnectOptions.html#method.new_without_pgpass)
  and locally resolved options/mod.rs confirm that bypassing passfiles still
  reads PG* defaults. The public native-construction seam is now named
  `connect_options_from_process` to expose that effect; pure injected parsing
  is separate. Clearing a constructor's environment in place would introduce
  process-global mutation, not an environment-free constructor.
- [Rust set_var safety](https://doc.rust-lang.org/std/env/fn.set_var.html#safety)
  explains why multi-threaded Unix programs cannot assume global environment
  mutation is safe. Tests use child environments; an extra hostile-parent
  matrix target protects that boundary. Root PG* rejection stays fail-closed.
- [PostgreSQL 18 identifiers](https://www.postgresql.org/docs/18/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS)
  allow spaces in quoted identifiers. Encoded identifier/application-name spaces
  are intentional; raw URL whitespace rejection prevents parser normalization,
  not legitimate percent decoding. The proposed blanket post-decode whitespace
  ban would reject supported inputs and is not applied.
- [Pinned harness admin URL mapping](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs)
  clones the admin URL and changes the database path, preserving query parameters.
  Preflight must therefore share the Rust validator and native credentials rather
  than rely on a different Python grammar and psql environment/passfile behavior.
- [Pinned JobsConfig validation](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/config.rs)
  has per-field checks and no additional cross-field rule. Setup without identity
  checks the supplied bounds and cannot build a worker; there is no uncovered
  current cross-field invariant. Both roots deliberately own separate schemas;
  an environment file for one root is not promised to configure the other.
- [Pinned worker iteration](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker.rs)
  caps a claim at available capacity, then spawns the returned batch.
  [Native claim transaction](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/claim.rs)
  marks that batch LEASED and commits before returning. The configuration fixture
  precommits its entire eligible set, smaller than claim batch size, so a database
  lease-count observation can detect excess configured capacity independently of
  delayed handler scheduling. The changed live probe still needs real execution.

The selected-file reader follows an operator-selected path; it never claimed a
filesystem sandbox or symlink prohibition. The earlier security defect was reuse
of attacker-prepopulated writable fixture directories, now separately prevented
by exclusive private creation. No global no-symlink policy is inferred.

## Disposable verification endpoint, 2026-09-10

The [official PostgreSQL image documentation](https://hub.docker.com/_/postgres)
was checked before provisioning: `POSTGRES_PASSWORD` initializes the default
superuser's password on an empty instance. A disposable `postgres:18` container
provided live acceptance evidence; the resolved digest and actual server version
are recorded in [validation](validation.md#configuration-live-completion-batter-5pm-2026-09-10).
This operational test setup adds no provisioning implementation or ordinary-test
database dependency to the workspace.

## Live endpoint query decoding correction, 2026-09-10

The pinned [harness AdminClient](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs)
passes its retained URL to `tokio_postgres::Config::from_str`. The resolved
tokio-postgres 0.7.18 `src/config.rs` (`UrlParser::parse_params` / `decode`) only
percent-decodes query values. SQLx 0.9.0 `src/options/parse.rs` instead uses
`Url::query_pairs`, matching the root's form-style `+` to space decoding.
These exact locked sources were inspected locally; the pinned harness source
was also checked upstream. The published source references are
[tokio-postgres 0.7.18](https://docs.rs/crate/tokio-postgres/0.7.18/source/src/config.rs)
and [SQLx 0.9.0](https://docs.rs/crate/sqlx-postgres/0.9.0/source/src/options/parse.rs).

The live handoff now replaces query `+` with `%20` after root validation, while
preserving userinfo and percent escapes. A native parser regression failed on
the original handoff and passes with the correction. The reference package adds
an exact dev dependency on the already resolved tokio-postgres version to test
the actual harness parser; Cargo regenerated only that package's dependency edge
in the lockfile, without upgrading packages. Live server authentication remains
separate evidence.

## Live endpoint hostname handoff correction, 2026-09-10

The locked [SQLx 0.9.0 URL parser](https://docs.rs/crate/sqlx-postgres/0.9.0/source/src/options/parse.rs)
percent-decodes Unix socket paths, but passes other URL host strings directly to
the TCP host setter. The root instead decodes its explicit TCP hostname. Thus
`local%68ost` selected `localhost` for preflight and a different DNS spelling for
fixture pools. The exact installed source was inspected together with the
[pinned harness admin URL mapping](https://github.com/bpcakes/postgres-test-harness/blob/3d525e6fc5745ce2e2437c7997de5cccdecff4ac/src/admin.rs),
which normalizes through `url::Url` and replaces only the fixture database path.

The shared live handoff uses the validated hostname with
[`Url::set_host` in url 2.5.8](https://docs.rs/url/2.5.8/url/struct.Url.html#method.set_host),
with the earlier query-space correction. Its native compatibility limits and
additional normalization are recorded below.
Regressions compare the independent expected host, port, username and database
against the locked native parsers after the harness's normalization steps.
The encoded-host regression failed before the repair. No upstream dependency
or public API change is needed.

## Native live URL compatibility, 2026-09-10

The same pinned SQLx URL parser retains IPv6 authority brackets. Its PostgreSQL
connection stream passes that host to
[`sqlx-core` 0.9.0 TCP connection](https://docs.rs/crate/sqlx-core/0.9.0/source/src/net/socket/mod.rs),
which uses Tokio's `(host, port)` address form. A native reproduction confirmed
that `[::1]` fails lookup there while bare `::1` resolves. The private live
fixture policy now rejects IPv6 literals before connection work; the root's
native-options IPv6 wire test remains supported. This avoids applying a private
live-runner workaround throughout the generic adapter's native URL consumers.

Related comparisons of the locked native parsers found two further scalar
differences: SQLx accepts case-insensitive TLS modes, while tokio-postgres 0.7.18
requires `disable` exactly; SQLx trims all leading path slashes before decoding,
while the harness client removes one. The live handoff serializes the validated
database as one encoded path component and the selected disabled TLS mode in
canonical spelling. Credential/query values retain their selected bytes.

SQLx's URL parser also calls `apply_pgpass` after parsing. An absent URL password
therefore differs from the root's explicit empty Setup password. A regression
using the existing private fake passfile failed against the earlier handoff.
The handoff now supplies an explicit empty query password when necessary; the
same real native parser then preserves the selected empty value. These checks
do not read a real credential file or establish live server authentication.

## Hosted Runledger implementation recheck, 2026-09-11

Rechecked the selected Runledger 0.12.0 revision
`0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4` before implementing the staged
worker host; the dependency graph did not change.

- The pinned
  [supervisor builder and driver](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/supervisor.rs)
  validate every returned build error before the first internal spawn. Build
  success is not an initialization acknowledgement. Dropping the supervisor
  requests shutdown and detaches task handles, so it is not join evidence.
- The pinned
  [worker loop](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker.rs)
  derives its claim filter from registered static handler types. It checks
  shutdown before starting a claim, but a claim already in progress may return
  and dispatch after the request; there is no linearized stop-claim receipt. The
  corresponding
  [claim query](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/claim_ids.sql)
  filters by those job types and contains no worker-instance selector. The
  staged control host therefore uses PostgreSQL's documented session advisory
  lock as an exclusive per-database ownership fence and retains that session
  through observed native driver completion.
- `run_until_shutdown` starts its bounded wait when the supplied external future
  resolves. Requesting only the cloneable native handle leaves that future
  pending, so the application host owns both signals and sends them in that
  order. The pinned
  [task group](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/task_group.rs)
  may spend up to `min(timeout, 1 second)` draining aborted tasks after the main
  bound. It returns the first observed error and logs later drain failures.
- The pinned
  [attempt execution](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker/execution.rs)
  gives each handler a native attempt deadline. The reference control handler
  creates a fresh Batter `OperationContext` from that deadline and injects only
  host-owned dependencies; it never inherits the submitting request token.
  Handler failures and the
  [completion path](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker/completion.rs)
  are durable job outcomes rather than supervisor-loop failures.

These semantics justify the explicit termination gate. The current outer drain
allowance is twenty seconds: ten native shutdown, one abort drain, six runtime
reconciliation close, two lease release and one scheduling margin. The startup witness is clamped before that
complete reserve. Returned timeout or panic remains unproven
transitive termination. An internal application owner retains and publishes the
native join even after wrapper cancellation or Drop; only its observed successful
return permits dependent cleanup. The final Batter report can still veto shared
dependency teardown from direct panic/abort outcomes, abort requests, unjoined
tasks or an `UnsafeTaskExit` cleanup skip.

The follow-up ownership repair was checked against PostgreSQL 18's primary
documentation. Session advisory locks remain held until explicit release or
session end, and `pg_advisory_unlock_all` is implicitly applied even after an
ungraceful disconnect ([advisory lock functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS)).
The dedicated probe connection sets only its own `idle_session_timeout`, whose
documented effect is to terminate a non-transactional idle session after the
configured interval; PostgreSQL cautions that generic pooling middleware may
react poorly to that setting ([client connection defaults](https://www.postgresql.org/docs/18/runtime-config-client.html#RUNTIME-CONFIG-CLIENT-OTHER)).
The implementation therefore detaches this connection from SQLx's reusable pool,
checks it every second, bounds each query locally, and treats loss as a driver
failure. This bounds ordinary stale-session detection; it is not a fencing token
across split-brain failover or an operating-system TCP guarantee.

## Probe ownership and release semantics, 2026-09-11 (batter-2zw)

Rechecked Runledger 0.12.0 at 0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4,
SQLx 0.9.0 and PostgreSQL 18 against these primary sources and Cargo's exact local
sources. No dependency revision changed.

- [Pinned Runledger builder and shutdown](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/supervisor.rs): build spawns after validation; Drop requests shutdown and detaches; abort drain adds up to min(timeout, one second).
- [Pinned cancellation](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/admin/recovery.rs): LEASED becomes CANCELED immediately, original expiry remains, already-terminal updates return invalid-state.
- [Pinned claiming](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/claim_ids.sql): only PENDING rows are claim candidates.
- [SQLx PoolConnection](https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolConnection.html): detach releases pool accounting. Local sqlx-core 0.9.0 src/pool/connection.rs runs after_release before the reuse ping; returning false closes instead. src/pool/options.rs marks parent() internal-only, so the reference does not use it.
- [PostgreSQL termination protocol](https://www.postgresql.org/docs/18/protocol-flow.html#PROTOCOL-FLOW-TERMINATION): Terminate and client close are not a backend-exit acknowledgement; matches local sqlx-postgres 0.9.0 src/connection/mod.rs.
- [Advisory lock functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS): unlock's boolean distinguishes removal from no lock held.
- [Tokio 1.53.1 Unix Signal](https://docs.rs/tokio/1.53.1/tokio/signal/unix/struct.Signal.html): recv is cancellation-safe; a completed receive is consumed. The foundation now retains that fact during registration.

## Late startup-control commit recheck, 2026-09-11 (batter-8q8.4)

Rechecked the exact Runledger 0.12.0 revision
`0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4`, SQLx 0.9.0 source revision
`003b698e99e024f3621b8043a2426fde5b741171`, and PostgreSQL 18.6. No
dependency revision changed.

- Runledger's pinned
  [standalone enqueue](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/src/jobs/queue/enqueue.rs)
  awaits the INSERT before dispatching COMMIT. Cancellation while the INSERT is
  lock-blocked therefore cannot produce a later committed row; the review's
  original trigger was rejected.
- SQLx's selected
  [transaction implementation](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-core/src/transaction.rs)
  clears its open flag only after the database commit future completes. Dropping
  that future after COMMIT dispatch queues a rollback through the PostgreSQL
  [transaction manager](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-postgres/src/transaction.rs).
  The queued rollback follows the already-dispatched COMMIT and cannot undo it.
- SQLx's PostgreSQL
  [connection close](https://github.com/launchbadge/sqlx/blob/003b698e99e024f3621b8043a2426fde5b741171/sqlx-postgres/src/connection/mod.rs)
  sends Terminate and closes the client transport. PostgreSQL's
  [termination protocol](https://www.postgresql.org/docs/18/protocol-flow.html#PROTOCOL-FLOW-TERMINATION)
  does not make that a server-backend exit acknowledgement or impose ordering on
  the separate advisory-lock and successor sessions.
- The deterministic live control uses a deferred constraint trigger, whose
  execution is postponed to transaction end as documented for
  [`CREATE CONSTRAINT TRIGGER`](https://www.postgresql.org/docs/18/sql-createtrigger.html),
  and identifies the exact blocking relationships with
  [`pg_blocking_pids`](https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-INFO-SESSION).
  It observes the predecessor backend blocked in COMMIT and a distinct successor
  backend blocked in catalog synchronization only after the successor's initial
  stale scan. Releasing the gate commits the old row after that scan. Continuous
  owner reconciliation cancels it while excluding the current witness; removing
  that production call makes the live regression time out.

The initial repair did not infer ownership from UUIDv7 order, but its exact-job
exclusion alone was insufficient once a delayed reconciliation outlived the lock
session. The review-loop repair below supersedes that assumption.

## Startup-control ownership fencing recheck, 2026-09-11

Research preceded the review repair and used the same pinned Runledger 0.12.0,
SQLx 0.9.0 and PostgreSQL 18 graph.

- PostgreSQL documents that [`idle_session_timeout`](https://www.postgresql.org/docs/18/runtime-config-client.html)
  terminates an idle session and warns about pooled connections. Session advisory
  locks are released at session end, while their application meaning remains
  application-defined ([advisory-lock functions](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS)).
  A lock check followed by destructive work on another session therefore is not
  a durable ownership proof.
- PostgreSQL [`nextval`](https://www.postgresql.org/docs/18/functions-sequence.html)
  is atomic and returns distinct values across concurrent sessions. Its values
  are not rolled back, so gaps are expected and harmless for an ordering fence;
  it requires `USAGE` or `UPDATE` on the sequence. The application migration
  uses the default cache of one, and lock acquisition obtains `nextval` on the
  exact advisory-lock session before publishing the epoch. The same configured
  database role was already used by the prior pooled call, so the session move
  adds no privilege requirement. Every new startup-control payload stores the
  value; missing legacy values are epoch zero.
- PostgreSQL [`pg_dump`](https://www.postgresql.org/docs/18/app-pgdump.html)
  includes sequence values in its data section by default. The operational
  contract therefore requires logical restores to keep the startup-control rows
  and owner-epoch sequence together; partial restores that reset one side are
  unsupported.
- PostgreSQL [`statement_timeout` and `lock_timeout`](https://www.postgresql.org/docs/18/runtime-config-client.html)
  bound the dedicated reconciliation session. SQLx's
  [`PoolOptions`](https://docs.rs/sqlx/0.9.0/sqlx/pool/struct.PoolOptions.html)
  acquisition timeout covers pool acquisition phases, not arbitrary executed SQL,
  while [`Pool::close`](https://docs.rs/sqlx/0.9.0/sqlx/struct.Pool.html#method.close)
  waits for checked-out connections. SQLx also documents that an incomplete
  close can leave remaining connection disposal to an internal task and that
  client-side drop need not promptly inform PostgreSQL. The implementation
  consequently uses server query bounds, lets SQLx test a returned runtime
  connection before reuse, and supplies a separate six-second close allowance;
  only completion proves close before lease release, while the epoch
  fence remains authoritative after a timeout.
- PostgreSQL does not define [`WHERE` expression evaluation order](https://www.postgresql.org/docs/18/sql-expressions.html#SYNTAX-EXPRESS-EVAL)
  and can reorganize Boolean predicates. The epoch filter therefore cannot rely
  on `job_type` being checked before a fallible text-to-integer cast. Its `CASE`
  expression casts only JSON numbers to PostgreSQL `numeric`; missing or other
  JSON types remain legacy epoch zero. PostgreSQL documents both
  [`CASE` short-circuit selection](https://www.postgresql.org/docs/18/functions-conditional.html#FUNCTIONS-CASE)
  and `jsonb` numbers' mapping to native `numeric` in its
  [JSON type rules](https://www.postgresql.org/docs/18/datatype-json.html).
- The pinned Runledger
  [failure-completion path](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker/completion.rs)
  treats a cancellation race as a completion-persistence failure for that job;
  it does not return it as a supervisor-loop failure. A late lower-epoch control
  can therefore consume one attempt before reconciliation without killing the
  worker or authorizing the current witness. The application no longer claims a
  no-claim barrier.
- The pinned Runledger
  [`worker_loop`](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/worker.rs#L100-L205)
  polls immediately, then waits the configured full polling interval after an
  empty claim. Its native validation requires a nonzero interval but does not
  relate that value to an application witness. A delayed predecessor retry can
  therefore wait one complete poll before its next claim. The application must
  reserve retry delay + poll interval + scheduling margin inside its witness;
  this is an application composition rule, not a Runledger guarantee.
- The pinned baseline migration already supplies
  [`idx_job_queue_type_status_created`](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-postgres/migrations/202603280001_runledger_baseline.up.sql)
  on `(job_type, status, created_at DESC)`, so the periodic candidate scan is not
  an unindexed full-table predicate. The added payload epoch remains a residual
  filter over those candidates.

Lease monitoring and reconciliation now race independently. Reconciliation owns
one lazy connection outside the application pool, each pass is bounded, and a
confirmed pool close precedes advisory-lease release. After a reconciliation
failure initiates native shutdown, lease monitoring continues through the native
join and that pool close; any later lease failure remains a supplemental typed
cause. Because the complete
read/cancel operation is idempotent—including readback of a cancellation whose
acknowledgement was lost—the application explicitly authorizes two consecutive
retries without error-string classification. A third consecutive failure stops
the critical worker and retains the final concrete cause; any successful pass
resets the streak.

## Agent-consumer lifecycle redesign: research closure (2026-09-11)

Owning Bead: `batter-gi4`. These are source-backed design decisions for the
pending redesign, not claims that the new APIs or their acceptance tests exist.
The native checkout inspected is Runledger
`50620137e36aab2333213fa8d8e51a095484e6eb`; the reference still pins
`0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4`. No dependency was changed by this
research. Existing witness-contract sections above describe the current code.

### Initialization, health and execution are distinct observations

Native [SupervisorBuilder::build](https://github.com/bpcakes/runledger/blob/50620137e36aab2333213fa8d8e51a095484e6eb/runledger-runtime/src/supervisor.rs)
validates configuration before spawning the enabled loops, but exposes no
per-loop initialization acknowledgement. Native loops have explicit local
initialization boundaries: worker validation and `WorkerLoop::new`; promoter
validation and registered-type collection; scheduler validation; reaper validation,
registry ownership and observer-set construction. Their subsequent normal passes
can perform database mutations. An empty worker registry waits without claiming.

Decision: add instance-local acknowledgement at those boundaries, before each
loop's first processing pass. Disabled loops contribute no required acknowledgement.
Serialize stop/failure and final acknowledgement so stopped startup cannot revive.
Initialization need not wait for queue activity or a successful database operation.
This does not promise that every loop waits at a global barrier before processing;
one initialized loop can already run while another initializes.

Production readiness combines native initialization, fresh dependency health and
explicit application approval. The reference must withhold approval while its
business handler is absent. A successful dependency query cannot prove job claim,
handler or commit correctness. Exercise that complete path in isolated acceptance
tests; do not recreate a production startup-control protocol to claim such proof.
Runtime progress monitoring, if later required, needs its own explicit contract.

### Native descendant settlement cannot be inferred by an adapter

Native [worker observers](https://github.com/bpcakes/runledger/blob/50620137e36aab2333213fa8d8e51a095484e6eb/runledger-runtime/src/worker/observers.rs)
own nested running callbacks as well as terminal callback tasks. Several Drop
paths request abort and then release the running callback's join handle. Terminal
shutdown can exhaust its abort-drain wait and only log unresolved tasks. Reaper
observer shutdown has the same log-only unresolved outcome. Consequently, merely
changing the outer supervisor's return type cannot establish descendant termination.

[Tokio JoinHandle](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html)
(resolved to 1.53.1 when checked) documents that dropping a handle detaches its task,
and abortion is a request whose completion must be observed. Joining a parent does
not join its independently spawned descendants. Decision: native settlement must
retain observation of its owned descendant set independently of loop-future
destruction, including running callbacks. Report the initiating cause, later
settlement errors, requested abortions and remaining unjoined work. Batter retains
that settlement observation independently of the registered wrapper and uses it
in its cleanup decision. Unknown termination never becomes clean by default.

The report covers a finite settlement episode and its initiating cause, not an
unbounded history of every completed job or best-effort notification. Ordinary
business failures remain native durable outcomes. Preserve all causes within the
declared settlement scope; do not silently truncate errors to achieve a memory cap.

[TaskTracker 0.7.19](https://docs.rs/tokio-util/0.7.19/tokio_util/task/task_tracker/struct.TaskTracker.html)
can witness destruction of tracked futures, but `close` does not prevent new
spawns and Drop does not abort tasks. It therefore cannot replace ownership,
admission closure, typed result retention and bounded joining by itself.

### Finite commands need an owner, not another deadline helper

The current `crates/batter/examples/finite_command.rs` explicitly requires callers
to keep polling its outer future through cleanup. `OperationContext::reserve_finalization`
creates two descendants of the same cancellation token; it neither shields cleanup
nor arranges for cleanup to execute. Process admission also requires a running,
ready service and does not end that service when one command completes.

Decision: use the existing owned-startup coordinator pattern for a separate finite
command entrypoint. Validate before invoking an inert factory; retain registered
resources, work outcome and LIFO cleanup in the coordinator. A cancelled borrowed
waiter has no ownership effect; dropping the command owner requests cancellation
while a live runtime continues bounded finalization. Returning with `?` from the
work callback cannot bypass cleanup. Native acquisition cancellation and remote
effects retain their actual contracts; no arbitrary detached-task joining follows.

### Budgets start at transitions and include nested settlement

Native [TaskGroup](https://github.com/bpcakes/runledger/blob/50620137e36aab2333213fa8d8e51a095484e6eb/runledger-runtime/src/task_group.rs)
adds an abort-drain allowance of up to one second after its graceful deadline.
Worker observers separately allow twenty seconds of terminal drain and a bounded
abort drain in production. A ten-second outer graceful limit can therefore abort
the worker before its observer allowance ends. The native drive loop also listens
to its supplied external future separately from the native shutdown handle;
requesting only the latter does not itself select the bounded external-stop branch.

Decision: make first-stop state and its absolute deadline authoritative across
request paths. Repeated requests and delayed polling cannot restart the allowance.
Nested phases consume that budget, with explicit abort/join reserve. Cleanup uses
its own owner and allowance after work settlement; it does not inherit work
cancellation. Sequential phase allowances add; concurrent components consume a
shared interval. Validate any enclosing total and reserve before starting work.
Remove witness retry/poll timing relationships when removing the witness. These
are phase constraints, not a general scheduling or budget-solving framework.

### Legacy controls have an offline retirement boundary

[PostgreSQL advisory locks](https://www.postgresql.org/docs/18/explicit-locking.html#ADVISORY-LOCKS)
belong to a session or transaction; release of the dedicated lock session does
not prove a separate enqueue session has stopped. PostgreSQL
[backend signaling](https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADMIN-SIGNAL)
also distinguishes a successfully sent signal from confirmed termination:
`pg_terminate_backend` with zero timeout only acknowledges sending.

Decision: retire controls offline, after stopping old producers and preventing
restart. Verify their actual sessions and transactions have ended on the intended
server before scanning; a free advisory lock or application-name-only snapshot
is insufficient. If that precondition cannot be established, do not cancel rows
under a claim of completed retirement. Cancel only matching nonterminal controls
through native cancellation, retain terminal history and applied migrations, and
leave the harmless sequence in place. Test a delayed enqueue on a separate session
to ensure retirement cannot declare success while it can still commit.

### Evidence still required

The design choices above are resolved; implementation correctness and agent
usability are not established by research. Deterministic tests must exercise
acknowledgement/stop races, wrapper and waiter loss, nested callback abortion,
combined errors and non-resetting budgets. Live tests must cover durable execution
and offline retirement orderings. Fresh agents must independently integrate and
then modify the public path against hidden failure scenarios. Record execution
results before claiming that the redesign improves review/fix convergence.

## Agent-consumer follow-up: remaining boundary decisions (2026-09-12)

Owning Bead: `batter-gi4`. This follow-up inspects the current uncommitted
implementation as well as the sources above. It records design decisions, not
completed implementation or new acceptance results. The root now has development
path patches for the three native packages in `../runledger`; the preceding
research entry's statement that dependencies were unchanged is historical.

### Register a native launch value rather than arbitrary application code

The draft `crates/batter-runledger/src/lib.rs::register` accepts a closure returning
a live native supervisor. A caller can construct that supervisor before calling
`register` and capture it in the closure. The function's type accepts this even
though it violates the documented no-work-before-registration contract. Duplicate
registration then drops an already-running native owner. This is a concrete
remaining agent-consumer footgun, not evidence that the managed driver is wrong.

Native `runledger-runtime/src/supervisor.rs` already separates validation from
its private `start` operation, but its public builder borrows a pool and `build`
immediately spawns. Decision: expose an owned, validated, inert native launch
value, produced by `SupervisorBuilder::prepare`. Its fields remain private; it
owns cloned native handles and configuration, never a running task. Preserve
`build` as the existing convenience path through preparation and start. The
canonical Batter adapter consumes the inert value and invokes native start only
inside managed ownership. Do not replicate the native builder in Batter or ask
application agents to write the managed settlement protocol.

Acceptance must include dropping preparation, duplicate registration and dropping
an unstarted process with zero native task starts. A compile-fail consumer must
be unable to pass a live supervisor or a closure in place of the launch value.
This constrains native construction, not arbitrary side effects in user-written
handler constructors or destructors.

### Keep the first cause, but allow a deadline to become earlier

The current native `shutdown.rs` stores `(started, cause)` together in `OnceLock`.
`task_group/report.rs` computes its two deadlines once. Consider parent drain at
t=0, native failure at t=1, and delayed adapter propagation of the parent stop at
t=2: `request_shutdown_since(t=0)` currently retains t=1. The parent still bounds
its own observation and skips uncertain cleanup, so this is not proof of false
clean termination. It does mean native settlement can exceed the interval the
adapter claims to share with its parent.

Decision: preserve the initiating native cause separately from the authoritative
deadline. A received earlier enclosing deadline may tighten the native deadline;
no request may extend it. Deadline changes must wake an already-running settlement
wait and affect both graceful and abort observation. Merely replacing `OnceLock`
with a minimum timestamp without changing the already-created sleeps is
insufficient. Native stop observation must also carry its timestamp into the
parent, rather than starting a fresh parent interval when the event is polled.
Add deterministic tests for both propagation orders, a tightening during each
phase, repeated later requests, and retention of the original native failure.

### Retire the known legacy producer through native admission policy

The native API already supplies a narrower durable control than another custom
queue trigger: `update_job_definition` can set only `is_enabled = false` while
retaining other definition fields. Its disable guard rejects active schedules.
Native [enqueue](https://github.com/bpcakes/runledger/blob/50620137e36aab2333213fa8d8e51a095484e6eb/runledger-postgres/src/jobs/queue/enqueue.rs)
requires an enabled definition and holds a shared row lock during insertion.
The actual old root uses additive `JobCatalog::sync_definitions`; that
[pinned implementation preserves a stored disabled state](https://github.com/bpcakes/runledger/blob/0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4/runledger-runtime/src/catalog/sync.rs).
Consequently, disabling the exact legacy definition serializes with that native
enqueue path and its normal startup sync cannot simply re-enable the producer.
This is an inference for the inspected producer/version, not protection against
arbitrary SQL writers, exact catalog sync or an administrator re-enabling it.

Decision: make native definition disable part of the offline retirement path,
then use `cancel_job_with_scope` for only the legacy type's nonterminal rows.
Preserve cancellation events, attempts, terminal rows, applied migrations and
the sequence. Missing definitions are an idempotent absence, not a reason to
create a fake handler/catalog. Do not use the exact-sync API with an empty
catalog: that public API explicitly rejects empty catalogs. Resolve uncertain
cancellation acknowledgement by reading actual state and retain the original
failure; a partial batch must never be reported as complete.

The separate offline precondition remains necessary: deployment tooling owns
stopping old producers and revoking their restart authority. Database credentials,
process names and an advisory-lock snapshot cannot establish that fact. The
retirement entrypoint must not claim deployment completion from a caller-provided
boolean. It verifies database identity, actual old session/transaction absence
and the native disabled definition; its report states exactly those observations.
Provisioning and deployment orchestration remain outside Batter.

There is an additional transaction case: PostgreSQL
[PREPARE TRANSACTION](https://www.postgresql.org/docs/18/sql-prepare-transaction.html)
detaches a transaction from its session while retaining its locks and allowing
later commit. Therefore, session disappearance cannot prove there is no later
commit. Check the target database's
[pg_prepared_xacts](https://www.postgresql.org/docs/18/view-pg-prepared-xacts.html)
and refuse completion while any prepared transaction remains; do not guess which
ones are harmless or roll them back automatically. In the native enqueue path,
the retained definition lock can also make disable wait: bound that wait and
report it rather than treating timeout as successful retirement.

Rejected shortcuts are now explicit. `NOLOGIN` does not disable privileges used
through another login's [role membership](https://www.postgresql.org/docs/18/role-membership.html).
`ALLOW_CONNECTIONS false` blocks ordinary new database connections but is not a
proof that existing producers stopped; PostgreSQL 18.6's
[database command implementation](https://github.com/postgres/postgres/blob/REL_18_6/src/backend/commands/dbcommands.c)
also rejects setting it from the target database itself. Adding a global database
maintenance controller would exceed this example's retirement responsibility.

Required live scenarios are a delayed enqueue on a separate session, a prepared
enqueue whose client exited, restarted additive catalog sync preserving disable,
a cancellation with uncertain acknowledgement, and unchanged domain/terminal
rows. The existing task-owned PostgreSQL 18.6 server was read-only checked during
this follow-up and has `max_prepared_transactions = 0`; that configuration does
not execute the prepared-transaction scenario. Run that case on a separately
provisioned disposable PostgreSQL 18 cluster with two-phase transactions enabled.

### External source changes cannot inherit root-only verification receipts

Cargo's [patch mechanism](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
supports the sibling development sources; the lockfile does not fingerprint
their working contents. Jig's pinned revision
`10a3dc9ae63547b09a48b05a463495bce2101f37` rejects parent-directory input globs in
`crates/jig/src/repository/affected/tests.rs`. Its `docs/public-contract.md`
explicitly excludes arbitrary external changes from repository freshness.
Adding `../runledger/**` to the root's exhaustive inputs is therefore invalid.

Decision for this uncommitted development phase: force the required Rust targets
when producing final evidence, record both repositories' relevant source content
identities before and after execution, and run with native source writers stopped.
A mismatch invalidates the run. Root receipt freshness alone cannot authorize
reuse while these path patches are active. Do not build a new Jig attestation
subsystem or vendor a second runtime to conceal the boundary. The eventual
versioned native dependency cutover is separate from this no-commit task.

### Scope and the remaining empirical question

Keep native task accounting in Runledger, managed process/command ownership in
Batter, translation in the optional adapter, and application schema/health/handler
approval in the reference root. A production startup control job adds a durable
protocol to answer a local initialization question and stays removed. The missing
business handler continues to withhold readiness approval.

Research resolves those responsibilities; it cannot establish review convergence.
Fresh agents must integrate and then modify the public path without private
implementation explanations. Evaluate against independently authored ownership,
deadline, readiness and failure-retention scenarios. Record semantic failures and
repair rounds, including reviewer disagreement. A smaller diff or a clean final
review alone is not evidence that a new agent can use the API correctly.

## Retirement research closure against the draft (2026-09-12)

Owning Bead: `batter-gi4`. This follow-up inspected the uncommitted retirement
draft and executed narrowly scoped PostgreSQL experiments. It does not establish
that the retirement command works: its live acceptance remains outstanding.

### Unknown activity must prevent completion

PostgreSQL's [statistics visibility contract](https://www.postgresql.org/docs/18/monitoring-stats.html#MONITORING-STATS-VIEWS)
exposes the existence, user and database of other sessions to ordinary roles,
while hiding other fields. The [18.6 implementation](https://github.com/postgres/postgres/blob/REL_18_6/src/backend/utils/adt/pgstatfuncs.c)
places backend type behind statistics permissions. The draft filters with
`backend_type = 'client backend'`; SQL excludes a hidden NULL value.

Executed on the task-owned PostgreSQL 18.6 primary: one acknowledged session ran
`pg_sleep`, while a separate connection used `SET ROLE pg_read_all_settings`.
That role observed the first session's database, user and application name but a
NULL backend type. Both connections exited normally; no role or schema was changed.
This reproduces the predicate's undercount, not a complete retirement invocation.

Decision: do not interpret unknown activity as absence. Refuse retirement for
other target-database backends unless their type is visible and explicitly
non-producing, such as an autovacuum worker. Unknown types, client sessions and
parallel execution remain blockers. Do not automatically grant privileges, kill
sessions or infer authority from an application name. Check before mutation and
again at completion. A restricted-role live control must exercise the actual
retirement path, not merely repeat the corrected predicate in a test.

### Prepared transactions require a separate observation

PostgreSQL [PREPARE TRANSACTION](https://www.postgresql.org/docs/18/sql-prepare-transaction.html)
retains transaction state independently of the originating connection, including
locks. [pg_prepared_xacts](https://www.postgresql.org/docs/18/view-pg-prepared-xacts.html)
is the appropriate database-scoped observation; an empty client-session set is
insufficient.

Executed on a separate disposable PostgreSQL 18.6 container, with networking
disabled and `max_prepared_transactions=10`: commit fixture schema first, then
hold a definition row `FOR SHARE`, insert a job and prepare the transaction. After
that client exited, independent SQL observed one prepared transaction, zero other
clients and zero visible jobs. Definition disable failed with a 100 ms lock
timeout. Explicitly committing that known fixture transaction exposed one job;
disable then succeeded and no prepared transactions remained. The disposable
container was stopped and removed.

Two exploratory attempts did not prove this sequence: the first contacted the
image's temporary initialization server; the second accidentally included fixture
DDL in the prepared transaction and received a missing-relation error. The final
probe waited for PID 1 to be the final postgres server, committed DDL separately,
and asserted the actual lock-timeout and before/after counts. Only that final
probe supports the lock-ordering result. This is a PostgreSQL semantic experiment,
not execution of Runledger's enqueue or the retirement command.

Decision: check prepared transactions before disable, so existing prepared work
produces an explicit refusal instead of merely exhausting the command budget.
Keep the post-disable check for races. Never resolve unknown prepared transactions
automatically. Native enqueue/disable ordering still needs its own live acceptance.

### A one-slot pool does not pin a maintenance session

SQLx 0.9.0 [pool options](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/pool/options.rs)
and the locally resolved `sqlx-core-0.9.0/src/pool/inner.rs` permit replacement
connections. Disabling idle/max-lifetime retirement does not prevent replacement
after a broken socket. `after_connect` errors close the new connection, log the
error and enter a retry loop. Therefore the draft's initial identity query plus
`max_connections(1)` does not establish identity for subsequent operations.

Decision: use a direct PostgreSQL endpoint; the mutating maintenance owner must reject physical-session replacement
after its first verified connection, with bounded acquisition and retained typed
diagnostics. Do not rely solely on the acquisition hook's logged error or forward
raw database errors into that log. After connection loss, stop mutation; optional
readback uses a separately identity-verified, read-only connection. Never replay a
cancellation automatically. Cluster system identifier plus database OID identifies
the database lineage, not a unique physical replica; maintenance also requires a
stable intended endpoint and excludes failover during the operation. This remains
example-owned maintenance policy, not a new Batter connection manager.

### Publish the primary failure before optional reconciliation

The inspected native `runledger-postgres/src/jobs/admin/recovery.rs` maps cancellation
begin/commit SQLx errors to `Error::ConnectionError(error.to_string())`. Its missing
job path also logs and discards rollback failure. The native error model already
has `QueryError::source_arc` and `from_query_sqlx_with_context`; preserving a
returned native error in Batter cannot recover causes already discarded upstream.

The retirement draft has a second independent loss point: after receiving a native
cancellation error, it awaits readback before returning that error. Command
cancellation during that await destroys both the readback future and its captured
original error. The command owner preserves returned work outcomes, not every
intermediate value in an application future.

Decision: repair source retention and secondary rollback failure in the native
cancellation path. Return and publish the retirement command's primary failure,
including job identity and acknowledged progress, before optional reconciliation.
Readback is a separate read-only operation against an already retained primary
report. A missing readback remains explicitly unobserved; it never upgrades the
primary failure to success. Do not add an application error side channel, a generic
failure journal, or another cleanup coordinator to compensate for this ordering.
Acceptance must interrupt reconciliation after the primary report is published
and prove that the original native/SQLx cause remains inspectable.

### Keep absence and compiler feedback accurately classified

Native disable returns absence without inserting a definition. If a definition is
absent, a later old additive catalog sync could insert it enabled. Accordingly,
`DefinitionState::Absent` is an observation, not a durable disabled tombstone.
Deployment restart revocation remains required in both absent and disabled cases;
only an existing disabled row has the inspected additive-sync preservation property.
Do not introduce a fake handler or silently claim a stronger database admission
guarantee for absence.

The draft's return type `Command<RetirementReport, RetirementError>` also failed
`cargo check -p batter-example-reference-service --all-targets --locked --quiet`:
the implemented inert specification is `Command<F>`, while its running owner and
reports use result/error generics. This is confirmed consumer/API naming friction,
not evidence of a runtime ownership defect or repeated review failure. Correct the
consumer against the actual public factory signature and include returning an
inert specification from a helper in fresh-agent acceptance. Do not redesign the
foundation solely to make this one incorrect annotation compile.
The annotation was corrected to return an opaque factory implementing the public
`CommandFuture` contract. The same all-target check and package formatting check
then passed on Rust 1.98.1. This corrects compilation only; it does not validate
the retirement behavior or close the operational gaps above.

The remaining design choices are settled. Implementation and acceptance remain:
repair the concrete gaps above, execute native and retirement fault scenarios,
then evaluate independent agent integration/modification and full review repair
convergence. More documentation cannot substitute for those results.

## Shared-join notification ownership (2026-09-12)

The resolved futures-util 0.3.34 package identifies upstream source commit
`705e6b5c0f06535b1aac1cb1989a172b3d45be8c`. Its
[Shared implementation](https://github.com/rust-lang/futures-rs/blob/705e6b5c0f06535b1aac1cb1989a172b3d45be8c/futures-util/src/future/future/shared.rs)
wakes registered wakers while holding the notifier mutex; dropping a registered
Shared observer also locks that mutex. A native registry waker that owns the
registry containing that observer can therefore destroy it recursively during
notification when the waker holds the final registry reference.

This was observed, not merely inferred: a focused callback probe reached runtime
destruction with one worker blocked in Shared::drop. Its GDB stack showed
Notifier::wake_by_ref -> registry ArcWake/drop -> RegistryInner/Entry destruction
-> Shared::drop -> the same notifier mutex. The process was explicitly terminated
after diagnosis; it was not counted as passed. A regression independently checks
that a pending notification does not retain the registry, keeping the old registry
alive during test cleanup to avoid hanging the regression itself. It failed before
the fix and passed afterward. The callback live probe and all 233 native runtime
library tests also pass after the wake signal was separated from registry ownership.
This is a correction in native descendant observation; adapters and application
agents acquire no additional joining or cleanup obligation.


## Native newer-Clippy baseline (2026-09-12)

The supplemental Rust 1.98.1 strict native Clippy run reports 18 `result_large_err`
diagnostics for the existing runtime error enum (maximum variant 160 bytes).
The same command against an untouched archive of native HEAD
`50620137e36aab2333213fa8d8e51a095484e6eb` reproduces all 18 diagnostics with the
same size. Native strict Clippy on exact Rust 1.94.0 passes. Both toolchains compile
and pass native library/doctest/cancellation tests; Batter's required complete
workspace matrices and strict Clippy pass on both.

Clippy's [upstream change #17130](https://github.com/rust-lang/rust-clippy/pull/17130),
merged June 2, 2026, extended `result_large_err` and `result_unit_err` to async
functions. That explains why this existing representation is newly diagnosed;
it does not demonstrate a new allocation, lost error, or lifecycle failure.
The [official configuration](https://doc.rust-lang.org/stable/clippy/lint_configuration.html#large-error-threshold)
documents the default 128-byte threshold. We preserve native source compatibility
rather than redesign the existing error payloads solely for this supplemental
newer-compiler lint. No lint threshold or assertion was relaxed. The exact failed
commands remain in validation evidence; strict native 1.98.1 Clippy is not claimed.

## Native review boundary decisions, 2026-09-12

The pinned SQLx 0.9.0 pool implementation exposes `begin_with`, allowing owned
cancellation to start with `BEGIN ISOLATION LEVEL READ COMMITTED` in the same
native transaction. PostgreSQL 18's [SET TRANSACTION contract](https://www.postgresql.org/docs/18/sql-set-transaction.html)
and [connection defaults](https://www.postgresql.org/docs/18/runtime-config-client.html)
confirm that a plain BEGIN inherits a configurable session isolation level.
The revised cancellation path establishes READ COMMITTED explicitly; it does not
modify the session default. A PostgreSQL 18.6 trigger oracle rejects cancellation
unless that actual transaction is READ COMMITTED, while the connection remains
configured SERIALIZABLE. This test failed before the change.

The native review also traced terminal-observer admission refusal to intentional
running-observer abortion. Its cancelled join must not initiate process failure.
Pre-stop interruptions are counted to keep history bounded; shutdown-time aborts
retain their joins. Uncaught panics and unexpected cancellations remain fatal,
and older Result drivers must retain that cause rather than return false success.
Handler timeout, caught panic and lease-maintenance failure are interruptions even
when the outer worker task joins successfully. Their settlement classification
must reflect that distinction; returned business errors remain durable outcomes.
Tests keep an actual application child alive at the native report boundary and
then join it independently, separating native task completion from transitive
application settlement.

## Remaining native design questions, 2026-09-12

Native full review pass two used stable fingerprint
`785bb346d7ca1c3e65faf6a751c108068b51c16d1b2902c64309946fe5da80a3`.
Its open questions were resolved against the actual consumer and failure contracts:

- Active observation belongs to the native driver. A regression let a descendant
  panic after the driver began waiting, withheld its separate join waiter, and
  reproduced the driver hanging. All Result entrypoints now drive shared registry
  observation; an application agent needs no supplementary watcher. Both actual
  supervised-worker destructor-panic integration cases pass through `join` and
  `run_until_shutdown`.
- A caught reaped-observer destructor panic had become an ordinary wrapper result.
  The report incorrectly approved cleanup. The outer catch now records the original
  panic as a callback failure before returning its typed normal wrapper output.
- Callback interruption cannot establish that arbitrary application children
  stopped. Historical interruption facts therefore permanently prevent cooperative
  cleanup and overall shutdown success for that runtime instance. Durable business
  failures remain separate. A joined configuration error permits cleanup but still
  fails shutdown; callers must use the appropriate predicate.
- The native Result API is a loop-observation compatibility path, not proof that
  every descendant has completed aborting. Dependency owners use the complete report.
  Handle-requested `run_until_shutdown` now has the same bounded drain behavior as
  its external signal path; this deliberate behavior change is documented and tested.
- One completion previously caused 256 unnecessary polls of 128 unrelated pending
  joins. Per-entry notifications remove that scan. Their wakers retain only a signal,
  and the signal queue retains weak waker references, preserving the proven absence
  of a Shared-notifier/registry destruction cycle.
- A separate exhausted-budget fixture found zero of 256 already-finished tasks.
  The resolved Tokio 1.53.1 [JoinSet source](https://docs.rs/tokio/1.53.1/src/tokio/task/join_set.rs.html)
  harvests nonblocking joins through `unconstrained`; the native task set now follows
  that approach for join observation only, never for application future polling.
- PostgreSQL 18 explicitly supports isolation characteristics in
  [BEGIN](https://www.postgresql.org/docs/18/sql-begin.html). SQLx 0.9.0 sends the
  supplied statement through its transaction manager. There is no evidence here
  that the cancellation helper needs a second abstraction to accommodate a proxy
  rewriting that statement. [PgBouncer's feature contract](https://www.pgbouncer.org/features.html)
  distinguishes session and transaction pooling from statement pooling, which
  disallows multi-statement transactions. No live PgBouncer compatibility claim
  is made. Reusing the older two-statement helper would also inherit its different
  rollback-error contract; that is not a valid reason to weaken the new cancellation
  boundary.

New report types remain exhaustive in this change. The review's future source-
compatibility observation is below the configured medium repair threshold; it is
not a current lost-error or cleanup defect. Adding new variants or fields later
requires an explicit compatibility decision, as with other public native types.

## Best-effort callback destruction, 2026-09-12

The next native review found a remaining callback-boundary inconsistency. Four
worker-observer regressions failed before repair: timeout and explicit abortion,
both before and during stop, let a future's destructor panic escape into a fatal
native descendant join. Reaped observers happened to catch timeout destruction
at an outer poll boundary; that was not a shared callback ownership contract.

The resolved futures-util 0.3.34
[CatchUnwind implementation](https://github.com/rust-lang/futures-rs/blob/705e6b5c0f06535b1aac1cb1989a172b3d45be8c/futures-util/src/future/future/catch_unwind.rs)
protects polling. It does not protect destruction of the wrapper itself. Native
observers and dead-letter hooks now use one private callback owner that catches
polling and synchronous future destruction, including destruction caused by a
native timeout or task abortion. Original polling and later destruction failures
are both retained. This adds no asynchronous Drop/finalization guarantee and does
not claim that application-created detached children stopped. Unexpected main
job-task failures remain fatal; best-effort callback interruption still disqualifies
cooperative cleanup. The process panic hook and existing native diagnostics retain
their documented behavior.

Eight running/terminal-observer controls now pass. Separate hook tests retain both
poll plus destruction failures and timeout plus destruction failures. The earlier
reaped test now requires both the timeout and its destructor failure; its former
single-fact expectation was strengthened after the shared owner made both causes
observable. A passing join never erases that callback interruption evidence.

## Native registry overhead probe, 2026-09-12

An external microbenchmark on a shared Linux development host (not an isolated
performance environment) compiled the current registry and shutdown sources
against Rust 1.98.1, Tokio 1.53.1 and futures-util 0.3.34 in release mode. Each case
ran 10,000 yielding tasks five times with alternating baseline/registry order;
values below are median nanoseconds per task. The registry variant includes a
concurrent active shutdown/descendant observer. The baseline uses native JoinSet.

| Runtime threads | Concurrent tasks | JoinSet | Registered plus active observer | Added time |
| --- | --- | --- | --- | --- |
| 1 | 64 | 492 ns | 1,444 ns | 952 ns |
| 1 | 256 | 557 ns | 1,678 ns | 1,121 ns |
| 4 | 64 | 1,066 ns | 4,472 ns | 3,406 ns |
| 4 | 256 | 618 ns | 3,483 ns | 2,865 ns |

The ownership/observation work has a measurable cost; it is not zero-cost task
wrapping. This bounded probe also executes registry observation concurrently with
join harvesting on four threads. It does not measure real queue/database throughput,
latency tails, or application callbacks, and is not a production capacity claim.
The design retains this accounting: it supplies the required ownership and failure
observations, and the deterministic regression independently prevents the former
quadratic unrelated-join scan. No high-throughput worker target is being claimed
or used to justify another abstraction.

Harness, generated lockfile and output are in `/tmp/batter-gi4-registry-bench/`;
`active-observer-release.log` is the final measurement. The copied registry differs
only in its module-source path annotation. Initial direct-rustc setup failed to
select compatible cached artifacts; an external Cargo workspace resolved the exact
dependency graph instead. Earlier debug and passive-observer timings are exploratory
and are not the measurements above. The native repository source was not changed
by the benchmark.


## Transaction-control classifications and CLI recovery, 2026-09-12

A deferred PostgreSQL constraint trigger can fail when COMMIT runs; it need not
fail the earlier mutation statement. PostgreSQL 18 documents that timing in
[SET CONSTRAINTS](https://www.postgresql.org/docs/18/sql-set-constraints.html).
The live regression deliberately raises SQLSTATE 23514 from a deferred constraint
trigger; it does not claim that ordinary CHECK constraints are deferrable.
The resolved SQLx 0.9.0 `sqlx-core/src/transaction.rs` propagates its transaction
manager's commit error. The hosted docs endpoint was unavailable during this
recheck, so the exact Cargo-resolved source was inspected locally instead.

The old native cancellation boundary fed begin/commit errors through the ordinary
statement classifier. Actual tests reproduced `db.query_failed` for a closed
pool at BEGIN and `db.business_rule_violation` for the deferred COMMIT rejection.
Preserving the SQLx cause alone did not make that API sensible for agents branching
on stable codes. The existing typed classification mechanism now supplies fixed
internal `TransactionBeginFailed` / `TransactionCommitUnconfirmed` kinds and
`db.transaction_begin_failed` / `db.transaction_commit_unconfirmed` codes.
SQLSTATE and original SQLx sources remain available for trusted diagnosis; these
codes do not establish rollback or authorize replay. Statement classification and
caller-owned transaction contracts remain separate.

The operator CLI now carries the retained facts across its final output boundary:
JSON includes stage, refusal kind, target identity, quiescence counts, cancellation
job ID, earlier acknowledged cancellations, sanitized native classification and
cleanup disposition. Session-replacement wrappers preserve their original facts.
Native error text is omitted. The actual executable rejects wrong identity, emits
repeated absent-definition success, and gives structured usage failures. Library
readback stays separate from the primary outcome and cannot turn it into success.

The managed failure-publication regression similarly exposed an ownership-boundary
error: a caught repeated-stop panic remained local until potentially unfinished
native settlement. Publishing at the shared catch point preserves it in the frozen
process report. The regression first failed with zero retained failures and passes
with the original payload visible before settlement finishes; later completion
never restarts skipped cleanup. No new application-level coordination protocol was
introduced for either repair. Executed results are in [validation](validation.md).

## Final descendant harvest and remaining contract questions, 2026-09-12

Notification-driven observation and a final settlement check have different needs.
An external scheduling probe paused a collector immediately after taking the ready
queue: a real Tokio task was already finished before another snapshot began, yet
that snapshot reported one unjoined task. The probe restored the held notifications
and awaited the actual task before its failed assertion. After repair the same
probe reports zero unjoined tasks. Evidence is in
`/tmp/batter-gi4-harvest-probe-{red,green}.log`; the temporary harness copies the
native registry, changes only its report-module path, and adds the scheduling probe.

Collectors now acquire registry state before taking the queue. Shutdown boundary
checks additionally inspect each finished handle once, independently of queued
notifications, and removal wakes registered waiters. Ordinary observation still
polls only notified joins. This avoids replacing a lost-notification defect with
an unbounded notification-drain loop or restoring the earlier quadratic scan on
normal task completion. Tests cover all three boundary consumers, the original
shared panic, delayed notification delivery without duplicate records, and the
existing zero-unrelated-repoll control. A shared join concurrently being polled
by another owner can still be pending; bounded reporting cannot await arbitrary
non-yielding execution beyond its allowance.

The remaining native questions do not change the chosen ownership contracts.
Rust 1.98.1's [`Error::source`](https://doc.rust-lang.org/std/error/trait.Error.html#method.source)
returns one optional cause, so `RollbackFailure` exposes the primary operation in
that chain and retains both operation and rollback in public typed fields. The
CLI inspects both explicitly without rendering native diagnostics. The cancellation
classifier stays inside its owned READ COMMITTED transaction, whose statement
snapshots follow [PostgreSQL 18's isolation contract](https://www.postgresql.org/docs/18/transaction-iso.html).
This avoids a second pool acquisition; it does not promise a single transaction-wide
snapshot. Caller-owned workflow transactions retain their existing caller-owned
commit/rollback contract. Batter's actual `register_managed` factory requires `Send`
and captures `PreparedSupervisor`, so the compiled adapter already enforces that
transfer property. Earlier interrupted application callbacks remain permanently
insufficient proof of dependency quiescence; durable business success does not
establish that detached children stopped.

## Remove the remaining settings launch shortcut, 2026-09-12

The reference's public `WorkerSettings::builder` still returned the native builder,
and its README directed application agents to that convenience method. The
configuration consumer then called immediate `build()`, dropped the resulting
supervisor and closed its pool without observing settlement. This was a remaining
application-level route around the intended ownership boundary, despite the
adapter's rejection of already-live supervisors.

Worker settings now return validated `JobsConfig` data only. The existing consumer
passes that data through native preparation and `batter_runledger::register`,
observes initialization, and awaits a successful managed shutdown. No alternate
settings-level launch wrapper was added. The production root already used this
prepared path, so its lifecycle policy is unchanged. The focused consumer test
passes in its cleared-environment child. The independent `InitializationFailure`
redaction/source regression was restored verbatim from the preserved index; its
type remains, even though the surrounding old worker tests were retired.

Other review questions retain the established scope: the reference's 20-second
startup deadline includes native initialization, and ownership handoff does not
claim readiness. A late native initializer fails and drains without approval;
starting a new independent allowance would exceed its parent. Native schema
compatibility explicitly checks public tables and trigger functions in
`runledger-postgres/src/migrations.rs`, matching retirement's pinned search path.
The legacy producer used a global submission and no schedule; retirement leaves
unknown tenant-scoped rows alone and refuses a definition with an active schedule
rather than reporting success. Deployed-version quiescence remains an external
maintenance precondition, not a fact inferred from this binary's empty registry.
