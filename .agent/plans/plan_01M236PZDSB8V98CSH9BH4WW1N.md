# Finite shutdown cause review follow-up

Owning Bead: `batter-299`. Baseline: `61a025f5ed3699405034c1407942cdb4c280297c`.
The user authorized fixing review findings, repeating comprehensive review until
clean, and committing the completed change. This includes the earlier uncommitted
variant implementation. Delivery scope and acceptance remain in Beads.

## Progress

Corrected variant rustdoc and the process contract: errors/panics initiate finite
shutdown; shutdown aborts are later report outcomes. Clarified that finite labels
can repeat. Added four bounded paused-clock tests in `shutdown_causes.rs`, covering
healthy critical components alongside finite errors/panics, first-cause retention
in both task-kind orders, a failing descendant, and a forced finite abort after an
explicit request. Updated the implemented status and coverage map.

The original four focused tests passed. Both Rust toolchains passed 441
test/doctest executions, formatting, compilation, Clippy and rustdoc, and all five
HTTP smoke profiles passed. Independent comprehensive review completed with a
matching complete fingerprint: Codex found no actionable defects; Claude identified
ambiguity about observation order and a future fixture watchdog-margin concern.

Clarified request priority over unobserved failures, increased the watchdog to ten
seconds, and added finite/critical precedence regressions. Both Rust 1.98.1 and
1.94.0 passed the final 445 test/doctest executions and their remaining verify
checks. The rebuilt HTTP example passed all five smoke profiles. Full logs are
listed in `docs/validation.md`.

The final independent Claude/Codex working-tree review completed. Codex found no
actionable findings and independently passed 52 focused tests and four doctests.
Claude found no correctness, concurrency, data-loss or security defect, and
offered two low-priority suggestions. Adjudication found neither actionable:
`#[non_exhaustive]` would not stop an existing wildcard match from selecting that
wildcard, and the intentional variant migration is already in CHANGELOG. The
watchdog has margin over the current five-second complete fixture budget, uses
cooperative futures, and explicitly disclaims wall-clock preemption in validation
evidence. Possible future budget changes are not a current defect.

Both reviews used HEAD/policy revision
`61a025f5ed3699405034c1407942cdb4c280297c` and committed `.reviewignore` exclusion
`.agent`. Claude used restricted file access. The parent's complete before/after
captures matched fingerprint
`d00c45b0f1880a4d49d7f5d81d95c976c5b3774faed948569cfc368b86694ae5`, with no
capture issues or submodules. Neither reviewer changed repository files.

All five final Jig targets passed with fresh evidence, followed by the required
`scripts/jig check test` (445 test/doctest executions, exit 0). Final artifacts:
`/tmp/batter-299-followup-jig-check-rebuilt.json`,
`/tmp/batter-299-followup-jig-evidence.json`,
`/tmp/batter-299-followup-jig-gates.json`, and
`/tmp/batter-299-followup-final-backend.log`. Bead `batter-299` is closed. Only
administrative tracker completion followed the stable code review; reviewed code
and documentation were unchanged during final verification.

## Surprises & Discoveries

Aborts occur after `wait_for_shutdown` selects the initial cause. Returned causes
from later task observations are discarded, preserving that trigger. Mixed-kind
tests use the drain signal to order the second failure after cause selection;
they do not rely on wall-clock scheduling or unordered competing completions.

The main lifecycle and process-ownership test files are at the existing 800-line
limit. New scenarios belong in a separate integration-test target instead of
growing those files or relaxing the budget.

The first final Jig attempt failed compilation against a reused library artifact:
the compiler reported `SharedShutdownReport`, absent from current source, and
could not see the present `FiniteTaskExit` variant. A verbose focused rerun
reproduced it while Cargo reused `libbatter-48469626179b4818.rlib`. After
`cargo clean -p batter`, all five gates and the final backend command passed
without source changes. This establishes the artifact mismatch and successful
rebuild; the artifact's origin remains unknown. Failure evidence is retained in
Jig receipt `receipt_01M238EYRMDXM1D61AN7MKCKCT` and
`/tmp/batter-299-build-metadata-diagnostic.log`. This compilation failure is
distinct from the earlier undiagnosed non-yielding test failure.

## Decision Log

Keep the reviewed runtime classification implementation unchanged. Correct its
documented reachability and add public-behavior regression coverage. Preserve
every earlier semantic assertion. Keep the initial non-reproducing subprocess
failure in historical validation evidence, without inventing a diagnosis.

Freeze included files before comprehensive review and use context-free Claude
and Codex reviewers with the committed `.reviewignore` policy. Fix only actionable
findings after each completed review, then repeat. Commit after clean review and
passing required checks; pushing or publishing is outside this authorization.

## Outcomes & Retrospective

The reported documentation defect and specific cause-coverage gaps are addressed.
The subsequent observation-order documentation issue is also addressed, with
regressions for both task kinds. Final source verification and comprehensive
review are complete with no actionable findings. Final Jig gates and commit are
the final delivery steps; all required Jig gates and the final backend test now
pass. The user authorized committing the completed implementation and these
records; Git records the resulting commit.

Claude noted optional coverage extensions: two simultaneous finite failures,
finite-first failure followed by a critical panic with cleanup skipping, and a
descendant admitted during drain failing before its deadline. No demonstrated
defect or blocking gap was identified. Its other questions concern the already
recorded undiagnosed historical subprocess failure and pre-existing component
early-success/empty-process behavior. The historical failure was not reproduced
by either final full matrix; its cause remains unknown, not accepted as harmless
noise. Linux and hosted execution were not performed for this change.

## Validation and recovery

Run both `bash scripts/verify.sh` toolchains (default and
`RUSTUP_TOOLCHAIN=1.94.0`), rebuild the HTTP example and execute all five smoke
profiles in `docs/testing.md`. Keep full logs outside the repository. Finalize
validation and tracker records before running Jig work checks and the final
`scripts/jig check test`; changes during those checks invalidate their evidence.
Review the stable working-tree diff with the comprehensive-review skill, preserve
its scope fingerprint and frozen reports, and repeat after actionable fixes.
All checks are rerunnable; failures must not be hidden by weaker assertions.
