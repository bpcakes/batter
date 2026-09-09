# Evidence-grounded design adjustment

This is a planning-session execution record, not an implementation ExecPlan or
a second backlog. The user authorized re-auditing commits through 22848ea and
adjusting intended design directly in Beads. No planning/review/conversion Bead
will be created. Existing delivery Beads and their epics own the resulting work.

## Progress

- [x] Capture clean Git baseline, current source/contracts and live tracker state.
- [x] Re-audit core ownership and HTTP observation against the historical sample.
- [x] Review the proposed self-contained delivery graph through four rounds.
- [x] Apply reviewed acceptance, priority and dependency changes using br.
- [x] Check the resulting graph and source documentation.
- [x] Finish applicable Jig gates and evidence recording.

## Surprises & Discoveries

The initial read with automatic import disabled exposed a local database behind
the committed export. A normal br read imported two completed issues; 43 issues
are now the baseline. No completed delivery will be reopened by this adjustment.
The reconciliation document still describes an absent HTTP panic test, although
the new adapter suite establishes unwinding propagation without recovery.

The first tracker application stopped after creating its two blocked delivery
tasks and removing the obsolete config dependency because an epic omitted the
optional dependencies field. The scratch orchestration was corrected and resumed
with the existing IDs and baseline guards; it created no duplicate tasks. Final
tracker readback matches every reviewed field and edge. The viewer selected the
live database after flush, so export consistency was checked independently with
`br --no-db list --all --deferred --json` rather than assuming its source path.

## Decision Log

Keep native execution mechanics and concrete errors. Strengthen the application
reference and its failure oracles; preserve completed lifecycle/HTTP hardening.
Beads remains the only authority for future delivery scope and dependencies.
Scratch proposals and review iterations stay outside the tracked workspace.

Configuration and contract validation were promoted from P2 to P1. Configuration
now supplies tested native constructors before command/worker consumers. The
reference gets separate command-side database and provider-restart proofs; final
acceptance consumes those proofs and the generated contract. Optional retry
extensions stay independent. The minimal existing SQLx lifecycle demonstration
keeps its purpose; new composition is owned by an unpublished reference package.

The first review corrected a false descendant-detection implication: a broken
wrapper can return successfully while hidden work survives. The remaining
component task must demonstrate that limit and a conforming component's actual
termination, not invent detection by the supervisor.

## Planning review evidence

Four sequential fresh-context reasoning-model reviews inspected the complete
proposed task graph. Separate fresh-context checks read one obscure task alone
after each round: provider protocol, config constructors, command database proof,
and optional metrics. All four standalone checks found the tasks implementable
after their declared prerequisites without further product decisions.

| Round | Material result | Disposition |
| --- | --- | --- |
| 1 | Correct hidden-descendant non-detection; define owner-scoped command/effect identity; connect provider producers to dependent metadata/admission tests. Tighten abrupt-restart oracle and remove stale reverse metadata. | Integrated before round 2. Also retargeted the two affected epic entrypoints to the new reference package. |
| 2 | No structural or minor changes required. | Verified round 1 corrections and dependency reversal ordering. |
| 3 | No structural or minor changes required. | Rechecked complete path, failure oracles and preserved ownership limits. |
| 4 | No structural or minor changes required; steady state. | Same delivery content as rounds 2/3; only review counters/headings changed. |

Each round checked self-containment, the blocking DAG, five sampled rationale
paragraphs and changes since the previous round. The scratch review/proposal
artifacts are under `/tmp/batter-design-adjustment-20260909/`; they are historical
session evidence, not an implementation prerequisite or another backlog. Review
did not select new upstream versions or claim that future integrations compile.

Six post-conversion graph passes checked: node/completed-work preservation;
blocking cycles and parent grouping; required producer/consumer order; actual
`br ready` versus prerequisites; priority/deferred state/reference resolution;
and agreement among the database, flushed export and viewer. All passed.

## Executed checks

- Baseline: clean `22848ea6884f13b014c74850e7bd888e0b8a94d8`; normal tracker import
  restored the current 43-issue inventory before planning.
- `br` readback matched reviewed descriptions, duplicate acceptance fields,
  priorities and exact dependency sets after mutation and `br sync --flush-only`.
- `br dep cycles --json`: zero active cycles. Independent full blocking-graph
  traversal likewise found no missing endpoints or cycles.
- `br ready --type task --json`: eight ready tasks, matching dependency-derived
  readiness. No epic was claimed. `batter-4t6` remains the reference entry point.
- `br --no-db list --all --deferred --json` and `bv --robot-triage`: 45 matching
  issues, with 15 closed and 3 deferred preserved; no unresolved draft aliases.
- Local link checks: all 73 links in the three changed design/status documents
  resolve. `git diff --check` passed.
- `scripts/jig work check --plan-id plan_01M2317Z2VXJBH0AT3NZ1CHE0C --json`
  passed all five configured targets: Clippy, formatting, tests, harness contract
  and file budget. Run: `run_01M23210Z439WQSJ6VHVFTDDSZ`.
- Linux x86_64, Rust 1.98.1 (`48a229cea`): the configured test command passed 425
  test/doctest executions across 43 result summaries, zero failures/ignored.
  This includes repeated foundation execution, not 425 unique tests. Receipt:
  `receipt_01M2323WA4H5NZWWYV0TF8ZEFG`. This planning session did not rerun the
  1.94 matrix, HTTP process smoke profiles or live PostgreSQL/provider tests.
- `scripts/jig work evidence` and `scripts/jig work gates` for this plan reported
  the required verify gate passed with fresh target inputs and no failed required
  gates. No source, dependency or test change followed those checks.

## Outcomes & Retrospective

Updated 19 existing Beads and created two delivery outcomes: `batter-8q8.1`
(command atomicity/concurrent identity) and `batter-8q8.2` (provider certainty and
restart reconciliation). All 15 completed issues and all three deferred statuses
remain intact. No meta-planning Bead or parallel Markdown backlog was created.

Added `docs/design-evidence.md`, refreshed the current reconciliation and linked
the rationale from implemented status. No Rust/Python source, dependency,
application test, deployment or publication change was made. The behavioral
re-audit uses source and prior execution evidence; this planning change does not
establish live PostgreSQL, provider or hosted validation.
