# Native reference compatibility

The announcement propagation regression (`batter-x36`) adds typed startup/cleanup
and actual-executable missing-receiver checks to the production-root live case.
No native dependency, migration, production behavior or 66-case inventory change
is involved.

The current-version provider follow-up (`batter-gg8`) adds the forward migration
`202609170001_provider_retry_eligibility.sql`. Outcome persistence owns durable
retry eligibility; native Runledger still owns attempts and scheduling. The
private lease transaction validates after lock acquisition and before commit.
The live runner now also requires its direct SQL boundary library probe after
the main inventory and maintenance-session case. This adds no backfill or
mixed-version reference obligation.

The subsequent boundary follow-up (`batter-8ou`) constrains both terminal writers
to unresolved source states before SQL acquisition. The live state probe also
checks stale-source rejection against terminal and mismatched unresolved rows,
including complete retained-row equality. The production readiness scenario owns
its phase-derived fixture budget. Native dependencies, migrations and the 66-case
inventory are unchanged.

Owning Beads: `batter-4t6` (compatibility), `batter-4jz` (reusable fixtures),
`batter-kjl` (failure retention), and `batter-in2` (trusted request metadata).
This API/evidence manifest accompanies the unpublished
[reference package](../examples/reference-service/README.md). Beads owns delivery
acceptance and status. The current native lifecycle and retirement redesign is owned by `batter-gi4`;
`batter-0cp` records the superseded example-owned host.

## Selected graph

The current coordinated PR graph is `batter-runledger -> runledger-postgres ->
batter-sqlx -> batter-core`, using sibling path dependencies. CI selects immutable
companion revisions in each repository's workflows; the adapter README records
the Runledger revision. Foundation packages remain unpublished. The canonical
transaction API is `run_atomic`, with intent-before-queue typestate, retained
poison causes and a distinct `PgAtomicUncertainty`. Public native transaction
views and the old session transaction wrapper have been removed.

The latest local validation used PostgreSQL 18.6, both supported Rust toolchains,
the 84-case SQLx suite, the reference inventory and adapter probe. The
[fresh consumer exercise](evidence/atomic-consumer-2026-09-21/README.md) records
its exact compile-only scope. Older evidence below retains its recorded source
scope; it does not validate the current graph by itself.

### Historical source selections

