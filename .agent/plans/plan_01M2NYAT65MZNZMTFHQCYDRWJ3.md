# Provider outcome certainty and restart reconciliation

This is the task-local ExecPlan for Bead `batter-8q8.2`, “Demonstrate provider
outcome certainty and restart reconciliation.” The exact Git baseline is
`abbe6f5c27887db9c5cbc7b7bd1fb6dd9967b00f` on `master`. At plan creation the
only tracked worktree modification was `.beads/issues.jsonl`; it contains
tracker work from this and earlier Beads and must not be restored or rewritten
from the baseline. The unrelated open Jig plan
`plan_01M28Y2SNSB5P3S7731R5DGRQD` predates this baseline and is not owned here.

The outcome is an executable reference-service worker that owns one selected
HTTP effect protocol. It must show, with a real local HTTP server and a real
PostgreSQL/Runledger worker process, that a provider can accept an effect while
the job remains unacknowledged, the worker can die abruptly, and a restarted
ordinary worker reconciles the original stable effect identity without accepting
another effect. The implementation is application code in
`examples/reference-service`; Batter remains a native operational foundation,
Runledger remains the only queue/lease/retry owner, and no generic provider or
exactly-once claim is added.

## Progress

- [x] 2026-09-16: Read the root guide, agent map, reference-service and native
  adapter guides, current delivery/runtime/configuration code, migration and live
  runner structure, and the locked Runledger handler/deadline APIs.
- [x] 2026-09-16: Verified `batter-8q8.2` is ready, all three blockers are
  closed, and moved it to `in_progress` with `br`.
- [x] 2026-09-16: Rechecked the official Resend idempotency documentation and
  current official Rust SDK `main` at commit
  `fddc9e46538da2d8af25be3cb61121ea0238ff0d`; details are in the Decision Log.
- [x] 2026-09-16: Completed four plan-review passes for self-containment,
  dependency/order, justification, and steady state. The fourth pass produced
  only the exact setting names and authoritative-attempt clarification below.
- [x] 2026-09-16: Add selected-provider configuration and an application-owned HTTP adapter
  without exposing credentials or hiding the send boundary.
- [x] 2026-09-16: Add durable effect identity/state and the Runledger delivery handler.
- [x] 2026-09-16: Install the real handler in the ordinary runtime and approve readiness.
- [x] 2026-09-16: Add offline protocol tests and live PostgreSQL/process crash-window tests,
  including target-generation fencing and terminal outcome projections.
- [x] 2026-09-16: Update current contracts, status, references, testing/validation evidence,
  package guides, and exact Jig input inventories where required.
- [x] 2026-09-16: Run focused checks, both complete toolchain matrices, both five-profile
  HTTP smoke sets, explicit live provider/database acceptance, and final Jig
  evidence. Review the final diff before closing the Bead.

## Surprises & Discoveries

- The current production root deliberately uses an empty `JobRegistry`, calls
  `without_readiness_approval`, and has a live test proving delivery work is not
  claimed. This task is the planned cutover: that test and its documentation
  must change rather than be preserved as a still-current invariant.
- `DeliveryJobPayload` already retains delivery, owner, record, generation and
  meaningful payload, while the command transaction already establishes a
  stable delivery UUID. The provider effect identity can be derived from and
  persisted with that existing identity; a second queue or attempt reservation
  is unnecessary.
- Runledger claims and increments an attempt before invoking the handler. Its
  current handler API has no attempt-neutral defer/refund operation. Therefore
  process-local provider admission can precede all application effect-state and
  provider calls, but it cannot precede the native durable claim. An admission
  rejection needs its own failure code and documentation that it consumes the
  already-started Runledger attempt; another claim engine would violate ownership.
- `JobExecution` exposes the authoritative runtime deadline and remaining
  budget, but cancellation still drops the handler future. Durable
  `reconcile_needed` state must be committed before the HTTP request can be
  polled, so a dropped request needs no caller-owned `AtomicBool` witness.
- Resend documents keyed request replay rather than lookup by idempotency key.
  The task explicitly requires reconciliation lookup. The local reference
  protocol is therefore informed by, but not compatible with or evidence for,
  Resend. Real provider adoption remains separate work.
