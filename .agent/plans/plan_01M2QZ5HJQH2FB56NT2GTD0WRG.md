# Make provider-effect transitions safe by construction

This task-local ExecPlan belongs to Bead `batter-8q8.5`. It is the required
ADR-010 design assessment after another review found caller-reconstructed
provider-effect invariants. Earlier completed plans remain historical evidence.

## 1. Outcome

The current-version reference worker derives legal work from retained state and
cannot mutate provider-effect state without first acquiring the exact live
Runledger lease row. Provider acceptance has one canonical identity check for
POST and GET. Local terminal work never waits for provider admission.

Completion signals are focused transition/transport/serialization regressions,
the existing two-toolchain and live acceptance suites, synchronized contracts
and validation evidence, and a closed owning Bead with no unresolved review
finding.

## 2. Scope

In scope: `examples/reference-service` worker state API, provider protocol,
focused tests, package contract, repository contracts/status/evidence, and Bead
`batter-8q8.5`.

Non-goals: no Batter or Runledger API, migration/backfill, rolling compatibility,
schema version, external provider claim, deployment, publication, staging,
commit, or push. The reference package need only run on the current version.

Constraints: preserve the dirty working tree; retain native queue ownership and
the 24-hour fixture protocol; do not infer that a Rust token can fence a remote
effect after a database lease expires.

## 3. Current-state evidence

- Fact: pinned Runledger `d57ec6b` uses `SELECT ... FOR UPDATE` plus
  `lease_expires_at > clock_timestamp()` in
  `runledger-postgres/src/jobs/queue/lifecycle/common.rs`; heartbeat and reaping
  lock the same `job_queue` row.
- Fact: PostgreSQL row locks block concurrent writers/lockers until transaction
  end and re-evaluate the row after a wait under READ COMMITTED; official
  PostgreSQL explicit-locking documentation was checked on 2026-09-17.
- Fact: `delivery/worker/state.rs` repeats identity/status predicates but neither
  locks the job row nor checks lease expiry; `worker.rs::confirm` duplicates the
  same weaker pattern outside the state module.
- Fact: `execute_retained` acquires provider admission before its local
  `retention_active` terminal decision.
- Fact: POST accepts an `AcceptedResponse` without the retained request echo,
  while GET requires it. Reqwest 0.12.28 exposes connect-phase errors through
  `Error::is_connect`; pinned hyper-util labels these `ErrorKind::Connect`, before
  request transmission.
- Decision resolved: post-expiry application mutation is not intended. A lost
  lease must prevent every state write.
- Decision resolved: `EXHAUSTED` intentionally differs from
  `MANUAL_RESOLUTION`. The former records an ended native attempt budget and may
  retain possible acceptance; the latter records that automatic action is unsafe
  because truth expired, conflicted, or the target changed.

Inference (high confidence): repeated defects come from a procedural state API.
The caller must remember SQL fencing, state ordering, admission ordering, and
response identity independently. Adding more call-site predicates would preserve
the root cause.

## 4. Decisions and design

### D-01 — Make a live-effect transaction the only mutation capability

- Status: accepted
- Context: a lease snapshot cannot prove continued authority across an await.
- Choice: the private state module begins a transaction, locks the exact
  `job_queue` row, verifies the unexpired lease after lock acquisition, and then
  performs one legal effect transition before commit. Move confirmation into the
  same boundary. Distinguish lease loss from storage and retained-data invariant
  failures.
- Why: this matches pinned Runledger serialization and prevents a mutation API
  from being called without a live fence.
- Alternatives: repeated CTE predicates remain copyable and omissible; holding a
  database transaction across provider I/O would be unsafe and would block
  heartbeat/reaping.
- Revisit when: Runledger exposes an upstream application-transaction extension
  with equivalent exact-lease semantics.

### D-02 — Plan retained work before provider admission

- Status: accepted
- Choice: a pure exhaustive planner maps snapshot state, local retention, and
  target generation to terminal completion, manual expiry, denial, reconcile,
  or dispatch. Only reconcile/dispatch actions acquire provider capacity.
- Why: ordering becomes an enum contract with a fast transition table rather
  than caller convention.

### D-03 — Require canonical acceptance identity on both protocol paths

- Status: accepted
- Choice: accepted responses require the complete retained request echo and one
  validator is shared by POST and GET. Only reqwest connect-phase failures become
  known-not-dispatched; timeouts, body failures, and all post-connect errors stay
  indeterminate.
- Why: a 200 alone cannot establish which canonical effect was accepted, while
  a connector failure occurs before an HTTP request can be sent.

## 5. Execution graph

### T-01 — Enforce live lease authority in the state API
- Outcome: every effect mutation serializes with heartbeat/reaping and rejects an expired or reassigned claim.
- Context: D-01 and pinned Runledger source above.
- Changes: `delivery/worker/state.rs`, `delivery/worker.rs`, focused state errors/tests.
- Depends on: none
- Parallel with: none
- Verify: package tests/Clippy plus live provider scenarios and SQL/source review.
- Recovery: source-only rollback; no migration or durable representation changes.
- Done when: no effect-mutating SQL exists outside the opaque live transaction and every transaction checks exact identity plus unexpired lease after locking.

