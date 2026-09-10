# Complete fixture acquisition ownership and repeat independent review

This plan follows `.agent/PLANS.md`. Owning Bead: batter-4jz, reopened for the user-authorized correction/review loop. Existing completed plans remain historical evidence. No commit, publication or upstream modification is authorized. Remaining detached-session/owner-loss scope on batter-kjl is not silently claimed complete.

## Purpose / Big Picture

An owned fixture run must wait for native database/template creation even when the body abandons the acquisition future. It must preserve producer failures and clean every created lease before reporting completion. A shared harness keeps its native admission and caller-owned server shutdown. Public errors distinguish a pool failure whose cleanup is pending from a low-level connection failure after cleanup. The explicit live command checks privileges needed by its actual fault injection.

## Progress

- [x] Research pinned upstream creation, cancellation, shared admission/shutdown and PostgreSQL lock privileges; inspect current review findings.
- [x] Implement registered acquisition producers, explicit result phases and caller-owned server lifetime.
- [x] Add deterministic cancelled-creation, shared-harness, privilege and discard-warning regressions.
- [x] Verify both Rust toolchains and all live/HTTP profiles: 563 executions, sixteen reference cases, ten SQLx cases and five HTTP profiles each. Final Jig completion remains after review convergence.
- [x] Research all review questions and repeat independent Codex/Cursor xhigh-fast reviews; round 2 completed with no actionable findings or blocking questions.
- [x] Record final passing Jig gates; plan/session closure is recorded by Jig.

## Surprises & Discoveries

The harness performs native creation with Tokio spawn_blocking, so abandoning an async waiter does not stop an already-running producer. Its deferred drain covers only already-accepted cleanup. Harness clones share admission; external shutdown is a no-op but owned-container shutdown tears down the shared server. PostgreSQL ACCESS EXCLUSIVE catalog locks require privileges absent from an ordinary CREATEDB role. The previous body ownership fix did not own the pre-registration creation phase.

## Decision Log

Retain independent producer tasks before they can be abandoned. Lease/template creation runs inside those producers and publishes ownership before delivering a value to the body. The driver joins every producer after the body exits, then closes all registered pools and leases and drains cleanup. Producer errors remain native, shared through Arc where both a body waiter and final report need the same cause. Pool initialization stays registered before polling and returns a distinct pending-cleanup error; the manual low-level API retains its completed-cleanup error contract.

Keep upstream admission authoritative. A run retains its own batch bound, but shared external lease owners must release capacity for waiting producers; cancellation of observation does not fabricate producer completion. Server shutdown belongs to the original harness owner and is removed from per-run reports. The run owns its acquisitions and cleanup only. Add must_use to the low-level resource owner with a compile-fail warning control. Public changes are to the current unpublished, uncommitted fixture feature and all in-repository consumers are updated together.

## Outcomes & Retrospective

Implementation, both full verification matrices, review convergence and final Jig gates are complete. No actionable review finding or required verification remains. Root causes comprise incomplete lifetime ownership, conflated error phases, a missing discard warning, and a prerequisite check that drifted from its test workload. Prevention combines explicit producer ownership, types that name the error phase, compiler diagnostics and executable prerequisite controls.

## Context and Orientation

`crates/batter-sqlx/src/test_support/runner.rs` currently registers leases only after awaiting native creation in the body. `report.rs` retains body/database/drain/shutdown results. `test_support.rs` owns manual fixture APIs and concrete error types. `examples/reference-service/tests/support` holds the live fixtures. `scripts/reference_live.py` is the exact inventory/prerequisite/process watchdog, with offline controls in `scripts/test_reference_live.py`. Pinned harness revision is 3d525e6fc5745ce2e2437c7997de5cccdecff4ac, Tokio 1.53.1 and SQLx 0.9.0. Provisioning remains external.

## Plan of Work

Introduce a private producer registry alongside acquired fixtures and retained templates. Scope acquisitions clone native harness/template handles into owned tasks. Register task handles before yielding, and register resulting leases/templates before returning access handles. Keep each acquisition permit with its producer. Add typed acquisition failures to reports, await all tasks before taking the final resource list, and preserve source identities with redacted formatting. Move template creation through the same producer owner because it has the same cancellation hazard; initializer closures must be Send and static to move into an owned task, and their own pools remain explicitly caller-managed.

Remove automatic harness shutdown from start and report. Document shared-admission waiting and verify a retained external lease can delay then unblock a run without being released by that run. Add a distinct pool-acquisition error for registered resources awaiting driver cleanup. Preserve manual Connect errors and add a must_use low-level fixture owner. Strengthen reference preflight for the exact system-catalog privilege required by fault injection, with offline and nonsuperuser live rejection controls.

Use acknowledged catalog locks to hold actual upstream creation while the body cancels an acquisition. Prove the run report remains pending until release, then contains the acquired resource and successful explicit cleanup. Cover empty databases, template clones and template preparation where native background creation can outlive a waiter. Keep stable template identities and reclaim fault-injection artifacts through upstream APIs, never prefix deletion. Retain current independent SQLx tests and all original compatibility probes.

## Concrete Steps

