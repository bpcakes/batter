# Implement the reference delivery command atomically


Owning Bead: `batter-kpd`. Beads owns scope, acceptance, priority and
dependencies; this file supplies the restartable implementation details required
by `.agent/PLANS.md`. Audited baseline: `acb5df4`, 2026-09-11. Do not commit,
publish or deploy without explicit user authorization.

## Purpose / Big Picture


After this change the unpublished reference service can accept an authenticated
delivery command for an exact version of a generic record, persist the command
identity, persist the delivery, and enqueue its Runledger job in one PostgreSQL
transaction. An exact retry returns the same delivery; a changed retry conflicts;
the owner can reconcile an uncertain POST by the idempotency key it already had.
The service exposes only Runledger's actual durable state and does not claim that
the later provider effect succeeded.

The behavior is visible through `POST /records/{record_id}/deliveries`,
`GET /deliveries/{delivery_id}`, and
`GET /delivery-commands/{idempotency_key}`. The production binary loads the
existing typed root settings, authenticates an opaque bearer token to one trusted
owner, runs both migration histories before readiness, and uses Batter's request,
SQLx lease, startup, HTTP-serving, and shutdown ownership.

## Progress


- [x] (2026-09-11 08:21Z) Audit repository guidance, Bead dependencies, current
  worktree, Jig state, locked native APIs, migration wiring, tests and docs.
- [x] (2026-09-11 08:21Z) Establish that the task is ready: all four blocking
  Beads are closed and the promised SQLx/Runledger/Batter interfaces exist.
- [x] (2026-09-11 09:18Z) Add the forward-only application schema, domain types, validation and
  handler-free Runledger producer definition.
- [x] (2026-09-11 09:18Z) Implement the one-lease, one-transaction submission and owner-scoped query
  paths with explicit commit/rollback/uncertain connection disposition.
- [x] (2026-09-11 09:18Z) Implement the authenticated Axum router and runnable startup root using
  every applicable typed constructor.
- [x] (2026-09-11 09:18Z) Add focused ordinary and live PostgreSQL tests, including replay,
  conflicting canonical input, owner isolation, stale generation, reconciliation,
  configured deadline/pool/admission behavior and repeated startup.
- [x] (2026-09-11 09:29Z) Update contracts, usage, status, package guidance and
  validation evidence; complete both toolchain matrices, all ten HTTP smokes,
  both 42-case PostgreSQL inventories and every required Jig gate; record the
  acceptance evidence on the owning Bead and close it.

## Surprises & Discoveries


- Observation: no `batter-kpd` implementation plan existed. The only plan that
  mentioned the Bead was the completed `batter-5pm` audit record, so changing it
  would corrupt history.
  Evidence: `rg -l 'batter-kpd' .agent/plans` found only
  `.agent/plans/plan_01M261TGMBZP6MKGDPNVS3S4VK.md` and the producer plan it
  references.
- Observation: Runledger exposes a handler-free producer model through
  `runledger_core::jobs::JobSpec` and
  `runledger_postgres::jobs::upsert_job_definition_tx`. Its `JobCatalog` stores
  handlers and is therefore not the right producer-only startup seam.
  Evidence: locked revision `0f464b4...`, `runledger-core/src/jobs/spec.rs`,
  `runledger-runtime/src/catalog/types.rs`, and
  `runledger-postgres/src/jobs/queue/definitions/crud.rs`.
- Observation: `batter_sqlx::PgLease` retires a checked-out connection unless
  acknowledged success explicitly returns it to the pool. This exactly supports
  conservative timeout and commit-acknowledgement handling without a new
  transaction manager.
  Evidence: `crates/batter-sqlx/src/lib.rs`.
- Observation: inserting command identity before the record check requires the
  composite record foreign key to be deferred. An immediate constraint would
  collapse a safe absent/foreign-owner rejection into an infrastructure error
  before the application could roll back deliberately.
  Evidence: focused PostgreSQL 18 tests now exercise a foreign-owner POST and
  require `record_not_observed` with no retained command, delivery, or job row.
- Observation: the first connected Jig profile rejected the new 989-line
  delivery module at the repository's 800-line hard limit.
  Evidence: `file_budget.max_lines` named `src/delivery.rs`; separating service
  orchestration leaves 768- and 226-line modules, and the final gate passes
  without a waiver.

## Decision Log


