# Repair readiness review findings

This ExecPlan is a living document maintained under `.agent/PLANS.md`. It owns
the requested comprehensive, low-severity, all-reviewer review-fix loop for
reopened Bead `batter-isq`. The original Git baseline was
`aeab19992a14adf7be0a493ba18f031172fd1d59`; the reconciled upstream base is
`fcaffe953a3000e0c09060884747354f8f376725`.

## Purpose / Big Picture

The initial readiness cutover correctly made a healthy dependency failure
unrepresentable, but two independent rounds exposed a related public-adapter
migration weakness. First, the new foundation reason was re-exported from the
same `batter_axum::ReadinessReason` path as the old response-extension type, so
old middleware could compile while every lookup silently returned `None`.
Removing only that re-export was insufficient because an identically named
foundation type could be imported into the stale lookup with the same result.

After repair, the foundation payload is unambiguously
`ReadinessUnreadyReason`; neither crate defines or aliases `ReadinessReason`, and
responses contain only `ReadinessDecision`. The adapter exposes public,
decision-safe `readiness_status` and `default_readiness_level` functions so
callers can render or partially customize policy without copying it. A
deterministic foundation test pins dependency sampling before lifecycle
sampling. Migration and ownership guidance name every affected boundary.

## Progress

- [x] (2026-09-15 07:32Z) Completed the initial Claude/Codex/Cursor review at verified working-tree fingerprint `46a456d37c331c1cba5c1fbcb5b8d3171f744d50fa824b72154ab95fc024dcb3`; Codex found no issue, while Claude and Cursor supplied the qualifying findings.
- [x] (2026-09-15 07:32Z) Verified the findings against the baseline API and current contracts, grouped the adapter migration failures under one causal mechanism, reopened Bead `batter-isq`, and started ordinary repair round 1.
- [x] (2026-09-15 07:49Z) Repaired the adapter type identity and reusable mapping API with public rustdoc and focused coverage.
- [x] (2026-09-15 07:49Z) Added deterministic foundation coverage for dependency-before-lifecycle sampling and corrected migration/ownership documentation.
- [x] (2026-09-15 07:55Z) Completed focused checks, both supported-toolchain matrices, all ten process smokes, and all five applicable Jig targets.
- [x] (2026-09-15 08:02Z) Completed ordinary review round 1 at verified fingerprint `0c017f21b580fdad834f11bd7bfa9c4a433190b4f32eacc5119f6216fe9dc7f3`; Codex and Cursor were clean, and Claude found one verified low substantive continuation of the silent extension-lookup hazard.
- [x] (2026-09-15 08:08Z) At the user's request, paused re-review, preserved the complete local tree, fast-forwarded `master` from `aeab19992a14adf7be0a493ba18f031172fd1d59` to upstream `fcaffe953a3000e0c09060884747354f8f376725`, replayed the readiness work, retained both validation histories, reconciled Beads additively to 127 in-sync issues, and restored the clean index.
- [x] (2026-09-15 08:21Z) Completed the round-two identity repair: renamed the foundation payload to `ReadinessUnreadyReason`, removed both stale `ReadinessReason` paths, added negative import and response-extension shape proofs, and migrated contracts.
- [x] (2026-09-15 08:27Z) Revalidated the reconciled final source on both supported toolchains and all ten HTTP profiles. The first strict Jig Clippy pass exposed an upstream-introduced generic async layout overflow in `batter-sqlx`; restoring the direct ordinary operation path and constraining the private custom outcome mapper removed the extra state-machine depth without a recursion-limit escape hatch.
- [x] (2026-09-15 08:29Z) Completed the final post-reconciliation Jig work check: all five applicable targets passed at target-validation receipt `receipt_01M2J2WW1029K073KHQDS4Q1T2`.
- [x] (2026-09-15 08:43Z) Completed ordinary review round 2 at verified fingerprint `2b319ed5118323ae5f05b7ccda97a09a8a7b5aef8337a3df805fe098ddc69e69`; all three reviewers reported no substantive defect. Only supporting closure obligations C1-C3 remain.
- [x] (2026-09-15 08:43Z) Consumed the loop's single closure batch before mutation. C1 finalizes the two status rows, C2 records the clean comprehensive result and fresh Jig policy evidence, and C3 explicitly preserves/discloses the untracked readiness module because the original empty index and no-staging authorization must remain intact.
- [x] (2026-09-15 08:56Z) Validated the cumulative closure bytes at target-validation receipt `receipt_01M2J428DGDGH4E9R6QMSG9XJ2`; current-worktree evidence and the required verify gate are fresh with no unresolved gate.
- [x] (2026-09-15 08:56Z) Completed one focused all-reviewer closure pass at unchanged fingerprint `17242cd53a83da2300bfcf241cb4089b1b0abc1ce1c45ad056a5796d8b733ef7`. Native Codex satisfied C1-C3 and found nothing else; Claude satisfied C1/C3 and found nothing else but could not inspect excluded Jig receipts; Cursor's sole supporting finding inferred that the final check had not run from the same exclusion. Parent and native read-only evidence directly disprove that inference with fresh receipt `receipt_01M2J427V8CVZK6SD5PPDQXVNZ`, so no correction batch was consumed.
- [ ] Reconcile any later findings under the bounded loop and close tracker/Jig evidence only after convergence.

