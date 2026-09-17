# Listener publication and diagnostic boundaries

Owning Bead: `batter-ws3`, discovered from `batter-8ou`. Preserve the existing
index and all worktree changes. The user authorized normal Jig workflow instead
of the earlier repair controller. No staging, commits or deployment.

## Progress

- [x] Read review reports, repository guidance and ADR-010; classify candidates.
- [x] Resolve listener-publication ownership and generation-ordering semantics.
- [x] Add transport-timeout and durable admission diagnostic regressions.
- [x] Execute focused database experiments for open questions; record limits.
- [x] Implement the justified bounded repair and update contracts/evidence.
- [x] Both Rust matrices, both live suites, ten HTTP smokes, independent review,
      final Jig api:test receipt, fresh gates, Bead and plan closure.

## Surprises & Discoveries

The latest review did not find a recurrence of terminal-source corruption. Its
remaining runtime finding is synchronous stdout inside protected async startup;
its other two actionable findings are diagnostic coverage gaps. Tokio stdout
uses blocking I/O internally, so merely changing the spelling to async is not a
cancellation proof. Address publication is application/test-process composition,
not a reason to add a generic foundation logging or discovery framework.

## Decision Log

User approved both choices: optional explicit Unix datagram announcement and
confirmation serialized with replacement. The application owns both contracts.
Record -> job -> effect ordering avoids holding the heartbeat lock while waiting
for replacement. FOR SHARE is sufficient to exclude generation updates while
allowing independent confirmations; the following statement reads fresh state.
No foundation API, migration/backfill or worker capacity ratio change is needed.

Recheck concrete native sources and execute focused scenarios before deciding
generation ordering, heartbeat/default-capacity changes or query rewrites.
Do not treat an unmeasured performance question as evidence for a ratio rule.
Preserve current-version-only reference scope. New stdout/announcement behavior
requiring a material contract choice must be flagged before dependent changes.
Diagnostic tests should exercise native errors and real persistence, not merely
repeat the pure string mapping. Source enum/terminal writer boundaries stay intact.

## Execution and validation

Relevant files: `examples/reference-service/src/runtime.rs`, `provider.rs`,
`provider/contract_tests.rs`, `delivery/worker/state.rs`, its private live tests,
and `tests/support/provider_effects.rs` plus process/listener support as needed.
Logs and experiment artifacts belong under `/tmp/batter-listener-review.Jb1ixP`.
Use the previously task-owned disposable PostgreSQL18 clusters on ports35471
and35472 with a fresh temporary role; verify zero remaining role-owned databases
and sessions before removing that role and stopping those exact servers.
Run both `scripts/verify.sh` toolchains, full explicit reference runner, and all
five rebuilt HTTP smoke profiles per toolchain. Freeze inputs for Jig work check;
inspect evidence/gates before metadata-only refresh. Update relevant status,
guarantees, testing, reference compatibility, references and validation.

## Outcomes & Retrospective

Focused native timeout test and package strict Clippy passed on Rust1.98.1.
The real provider outcome suite passed in 34.06s with its new persisted admission
code assertion. Minimal SQL reproduction confirmed statement-snapshot behavior
across an effect-row lock wait. A 100,000-row synthetic forced-generic plan used
BitmapOr with zero searches on the NULL branch. This is not a full-query or
production-load benchmark. Native heartbeat source confirms its shared job-row
lock and one-third-TTL budget. Both approved runtime repairs are implemented.
Focused state tests passed both record/confirmation orderings and lease expiry
while waiting, with an independent NOWAIT job-lock check. Unix announcement and
configuration tests passed. The saturation test now recognizes macOS ENOBUFS
as immediate failure; a process control's unavailable /bin/true was replaced by
the existing supported /bin/sh path. Both full Rust matrices, both66-case live
inventories plus maintenance/state probes, and ten rebuilt HTTP smokes passed.
Two independent scoped native reviewers found only the stale README discovery
paragraph; both verified its correction. The first final Jig test attempt failed
in a smoke-runner prerequisite; its assertion was absent from the saved preview.
An isolated13-control rerun and unchanged full Jig api:test rerun passed, the
latter receipt `receipt_01M2REYASH3V2ZS0N593XSMYXW`. Cause remains unestablished;
no source or test limit was changed to obtain the later pass.
The disposable SQL-mechanism database and temporary roles were removed after
zero-session checks; both task-owned servers were stopped, retaining their data
directories. These were disposable fixtures, not application data.
Those exact servers were restarted with a recreated disposable role for the
final live matrix. Zero role-owned databases/sessions were verified again; roles
were dropped and servers stopped, with the data directories retained.
Passing tests/reviews do not constitute a fresh-agent usability evaluation.

## Recovery

Keep all pre-existing changes and append-only Jig history. Do not weaken semantic
tests or call a timed-out cleanup complete. Leave database data directories
intact. Stop and ask for direction if a material application contract cannot be
resolved from repository evidence.
