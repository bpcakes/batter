# Native reference compatibility

Owning Beads: `batter-4t6` (compatibility), `batter-4jz` (reusable fixtures), `batter-kjl` (failure retention). This API/evidence manifest accompanies the unpublished
[reference package](../examples/reference-service/README.md). Beads owns delivery
acceptance and status. Worker-host delivery is owned by `batter-0cp`.

## Selected graph

Selection date: 2026-09-09. Both Git revisions were verified against the remotes;
Cargo fetched them and generated the lockfile. No dependency uses an absolute
local path. No library depends on Runledger. The optional SQLx `test-support` feature selects
the pinned harness; the default adapter graph excludes it. The
separately selected `batter-sqlx` adapter uses the same SQLx 0.9 graph; its
connection-disposition contracts are exercised by its own live suite, not by
these reference probes.

| Source | Selected version/revision | Features and boundary | Disposition |
| --- | --- | --- | --- |
| SQLx registry | 0.9.0 | `runtime-tokio`, `postgres`, `uuid`, `chrono`, `json`, `migrate`, `macros`; one resolved SQLx/core/PostgreSQL version | Compiled on Rust 1.98.1 and 1.94.0; live transactions executed on Linux |
| Runledger Git | core/postgres/runtime 0.12.0 at `0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4` | Native `DbPool = PgPool`, `DbTx = Transaction<Postgres>` | Compiled on both toolchains; migration, enqueue and controlled worker path executed |
| postgres-test-harness Git | 0.2.0 at `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` | `default-features = false`; external PostgreSQL through tokio-postgres; optional SQLx test-support dependency; reference development dependency | Compiled on both toolchains; external lease cleanup paths executed |
| Runledger registry | 0.12.0 | Downloaded manifest requires SQLx 0.8.6 and Rust 1.88 | Inspected-only; incompatible with the selected native SQLx 0.9 type identity |

Identical Runledger version numbers do not imply identical registry and Git
sources. Reproduce with the full revision and Cargo.lock. Runledger enables SQLx
defaults internally; optional/target-specific entries in Cargo.lock do not add
other database backends or non-Unix platforms to Batter's support policy.

## Executable contracts

