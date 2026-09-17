# Terminal transition and readiness budget ownership

Owning Bead: `batter-8ou`, discovered from `batter-gg8`. Git baseline:
`abbe6f5c27887db9c5cbc7b7bd1fb6dd9967b00f` plus the existing staged/unstaged work.
Preserve that index. The user selected normal Jig work instead of the previous
repair controller; do not restart that abandoned run.

## Progress

- [x] Assess the private state boundary and classify the review findings.
- [x] Demonstrate terminal-source rejection failure, then constrain shared writes.
- [x] Move readiness fixture selection and its derived budget into the scenario.
- [x] Resolve upstream questions and update contracts/evidence.
- [x] Run both verification scripts, both live suites, ten HTTP smoke profiles,
      final independent review and Jig evidence/gates; close the Bead and plan.

## Surprises & Discoveries

Current manual-transition callers are legal, but SQL source equality is not
transition authorization. The business-denial method validates locally; its
manual sibling does not. This is an application-private invariant leak, not an
observed production corruption or a gap in Batter's process API. Review comment
counts also included unsupported capacity-ratio and untracked-file concerns.
Those are not grounds for new production configuration restrictions or staging.
Independent re-review requested a live stale-source oracle in addition to the
offline rejection table. The existing state probe now checks all terminal states
and a mismatched unresolved state, with full-row equality after both writers.

## Decision Log

The shared terminal writer will accept only a private nonterminal-source enum,
constructed at both entry points. This centralizes validation without imposing
typestate plumbing on every worker call site or adding a provider framework.
Audit the neighboring mutation methods for the same absorbing-terminal rule.
The readiness scenario will own its fixture entrypoint and derive its observation
budget from the same named limits used by the two signal cases, with explicit
process/fixture headroom. Retain every phase assertion and error-combining path.
This is a completion-observation bound, not a guarantee of cleanup on timeout.

## Execution and verification

Edit `examples/reference-service/src/delivery/worker/state.rs` and its private
tests. First run `cargo test -p batter-example-reference-service --lib --locked
terminal_writers_reject_terminal_sources_before_sql_acquisition`; it must expose
the unguarded manual writer, then pass across all seven source states and both
writers. Existing live provider cases cover legal terminal transitions; add
focused boundary probes where needed.
Edit `tests/support/production_root.rs` and its single `tests/reference_live.rs`
caller; keep the live inventory unchanged. Use existing offline process controls
to assert the budget includes the serial phase allowances.
Run both root `scripts/verify.sh` toolchains, rebuild and execute all five HTTP
profiles on each, and both explicit reference live suites on the task-owned
PostgreSQL18 clusters. Keep logs under `/tmp/batter-terminal-review.IZm879`.
Update status, guarantees, testing, references and validation. Existing exhaustive
Jig scopes already include these paths. Freeze source for independent review and
`scripts/jig work check`; inspect evidence/gates before reusing Rust receipts.

## Outcomes & Retrospective

The offline test reproduced the manual-source hole before repair and passed
afterward. Both final verification matrices, all ten HTTP smokes and both full
66-case live inventories plus maintenance/state probes passed. Two independent
native reviewers accepted final scoped source; one requested the added live
stale-source prevention test. Jig validation `receipt_01M2RA55CX2PBZNCH6FNWWK9WB`
includes API-test receipt `receipt_01M2RA54J4PF7PZ39J4YAQSFAD`; evidence/gates
were fresh. Closure refreshes only metadata policy checks. Fresh-agent
implementation/modification evaluation is not claimed. No backfill, mixed-version
compatibility, public framework API, staging, commit, publication or deployment
was needed. Temporary roles were removed after zero-database/session checks;
both disposable clusters were stopped and their directories retained.

## Recovery

Preserve all pre-existing changes. Failed tests remain evidence, not grounds to
weaken assertions. Retain pending fixture owners and both failure causes. Stop
only the selected disposable servers and remove only the task's temporary roles
after successful fixture cleanup; leave their data directories intact.