### T-02 — Make retained-state planning exhaustive
- Outcome: terminal/expired work cannot enter provider admission and legal ordering has table-driven proof.
- Context: D-02.
- Changes: `delivery/worker.rs`, `delivery/worker/tests.rs`, package contract.
- Depends on: T-01
- Parallel with: none
- Verify: exhaustive planner unit table and existing live expired/terminal/admission cases.
- Recovery: source-only rollback.
- Done when: provider admission is reachable only from planner actions that need provider I/O.

### T-03 — Unify provider identity and transport classification
- Outcome: POST and GET require the same canonical accepted effect; only connect-phase failures authorize replay.
- Context: D-03 and reqwest 0.12.28 pinned source.
- Changes: `src/provider.rs`, offline real-transport tests, protocol documentation/references.
- Depends on: T-02
- Parallel with: none
- Verify: missing/mismatched POST echo, GET parity, closed-listener connect failure, stalled/oversized response regressions.
- Recovery: source-only rollback.
- Done when: acceptance cannot be constructed without exact request identity and ambiguous failures remain uncertain.

### T-04 — Reconcile public contract and evidence
- Outcome: public provider JSON, EXHAUSTED/manual distinction, lease boundary, status, validation and Bead all match executable behavior.
- Context: repository change policy.
- Changes: reference README/AGENTS, relevant docs, serialization regression, validation, Bead/Jig records.
- Depends on: T-03
- Parallel with: none
- Verify: diff review, docs checks, Jig gates.
- Recovery: source/document edits only; append-only records remain history.
- Done when: claims are tied to executed evidence and the Bead can be closed without an open question.

Critical path: T-01 -> T-02 -> T-03 -> T-04.

## 6. Verification

Run focused package tests and strict Clippy first. Then run `bash scripts/verify.sh`
and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; rebuild and run the five HTTP
smoke profiles on each toolchain; execute the exact reference live inventory and
maintenance probe against two disposable PostgreSQL 18 clusters; finish with
`scripts/jig work check --plan-id plan_01M2QZ5HJQH2FB56NT2GTD0WRG`, evidence,
gates, diff review, and plan finish. Reuse fresh receipts only when the repository
rules' source/toolchain/environment conditions are demonstrably unchanged.

## 7. Rollout and recovery

This is unpublished current-version reference code with no migration or mixed
version obligation. The change is a direct source cutover. A failed check leaves
the Bead and plan open; do not weaken semantic tests or claim live evidence.

## 8. Risks and open decisions

- Row-lock waits consume the handler budget. Mitigation: one short transaction
  per state operation; never hold it across provider I/O.
- A lease can expire after the durable pre-dispatch marker and before POST poll.
  This cannot be eliminated for an unfenced external provider; the retained
  `RECONCILE_NEEDED` marker and stable idempotency key are the recovery contract.
- Connect classification must not expand beyond pinned reqwest/hyper connector
  errors. Timeouts remain indeterminate.
- No open decision currently blocks execution.

## Progress

- [x] 2026-09-17: Reopened Bead, inspected pinned Runledger/reqwest/PostgreSQL semantics, resolved both review questions, and recorded the ADR-010 assessment.
- [x] 2026-09-17: Implemented T-01 through T-04, including the opaque live-effect transaction, exhaustive planner, shared acceptance validator, connector classification, public JSON regression and live stale-lease scenario.
- [x] 2026-09-17: Passed focused tests/Clippy, both complete Rust matrices, all ten rebuilt HTTP smokes, and the exact 66-case PostgreSQL 18.6 inventory plus maintenance probe on both toolchains.
- [x] 2026-09-17: Refreshed the final Jig profile after the tracker-only
  change, confirmed fresh evidence with no unresolved gates, and closed Bead
  `batter-8q8.5`.

## Surprises & Discoveries

- The existing Runledger source already provides the exact serialization model
  the reference should mirror; the reference weakened it while describing its
  writes as lease-fenced.
- The first complete live matrix took 122.56 seconds because the sequential
  outcome case now includes an acknowledged database lock wait. This remains
  inside its existing 180-second watchdog; the minimum-toolchain matrix took
  99.81 seconds.

## Decision Log

- 2026-09-17: Accepted D-01 through D-03. No foundation redesign or backfill.

## Outcomes & Retrospective

Implementation, runtime acceptance, repository policy gates, and tracker
closure are complete. The root cause was the procedural state boundary, not
missing Batter/Runledger machinery. Centralizing the live row lock and
exhaustive decision plan removed caller-reconstructed fencing and ordering.
The final Jig profile reused unchanged Rust receipts, re-executed the repository
contract and file-budget checks, and reported fresh evidence with no unresolved
gates.
