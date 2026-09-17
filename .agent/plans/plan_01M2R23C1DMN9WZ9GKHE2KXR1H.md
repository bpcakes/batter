# Own provider transport and terminal projection invariants

## Outcome

Bead `batter-8q8.5` owns this current-version reference-service correction.
Delivery queries must reflect terminal native work even when the handler cannot
write again. Provider capacity ends with the response exchange; diagnostics
retain bounded causes.

## Scope

Existing staged work is preserved. No migration,
backfill, upstream redesign, commit or publication is involved.

## Evidence and design

The worker has valid early returns before effect persistence, and lease loss
forbids repairing them with another write. Both public reads must use one
validated command loader and pure queue/effect projection. A nonterminal effect
on a dead-lettered spent budget projects exhausted; other native termination
projects manual resolution. Retained acceptance and provider identity survive.

Transport methods consume the admission permit and release it before returning
to SQL callers. Send and body errors are distinct; only send errors may prove
connection failure. Closed diagnostic codes preserve error classes without
provider content. Retry delay remains a lower bound; the reference deliberately
accepts a maximum of 24 hours, using a separate policy constant. Cleartext uses
literal loopback only. Canonical request fields source effect identity.

## Tasks

### T-01 — Unify queue-aware delivery reads
- Outcome: native termination cannot leave a public retryable provider state.
- Changes: delivery loader/projection and live read regressions.
- Depends on: none
- Verify: terminal/uncertain/corrupt rows through both read paths.
- Done when: both paths agree and no repair write is required.

### T-02 — Own provider capacity and classified diagnostics
- Outcome: permits end before SQL and body errors cannot imply non-dispatch.
- Changes: provider transport, worker, configuration, focused tests.
- Depends on: T-01
- Verify: permit release, send/body/size/deadline classification, header failure,
  retry lower bounds and literal-loopback validation.
- Done when: invariants live at one transport boundary.

### T-03 — Prove integration and reconcile evidence
- Outcome: readiness and provider integration regressions pass.
- Changes: live tests, contracts, references, status, validation, Bead.
- Depends on: T-02
- Verify: both verify.sh matrices, ten HTTP smokes, both exact live inventories
  plus maintenance probes, final Jig check/evidence/gates.
- Done when: executed evidence supports claims and Bead/plan close.

## Risks and recovery

Queue and effect facts must be read in one statement. Projection must never
erase possible acceptance. There is no claim of external-effect fencing after
lease expiry. Source edits are reversible; preserve staged baseline and existing
append-only records. No open decision blocks implementation.

## Verification

Run `bash scripts/verify.sh` on Rust 1.98.1 and 1.94.0, rebuild and execute
all five HTTP smoke profiles on each, and run `bash scripts/test_reference_live.sh`
against the two disposable PostgreSQL 18.6 clusters on each. Require the exact
66-case inventory and separate maintenance probe. Finish with Jig plan
`plan_01M2R23C1DMN9WZ9GKHE2KXR1H` check/evidence/gates and diff review.

## Progress

- [x] Investigated current code, reviewer claims, and ADR-010 ownership.
- [x] Implement T-01 and T-02 with focused regressions.
- [x] Complete T-03: both Rust matrices, ten HTTP smokes, both 66-case live
  inventories and maintenance probes, and fresh passing Jig evidence.

## Surprises & Discoveries

The proposed retry-delay clamp contradicts the repository lower-bound invariant.
The provider construction error can arise from headers, not only serialization.
Canonical validated worker inputs exclude that header failure. The live outage
control must connect to the administrative database to disable connections to
its disposable target; the corrected outage/recovery test passed. An initial
Jig batch was invalidated by concurrent documentation edits; the frozen rerun
passed all targets.

## Decision Log

- 2026-09-17: Move terminal projection to the read boundary. Keep the existing
  accepted 24-hour retry policy explicit and independent of retention.

## Outcomes & Retrospective

Implementation and validation are complete. Native termination is accounted for
at the shared read boundary, while transport owns permit release and phase
classification. All scoped findings and open questions are resolved. Existing
staged work remains preserved; new changes are unstaged. No fresh-agent usability
evaluation or independent re-review of this repair is claimed.
