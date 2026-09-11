# Native reference compatibility

Owning Beads: `batter-4t6` (compatibility), `batter-4jz` (reusable fixtures), `batter-kjl` (failure retention). This API/evidence manifest accompanies the unpublished
[reference package](../examples/reference-service/README.md). Beads owns delivery
acceptance and status.

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
and role, checks an exact inventory of forty ignored cases, and invokes
`cargo test -p batter-example-reference-service --test reference_live --locked
-- --ignored`. Every named case must pass, with zero filtered or ignored cases.

| Required contract | Public API and probe | Evidence / limitation |
| --- | --- | --- |
| Native types | Example library `native_pool`, `native_transaction`, `native_connection`; native arguments to `enqueue_job_with_outcome_tx` | Compiler-checked identities and rustdoc sample. `cargo metadata --locked --format-version 1` and `cargo tree -p batter-example-reference-service --duplicates` resolve SQLx 0.9.0 only |
| Independent SQLx verification | `cargo test -p batter-example-postgres-lifecycle --test native_sqlx --locked` | Native pool, connection and transaction signatures compile; an unpolled factory opens no connection. This focused target requires neither Runledger nor the harness and provides a template for optional adapter verification |
| Fresh/repeated startup | `migrate_after_idempotency_cutover`, `ensure_schema_compatible_after_idempotency_cutover`; `migrations_and_transactional_enqueue` | Empty startup is rejected before initialization; upstream/application migrations share `_sqlx_migrations`; checksums/history survive repeated startup |
| Initialized-schema upgrade | Same public migration/check entrypoints; `initialized_schema_upgrade` | Fixture through upstream version `202608240002` is rejected for missing `202609050001`, then upgrades; application row 42 and history survive repeated startup |
| Transactional enqueue | `enqueue_job_with_outcome_tx`; `migrations_and_transactional_enqueue` | READ COMMITTED is set/read back. Application insert and enqueue both disappear after rollback; committed retry returns Existing with the original ID; another owner gets a distinct Inserted job |
| Immutable canonical fields | Same transactional API/probe | Payload, priority, max attempts, timeout, schedule and stage changes each yield `job.idempotency_conflict`. A later stored priority change preserves the original snapshot/retry ID. REPEATABLE READ yields `job.enqueue_idempotency_unsupported_isolation` |
| Scoped startup witness | `JobCatalog::sync_definitions`, `Supervisor::builder`, `run_until_shutdown`, `shutdown_handle`; `worker_startup_witness_and_shutdown` | Continuously driven supervisor races actual handler acknowledgement of the submitted job ID against unexpected exit; awaited shutdown precedes independent SUCCEEDED readback |
| Lease ownership | `empty_database`, `cleanup`, `defer_cleanup`, Drop, `drain_deferred_cleanup`; `lease_cleanup_defer_and_drop` | Every native pool closes before disposal. Independent `pg_database` reads confirm presence and post-drain absence. Dropping a never-polled consuming cleanup future also transfers fallback cleanup |

The upgrade fixture inspects the public `MIGRATOR` bundle and uses SQLx
`Migrate::apply("_sqlx_migrations", ...)` on a disposable connection solely to
construct the exact older schema. It never runs/undoes the raw upstream migrator
on a shared pool. Tested upgrades use the public cutover-aware entrypoint. The
application migrator deliberately ignores unrelated Runledger history while
checking its own versions/checksums. No migration was overwritten and no upstream
SQL source was copied into Batter.

## Runtime limits

The startup witness proves schema checks, catalog sync, claim/dispatch to one
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

The bounded shutdown APIs allow extra abort cleanup of `min(timeout, 1 second)`.
Non-yielding work can exceed cooperative timing assumptions. Supervisor Drop,
error or timeout is not successful transitive-stop evidence. The witness uses
a 10-second shutdown budget, a 20-second acknowledgement limit, and an independent
runner process bound. No tracing subscriber or panic hook is installed. Upstream
logging and default panic-hook output remain outside Batter's diagnostic promises.

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

Those runs predate the native URL handoff corrections, including query-space,
host/database/TLS and empty-password normalization and the fixture caller changes.
The reconciled inventory contains forty cases. The current handoff has offline
regression evidence; the combined live suite has not been rerun on it. `batter-5pm` remains open for current AC5/AC8 live revalidation
on both supported toolchains. Passwordless Setup needs separate focused evidence;
the complete suite requires the primary SCRAM controls. The historical runs do not establish that pending acceptance.

The current runner's preflight is `examples/reference_preflight.rs`, sharing
`tests/support/live_endpoint.rs` with fixture acquisition. It reuses RootSettings
and `connect_options_from_process` and performs the version/privilege check through
native SQLx before the exact live inventory. The held-worker configuration probe
now checks committed LEASED rows while handlers are held, rather than inferring
capacity from a 150 ms quiet period. The pinned worker commits a complete claim
batch before spawning its handlers; the fixture precommits fewer eligible jobs
than its configured batch size. Live execution evidence remains separate.
