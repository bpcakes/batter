# Preserve remote-effect certainty across resumed attempts

This task-local ExecPlan belongs to Bead `batter-8q8.5`. It corrects the
provider-effect design reviewed on 2026-09-17. The completed
`batter-8q8.2` plan remains historical evidence and is not rewritten.

## 1. Outcome

The reference worker must derive every next action from the retained provider
effect state. Restart, local capacity pressure, target replacement, slow response
bodies, and terminal redelivery must not erase uncertainty, authorize an unsafe
replay, or overwrite terminal truth.

Completion signals:

- state-transition SQL rejects illegal source states;
- `RECONCILE_NEEDED` is reconciled before generation denial or replay;
- bounded provider concurrency waits inside the native work budget and has a
  purpose-specific setting;
- response headers and bounded body streaming share one operation boundary;
- focused and live failure scenarios prove the corrected behavior;
- contracts, status, validation evidence, Jig inputs, and Bead state agree.

## 2. Scope

In scope: the unpublished reference service worker, provider transport,
application settings, provider fixtures, compatibility contracts, validation
evidence, and tracker/Jig records.

Non-goals: no Batter public API, Runledger queue/attempt semantics, provider
protocol version, database schema, deployment, publication, or exactly-once
claim changes. Do not add another queue, claim-refund protocol, or generic
provider adapter.

Constraints: preserve the user's existing working tree and index; Unix only;
ordinary tests stay database-independent; live database/provider claims require
executed evidence against the documented prerequisites.

## 3. Current-state evidence

- Fact: `delivery/worker.rs:70-146` admits before loading retained state and
  checks generation before `RECONCILE_NEEDED` reconciliation.
- Fact: `delivery/worker.rs:474-494` fences failed-attempt writes on the
  Runledger lease but not on the effect source state.
- Fact: the completed plan explicitly selected immediate rejection and reused
  `BATTER_BULKHEAD_CAPACITY`; the implementation therefore reflects a flawed
  decision rather than a transcription error.
- Fact: `provider.rs:96-139` wraps `Client::execute` in `OperationContext` but
  streams the body afterward.
- Fact: Runledger bounds active handlers with `JOBS_MAX_GLOBAL_CONCURRENCY`;
  `Bulkhead::enter(..., Admission::Wait)` is deadline/cancellation aware. Thus
  provider waiters remain bounded by the upstream active-handler bound.
- Fact: Batter already provides the required operation and admission primitives.
  The remote effect state and lookup semantics are application policy.
- Fact: validation records a completed 66-case PostgreSQL 18.6 run while several
  summaries still describe the provider revision as unexecuted.

Inference: the common root cause is an attempt-centric orchestration model. It
treats retained state as data read inside a fresh pipeline rather than as the
authority that selects legal actions and transitions. Confidence: high.

Unknown resolved during implementation: exact fixture mechanics for forcing an
admission wait to cross the work deadline without adding a test-only production
branch. If the existing provider barrier cannot do this, use a direct worker
fixture with real SQL and the existing bounded loopback provider.

## 4. Decisions and design

### D-01 — Keep the remedy application-owned

- Status: accepted
- Context: Batter enforces deadline/cancellation and bounded admission, while
  the selected remote-effect protocol owns lookup, replay, retention, and target
  generation meaning.
- Choice: repair the reference-service state machine and adapter; do not change
  the foundation or create a public provider abstraction.
- Why: moving application policy into Batter would reverse dependency ownership
  and would not encode arbitrary provider semantics.

### D-02 — State selects the attempt plan

- Status: accepted
- Choice: load and validate the retained effect first. Terminal states return
  without provider admission. Fresh/known-undispatched states may generation-
  deny before dispatch. Uncertain state must reconcile first; only an
  authoritative retained-window absence permits a generation check and replay.
- Why: uncertainty is durable truth across attempts, while the current handler
  invocation is transient.

### D-03 — Legal source states are database preconditions

- Status: accepted
- Choice: every mutating helper constrains the current effect state in SQL and
  treats a zero-row result as an invariant failure. Known-undispatched and
  uncertain failure transitions originate only from `RECONCILE_NEEDED` after
  dispatch became possible. Terminal redelivery is handled before mutation.
- Why: control-flow checks alone cannot make illegal transitions
  unrepresentable and are easy to reorder incorrectly.

### D-04 — Durable work waits on a dedicated provider bound

- Status: accepted
- Choice: add validated `BATTER_PROVIDER_CAPACITY` and use `Admission::Wait`.
  The waiting population is bounded by Runledger global handler concurrency and
  the wait consumes the existing provider work deadline. Admission interruption
  records a state-appropriate failed attempt during the reserved final-state
  interval; it never changes uncertain work into known-undispatched work.