From /home/aa/Documents/batter, run focused offline contracts and `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings`. Provision a dedicated UTF-8 PostgreSQL 18 disposable server outside repository code. Run the exact reference runner, including cancelled-creation and prerequisite negatives, against it. Then run `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`; run both toolchains' reference and SQLx live commands and build/run HTTP default, SIGINT, deadline, WARN and WARN-deadline profiles. Do not globally export DATABASE_URL during workspace compilation. Update docs/validation.md with executed evidence. Inspect Jig evidence/gates and run work check with this plan ID after metadata is final.

## Validation and Acceptance

An acquisition cancelled after PostgreSQL acknowledges blocked native creation must not let the run publish a clean report while creation remains blocked. After release, every created disposable database must appear in the report and be independently absent, with all producer results retained. Shared-harness admission must wait until the external owner releases its lease, and run completion must not shut down that owner's server. Typed errors distinguish pending cleanup from completed cleanup. Ignoring a low-level fixture produces the intended compiler diagnostic. A CREATEDB-only role must be rejected by preflight before fixtures run; the configured administrative role passes. No existing semantic test is weakened.

For each review round, resolve open questions from pinned primary sources, freeze the complete working-tree scope using the comprehensive-review helper, and run independent Codex and Cursor with xhigh/fast. Fix plausible actionable findings and add regressions before rerunning. Completion requires no actionable findings from the final completed reviews, fresh passing verification evidence, and a closed owning Bead/plan. Scope changes invalidate a same-scope review.

## Idempotence and Recovery

Keep native producer ownership alive through cancellation; a pending driver is not completed cleanup. Tests explicitly release catalog locks before observing/recovering database outcomes. Use a dedicated disposable server and stop it after final evidence. Runtime destruction, internal driver failure and arbitrary detached sessions remain outside the completion guarantee. No migrations or user data are overwritten. All code remains uncommitted unless separately requested.

## Interfaces and Dependencies

The optional adapter feature continues to depend only on native SQLx, Tokio, futures-util, the external harness and generic test support. No core/leaf reverse dependency or new provisioning/admission engine is added. A producer report distinguishes native acquisition failures from Tokio join failures; body errors remain concrete. The caller retains server shutdown authority. Public rustdoc and examples explain acquisition, observation cancellation, shared capacity, error phase and explicit manual teardown.


Round 1 implementation note: the new initializer-panic regression failed before
recovery because upstream does not queue abandoned initializing templates. The
owned template path now catches initializer task failure and returns it to
upstream's awaited abort path. The test independently requires catalog absence
before recovery. Manual pool construction also retains each lazy pool before
polling initialization, including the failed pool. Native minimum maintenance
remains SQLx-owned. A real CREATEDB-only role was rejected before inventory,
and all sixteen live cases then passed on the administrative endpoint.


Round 1 validation: final source passed both Rust matrices, all live inventories,
all five HTTP profiles and prerequisite rejection controls. The first independent
post-fix review is next; keep this plan open until the user-requested loop reaches
no actionable findings and final Jig gates pass.


Round 1 review completed: Codex no actionable findings; Cursor identified missing
report discard diagnostics and uninformative terminal test output. Scope fingerprint
was complete and unchanged at eedb50a239f97287834113aed4b87405bd9911670e5c7be940a2b9c7083f6ac5,
with .agent excluded by HEAD .reviewignore. Cursor's 180-second watchdog concern
was not sustained by measured 5.54–5.61-second serial runs. Open questions were
researched before round 2: registered producers expose no abort path, native
server shutdown is caller-owned, manual initialization panic/cancellation remains
outside owned-run guarantees, and live initializer connections remain unverified.
Cold initialization uses fresh-cluster evidence; warm reuse may call no initializer.

Round 2 adds must-use owned reports and a borrowed report view because the new
compile-fail initially proved bare references do not inherit the warning. Known
redacted report summaries survive through the consumer's combined-error boundary;
separate offline tests check actual panic text and unknown-error privacy. No native
source-chain formatting is introduced. Fingerprint tests vary identity and kind.
Full verification and independent reviews remain before completion.

Round 2 full verification passed on both toolchains: 563 Rust executions across
65 summaries, sixteen reference cases, ten SQLx cases and all five HTTP profiles.
Catalog state matches round 1 exactly. Starting the second independent review
with no unresolved research question within the supported contract.


Round 2 final review: Codex and Cursor both completed with no actionable findings
or blocking questions. Official parent fingerprint matched initial and final
0563528e8ef9fc7e84786d34119aa3cab3b3ab9567d89032c45bf9d5fde9d3ea,
complete=true, issues=[], HEAD 662f2ba7271ba332f6ffdab0114d45ab59688569.
Excluded .agent from trusted HEAD .reviewignore, no explicit exclusions. Codex
also reran focused offline/doctest/runner controls; Cursor returned no coverage
warning. Residual gaps are explicit owner discard, detached-run observation loss,
manual live-role negative control rather than a persisted provisioning test, and
serial dedicated-server fault tests. None adds a current blocking requirement.
Review scope contains all source/test/dependency changes; only completion evidence
and tracker metadata change afterward. Remaining owner-loss scope stays batter-kjl.


Final completion evidence: all five Jig targets passed with fresh receipts for
this plan and worktree (api:test receipt_01M243J21JJFEP60T0NR8BFP4N).
Both evidence and gates commands report passed/fresh. The owning Bead is closed,
and the dedicated PostgreSQL cluster was stopped with awaited fast shutdown.
No commit, push, publication or deployment occurred.