- Decision: use one application command-identity row and one delivery row. Insert
  the owner-scoped identity first with `ON CONFLICT DO NOTHING`; an existing row
  is compared under the same transaction, while a new row proceeds to target
  locking, native enqueue, and delivery insertion. All rows disappear together
  on rollback.
  Rationale: PostgreSQL's unique `(owner_id, idempotency_key)` constraint is the
  concurrency authority. It avoids SELECT-before-INSERT races and avoids an
  application mutex or repository abstraction.
  Date/Author: 2026-09-11 / Codex.
- Decision: an exact replay returns its committed delivery without re-enqueueing.
  Store the complete immutable enqueue payload and scheduling inputs with the
  command so future code changes cannot silently alter the original request.
  Rationale: the first transaction already proves the delivery and job were
  committed together; another enqueue is unnecessary and could couple replay to
  changed defaults.
  Date/Author: 2026-09-11 / Codex.
- Decision: scope the upstream idempotency key to the trusted owner through
  `organization_id = Some(owner_id)` and namespace it from the stable delivery
  identifier. Use explicit fixed enqueue overrides and a versioned JSON payload.
  Rationale: request keys remain application-owned; Runledger receives stable,
  immutable identity without allowing a foreign owner to alias it.
  Date/Author: 2026-09-11 / Codex.
- Decision: use a small application-owned bearer-token authenticator configured
  by the typed reference root. Authentication inserts a typed owner extension;
  handlers never derive authority from a path, correlation identifier, or
  forwarded header.
  Rationale: this keeps production authorization present while leaving broader
  trusted-peer/metadata policy to `batter-in2`.
  Date/Author: 2026-09-11 / Codex.
- Decision: synchronize the delivery job definition with `JobSpec` plus native
  transactional definition upsert, not a placeholder handler or worker catalog.
  Rationale: provider execution belongs to `batter-8q8.2`, and worker hosting
  belongs to `batter-0cp`; a no-op handler would fabricate successful execution.
  Date/Author: 2026-09-11 / Codex.

## Outcomes & Retrospective


The readiness audit found no missing prerequisite or upstream API blocker. The
reference binary now serves authenticated submit and both reconciliation routes,
using one budget, one conservative `PgLease`, and one native READ COMMITTED SQLx
transaction for command identity, delivery, and Runledger enqueue. Exact replay
preserves identity, changed input conflicts, generation and owner scope are
enforced, and post-BEGIN uncertainty has an explicit key-owned query path.

Both supported toolchains passed the complete repository verification, all ten
HTTP process smokes, and the real 42-case PostgreSQL 18.6 inventory. Package
tests, four doctests, strict Python inventory controls, warning-denied Clippy,
and all five Jig targets also pass. Review corrected the command/record foreign
key to be initially deferred so the application can distinguish an absent or
foreign-owned record and acknowledge rollback, and the final file split satisfies
the source budget without a waiver. Worker hosting, provider execution, and true
concurrent/fault-injected commit-acknowledgement proof remain with their named
downstream Beads; no such success is claimed here.

## Context and Orientation


`examples/reference-service` is an unpublished application package. Its current
library exports native SQLx/Runledger type-identity probes and `src/config.rs`
exports validated request, pool, process, and worker constructors. It has no
business command or production binary. `migrations/202609090001_compatibility_probe.sql`
is already applied alongside Runledger's migrations through an intentionally
shared `_sqlx_migrations` table; never edit it. Add a later forward-only migration.

`batter::operation::OperationContext` supplies one absolute deadline and downward
cancellation. `batter_sqlx::PgLease` acquires under that context, detaches its
connection by default, and only returns it after explicit success.
`runledger_postgres::jobs::enqueue_job_with_outcome_tx` takes the same native
SQLx transaction and requires READ COMMITTED for keyed enqueue. It enforces the
canonical enqueue request and retains exact keyed job identity. The reference
command must not acquire another connection while this transaction is active.

The authoritative write set is: the owner-scoped command identity, the delivery
row, and the Runledger queue/event rows created by the native enqueue helper.
The selected record row is locked and checked for both owner and generation but
is not rewritten. There is no audit table, outbox, second queue, generic
repository, transaction retry manager, or provider effect in this task.

The application idempotency key and request payload are retained indefinitely in
this initial reference. Their limits are explicit: a nonblank UTF-8 key of at
most 128 bytes and a JSON value whose encoded form is at most 16 KiB. Canonical
comparison uses PostgreSQL `jsonb` equality, so object member order is not
meaningful. The exact owner, record identifier, expected positive generation,
and payload must match. Reusing a key under another owner creates an independent
command, while foreign-owner delivery/key lookup returns the same not-observed
shape as absence.