## Surprises & Discoveries

- Observation: moving `ReadinessReason` into the foundation while re-exporting it from Axum preserved the exact old import, so `response.extensions().get::<ReadinessReason>()` stayed source-valid even though the response now stores only `ReadinessDecision`.
  Evidence: baseline Axum inserted `Extension(reason)` and exported its local `ReadinessReason`; the changed adapter exports the foundation type under the same path but inserts `Extension(decision)`.
- Observation: the baseline's public `ReadinessReason::level` was not only a renderer implementation detail; a `with_level` callback could use it to override one case and delegate all others to the supported default.
  Evidence: the current `with_level` still accepts a function pointer, but its replacement `default_level` is private, leaving external callbacks no supported fallback.
- Observation: the production evaluator reads dependency then lifecycle in source order, but the existing exhaustive table exercises only already-sampled values.
  Evidence: both Claude and Cursor independently identified that swapping the two reads could survive current tests and publish stale Ready or dependency WARN across a concurrent drain.
- Observation: removing only the Axum re-export cannot make every stale typed lookup fail loudly while the foundation exposes an identically named `ReadinessReason`.
  Evidence: after changing the import to `batter::readiness::ReadinessReason`, old middleware that matches only unready variants compiles but still receives `None` because the response stores only `ReadinessDecision`.
- Observation: upstream `fcaffe9` changes retry/operation behavior and the same shared contracts, validation log, tracker export and Jig state, but does not touch readiness source or adapter tests.
  Evidence: the fast-forward added 25 paths; Git merged every readiness/code contract path automatically, while only the append-only validation tail and Beads export required semantic union. `br sync --reconcile` updated `batter-4c4`, retained DB-only `batter-isq`, deleted nothing, and exported 127 issues.
- Observation: the upstream retry refactor routed ordinary `OperationContext::run` through a second generic async function. The repository's strict workspace Clippy command then overflowed the compiler query-depth limit while laying out the already-deep SQLx verification future, although the ordinary verification profile still passed.
  Evidence: `scripts/jig check clippy` reproduced the failure at `crates/batter-sqlx/src/verification/executor.rs:32`; directly instrumenting ordinary runs again and using a function pointer for the crate-private custom mapper made the exact command pass with no crate-level `recursion_limit` attribute.
- Observation: external focused-review evidence excludes `.agent`, so those reviewers cannot independently inspect the repository's authoritative final Jig receipt even when it is fresh for their exact fingerprint.
  Evidence: Claude marked only that part of C2 uncertain and Cursor inferred it was missing; native Codex and the parent both ran read-only `work evidence`/`work gates` after the closure edit and observed receipt `receipt_01M2J427V8CVZK6SD5PPDQXVNZ`, exact current-worktree matching, and no unresolved gate.

## Decision Log

- Decision: treat the silent old extension lookup and lost default-mapping composition as one verified low-severity substantive adapter-contract group.
  Rationale: both arise because the breaking cutover preserved an old type path while hiding replacement adapter policy; together they can produce silent runtime drift or policy copies instead of an explicit source migration.
  Date/Author: 2026-09-15 / Codex
- Decision: stop re-exporting `ReadinessReason` from `batter-axum`, but continue re-exporting the newly named `ReadinessDecision`.
  Rationale: every pre-cutover reason import came from Axum, so removing that re-export makes old extension lookups fail loudly. New pattern matching deliberately imports the semantic reason from the foundation.
  Date/Author: 2026-09-15 / Codex
- Decision: expose free adapter functions `readiness_status(ReadinessDecision)` and `default_readiness_level(ReadinessDecision)`.
  Rationale: HTTP status and tracing severity remain adapter-owned while custom rendering and partial severity overrides can reuse one exhaustive implementation without a generic associated-function type parameter or duplicated tables.
  Date/Author: 2026-09-15 / Codex
- Decision: route production sampling through a private generic `sample_and_classify` helper whose typed callbacks enforce dependency then lifecycle order, and unit-test its call sequence.
  Rationale: this adds deterministic prevention without public hooks, allocation, dynamic dispatch, or a fake asynchronous transition seam.
  Date/Author: 2026-09-15 / Codex
