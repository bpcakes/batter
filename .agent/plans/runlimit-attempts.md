# Native outcome-aware attempt composition

Owner: `batter-mzd`. Baseline: `ac1275307b448244c4350f7174227e5b584031cb`.

## 1. Outcome

Consumers use one protected optional runner for upstream attempt admission,
credential verification and acknowledged atomic application/attempt completion.
Deliver a reviewed PR and prove an external git consumer builds with exact pins.

## 2. Scope

Update the existing adapter for current native Runlimit types; add an optional
PostgreSQL attempt runner and examples/tests/docs. Preserve `Quota`/`HttpQuota`
workflow semantics, no-refund traffic accounting and optional feature isolation.
Do not implement limiter SQL, policy algorithms, credentials or application audit
schemas in Batter. No automatic replay or production authentication enablement.

## 3. Current-state evidence

`batter-sqlx::run_atomic` owns transaction completion and retains unconfirmed
outputs in typed uncertainty. `OperationContext::run_resolved` resolves retained
outcomes before telemetry. `Quota::run` currently owns quota-before-work only.
Runlimit master has separate `CheckError`/`CheckAllError`, bound checks and an
exhaustive `Denial`; updating older adapter pins needs source changes independently
of the additive attempt feature. Runledger sibling paths currently impede a
self-contained git consumer and require explicit dependency packaging validation.

## 4. Decisions and design

### D-01 — Library-owned finalization sequence

- Status: accepted
- Context: verification alone cannot decide TOTP replay or disabled-account checks.
- Choice: bounded admission, verification outside a long-lived transaction,
  upstream receipt claim/fence inside `run_atomic`, application SQL returning a
  typed accepted/rejected outcome, upstream completion, acknowledged publication.
  A rejected authentication commits failure state/audit; infrastructure errors
  reject the transaction. A stale claim does not invoke application writes.
- Why: make the supported ordering the canonical path, with no paired reset call.
- Alternatives: preselected success before SQL checks, post-commit cleanup and
  application-owned limiter SQL violate the intended boundary.
- Revisit when: executable invalid-state review finds a missing generic primitive.

Keep normal quota semantics unchanged. Preserve full atomic outcomes through
outer deadline resolution; cancellation/uncertainty never imply safe replay.
New receipt/outcome types belong upstream; adapter types own execution, not policy.

## 5. Execution graph

### T-01 — Compatible native quota adapter

- Outcome: current native Runlimit contracts compile without changing quota workflow.
- Changes: adapter pins/types/tests, facade example and feature matrix.
- Depends on: none
- Verify: `cargo test -p batter-runlimit --all-features --locked`, quota example.
- Recovery: isolated branch; no production deployment or persistent change.
- Done when: tests and existing consumer examples pass against exact upstream pin.

### T-02 — Protected attempt runner

- Outcome: attempt retry state and application outcomes commit atomically.
- Changes: optional adapter runner, shared `batter-sqlx::run_atomic_in` retaining
  acknowledged outcomes across operation-boundary races, live tests, generic
  example, public API docs and isolated-consumer graph checks.
- Depends on: T-01
- Verify: live PostgreSQL rejection/audit, success/reset, stale claim, rollback,
  cancellation, deadline-at-ack and no-work-before-poll regressions; compile-fail
  and API review for consuming transitions and caller ordering.
- Recovery: feature opt-in; upstream owns additive schema; disable adoption only.
- Done when: typed outcomes retain all acknowledged/uncertain state and tests pass.

T-02 also requires the companion Runlimit attempt implementation; coordinate its
claim/completion seam before independent code diverges. Integrator owns packaging,
review and delivery; adapter owner owns implementation and focused evidence.

## 6. Verification

Run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`,
the HTTP process smoke from `docs/testing.md`, required Jig gates and relevant live
SQLx/attempt tests using a disposable PostgreSQL server. Verify optional features
and a fresh external consumer of exact git revisions, with no sibling path
dependency requirement. Native Codex review covers the unchanged baseline through
all fix commits. Record actual outcomes; a checked-in test is not execution evidence.

## 7. Rollout and recovery

Deliver Runlimit PR first, pin its exact head, then deliver Batter PR. Keep
Runledger/foundation type identity consistent; use explicit downstream patches
only if Cargo's source resolution requires them and test the documented graph.
No merge, publication or production migration is authorized by this delivery.
Existing consumers opt into attempts; they retain ordinary quota semantics.

## 8. Risks and open decisions

The adapter owner tests deadline/ack races and stale claim behavior. The upstream
owner supplies locking/persistence. Integrator proves git dependency portability.
Consumer policy values, alert routing and credential verification remain outside
the library. Do not disguise unresolved uncertainty as successful rollback.

## Progress

- [x] Inspected baseline and upgraded adapter to current typed quota APIs.
- [x] Protected runner and ten explicit PostgreSQL 18 live tests complete.
- [ ] Full toolchain/feature validation and external git consumer pass.
- [ ] Native review clean and PR delivered.

## Surprises & Discoveries

Final authentication outcome depends on application transaction checks; the
upstream seam must claim before application work and complete after its decision.
The same deadline/acknowledgement composition is needed by ordinary SQLx consumers,
so `run_atomic_in` owns it once in the SQLx adapter rather than duplicating it in
the attempt runner and downstream applications. Git source selection resolves the
Runledger companion without another repository change; consumers repeat the root
foundation patch because Cargo does not inherit dependency patches.

## Decision Log

2026-09-21: selected typed authentication outcome distinct from transaction error,
and retained atomic completion before operation-boundary telemetry.
2026-09-21: added the shared context-aware atomic helper and explicit Git source
patching based on downstream compilation evidence.

## Outcomes & Retrospective

Work is in progress. Focused existing quota tests passed during compatibility
adaptation; final full-tree validation and delivery remain outstanding.

Focused implementation evidence (2026-09-21, macOS arm64, Rust 1.98.1):
`cargo test -p batter-sqlx --lib` passed 140 tests, including acknowledged-result
retention, simultaneous deadline/unobserved completion, and no-acquisition
preflight. `cargo test -p batter-runlimit --all-features` passed adapter tests and
rustdocs; ten live cases were then explicitly executed with a disposable
PostgreSQL 18.6 and all passed. `cargo clippy -p batter-runlimit -p batter-sqlx
--all-features --all-targets -- -D warnings` passed. The quota-service example
compiled. These used temporary sibling Runlimit patches during coordination;
final exact-git verification remains required. ADR-010 assessment is recorded in
`crates/batter-runlimit/README.md`.

The later fourth atomic-context test advances the paused clock across the deadline
inside the same poll that returns the simulated acknowledged outcome; both success
and native error remain intact. All four focused tests pass (141 library tests
are now discovered). This distinguishes observed acknowledgement from the separate
simultaneous-but-unobserved completion test, which remains interrupted.