An uncertain outcome means the caller did not receive durable commit or rollback
acknowledgement after a transaction began. It does not mean success or rollback.
The caller must query `GET /delivery-commands/{idempotency_key}`. Absence while
the original server session may still settle also does not certify rollback.
There is no automatic submission retry.

## Plan of Work


Add a forward-only migration under `examples/reference-service/migrations` for
`reference_records`, `reference_delivery_commands`, and `reference_deliveries`.
Use UUID identities, positive generations, JSONB payloads, immutable enqueue
columns, unique owner/key identity, unique delivery/job identities, foreign keys,
and database checks matching the public bounds. Keep record preparation out of
the public API; tests and future operator setup may insert/replace generations
through verified SQL.

Add `examples/reference-service/src/delivery.rs`. Define the versioned job type
and payload, validated submit input, closed public delivery state, accepted versus
replayed disposition, owner-scoped query result, concrete domain rejections, and
concrete storage/uncertainty errors. Define one `DeliveryService` around `PgPool`.
Validation occurs before acquisition. Submission runs inside one operation
boundary, acquires one `PgLease`, begins one READ COMMITTED SQLx transaction, and
passes `&mut Transaction` to every application query and native Runledger
enqueue. On acknowledged commit, return the lease to the pool. On an acknowledged
rollback, return it only after rollback completes. On commit failure, rollback
failure, or interruption after begin, retire the lease and return uncertainty.
Interruption before begin stays an ordinary bounded infrastructure failure.

Add `examples/reference-service/src/auth.rs` and extend `src/config.rs` with the
minimum serving-only owner/token settings. Keep the token in `SecretString`,
validate its length without formatting it, and produce an authenticator rather
than exposing raw authority fields. Setup mode remains usable without auth.
Update existing configuration fixtures so new success paths include fake
credentials and all redaction tests cover their markers.

Add `examples/reference-service/src/http.rs`. Build the three routes around the
real `DeliveryService`, `RootSettings::request_policy`, and `RootSettings::bulkhead`.
Authentication must run on all business routes and insert the trusted owner.
Bound the complete request body and also validate the meaningful payload. Map
accepted/replayed, record-not-observed, stale generation, idempotency conflict,
infrastructure failure, and uncertain submission to distinct sanitized response
shapes. The reconciliation response carries the retained key path and explicitly
states that absence does not prove rollback. Map every currently locked
Runledger `JobStatus` into the closed application `DeliveryState`; unknown durable
vocabulary is an internal error, never fabricated success.

Add `examples/reference-service/src/runtime.rs` and `src/main.rs`. Parse only an
optional explicit settings-file argument; load `RootSettings::from_process`,
construct the configured supervisor, reserve pool cleanup before acquisition,
connect with the configured native options, run Runledger migration and
compatibility checks plus the application migrator, transactionally upsert the
producer job definition, build the authenticated router, bind the configured
listener, register HTTP and Unix signals, and approve readiness. Retain fixed
sanitized process diagnostics and inspect all shutdown/cleanup reports. The
runtime does not start a Runledger worker.

Add ordinary module tests for validation, canonical state mapping, authentication,
sanitized HTTP outcomes, and no-database rejection. Add a focused live case to
the existing `reference_live` inventory using its external PostgreSQL 18 fixture.
The case runs the production migration/definition setup twice, prepares records
with direct fixture SQL, exercises the real service/router, checks exact replay,
payload conflict, owner isolation, stale generation, query reconciliation and
database counts, and proves a queued delivery remains pending. Add configured
root cases that make request deadline, pool acquisition timeout, bulkhead
capacity, and process capacity observably affect the production composition.
Do not claim the later controlled concurrency, forced rollback, or true lost-ack
proof owned by `batter-8q8.1`.

Update `examples/reference-service/README.md`, `docs/integrations.md`,
`docs/testing.md`, `docs/usage.md`, `docs/status.md`, and `docs/validation.md`.
Update `docs/references.md` only if implementation reveals a new upstream semantic
not already recorded for these exact locked versions. Update the nearest AGENTS
guide when entrypoints or invariants change. The `.jig.toml` and
`.agent/jig-contract.json` scopes already include `examples/*/src/**`, tests and
migrations; edit them together only if a genuinely new source root is introduced.

## Concrete Steps


All commands run from `/home/aa/Documents/batter`.

