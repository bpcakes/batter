# Preserve verifier coverage and bound evaluation

Owning Bead: batter-7r3.5. Jig plan: plan_01M2DBSBHNTWQWG8C8FDPFSKGS. Follow .agent/PLANS.md. The user authorized implementation, full verification, a fresh Claude/Codex/Cursor comprehensive review, and a commit only if no medium-or-higher finding remains. Residual low findings must be recorded in Beads. Do not launch another repair loop after that review.

## Progress

- [x] 2026-09-13: Inspect source, frozen reviews, upstream semantics and prior recovery; reopen owning Bead.
- [x] Preserve discovered identities and effective permission sources; add cross-schema and identifier regressions.
- [x] Bound cooperative evaluation and remove unnecessary role/object and parameter-default scans; add cancellation and capacity controls.
- [x] Document and test expected idle rollback notice without weakening raw-state recovery.
- [x] Execute both Rust matrices, both live SQLx runners, HTTP smokes and Jig gates; update evidence.
- [x] Freeze all three reviews, adjudicate and record findings. Conditional commit blocked by medium batter-7r3.12; low batter-7r3.13 also recorded.

## Surprises & Discoveries

SQLx 0.9's public transaction check uses managed depth; authoritative protocol transaction status is private. Idle rollback produces an expected warning but is necessary to recover abandoned raw state. Discovery excluded every multirange although load_types already resolves its source range ACL. PostgreSQL permits separate namespaces. Existing scale tests cover many objects with one role and do not establish runtime responsiveness.

## Decision Log

Keep the public serving-pool/context/policy API and current lease retirement. Keep raw rollback and explicitly test its notice. Preserve each discovered type identity; resolve permission sources in the loader, without dropping aliases before policy evaluation. Render structured object identities canonically at the finding boundary. Evaluation stays on its owned async future, with bounded work checkpoints and explicit capacity errors; do not create detached blocking work. Existing exhaustive Jig globs cover files under the adapter src/tests roots; no new root or input exemption is needed.

## Outcomes & Retrospective

The final review exposed another semantic coverage gap: the generic schema ACL evaluator lacks the session-dependent temporary-namespace rule. Native PostgreSQL and a public-API probe demonstrate both a false required failure and an excess-authority false pass. This is adapter coverage debt; neither extra caller instructions nor claiming the ordinary ACL model covers every schema addresses it. A future repair must classify the surface and implement native semantics or explicitly reject unsupported coverage. No dependent repair continues in this work; user instructions require flagging new medium-or-higher findings. See .agent/reviews/batter-7r3.5-root-repair.md and Beads batter-7r3.12/.13.

Implementation complete. 52 focused unit tests and three guard mutation controls pass. Native cross-schema and idle-warning controls pass. Both full Rust matrices, both 49-case live runners, all ten HTTP smoke invocations, both normal/violation examples and all five Jig gates pass. All three final reviews completed on the same complete fingerprint. Confirmed medium temporary-schema semantic gap blocks commit; low PUBLIC relation/column precedence issue recorded. No further repair loop. Earlier green receipts describe the earlier source only.

## Context and orientation

crates/batter-sqlx/src/verification.rs owns acquisition, transaction normalization, snapshot setup, inspection and acknowledged rollback. verification/authority/discovery.rs chooses catalog objects and expands policy defaults. authority/database.rs resolves selected types to effective ACL sources. authority/requests.rs and privileges.rs compare authority; required.rs handles current-role requirements. Findings in report.rs currently carry string object identities. These changes must preserve actual selected-object policy overrides, PUBLIC and role separation, and all native failures.

## Plan of work

First retain type identities throughout discovery and effective ACL loading, centralize diagnostic rendering, and prove cross-schema source grants and colliding dotted names with PostgreSQL and offline controls. Then add an internal cooperative work budget, thread it through evaluation loops, restrict ordinary ACL comparison to reachable grantees/owners/active superusers, and calculate implicit parameter defaults per active role rather than rescanning a role list. Test a loaded snapshot with many roles and objects under OperationContext cancellation/deadline and explicit capacity failure. Preserve direct offline semantic coverage while making it use the same async evaluator. Finally exercise the expected SQLx notice on an idle connection and preserve written/empty/aborted reset regressions.

## Concrete steps and validation

Run cargo test -p batter-sqlx --lib --locked and cargo clippy -p batter-sqlx --all-features --all-targets --locked -- -D warnings after focused changes. Use a dedicated disposable PostgreSQL 18.6 environment with the existing external fixture runner; never reuse an unrelated database. Run bash scripts/test_sqlx_live.sh with its required environment on Rust 1.98.1 and 1.94.0. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Build batter-axum's http_service example separately on each toolchain and run scripts/smoke_http.py in TERM, INT, deadline, WARN and WARN/deadline modes. Run scripts/jig work check --plan-id plan_01M2DBSBHNTWQWG8C8FDPFSKGS, then evidence/gates with --freshness-timeout-ms 30000. Store exact execution outcomes in docs/validation.md. Update status, contracts, examples when diagnostic representation or capacity errors change.

The final comprehensive review uses the cumulative working tree against HEAD, trusted .reviewignore and all three providers. Make no reviewed-file changes while reviewers run. Verify before/after fingerprints. If clean or low-only, log low issues, settle Bead and Jig evidence, stage the final files and commit the full authorized verifier change. If medium/high/critical remain, report them and do not commit.

## Idempotence and recovery

Preserve all initially staged work. Do not reset or overwrite unrelated changes. Keep PostgreSQL provisioning external and teardown task-owned resources after tests, including failures. Capacity/cancellation errors retire the owned lease and cannot become partial policy passes. Failed test commands remain in validation evidence. Do not change Cargo.lock by hand.

## Interfaces and dependencies

Use existing validated QualifiedName, Identifier and RoutineSignature values for rendering. Private cooperative checkpoints may use std Future polling and the existing OperationContext; no new public runtime or dependency is needed. Document any new public error variant and exercise it through the existing verifier entrypoint/example. No schema migration or persisted database change is introduced.