- The current shell initially had neither `POSTGRES_TEST_ADMIN_URL` nor
  `POSTGRES_TEST_OBSERVER_URL`, and the explicit native preflight failed before
  fixture creation as required. PostgreSQL 18.6 was already installed locally,
  so two distinct disposable SCRAM loopback clusters were started with the
  documented primary settings. The exact runner then passed all 66 cases plus
  the separate maintenance-session probe; both clusters were stopped and their
  temporary data removed afterward.
- The first live execution exposed a stale process oracle: startup interruption
  must still exit 1 with the fixed diagnostic, but an already-running production
  root completes checked signal shutdown with exit 0 and empty streams. Splitting
  those expectations made all three provider cases pass. A following full run
  exposed `/bin/true` as a non-spawning historical assumption on macOS; the
  actual process control now uses `/usr/bin/true`, which exists on both supported
  operating systems.
- The multi-phase outcome probe initially reused the generic 30-second fixture
  bound even though the locked native retry schedule itself consumes roughly 30
  seconds. It now owns a 75-second completion bound. Its admission phase runs
  last because the capacity-denied job is correctly retryable; allowing later
  workers in the same scenario to reclaim it would contaminate unrelated
  provider phases rather than test admission isolation.
- The first Jig work check exposed line-debt growth in `delivery.rs` and
  `startup_process.rs`. Moving transaction persistence into the existing
  delivery service module and the offline process contracts into a nested test
  module brought both below the 800-line hard limit; the targeted and final
  file-budget gates then passed.
- The current process-test launcher clears the child environment and centrally
  supplies serving settings. Requiring provider URL/token means that helper and
  configuration fixtures must be updated together. Crash acceptance should use
  the actual production binary, not a test-only production-mode branch.

## Decision Log

- Decision: use an application-owned `reqwest::Client` transport rather than the
  Resend SDK.
  Rationale: the current SDK waits on an internal Governor limiter at
  `Config::send` before request build and `client.execute`, so the application
  cannot place its durable dispatch-possible transition at the actual execution
  boundary. A direct client also keeps this reference tied only to the selected
  fixture protocol. Pin a Rust-1.94-compatible `reqwest` release with rustls and
  JSON only; do not enable native TLS or automatic retries.

- Decision: use a local “reference effect protocol v1” with `POST /effects` and
  `GET /effects/{idempotency_key}`.
  Rationale: POST uses `Idempotency-Key`, exact canonical request matching, and a
  24-hour retention contract. Same key/same payload returns the original effect
  ID; a changed payload returns a typed conflict. GET returns the retained
  effect and canonical request, an authoritative retained-window absence, or an
  explicit expired/unknown result. This lookup resolves a crash marker without
  blindly POSTing again.

- Decision: persist a `reference_delivery_effects` row in the original command
  transaction and backfill every existing delivery in a new forward-only
  migration.
  Rationale: it binds one provider key and canonical payload to the original
  delivery before a worker can run. It stores only application identity and
  outcome state; `job_queue`, leases, attempts and scheduling remain native.

- Decision: retain queue state and provider outcome as separate projections.
  Rationale: a job can be pending while the latest provider outcome is known
  undispatched, leased while dispatch is uncertain, succeeded after a business
  denial, or dead-lettered after attempt exhaustion. Collapsing them would lose
  distinctions required by the Bead.

- Decision: create a dedicated provider `Bulkhead` from the already validated
  `BATTER_BULKHEAD_CAPACITY` and use immediate rejection.
  Rationale: admission happens before effect transition/provider I/O, holds a
  permit through the full handler slice, and creates no unbounded semaphore
  waiter set. HTTP request admission and provider admission remain separate
  process-local uses of the same configured bound. Overload is an application
  admission failure code, not a provider response.

- Decision: reserve a small part of `JobExecution::remaining_budget` for the
  final application-state write, and use the remaining duration to construct an
  `OperationContext` for provider admission/I/O.
  Rationale: the native deadline remains authoritative; application budgeting
  cannot extend it. Timeout or cancellation after the durable dispatch-possible
  commit leaves `reconcile_needed` automatically.

- Decision: classify responses only by exact protocol status plus structured
  code/schema; never by error message text.
  Rationale: a typed rate-limit/definite-denial response can prove no acceptance
  under the selected protocol. Unknown status, malformed body, lost response,
  timeout or cancellation after dispatch stays indeterminate regardless of text.