Set `POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL` to two distinct
disposable local PostgreSQL 18 clusters, using the primary superuser/SCRAM/autovacuum
prerequisites in [testing](testing.md#explicit-reference-compatibility-probes), and run `bash scripts/test_reference_live.sh` from the root. It preflights the server
and role, checks an exact inventory of 54 entries (52 live probes, the offline
acquisition-signal control and its private child entry), and invokes
`cargo test -p batter-example-reference-service --test reference_live --locked
-- --include-ignored`. Every named entry must pass, with zero filtered or ignored cases.

| Required contract | Public API and probe | Evidence / limitation |
| --- | --- | --- |
| Native types | Example library `native_pool`, `native_transaction`, `native_connection`; native arguments to `enqueue_job_with_outcome_tx` | Compiler-checked identities and rustdoc sample. `cargo metadata --locked --format-version 1` and `cargo tree -p batter-example-reference-service --duplicates` resolve SQLx 0.9.0 only |
| Independent SQLx verification | `cargo test -p batter-example-postgres-lifecycle --test native_sqlx --locked` | Native pool, connection and transaction signatures compile; an unpolled factory opens no connection. This focused target requires neither Runledger nor the harness and provides a template for optional adapter verification |
| Fresh/repeated startup | `migrate_after_idempotency_cutover`, `ensure_schema_compatible_after_idempotency_cutover`; `migrations_and_transactional_enqueue` | Empty startup is rejected before initialization; upstream/application migrations share `_sqlx_migrations`; checksums/history survive repeated startup |
| Initialized-schema upgrade | Same public migration/check entrypoints; `initialized_schema_upgrade` | Fixture through upstream version `202608240002` is rejected for missing `202609050001`, then upgrades; application row 42 and history survive repeated startup |
| Transactional enqueue | `enqueue_job_with_outcome_tx`; `migrations_and_transactional_enqueue` | READ COMMITTED is set/read back. Application insert and enqueue both disappear after rollback; committed retry returns Existing with the original ID; another owner gets a distinct Inserted job |
| Immutable canonical fields | Same transactional API/probe | Payload, priority, max attempts, timeout, schedule and stage changes each yield `job.idempotency_conflict`. A later stored priority change preserves the original snapshot/retry ID. REPEATABLE READ yields `job.enqueue_idempotency_unsupported_isolation` |
| Scoped startup witness | `JobCatalog::sync_definitions`, `Supervisor::builder`, `run_until_shutdown`, `shutdown_handle`; `worker_startup_witness_and_shutdown` | Continuously driven supervisor races actual handler acknowledgement of the submitted job ID against unexpected exit; awaited shutdown precedes independent SUCCEEDED readback |
| Hosted probe registry | `worker::prepare_probe_worker`; `hosted_worker_probe_registry_and_normal_drain` | Typed control payload reaches its generation-bound handler and durable SUCCEEDED state before component acknowledgement. A delivery submitted through the production command path beforehand remains PENDING with zero attempts because the registered-type claim filter omits it. Its request deadline is independent of the fresh operation context derived from the control attempt deadline. A continuously monitored session advisory lock rejects a second active probe owner for the same database; pending/leased controls are canceled before the exact witness is enqueued. Duplicate Batter registration retains an awaitable, already-stopping host, while dropping an unstarted successful registration still requests and observes native shutdown |
| Hosted witness failure | Same preparation path; `hosted_worker_witness_failure_prevents_readiness` | A PostgreSQL trigger rejects only the control job's SUCCEEDED persistence after the real handler invocation. The witness times out, no running handoff is returned, owned partial-startup cleanup runs, and the one durable attempt remains non-successful |
| Hosted drain and uncertainty | `worker::WorkerHost`, `TerminationGate`, `DependencyCleanup`; `hosted_worker_in_flight_finishes_after_drain`, `hosted_worker_dropped_owner_is_observed_before_cleanup`, `hosted_worker_timeout_skips_dependencies`, `hosted_worker_lease_loss_stops_host` | Drain requests native stop-claiming before starting the bounded external shutdown window. An acknowledged in-flight handler can complete afterward. Dropping the wrapper requests shutdown while an independent owner retains the native join; dependent cleanup waits for that observation. The advisory-lock connection is released before that observation; a one-second monitor with a two-second query bound turns session loss into retained driver failure and unproven termination. Concurrent successor preparation is generation-fenced from the stopping predecessor. A real native timeout preserves `RuntimeError::ShutdownTimeout`, leaves the gate Unproven, invokes no dependent finalizer and retains a nested `UnsafeTaskExit` report |
| Hosted retry/configuration | Same host constructor; `hosted_worker_retry_attempt_accounting`, `configured_worker_concurrency` | Controlled retry produces two handler invocations and exactly two durable attempt rows; replay creates neither. Held handlers distinguish configured concurrency 1 from 2 through the hosted path |
| Lease ownership | `empty_database`, `cleanup`, `defer_cleanup`, Drop, `drain_deferred_cleanup`; `lease_cleanup_defer_and_drop` | Every native pool closes before disposal. Independent `pg_database` reads confirm presence and post-drain absence. Dropping a never-polled consuming cleanup future also transfers fallback cleanup |

The upgrade fixture inspects the public `MIGRATOR` bundle and uses SQLx
`Migrate::apply("_sqlx_migrations", ...)` on a disposable connection solely to
construct the exact older schema. It never runs/undoes the raw upstream migrator
on a shared pool. Tested upgrades use the public cutover-aware entrypoint. The
application migrator deliberately ignores unrelated Runledger history while
checking its own versions/checksums. No migration was overwritten and no upstream
SQL source was copied into Batter.

## Runtime limits

Each startup witness proves schema checks, catalog sync, claim/dispatch to one
registered handler, and persisted completion for that job. It does not prove
readiness of every worker, intent-promoter, scheduler or reaper loop. `build()`
spawns loops without acknowledging initialization. Keep driving
`run_until_shutdown` to observe later task exits.

At this revision, shutdown is checked before claiming; an already in-flight
claim may return and dispatch after a stop request. No linearized stop-claim
acknowledgement exists. Cooperative worker drain joins execution tasks and
terminal observer work; normal supervisor completion joins the selected loops.
This cannot prove arbitrary detached descendants created by handlers stopped.
The caller receives only the first observed runtime failure; additional drained
failures are logged upstream and cannot be reconstructed by Batter.

The hosted driver owns a separate external shutdown trigger because requesting
the cloneable native shutdown handle alone does not start
`run_until_shutdown`'s timeout clock while its supplied future remains pending.
At process drain the host requests the native handle first, then resolves that
future. The bounded shutdown APIs allow extra abort cleanup of
`min(timeout, 1 second)`. An internal observer retains the native join and
publishes its result independently of any wrapper or startup waiter. Dependency
cleanup waits for that publication; an unsafe final Batter process report still
forces shared dependency cleanup to be skipped.

Runledger claims by static registered job type and has no instance selector at
this revision. The staged probe holds a PostgreSQL session advisory lock from
before catalog synchronization through native driver completion. A second probe
host for the same healthy database lease is rejected, so rolling overlap is not
supported by this staging contract. The dedicated session is checked every second
throughout preparation and native execution with a two-second local bound and a
session-local ten-second `idle_session_timeout`; failure requests native stop,
retains both lease and shutdown failures, and leaves transitive termination
unproven. After confirmed lease loss, a successor may acquire before predecessor
shutdown observation; its witness generation cannot be completed or terminally
failed by the predecessor and is retried after the shutdown reserve. Each such
retry consumes a durable attempt, so the control has no finite attempt budget and
the witness is bounded only by its deadline. An independent preparation owner
retains the acquired session across waiter cancellation. Release attempts finish
before native observation and record unlock and local closure separately; errors
and timeouts remain unsuccessful, unconfirmed outcomes. An unusable session skips
unlock. SQLx client close does not acknowledge remote backend exit.
While holding the lock, the next owner cancels pending or leased control rows
before enqueueing its unique witness. Session end releases the lock; this does not
fence split-brain database failover or prove that Batter remotely terminated
another session.

Non-yielding work can exceed cooperative timing assumptions. Supervisor Drop,
error or timeout is not successful transitive-stop evidence. The witness uses a
10-second native shutdown budget and one shared 14-second end-to-end reserve before its parent startup deadline,
and caps its requested 20-second acknowledgement interval to the remaining safe
window. No tracing subscriber or panic hook is installed. Upstream logging and
default panic-hook output remain outside Batter's diagnostic promises.

The temporary one-connection preparation pool closes each returned connection
before SQLx's return-to-pool ping. Pinned native catalog/cancellation/enqueue APIs
hide their checkouts, so this prevents cancelled blocked queries from parking the
application pool. It is explicitly closed before native construction. Witness
readback uses batter-sqlx PgLease and permits reuse only after acknowledged reads.
BATTER_POOL_MAX_CONNECTIONS limits the application pool: one detached control
session is additional while running, and one temporary preparation connection is
additional during initialization. These are local allocations, not a bound on
residual remote backends. Takeover fixtures reserve two overlapping control
sessions plus one preparation connection.

Pinned cancel_job directly makes a LEASED row CANCELED and preserves its original
lease expiry; it is no longer selectable as PENDING or LEASED. If a competing
terminal transition wins first, the native invalid-state error is reconciled by
reading back SUCCEEDED, CANCELED or DEAD_LETTERED. Other failures remain errors.
Cancellation does not prove an executing handler stopped.

## External harness contract

The selected harness requires PostgreSQL **major 18**, including `uuidv7()`,
CREATE/DROP DATABASE authority and a local non-TLS administrative endpoint. The
runner checks the major version and role flags; the harness performs its own
capability and operation checks. Use a disposable server.

Each harness reserves 8 downstream connection permits per lease from a budget
of 16, allowing floor(16 / 8) = 2 simultaneous leases per harness. Original
compatibility pools have a maximum of 4; their independent administrative
observer pool also has a maximum of 4. New fixture declarations are listed below. Harness lifecycle sessions
and concurrent tests additionally consume server connections. These local permit
limits do not bound server-wide or fleet usage.

Awaited cleanup holds its permit until completion. Explicit deferral completes
at queue acceptance and releases the permit before deletion finishes. Drop queues
fallback cleanup while retaining its permit through completion. A polled cleanup
waiter may be cancelled after ownership transfers to the queue; the selected
source retains eventual failures for deferred drain. That cancelled-waiter path
is inspected-only here; the executed failure/ownership extension is described below. No runtime-death or arbitrary async-drop guarantee follows.

External `shutdown()` is a no-op, including deferred cleanup. Explicitly await
`drain_deferred_cleanup()` before teardown. A lease's Drop is destructive fallback,
not retention after uncertain work termination. Pool closure cannot prove detached
server-session quiescence; these probes create no detached connections.

## Reproduction scope

[Validation](validation.md) records commands, lock hash, toolchains, PostgreSQL
version, repaired development failures and final gates. New live compatibility
evidence is Linux-only. Existing macOS evidence does not validate this new graph;
macOS and hosted execution remain unverified for this change. The package stays
unpublished. Business commands, provider effects and a
Batter-hosted Runledger adapter remain separate Beads.

## Reusable fixture acceptance

`fixture_template_reuse_and_isolation` uses stable migration/setup inputs and
retains upstream templates in one runtime. A cache hit may initialize zero times;
retained reuse adds no initializer calls. Concurrent clones receive distinct
writes, checked by independent reads. Changed SQL produces a different template
and default value 20. A second suite checks stable template identities. Repeated
runs retain two schema templates plus one stable foreign-template rejection control,
without adding per-invocation identities. Offline tests also check bundle
order/revision, connection arithmetic and report redaction with concrete causes.

`fixture_one_slot_lock_operation` observes the exact blocker/waiter relation from
an independent pool and rejects a wrong-blocker observation by timeout. Explicit
unlock and operation join precede row readback and body return. The runner retains
all acquired resources, closes each database's pools, cleans its lease, then
drains deferred cleanup. Server shutdown remains caller-owned. Catalog observers independently check
absence after the complete report. Suite initializer/catalog pools are separately
bounded to one connection each except the creation-cancellation observer, which
uses two for holding and observing its catalog lock; the lock operation declares three one-slot pools
under the eight-connection per-lease limit. Upstream sessions and independent
harnesses remain additional server usage.

The live inventory preserves the original compatibility probes and includes
fixture lifecycle and configured-constructor cases. They cover retained body error, over-budget rejection,
partial multi-pool and sibling acquisition failure, body panic, batch-capacity
rejection, held-checkout close ordering, cancelled/resumed waiting, foreign-template
rejection, simultaneous body and actual lease-cleanup failure, and observer failure
without replacement of the body report. The real cleanup-failure injection uses an
acknowledged catalog lock on a dedicated endpoint and explicitly cleans its residual
through upstream ownership after releasing the lock. The batter-kjl extension observes native-detached and adapter-retired session
identities independently of pool closure, retains the database on timeout and
observer failure, and resumes the same cleanup after explicit retry. It also
covers handled pool errors, assertion/Script failures, distinct deferred and
consuming errors, waiter loss and runtime loss. It does not terminate remote
sessions itself; tests explicitly release their blockers. Runtime loss can trigger
destructive native Drop while a checkout remains held.


The acquisition cases cancel delivery only after PostgreSQL acknowledges a
catalog-blocked native creation, for empty databases, clones and template
preparation. The report must remain pending until unlock and producer completion.
Disposable database absence is checked before upstream recovery; retained test
templates use stable inputs and are pruned through upstream tagged-resource
cleanup. Abandoned template initializer errors and panic must survive in the
report and finish native abort before recovery. A shared-harness case holds the
only native lease slot, cancels a report wait, releases that external lease and
verifies the run completes without taking server ownership. Pool-error controls
observe the failed fixture and its first connected pool while cleanup is still
pending; the low-level counterpart independently requires completed cleanup.

Use the [canonical live-runner prerequisites](testing.md#explicit-reference-compatibility-probes)
for both dedicated endpoints. That section owns the required server authority,
autovacuum settings, authentication and distinct-cluster checks. The strict runner
rejects unmet prerequisites before inventory and executes the complete case set
serially.

## Typed configuration constructor extension (batter-5pm)

The selected upstream pins remain unchanged. The reference package now owns
validated RootSettings/PoolSettings/WorkerSettings and explicit native constructor
methods. Existing worker witness and fixture pool creation consume those methods.
The foundation supplies only std-based source/bounds/redaction mechanics; direct
Axum/Batter/url/percent-encoding edges belong to this example package.

Offline configuration tests execute native field assertions, HTTP deadline and
admission behavior, partial startup failure, child-process PG*/passfile policy,
tracing redaction and an IPv6 PostgreSQL startup/password handshake. Native SQLx
option formatting remains a trusted exposure and its bare-IPv6 URL formatter
limitation is documented in [references](references.md). TLS mode preservation
is tested through native options; the graph has no SQLx TLS backend and TLS
negotiation is unverified.

The original typed-configuration change extended its then-sixteen-case inventory to
nineteen with configured pool timeout/reuse, configured worker concurrency and owned
startup closing its pool before fixture lease cleanup. All nineteen cases passed
on a disposable PostgreSQL 18.6 Docker endpoint on 2026-09-10, using Rust 1.98.1
and 1.94.0. Both runs passed the native preflight and exact execution inventory.
[Validation](validation.md#configuration-live-completion-batter-5pm-2026-09-10)
records the image, commands, results and remaining platform/TLS limits.

Those runs predated the native URL handoff corrections, including query-space,
host/database/TLS and empty-password normalization and the fixture caller changes.
At that historical checkpoint the reconciled inventory contained forty cases and
live revalidation remained pending. Subsequent forty-, 42- and 47-case suites
passed on both supported toolchains and closed that acceptance gap. The subsequent
49-case inventory passed completely on Linux/Rust 1.98.1; that follow-up had
Clippy/compile evidence only on 1.94.0. The current ownership repair expands the
live inventory to 52 (54 total runner entries), with cancellation, leased/terminal reconciliation and real-child
startup-signal cases. All 54 entries passed on Linux with both Rust 1.98.1 and
1.94.0; exact execution evidence is recorded in validation.
Passwordless Setup has separate focused evidence; the complete suite continues
to require the primary SCRAM controls.

The current runner's preflight is `examples/reference_preflight.rs`, sharing
`tests/support/live_endpoint.rs` with fixture acquisition. It reuses RootSettings
and `connect_options_from_process` and performs the version/privilege check through
native SQLx before the exact live inventory. The held-worker configuration probe
now checks committed LEASED rows while handlers are held, rather than inferring
capacity from a 150 ms quiet period. The pinned worker commits a complete claim
batch before spawning its handlers; the fixture precommits fewer eligible jobs
than its configured batch size. Live execution evidence remains separate.
