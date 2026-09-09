# Native reference compatibility

Owning Bead: `batter-4t6`. This API/evidence manifest accompanies the unpublished
[reference package](../examples/reference-service/README.md). Beads owns delivery
acceptance and status.

## Selected graph

Selection date: 2026-09-09. Both Git revisions were verified against the remotes;
Cargo fetched them and generated the lockfile. No dependency uses an absolute
local path. The four libraries have no new Runledger or harness dependency. The
separately selected `batter-sqlx` adapter uses the same SQLx 0.9 graph; its
connection-disposition contracts are exercised by its own live suite, not by
these reference probes.

| Source | Selected version/revision | Features and boundary | Disposition |
| --- | --- | --- | --- |
| SQLx registry | 0.9.0 | `runtime-tokio`, `postgres`, `uuid`, `chrono`, `json`, `migrate`, `macros`; one resolved SQLx/core/PostgreSQL version | Compiled on Rust 1.98.1 and 1.94.0; live transactions executed on Linux |
| Runledger Git | core/postgres/runtime 0.12.0 at `0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4` | Native `DbPool = PgPool`, `DbTx = Transaction<Postgres>` | Compiled on both toolchains; migration, enqueue and controlled worker path executed |
| postgres-test-harness Git | 0.2.0 at `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` | `default-features = false`; external PostgreSQL through tokio-postgres; development dependency only | Compiled on both toolchains; external lease cleanup paths executed |
| Runledger registry | 0.12.0 | Downloaded manifest requires SQLx 0.8.6 and Rust 1.88 | Inspected-only; incompatible with the selected native SQLx 0.9 type identity |

Identical Runledger version numbers do not imply identical registry and Git
sources. Reproduce with the full revision and Cargo.lock. Runledger enables SQLx
defaults internally; optional/target-specific entries in Cargo.lock do not add
other database backends or non-Unix platforms to Batter's support policy.

## Executable contracts

Set `POSTGRES_TEST_ADMIN_URL` to a disposable local PostgreSQL 18 endpoint and
run `bash scripts/test_reference_live.sh` from the root. It preflights the server
and role, checks an exact inventory of four ignored cases, and invokes
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
of 16. Each application pool has a maximum of 4; where present, the independent
administrative observer pool also has a maximum of 4. Harness lifecycle sessions
and concurrent tests additionally consume server connections. These local permit
limits do not bound server-wide or fleet usage.

Awaited cleanup holds its permit until completion. Explicit deferral completes
at queue acceptance and releases the permit before deletion finishes. Drop queues
fallback cleanup while retaining its permit through completion. A polled cleanup
waiter may be cancelled after ownership transfers to the queue; the selected
source retains eventual failures for deferred drain. That cancelled-waiter path
is inspected-only here; broader failure/cancellation fixture delivery belongs to
`batter-kjl`. No runtime-death or arbitrary async-drop guarantee follows.

External `shutdown()` is a no-op, including deferred cleanup. Explicitly await
`drain_deferred_cleanup()` before teardown. A lease's Drop is destructive fallback,
not retention after uncertain work termination. Pool closure cannot prove detached
server-session quiescence; these probes create no detached connections.

## Reproduction scope

[Validation](validation.md) records commands, lock hash, toolchains, PostgreSQL
version, repaired development failures and final gates. New live compatibility
evidence is Linux-only. Existing macOS evidence does not validate this new graph;
macOS and hosted execution remain unverified for this change. The package stays
unpublished. Business commands, provider effects, reusable fixtures and a
Batter-hosted Runledger adapter remain separate Beads.