- Decision: check authoritative owner/record/generation under database state
  before dispatch and again before confirming provider acceptance.
  Rationale: a replacement that races after the first check cannot prevent the
  already-issued old request, but the second conditional transition prevents
  applying confirmation to the replacement. The original effect identity and
  provider acceptance remain as manual-resolution evidence.

- Decision: compare the current Runledger attempt with the locked job row's
  retained maximum to make the last failure terminal and persist `exhausted`,
  rather than assuming counter semantics or relying only on a best-effort
  dead-letter callback.
  Rationale: the public handler result is the authoritative attempt outcome.
  Application projection must commit before terminal return and not depend on an
  unreportable hook write.

- Decision: crash acceptance runs a parent-owned Axum fixture and the actual
  `batter-example-reference-service` binary twice against one disposable DB.
  Rationale: the fixture remains alive across process death, can acknowledge
  acceptance before withholding the POST response, and counts POSTs, accepted
  effects and GET reconciliations independently. SQL counts native attempts.
  SIGKILL establishes the unacknowledged crash window; SIGTERM on restart checks
  ordinary shutdown after reconciliation.

- Decision: official evidence checked on 2026-09-16 consists of Resend’s
  idempotency/API error pages and official Rust SDK commit
  `fddc9e46538da2d8af25be3cb61121ea0238ff0d`.
  Rationale: docs state same-key/same-payload replay, different-payload conflict,
  concurrent-key retry and 24-hour retention. SDK `src/config.rs` lines 172–182
  put `until_ready_with_jitter`, request build and execute in one call; its
  builder has no switch making that wait application-owned. Record primary links
  and the fixture/non-adoption limit in `docs/references.md`.

## Outcomes & Retrospective

Complete. The reference application now owns a seven-state durable provider
projection, stable canonical effect identity, strict typed HTTP classification,
admission-before-application-mutation, authoritative owner/generation fencing,
lookup-before-replay reconciliation, bounded exhaustion and manual resolution.
The live crash witness observed one POST, one accepted effect, two native
attempts and one keyed reconciliation across an actual SIGKILL and ordinary
restart, followed by checked clean shutdown.

The exact PostgreSQL 18.6 runner passed all 66 required entries plus the separate
maintenance-session probe. Both Rust 1.98.1 and minimum Rust 1.94.0 complete
verification matrices passed after the live-test repairs. Both five-profile HTTP
smoke sets remain valid because no HTTP example, command, dependency, toolchain,
environment or prerequisite input changed after those executions. Final Jig
receipts and the Bead closure comment record the repository gate evidence.

The selected protocol is demonstrated only by the real local fixture. A real
provider still requires an independent primary-source audit and adapter; no
exactly-once claim, publication, deployment or hosted-CI execution follows from
this work.

## Context and orientation

`examples/reference-service/src/delivery.rs` owns the existing one-transaction
command, stable `delivery_id`, versioned `DeliveryJobPayload`, owner-scoped
queries and queue-only `DeliveryState`. Its `service.rs` owns explicit
transaction interruption/commit ambiguity. Preserve that command contract and
extend its new-delivery transaction with effect identity, not a second enqueue.

`examples/reference-service/src/runtime.rs` owns the only protected production
composition root. It registers pool cleanup, schema, health, HTTP and inert
Runledger preparation. Replace the empty registry with exactly one delivery
handler and use ordinary startup readiness approval once all components register.
No provider task, reconciliation loop or detached work is needed; the handler is
invoked only through Runledger.

`examples/reference-service/src/config.rs` and `config/` own serving settings.
Add provider parsing there. `PreparedServing` remains inert and non-cloneable;
constructing the HTTP client/bulkhead is inert, and network work begins only when
the handler is polled.

`examples/reference-service/migrations/` is forward-only SQLx history. Add a
timestamped migration; never edit the existing delivery migration. The app
migrator uses `set_ignore_missing(true)` for historical fixture schemas, but
current fresh/live tests must exercise the new migration and compatibility.

`examples/reference-service/tests/reference_live.rs`, `tests/support/`, and
`scripts/reference_live.py` own explicit PostgreSQL acceptance and exact case
inventory. The provider fixture is suite-specific and stays here. It uses real
loopback HTTP, not a transport mock. Missing PostgreSQL prerequisites still fail
the explicit live target before fixtures run.

