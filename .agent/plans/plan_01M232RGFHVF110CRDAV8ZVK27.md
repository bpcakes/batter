# Operational reuse roadmap audit

This execution record covers the requested tracker audit and task creation for
the operational outcome now owned by `batter-7r3`. It does not execute those
implementation tasks. Beads remains the source of delivery scope, acceptance,
priority, status and dependencies. The Git baseline is `22848ea`.

## Progress

- [x] Read repository ownership guidance, current implementation boundaries and
  all 45 existing open, closed and deferred issues through `br` and `bv`.
- [x] Compare demonstrated downstream duplication against the existing roadmap.
- [x] Create one outcome and six bounded delivery tasks with source entrypoints,
  explicit ownership, failure-contract acceptance and actual adoption evidence.
- [x] Extend seven overlapping existing issues, preserving their statuses,
  priorities, ownership, labels and original acceptance criteria.
- [x] Wire genuine API prerequisites and a related reference outcome; refresh
  the export with `br sync --flush-only`.
- [x] Verify the 52-issue inventory, intended changes, preserved existing state,
  zero dependency cycles and three ready foundational extraction tasks.
- [x] Run required Jig verification: Clippy, formatting, tests, harness contract
  and file-budget checks all passed in `run_01M2335RVVNSQWQYNKP0VNGB92`.

## Surprises & Discoveries

Configuration, request metadata, PostgreSQL fixtures and cleanup failures were
already planned, principally inside a future transactional reference service.
Owned health monitoring, optional SQLx connection disposition, startup helpers,
Axum defaults and read-only PostgreSQL verification lacked dedicated reusable
delivery tasks. Completed HTTP observation work already provides underlying
middleware and must be consumed rather than reopened.

The deferred broad-abstraction task had an overly general title. Its scope now
clearly retains the two-consumer threshold for speculative abstractions and
stable adapters while allowing the separately authorized, evidenced experimental
extraction track. Examples do not count as independently deployed consumers.

## Decision Log

- Extend matching issues instead of creating competing configuration or fixture
  backlogs. Record test reuse and native ownership in the affected tasks.
- Let health, basic SQLx and startup work proceed independently of the complete
  Runledger/harness reference bundle. Add blocking edges only where a task must
  consume another task's API; outcome grouping is not a prerequisite.
- Keep SQLx and Axum out of the foundation, and keep generic test support a leaf.
  PostgreSQL provisioning remains external; existing Jig process ownership must
  be assessed before any consumer duplicates it.
- Require pinned actual downstream consumption and removal of duplicate
  implementations. Preserve native queries, transactions, domain authority,
  migrations and explicit policy in applications.
- Mutate issues only through `br`; do not implement, claim, close, commit,
  publish or deploy the newly planned work.

## Outcomes & Retrospective

The tracker now contains the extraction outcome and six delivery tasks, with
seven existing issues extended to share or consume their mechanics. CLI checks
confirm the intended seven additions and seven existing updates, preserved
existing statuses/priorities/ownership, and no dependency cycles. `br ready`
confirms the health, SQLx and startup tasks are available. The refreshed `bv`
triage agrees with the new inventory. All 90 existing graph edges were preserved;
the only additions are six parent-child links, ten blocking prerequisites and
one related-outcome link. Required Jig checks passed, and `git diff --check`
reported no whitespace errors. These checks validate the roadmap edit and the
existing workspace; the new APIs remain open delivery work in Beads.

## Verification and recovery

Run `br list --all --deferred --limit 0 --json`, `br show <id> --json`,
`br ready --type task --limit 0 --json`, `br dep cycles --json` and
`bv --robot-triage` to inspect the authoritative result. After tracker mutations,
run `br sync --flush-only`. If interrupted, inspect current issue state before
updating; do not recreate existing tasks or overwrite concurrent changes.

Run `scripts/jig work check --plan-id plan_01M232RGFHVF110CRDAV8ZVK27`, inspect
work evidence/gates, and finish only after the configured checks pass. Existing
unrelated working-tree documentation and execution records must be preserved.