- Decision: rename the foundation payload to `ReadinessUnreadyReason` and the decision accessor to `unready_reason`, with no `ReadinessReason` alias in either crate.
  Rationale: a pre-release foundational cutover should remove the ambiguous old type identity rather than carry duplicate extensions or rely on users not accepting an IDE import replacement. The new name states its inhabited domain and makes both stale import paths fail.
  Date/Author: 2026-09-15 / Codex
- Decision: reconcile the dirty working tree by a named all-inclusive stash, fast-forward, stash replay, additive Beads reconciliation and an explicit index reset.
  Rationale: this preserves all tracked/untracked work and both append-only histories, keeps the branch linear with upstream, exposes textual conflicts, and leaves the original unstaged handoff state intact. The stash remains until post-reconciliation validation proves the replay.
  Date/Author: 2026-09-15 / Codex
- Decision: keep ordinary `OperationContext::run` on the direct scoped-dispatch path and restrict the crate-private composite outcome mapper to a function pointer.
  Rationale: the ordinary boundary does not need the retry-only wrapper or a capturing outcome closure. Removing those generic state-machine layers fixes the exact downstream compiler-depth regression at its source while preserving telemetry and retry semantics and avoiding a global compiler-limit exception in `batter-sqlx`.
  Date/Author: 2026-09-15 / Codex
- Decision: reject the focused Cursor supporting finding rather than consume the correction allowance.
  Rationale: its mechanism is absence of final Jig evidence, but the exact final closure bytes have a fresh authoritative receipt and passed gate. Quoting that receipt in an included self-referential ledger append would immediately stale the quoted policy receipt; the existing ledger accurately says the check follows the append, and the receipt store proves it did. Claude's uncertainty is the same evidence-access limitation, not a repository gap.
  Date/Author: 2026-09-15 / Codex

## Outcomes & Retrospective

Implemented and converged. The loop used two ordinary substantive repair rounds,
three complete comprehensive reviews, one supporting closure edit batch, zero
closure correction batches, and one focused closure verification pass. No
reviewer recovery was used. The complete clean substantive baseline fingerprint
is `2b319ed5118323ae5f05b7ccda97a09a8a7b5aef8337a3df805fe098ddc69e69`;
the terminal closure fingerprint is
`17242cd53a83da2300bfcf241cb4089b1b0abc1ce1c45ad056a5796d8b733ef7`.

The delivered model makes healthy dependency failure unrepresentable, moves
combined classification into the foundation, keeps HTTP/severity mapping in
Axum, removes the ambiguous old reason identity, and retains dependency-first /
lifecycle-second ordering. Upstream `fcaffe9` was reconciled before the terminal
review; its strict Clippy layout regression was repaired at the private operation
seam. Both supported-toolchain matrices, all ten HTTP process profiles, focused
failure-path suites, and the final current-worktree Jig gate passed. Bead
`batter-isq` is closed and Beads is in sync.

Current Linux, hosted CI and live PostgreSQL were not executed. No commit, push,
publication, deployment, or staging occurred. The original empty index remains
empty; `crates/batter/src/readiness.rs` is intentionally untracked and must be
added explicitly by any eventual authorized commit.

## Context and Orientation

`crates/batter/src/health/observation.rs` owns the exhaustive conversion from the
diagnostic `HealthStatus` observation into `DependencyReadiness` and its narrower
`DependencyUnreadyReason`. `crates/batter/src/readiness.rs` owns
`ReadinessDecision`, `ReadinessUnreadyReason`, and `ReadinessEvaluator`; the evaluator
samples dependency health before reading lifecycle state.

`crates/batter-axum/src/readiness.rs` wraps the evaluator and translates its
decision into Axum `StatusCode` plus tracing `Level`. Before this repair those
translation functions are private. `crates/batter-axum/src/lib.rs` re-exports
both decision and reason, preserving the old reason path. Operational coverage
lives in `crates/batter-axum/tests/operational/readiness.rs`.

`CHANGELOG.md` owns breaking migration instructions. `crates/batter/AGENTS.md`
currently attributes both dependency projection and combined decision to
`src/readiness.rs`, although the projection lives beside `HealthStatus` in
`src/health/observation.rs`.

## Plan of Work

Make the adapter public boundary explicit. Export only `ReadinessDecision`, make
the two adapter mappings public under names that state their purpose, and use
those functions inside `ReadinessPolicy`. Document each public function,
including a custom level callback that delegates unmatched decisions to the
default.

Migrate operational tests to `ReadinessUnreadyReason`, assert response status
and level through the public mapping functions, and demonstrate partial fallback
rather than a constant replacement. Compile-fail rustdoc must reject both stale
`ReadinessReason` import paths. Responses must carry the decision but not a
separate unready-reason extension.