The locked Runledger revision is
`d57ec6be61e9f00ccce373b19ca356cafe98f206`. Implement
`runledger_core::jobs::JobExecutionHandler`, register its
`into_job_handler()` adapter in `JobRegistry`, and use `JobExecution` only for
deadline/attempt information. Do not duplicate lease validation or write native
attempt rows.

## Required behavior and state machine

The provider request is a versioned struct with application effect/delivery UUID,
authenticated owner UUID, record UUID, captured positive generation, and
meaningful JSON payload. Serialize the struct directly for stable field order;
retain the same JSONB in PostgreSQL and have the fixture compare parsed JSON.
The provider key is deterministic bounded ASCII derived from `delivery_id`, never
a fresh per-attempt UUID.

Persist one of these application states (SQL spellings may differ, but the typed
projection is exhaustive):

1. `awaiting_attempt`: no provider call has become possible.
2. `retryable_undispatched`: admission or a protocol-declared refusal proved no
   acceptance; another native attempt is permitted while budget remains.
3. `reconcile_needed`: the dispatch-possible marker committed and no exact
   accepted/denied resolution was durably applied.
4. `confirmed`: response or lookup recovered the same key, canonical payload and
   provider effect ID, and the original target remains authoritative.
5. `business_denied`: authoritative target check or selected protocol produced a
   definite non-retryable denial without an accepted effect.
6. `manual_resolution`: acceptance/absence cannot safely be resolved, retention
   expired, payload conflict occurred, or acceptance belongs to a stale target.
   Preserve any known provider effect ID.
7. `exhausted`: retryable undispatched or reconciliation failure consumed the
   final native attempt. Retain whether provider acceptance remains possible.

The handler flow is:

1. Decode/validate `DeliveryJobPayload`, including job organization matching the
   owner. Invalid persisted input is terminal and does no provider I/O.
2. Build a child operation budget from native remaining work budget and acquire
   provider admission immediately. Rejection records retryable-undispatched or
   exhausted with an application admission code; it does no effect reservation
   or HTTP call.
3. Lock/load the effect row, verify key/canonical request/payload identities, and
   query authoritative `reference_records` by owner and ID. Missing/stale
   pre-dispatch targets become business-denied and return success so a normal
   denial does not drain the process.
4. Confirmed/business-denied returns idempotent success without HTTP.
   Manual/exhausted is terminal without HTTP.
5. `reconcile_needed` calls GET first. Matching retained acceptance continues to
   conditional confirmation. Authoritative retained-window absence permits one
   POST with the original key/body in the same admitted slice. Expired/unknown or
   conflicting lookup records manual resolution. Lookup transport failure keeps
   uncertainty and returns retryable or exhausted based on the native attempt.
6. Before POST, commit `reconcile_needed`, dispatch timestamp and 24-hour
   resolution deadline. Only after acknowledged commit may the adapter poll
   `client.execute`. Process death, task cancellation or timeout cannot erase
   uncertainty.
7. Typed accepted POST echoes effect identity and yields a provider effect ID.
   Declared same-key replay yields that same ID. Structured business denial
   records business-denied. Structured no-acceptance retry records
   retryable-undispatched or exhausted. Conflict/unknown/malformed/lost responses
   never authorize blind replay.
8. Confirmation re-locks and rechecks owner/generation. If unchanged, store
   confirmed/provider ID. If replaced, store manual-resolution with the original
   effect/provider identities and do not mutate the replacement.

All DB transitions need acknowledged commits before control-flow changes.
Ambiguous DB writes remain conservative. Error displays are static/sanitized;
native causes remain sources and are never printed by automatic observations.

## Plan of work

### 1. Provider settings and transport

Add `src/config/provider.rs` with purpose-specific `ProviderSettings` and required
`BATTER_PROVIDER_BASE_URL`/`BATTER_PROVIDER_TOKEN`. Accept HTTPS generally and
cleartext HTTP only for
loopback/localhost fixtures. Reject URL credentials, query/fragment ambiguity,
unsupported base paths, blank/oversized tokens and unknown reserved keys through
existing source rules. Keep Debug/Display redacted. Update configuration doctests,
fixtures and boundary/diagnostic tests. Maintenance continues ignoring known
serving keys from captured environment and rejecting them in file/overrides.