On 2026-09-20, `batter-44w` advanced the root Runledger graph from
`638ee3480f69962597147f5d7bd52822267560b7` to PR
[#19](https://github.com/bpcakes/runledger/pull/19), immutable commit
`c541dad69fcb6c03b39541084538681b2d710a32`. This includes opaque durable-intent and schema
capabilities plus the intervening native settlement and unconfirmed-commit
hard cuts. All three native packages use this exact pin; no path override is
required. The archived `batter-gi4` consumer remains historical evidence at its
recorded pin, not validation of the new graph.
The initial PR implementation at Runledger `969e86b` passed behavioral and CI
checks but failed construction-invariant review: arbitrary executor implementations
could claim transaction or session identity. That historical revision sealed transaction
execution to native resources and private-representation views, and schema checks
consume one retained session view. External compile-fail tests cover newtypes,
routing executors, forgery and resource replacement. Expanded PostgreSQL parity
and owning-adapter cancellation checks pass locally. Full verification passes on
Rust 1.98.1 and 1.94.0; the full PostgreSQL suite and all five HTTP smokes pass.
The bridge also passes repeated runs against one database on both toolchains,
with cleanup verified after each run. All five final Jig targets pass.
Hosted evidence for the repaired commits is tracked in the linked PRs.

Earlier paired-checkout validation retains its historical scope. The optional `batter-runledger` adapter
selects the native runtime. The foundation remains independent of it. SQLx and its
optional test-support harness retain one native type graph.

| Source | Selected version/revision | Features and boundary | Disposition |
| --- | --- | --- | --- |
| SQLx registry | 0.9.0 | `runtime-tokio`, `postgres`, `uuid`, `chrono`, `json`, `migrate`, `macros`; one resolved SQLx/core/PostgreSQL version | Compiled on Rust 1.98.1 and 1.94.0; live transactions executed on Linux |
| Runledger historical Git | core/postgres/runtime 0.12.0 at `c541dad69fcb6c03b39541084538681b2d710a32` (PR #19) | Superseded native transaction/session views; native SQLx types, inert preparation, initialization observation and consuming settlement | Historical evidence only; current coordinated graph uses sibling sources and workflow companion pins |
| postgres-test-harness Git | 0.2.0 at `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` | `default-features = false`; external PostgreSQL through tokio-postgres; optional SQLx test-support dependency; reference development dependency | Compiled on both toolchains; external lease cleanup paths executed |
| reqwest registry | 0.12.28 | Application-only provider transport with `json` and `rustls-tls-webpki-roots`; defaults disabled, redirects disabled at construction, no proxy discovery or automatic replay | Compiled on Rust 1.98.1 and minimum Rust 1.94.0; the selected protocol executed through the real loopback fixture and production worker on both toolchains |
| Runledger registry | 0.12.0 | Downloaded manifest requires SQLx 0.8.6 and Rust 1.88 | Inspected-only; incompatible with the selected native SQLx 0.9 type identity |

Identical Runledger version numbers do not imply identical registry and Git
sources. Reproduce with the full revision and Cargo.lock. Runledger enables SQLx
defaults internally; optional/target-specific entries in Cargo.lock do not add
other database backends or non-Unix platforms to Batter's support policy.

## Executable contracts

Set `POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL` to two distinct
disposable local PostgreSQL 18 clusters, using the primary superuser/SCRAM/autovacuum
prerequisites in [testing](testing.md#explicit-reference-compatibility-probes), and run `bash scripts/test_reference_live.sh` from the root. It preflights the server
and role, builds the production and signal-fixture executables with the invoking toolchain, checks an
exact inventory of 66 entries (61 live database probes, two offline
synthetic-acquisition signal controls, two offline executable-composition signal
controls and the private child entry), and invokes
`cargo test -p batter-example-reference-service --test reference_live --locked
-- --include-ignored`. Every named entry must pass, with zero filtered or ignored cases. The runner also
requires the separate library physical-session replacement probe. The earlier
58-entry inventory and the separate library probe passed on Linux with Rust 1.98.1
and exact 1.94.0. After the request-metadata harness repair, the complete current
64-entry run plus the separate library probe passed on macOS arm64 against two
task-owned PostgreSQL 18.4 containers with Rust 1.98.1 and exact 1.94.0 on
2026-09-16. The expanded 66-entry baseline plus the separate library probe then
passed on macOS arm64 against two distinct disposable PostgreSQL 18.6 clusters
with Rust 1.98.1, including both added provider-effect cases. The current
state-first provider assertions retain that inventory and passed on the same
PostgreSQL version with both supported Rust toolchains. The eight `batter-lp2.4`
protected-startup rows are listed in
[testing](testing.md#protected-startup-consumer-process-cases).

| Required contract | Public API and probe | Evidence / limitation |
| --- | --- | --- |
| Native types | Example library native pool/connection signatures; canonical `run_atomic` with scoped SQLx execution | Compiler-checked identities and rustdoc sample. `cargo metadata --locked --format-version 1` and `cargo tree -p batter-example-reference-service --duplicates` resolve SQLx 0.9.0 only |
| Independent SQLx verification | `cargo test -p batter-example-postgres-lifecycle --test native_sqlx --locked` | Native pool, connection and transaction signatures compile; an unpolled factory opens no connection. This focused target requires neither Runledger nor the harness and provides a template for optional adapter verification |
| Fresh/repeated startup | `migrate_after_idempotency_cutover`, `ensure_schema_compatible_after_idempotency_cutover`; `migrations_and_transactional_enqueue` | Empty startup is rejected before initialization; upstream/application migrations share `_sqlx_migrations`; checksums/history survive repeated startup |
| Initialized-schema upgrade | Same public migration/check entrypoints; `initialized_schema_upgrade` | Fixture through upstream version `202608240002` is rejected for missing `202609050001`, then upgrades; the application owner-epoch sequence advances and application row 42/history survive repeated startup |
| Transactional enqueue | `run_atomic`, consuming intent-to-queue transition; `migrations_and_transactional_enqueue` | Application writes and enqueue share the owned transaction. Rejected work rolls back; successful output is withheld until commit acknowledgement. Retry returns Existing with the original ID; another owner gets a distinct Inserted job |
| Immutable canonical fields | Same transactional API/probe | Payload, priority, max attempts, timeout, schedule and stage changes each yield `job.idempotency_conflict`. A later stored priority change preserves the original snapshot/retry ID. REPEATABLE READ yields `job.enqueue_idempotency_unsupported_isolation` |
| Isolated durable execution | `isolated_durable_execution_and_shutdown` | Actual typed handler invocation and independent persisted success after awaited shutdown; test-only proof, separate from production initialization |
| Native initialization | `native_initialization_without_queue_writes` | Local loop acknowledgement succeeds while job-queue writes are blocked; application approval remains independent |
| Managed native ownership | `native_in_flight_finishes_after_drain`, `native_owner_drop_retains_settlement`, `native_unjoined_callback_blocks_dependency_cleanup` | Real admitted work completes after drain; wrapper loss retains settlement; a callback held beyond bounded report publication remains unjoined and prevents dependent cleanup |
| Business outcomes and configuration | `native_business_failure_preserves_process`, `configured_worker_concurrency` | Durable business failure does not become process failure; held native handlers exercise configured concurrency |
| Production readiness | `production_root_registers_provider_worker` | Actual production composition binds port zero in each child, reports the selected loopback address through an explicitly configured parent-owned Unix datagram receiver, reaches readiness, executes and confirms provider-backed work, creates no control jobs and sequentially awaits SIGTERM and SIGINT cleanup with empty stdout/stderr |
| Confirmation/replacement serialization | `provider_state_lock_and_retry_boundaries` | Witnessed record-first and confirmation-first lock orders, retained acceptance/manual-resolution identity, expiry rollback during the record wait and heartbeat job-row availability; focused PostgreSQL18.6 pass |
| Provider crash reconciliation | `provider_effect_crash_and_restart` | Parent-owned real HTTP fixture accepts behind a withheld response; the real production binary is SIGKILLed while its native job remains leased and the application row is `reconcile_needed`, then ordinary restart resolves through GET. The repeated crash changes generation before restart and requires accepted truth to become manual resolution. The complete scenario passed on PostgreSQL 18.6 with both supported Rust toolchains. |
| Provider outcome boundaries | `provider_effect_outcome_contracts` | Same-key replay/mismatch, exact POST/GET canonical acceptance identity, connector-only known non-dispatch, business denial, opaque-text lookup-before-replay, fresh deadline renewal after authoritative absence, exhaustion, accepted generation replacement, retention expiry/manual resolution, bounded provider waiting, uncertain admission interruption, terminal redelivery without provider admission, and stale lease revocation at the job-row fence share one real HTTP fixture. Prior state-first evidence passed on both supported toolchains. |
| Request metadata boundary | `http::in_process_client`, `InProcessRequestClient`, `TrustedPeerPolicy`, `TrustedRequestMetadata`, `BearerAuthenticator`; ten ordinary `http::tests` cases plus the adapter's real-socket peer oracle | The lower-level client cannot be served or expose its router and requires an explicit synthetic peer for every request. The adapter compares the native server peer with the client's independently observed socket address; the application retains only its IP. It consumes shared server `CorrelationId`, replaces prior owner extensions through bearer authentication and keeps `OperationContext` separate. Forged metadata, concurrency, missing peer data, nested operations, Debug redaction and forced cancellation require no PostgreSQL; an explicit three-second test guard is tighter than the test operation budget. Production authentication and domain bodies exclude the selected peer. A bare liveness request proves probes remain outside the peer boundary; malformed JSON, invalid UUID paths and body-limit overflow prove all documented native extractor rejections remain outside the application problem envelope |
| Production request-metadata registration | Application-owned `http::register_in`; ordinary `canonical_registration_supplies_native_peer_to_the_business_boundary`; live `production_root_registers_provider_worker` | The canonical operation constructs the trusted-peer router and selects native peer registration together. The ordinary real-socket case requires a matched business route to return authenticated-boundary 401 rather than missing-peer 500 without PostgreSQL. The live production child repeats that assertion in the complete root and requires matching generated header/body identity |
| Offline retirement | Seven `retirement_*` cases plus the required library session-replacement probe | Preserves terminal/domain rows, migrations and sequence; old additive catalog retains disable; rejects wrong identity, hidden sessions, pending/prepared enqueue and physical replacement. Native commit failure survives cancelled reconciliation and an actual lost COMMIT response |
| Lease ownership | `empty_database`, `cleanup`, `defer_cleanup`, Drop, `drain_deferred_cleanup`; `lease_cleanup_defer_and_drop` | Every native pool closes before disposal. Independent `pg_database` reads confirm presence and post-drain absence. Dropping a never-polled consuming cleanup future also transfers fallback cleanup |

The upgrade fixture inspects the public `MIGRATOR` bundle and uses SQLx
`Migrate::apply("_sqlx_migrations", ...)` on a disposable connection solely to
construct the exact older schema. It never runs/undoes the raw upstream migrator
on a shared pool. Tested upgrades use the public cutover-aware entrypoint. The
application migrator deliberately ignores unrelated Runledger history while
checking its own versions/checksums. No migration was overwritten and no upstream
SQL source was copied into Batter.

## Runtime limits

Canonical registration consumes native `PreparedSupervisor`, which validates and
owns configuration without launching work. The adapter starts it after Batter
registration validation, acknowledges each enabled loop's local initialization,
and retains native settlement independently of the direct component waiter.
Initialization proves neither fresh database health nor durable job execution.
The terminal-projection follow-up uses one command loader for both public read
routes and exact replay, deriving unresolved termination from the same native
job/effect snapshot without altering retained acceptance facts. Its live command
probe covers exhausted, cancelled and early-terminal jobs and missing-effect
corruption. The final-attempt lease-loss probe preserves the uncertain stored
effect while both reads project exhaustion. Production readiness is additionally
tested after the freshness interval and through a disposable database outage
and recovery.

Production readiness additionally requires fresh dependency observations and
explicit application approval. The installed delivery handler permits ordinary
approval after registration. Production startup has no control job, advisory lease, owner epoch
allocation or reconciliation driver.

Runledger owns direct loops and its tracked descendants. Complete reports retain
first cause, later failures, abort requests and unresolved work. Batter freezes
that evidence at its reporting deadline and skips dependent cleanup when native
settlement is not cooperative; late observation cannot retroactively run cleanup.
The earliest native or parent stop timestamp anchors graceful and abort deadlines;
later requests can tighten active waits. Cleanup has its own explicit allowance.
Existing native `build()` still launches immediately and legacy Result-returning
drive methods remain available outside the protected adapter path.

An in-flight claim can still dispatch after stop. No linearized no-claim barrier,
arbitrary-handler-spawn join, remote backend termination or runtime-death cleanup
guarantee is claimed. Non-yielding futures can outlast the reporting window.
Native/default panic-hook output remains outside Batter's diagnostic promises.

Retirement is an explicit offline command, after external revocation of old
producer restarts. It checks expected cluster/database identity, other sessions
(including hidden backend types) and prepared transactions, disables the exact
legacy definition through native policy, then cancels its pending/leased global
controls. A replacement physical connection cannot continue mutation. Completion
preserves terminal/domain history, applied migrations and the epoch sequence.
An absent definition is reported as absent, not a disabled tombstone.

Cancellation cannot prove a handler stopped. SQLx mutation/commit causes and
secondary rollback errors remain retained; an ambiguous COMMIT stays failed even
when separately verified read-only reconciliation observes CANCELED. Readback is
another owned finite command and cannot overwrite or erase the primary report.
Database snapshots do not establish deployment restart revocation.

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

Live compatibility
evidence for the current state-first provider correction is macOS arm64 on Rust
1.98.1 and exact 1.94.0 against PostgreSQL 18.6; hosted CI and other hosts are
unverified for that repair. The package stays unpublished. The producer command,
optional native adapter and selected local provider-effect protocol are
implemented. The 66-entry suite passed on both supported toolchains. It makes no
claim about a real external provider or effects outside the 24-hour fixture contract.
Complete redesign acceptance remains open.

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

## Purpose-qualified configuration and native preparation

The selected upstream pins remain unchanged. The reference package now owns
distinct validated `ServingSettings` and `MaintenanceSettings`, section-level
`PoolSettings`/`WorkerSettings`, and inert prepared ownership values. The serving
schema requires a password-qualified endpoint, concrete authentication, a native
validated worker configuration and Batter/Axum operational witnesses. Serving
preparation pins the code-selected direct-peer policy as a separate typed input;
it is not an environment-selectable setting. The
database-only maintenance schema permits passwordless local trust authentication,
ignores known serving-only values in captured environment without parsing them,
but rejects those keys in dedicated inputs and cannot be converted into serving. Native runtime
preparation and fixture pool creation consume their corresponding purpose types.
The foundation supplies only source/bounds/redaction mechanics and reusable
operational witnesses; direct Axum/Batter/url/percent-encoding policy remains in
this application package.

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


Those runs predated the native URL handoff corrections, including query-space,
host/database/TLS and empty-password normalization and the fixture caller changes.
At that historical checkpoint the reconciled inventory contained forty cases and
live revalidation remained pending. Subsequent forty-, 42- and 47-case suites
passed on both supported toolchains and closed that acceptance gap. The subsequent
49-case inventory passed completely on Linux/Rust 1.98.1; that follow-up had
Clippy/compile evidence only on 1.94.0. The preparation-ownership repair expanded
the live inventory to 52 (54 total runner entries), with cancellation,
leased/terminal reconciliation and real-child startup-signal cases. The
late-commit repair added one exact-backend reconciliation case, producing 53 live
cases and 55 total runner entries. The review repair adds the blocked-reconciliation
lease-monitor case, producing 54 live cases and 56 total entries. Those hosted
protocol cases were later replaced by native lifecycle and offline retirement
acceptance under `batter-gi4`; current inventory and execution are recorded above.
Passwordless maintenance has separate focused evidence; the complete suite continues
to require the primary SCRAM controls.

The current runner's preflight is `examples/reference_preflight.rs`, sharing
`tests/support/live_endpoint.rs` with fixture acquisition. It reuses
`MaintenanceSettings::prepare` and performs the version/privilege check through
native SQLx before the exact live inventory. The held-worker configuration probe
now checks committed LEASED rows while handlers are held, rather than inferring
capacity from a 150 ms quiet period. The pinned worker commits a complete claim
batch before spawning its handlers; the fixture precommits fewer eligible jobs
than its configured batch size. Live execution evidence remains separate.