Next, refactor `ReadinessEvaluator::decision` through a private generic helper
that invokes the dependency sampler before the lifecycle sampler and then calls
the existing exhaustive classifier. Add a unit test with ordered callbacks that
would fail if invocation order is reversed and returns Draining even after a
healthy dependency sample. Keep all public behavior and zero-allocation design.

Update `CHANGELOG.md` with the removed Ready variant, old extension lookup,
removed associated helpers and exact replacements. Correct the foundation guide
so health observation owns projection and readiness owns combination. Update
public contracts and validation only where the new adapter functions or executed
review evidence change their claims. Record actual findings and evidence on Bead
`batter-isq` through `br`, never by hand-editing its JSONL export.

## Concrete Steps

Work from `/Users/aa/Documents/batter`. Use `apply_patch` for files. Run narrow
checks first:

    cargo fmt --all -- --check
    cargo test -p batter --lib readiness --locked
    cargo test -p batter --doc --locked
    cargo test -p batter-axum --test operational readiness --locked
    cargo test -p batter-axum --doc --locked
    cargo clippy -p batter -p batter-axum --all-targets --locked -- -D warnings

Then follow repository-required verification:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Because the reconciled upstream changes operation/retry internals, also run the
operation, legacy retry, attempt-deadline and telemetry integration targets and
the exact strict Jig Clippy target before the complete work check.

Rebuild the HTTP example under each toolchain and run all five documented smoke
profiles: default, SIGINT, deadline, WARN filter, and WARN filter plus deadline.
Run the connected Jig work check for plan
`plan_01M2HZPHWFN5YAJ39RR211PFB4`, inspect evidence/gates, and record exact
outcomes in `docs/validation.md`.

After successful validation, capture a fresh working-tree fingerprint and run a
new ordinary comprehensive pass with Claude, Codex, and Cursor over the entire
same working-tree scope. Keep `.agent` excluded through the trusted baseline
policy. Reconcile every original and new finding. Use ordinary rounds for any
new substantive defect; use the single closure batch only if a complete review
leaves supporting obligations alone.

## Validation and Acceptance

Both `use batter_axum::ReadinessReason` and
`use batter::readiness::ReadinessReason` must fail to compile.
`ReadinessUnreadyReason` remains available for matching inside
`ReadinessDecision`. `readiness_status` must return 200 only for Ready and 503
for every Unready decision. `default_readiness_level` must return INFO for Ready,
Starting and Draining, and WARN for Stopped plus every dependency reason. A
custom callback must be able to change one case and delegate the rest.

The sampling-order unit test must fail if lifecycle is invoked before dependency
or if a post-dependency Draining observation does not override healthy cached
state. Existing exhaustive classification, compile-fail Healthy rejection,
empty-body responses, read-only probe count, drain precedence, override, error
redaction and event-count checks must remain strong.

Both supported verification matrices, ten rebuilt process smokes, all applicable
Jig targets, and the final all-reviewer pass must complete. Convergence requires
no outstanding low-or-higher substantive or supporting finding, complete
reviewer coverage, and a verified unchanged final fingerprint.

## Idempotence and Recovery

Cargo tests and Jig checks are safe to rerun. Before validation, preserve existing
output state and use the repository's normal target directory. If a repair causes
a mechanical failure, use at most the loop's two mechanical stabilization passes;
one behavioral correction is available within the round only after reconciling
the ledger. Do not retry flaky assertions until green or weaken tests.

Every repository change after a fingerprint must be attributable to this plan,
validation, Jig, or Beads. Stop on unexplained drift. Preserve the pre-existing
Beads history, index state, and all `.agent` records. No commit, push, publish,
release, or deployment is authorized.

## Artifacts and Notes

Initial comprehensive review: Claude completed with two low findings and one
test gap; Codex completed with no actionable findings; Cursor completed with two
low supporting findings and the same sampling-order gap. Parent pre/post
fingerprints matched and were complete. Claude and Cursor independently found
the missing helper migration and sampling-order proof. No reviewer recovery was
used. Ordinary round 1 produced one further low substantive identity finding
from Claude; round 2 was substantively clean across all three reviewers. Focused
closure verification inspected the full cumulative three-path support patch;
the only adverse verdict was Cursor's disproven inference about excluded Jig
evidence, with no substantive or collateral defect reported.

## Interfaces and Dependencies

The intended adapter additions are:

    pub const fn readiness_status(decision: ReadinessDecision) -> StatusCode;
    pub const fn default_readiness_level(decision: ReadinessDecision) -> Level;

Neither `batter_axum` nor `batter` exposes `ReadinessReason`; match
`batter::readiness::ReadinessUnreadyReason` inside `ReadinessDecision`. The
upstream reconciliation also keeps ordinary operation instrumentation direct and
uses a function pointer at the crate-private retry outcome seam. No dependency,
feature, runtime task, allocation, dynamic dispatch, lock, wire body, or Cargo
lockfile change is planned.