Add an internal provider module owning an inert client, versioned request and
strict response DTOs/outcome enums. Bound response bodies before JSON decoding.
Disable redirects so credentials/effects cannot move authority. Do not configure
reqwest retries. Build the request before the caller marks dispatch possible;
`execute` is the first operation after the marker.

Offline tests start a real loopback Axum fixture and prove identical replay,
mismatch conflict, declared no-acceptance, malformed/unknown classification,
lookup acceptance/absence/expiry, body bounds, redirect refusal, and credential
header behavior without formatting the token.

### 2. Durable effect identity and queries

Add a forward migration for `reference_delivery_effects`, constraints and useful
owner/key indexes. Backfill existing delivery rows with stable key/versioned
request. New submissions insert it in the original transaction. Retained-command
invariants verify the effect identity rather than accepting missing/mismatched
state.

Add typed provider outcome data to `Delivery`; extend owner-scoped query SQL to
join/validate the effect row. Keep queue state separate. Add pure vocabulary and
identity tests plus live initial-state/replay/owner-isolation checks.

### 3. Handler state machine

Create `src/delivery/worker.rs` and a smaller private state module if file budget
requires it. Implement locked Runledger execution-handler contract with explicit
SQL helpers for load/target check, mark-dispatch, known-undispatched, conditional
confirm, business denial, manual and exhausted.

Map outcomes to stable Runledger codes/sanitized messages. Apply retry lower
bounds only from validated structured fields and cap inside retention/deadline.
Timeout/cancellation/panic never become expected retryable provider failures. Add
unit tests for mapping and attempt exhaustion.

### 4. Production composition and readiness

Carry inert provider settings and bulkhead through `PreparedServing`. In
`runtime::run`, construct `DeliveryWorker` after pool/schema, register it in a
`JobRegistry`, prepare native runtime with it, and use ordinary protected startup
so readiness approves only after HTTP, health and handler-bearing native runtime
registration. Update runtime readiness tests.

Replace the old live probe asserting empty registry/permanent 503. The new probe
asserts eventual readiness, no startup control jobs, and no provider calls when
no work exists. Provider execution has focused probes.

### 5. Crash/restart and failure acceptance

Add a suite-private Axum fixture owning retained key/payload/effect rows and
counters for POST requests, newly accepted effects and GET reconciliations. Its
barrier inserts/announces acceptance before withholding the first response. It
outlives both workers and shuts down explicitly.

Extend the Unix process owner with exact provider environment injection and a
separately asserted abrupt-kill operation. Preserve bounded captures, secret
checks, process reaping and ordinary signal-shutdown checks. Add no production
test mode or witness variable.

The central crash case must:

1. initialize real schema, authoritative record, command/effect/job;
2. launch actual production binary with short validated poll/lease/reaper values;
3. observe fixture acceptance, independently assert leased/unacknowledged job and
   `reconcile_needed` app state;
4. SIGKILL/reap before releasing the held response;
5. launch the same binary with same DB/fixture;
6. wait for lease recovery/retry and terminal success;
7. assert one POST, one accepted effect, two native attempts, at least one GET,
   stable app/provider IDs, and no second acceptance;
8. SIGTERM restart and require its normal checked shutdown contract.

Add focused live cases for same-key conflict, known admission/no-acceptance,
business denial, undispatched exhaustion, lookup expiry/manual resolution,
generation replacement while accepted response is held, and an undeclared status
text negative control. Keep abrupt process acceptance isolated so graceful
completion cannot satisfy it.

### 6. Contracts and evidence

Update reference README/AGENTS, root README package/status prose,
`docs/integrations.md`, `docs/testing.md`, `docs/references.md`,
`docs/reference-compatibility.md`, `docs/status.md`, and `docs/validation.md` where
current claims change. Remove current “handler absent / empty registry / readiness
withheld” claims. Preserve history and restrict evidence to the selected protocol.

Update `scripts/reference_live.py` exact inventory/announcement. Inspect
`.jig.toml` and `.agent/jig-contract.json` exhaustive scopes for new source,
helper and migration; edit together if coverage is not recursive. Inspect
file-budget/archive fixture assumptions. Do not rewrite immutable prior outcomes.