- Alternatives: immediate rejection burns already-claimed attempts during
  ordinary local contention; requiring provider capacity to equal worker
  concurrency makes the provider bulkhead ineffective.

### D-05 — HTTP completion includes the bounded body

- Status: accepted
- Choice: `OperationContext::run` owns request execution plus bounded body
  collection and returns a buffered status/body value for pure classification.
- Why: response headers do not complete an HTTP exchange and must not consume
  the final-state reserve outside the work boundary.

## 5. Execution graph

### T-01 — Enforce state-first orchestration and legal transitions
- Outcome: resumed attempts preserve uncertainty and terminal truth.
- Changes: `src/delivery/worker.rs`; live worker fixtures.
- Depends on: none
- Verify: focused compilation; live scenarios for cross-attempt replacement,
  uncertain admission interruption, and terminal redelivery.
- Recovery: revert the cohesive worker/test change; no schema or persisted-format
  rollback is needed.
- Done when: each state has one explicit path and SQL rejects every illegal source
  transition exercised by the regressions.

### T-02 — Separate and bound provider concurrency
- Outcome: ordinary local contention waits without immediate attempt loss.
- Changes: `src/config.rs`, `src/config/provider.rs`, configuration tests,
  runtime preparation, package documentation.
- Depends on: T-01
- Verify: numeric/source-policy tests and a bounded contention scenario.
- Recovery: revert the new setting before release; no persisted state changes.
- Done when: HTTP and provider capacities are independently validated and waiters
  remain bounded by documented worker concurrency.

### T-03 — Bound the complete provider exchange
- Outcome: a stalled response body cannot escape the provider work deadline.
- Changes: `src/provider.rs` and its real loopback tests.
- Depends on: none
- Verify: a server flushes headers, withholds the body, and the call returns the
  typed deadline interruption within a test bound.
- Recovery: not needed; private adapter change with focused regression.
- Done when: all response reads occur inside `OperationContext::run`.

### T-04 — Prove cumulative failure behavior
- Outcome: independent scenarios falsify the reviewed regressions and collateral
  terminal-state damage.
- Changes: `tests/support/provider_effects.rs`, provider fixture support,
  `tests/reference_live.rs`, and exact runner inventory only if a new case is
  added rather than extending the existing matrix.
- Depends on: T-01, T-02, T-03
- Verify: focused ordinary tests plus exact live PostgreSQL/provider acceptance.
- Recovery: fixtures are disposable; failures retain existing diagnostic owners.
- Done when: the original scenarios fail against the old ordering and pass with
  state-first orchestration.

### T-05 — Reconcile contracts and evidence
- Outcome: package guidance and validation claims describe the corrected design
  and only executed results.
- Changes: nearest `AGENTS.md`, README, integrations/testing/status/compatibility/
  validation docs, Jig exhaustive scopes if new files are added, and Bead notes.
- Depends on: T-04
- Verify: contract/Jig checks and manual diff audit for contradictory claims.
- Recovery: retain explicit unverified labels for any unavailable live prerequisite.
- Done when: one consistent evidence statement remains and the owning Bead links
  exact receipts.

### T-06 — Complete repository verification
- Outcome: the corrected cumulative working tree satisfies repository gates.
- Changes: validation evidence and generated Jig/Beads state only.
- Depends on: T-05
- Verify: focused package checks, both `scripts/verify.sh` toolchains, rebuilt
  five-profile HTTP smoke suite, exact live suite when prerequisites are present,
  `scripts/jig work check`, evidence, gates, and finish.
- Recovery: investigate failures without weakening semantic tests or overwriting
  user staging.
- Done when: required receipts are fresh for the final inputs and remaining
  environmental limitations are explicit.

Critical path: T-01 -> T-02 -> T-04 -> T-05 -> T-06. T-03 can proceed
independently before T-04, but this single executor will avoid overlapping edits.

## 6. Verification and recovery

No forward migration or data rewrite is required. This changes how future
attempts interpret already-retained states; existing `RECONCILE_NEEDED` rows
become safer because they reconcile before any denial or replay. If live
prerequisites are absent, stop short of closing the Bead and label the live
acceptance unverified. Publication and deployment remain separate user decisions.

## 7. Risks and open decisions

- Risk: adding provider waiting could hide an unbounded queue. Mitigation: the
  only callers are Runledger handlers, whose active population is bounded by
  validated global concurrency; document and test this composition.
- Risk: an SQL guard can reject a legitimate transition omitted from the design.
  Mitigation: enumerate all seven state paths and exercise both success and
  zero-row invariant behavior.
- Risk: fixture-only confidence misses real body streaming. Mitigation: use a
  real loopback HTTP server that flushes headers before stalling the body.