Claim and start the work identity:

    br update batter-kpd --status in_progress --json
    scripts/jig work start --title "Implement atomic reference delivery command" \
      --body-file .agent/plans/batter-kpd.md --base acb5df4 --json

During development, run focused offline checks:

    cargo test -p batter-example-reference-service --lib --locked
    cargo test -p batter-example-reference-service --test configuration --locked
    cargo test -p batter-example-reference-service --all-targets --all-features --locked
    cargo check -p batter-example-reference-service --all-targets --all-features --locked

If externally selected disposable PostgreSQL 18 primary/observer URLs are
available, run the repository-owned preflight and exact live inventory on each
supported toolchain without printing either URL:

    RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh

Final verification is:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Rebuild the HTTP example with `RUSTUP_TOOLCHAIN=1.94.0` and repeat all five
smokes. Inspect `scripts/jig work evidence --plan-id <plan-id>` and
`scripts/jig work gates --plan-id <plan-id>`, then run
`scripts/jig work check --plan-id <plan-id>`. A current successful `api:test`
receipt and every applicable required gate must be present.

## Validation and Acceptance


The new live command case must observe one delivery and one matching Runledger job
after the first POST; exact replay must return their original identifiers without
adding rows. A changed payload under the same owner/key must return conflict and
leave both stored payloads unchanged. Another owner using the same key against
its own record must get a distinct identity, and cannot query the first owner's
delivery or key. A command naming an old generation after fixture replacement
must return stale conflict and create no command, delivery or job.

Both GET paths must return the persisted delivery identifier, owner-owned target
identity, exact target generation, retained payload, and mapped current Runledger
state. A new delivery remains `pending`; no fixture or placeholder handler may
turn it into successful execution. Missing/foreign key reconciliation must state
`not_observed` and that absence does not prove rollback.

Source review and tests must show every invariant-bearing query receives the
active transaction connection. There must be no pool query or nested acquisition
between transaction begin and commit/rollback. Acknowledged success returns the
lease to the pool; all uncertain paths retire it. No transaction is replayed.

Runtime tests must use the same `RootSettings`, router, pool and supervisor
constructors as `main`: changing request timeout, pool acquire timeout, bulkhead
capacity and process capacity must alter held-work outcomes. Production auth
must remain present in these tests; only fake token values may replace its input.

The status table must change the SQLx transactional reference row from not
implemented to the precise delivered scope. Validation records only commands
actually executed, exact counts, toolchains, PostgreSQL prerequisites, platform,
and residual limitations. Tracker closure follows only after the requirement-by-
requirement audit and successful gates.

## Idempotence and Recovery


Both migration entrypoints and the producer definition upsert are safe on repeated
startup. The new migration is additive and forward-only; never edit a historical
migration. Failed startup retains the pool cleanup reservation. Failed command
transactions explicitly roll back when possible; uncertain sessions are detached
from the pool and reconciled by owner/key. Re-running an exact committed request
does not enqueue again. No cleanup or retention deletion is added in this task.

If a live test fails, retain its fixture report and let the existing harness drive
cleanup before interpreting assertions. If endpoint prerequisites are unavailable,
record the live target as unexecuted rather than converting it to an ignored or
mocked pass. Preserve unrelated worktree and append-only Jig state.

## Interfaces and Dependencies


The final library exports documented application types and functions sufficient
for focused integration and later worker tasks: a typed trusted `OwnerId`,
validated `SubmitDelivery`, `Delivery`, closed `DeliveryState`,
`DeliveryService::{submit,get_by_id,get_by_key}`, the versioned delivery job
type/payload and producer definition, the authenticated router constructor, and
the startup/schema initialization entrypoint. Concrete error enums preserve
validation, domain, SQLx, Runledger, commit, rollback and interruption identity;
wire responses serialize only fixed safe fields.

Use the existing locked `sqlx` 0.9.0, Runledger Git revision
`0f464b4f8fb5449d8df5b9071eb7b9ec49d1b8d4`, Batter, Batter Axum, Batter SQLx,
Axum, Tokio and serde libraries. Move dependencies needed by production from
dev-dependencies to dependencies and let Cargo regenerate `Cargo.lock`; add no
ORM, DI container, auth framework, queue, transaction manager or executor
abstraction.

## Revision Note


Created after the 2026-09-11 readiness audit. It corrects the historical handoff's
implicit catalog assumption by selecting Runledger's handler-free producer API,
names the exact idempotency/generation lifecycle and transaction disposition,
and adds the missing production-root/configuration behavior and proof boundaries.