Update the Bead for material discoveries, then close with an outcome-specific
reason and `br sync --flush-only` after all evidence. No commit/push/publish/deploy.

## Concrete steps and commands

1. Re-run `git status --short` before each edit batch and inspect overlaps.
2. Patch manifest/settings/transport; let Cargo refresh `Cargo.lock`, then run
   `cargo check -p batter-example-reference-service --all-targets --all-features --locked`.
3. Patch migration/delivery/handler/runtime; run targeted tests and
   `cargo test -p batter-example-reference-service --doc --locked`.
4. Add protocol/live tests and run their exact targets under both fixed
   toolchains once stable.
5. Run explicit live cases through the repository runner against the selected
   disposable PostgreSQL 18 endpoints. Do not independently provision databases.
6. Run fmt and clippy/focused checks before expensive matrices.
7. Run `bash scripts/verify.sh` and
   `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`.
8. On each fixed toolchain rebuild the Axum example and run the HTTP smoke in
   default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and combined
   warn/deadline modes.
9. Inspect Jig evidence/gates for plan
   `plan_01M2NYAT65MZNZMTFHQCYDRWJ3`, run final `scripts/jig work check
   --plan-id plan_01M2NYAT65MZNZMTFHQCYDRWJ3`, and require backend `api:test`.
   Use 30000 ms freshness reads if collection reports `collection_limit`.
10. Record OS/architecture, Rust/Cargo versions, PostgreSQL/provider fixture
    prerequisites, exact commands/outcomes, lock hash and limits in validation.
11. Review `git diff --check`, full diff/stat, Bead JSON, live inventory and Jig
    inputs. Only then close the Bead and finish Jig plan/session.

## Validation and acceptance mapping

- Stable identity/conflict: migration constraints, identity tests, real fixture
  POST replay/conflict, command replay live test.
- Known-undispatched vs indeterminate: adapter tests, durable transition tests,
  unknown-status control and crash-marker evidence.
- Abrupt crash: production SIGKILL with independently queried leased job/held
  response, then restart counts.
- Admission ordering: denial causes no effect reservation/provider call; docs
  state native claim-before-handler limitation and attempt consumption.
- Owner/generation fencing: conditional SQL and held-response replacement test.
- Distinct outcomes: exhaustive projection test plus live business-denial,
  retryable-undispatched, reconcile-needed and exhausted rows.
- Expiry/unknown: forced expiry, no later POST, persisted manual state, terminal
  native outcome.
- No process drain: runtime stays ready through domain outcomes; shutdown is
  requested separately.
- No broader guarantee: docs restrict evidence to local protocol and state the
  24-hour/manual boundary.

## Idempotence and recovery

Migration is forward-only and SQLx-tracked; do not manually rerun statements.
Live probes receive disposable DBs from the harness. Provider fixture state is
parent-owned and must shut down after child reaping.

If a worker dies before POST, state is awaiting/retryable or conservatively
reconcile-needed; lookup proves absence before POST. If after acceptance, lookup
returns the same effect. If marker commit acknowledgement is ambiguous, that
invocation performs no provider I/O. If confirmation commit is ambiguous,
restart reconciles the key and conditionally reapplies the same provider ID.

Failures after spawn must kill/reap children, close provider serving, close
application/diagnostic pools, and then release the DB lease. Combine body/cleanup
failures with existing test support. Never turn stuck process/cleanup into success.

## Interfaces and dependencies

- New direct dependency: pinned compatible `reqwest`, minimal rustls/JSON
  features, application package only; verify Cargo.lock on both Rust versions.
- Delivery JSON gains a provider-outcome field. Document/update HTTP tests; add
  no generic library API.
- Provider config becomes part of `ServingSettings`/`PreparedServing`;
  maintenance remains database-only.
- Handler implements locked native `JobExecutionHandler` and adapts into
  `JobRegistry`; no Batter, batter-runledger or Runledger changes are planned
  unless evidence proves a missing ownership contract. A recurring lifecycle
  defect triggers ADR-010 assessment before dependent patching.
- Protocol fixtures/process controls are test-owned and Unix-only. Add no Windows
  path or fallback.
