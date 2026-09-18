# Protected router composition and feature oracle

Owning Bead: batter-3uy. Baseline: HEAD de7bd2d07628cc4f18a6dd476c4217a602c0f18f with pre-existing dirty Runlimit work. Preserve all existing changes and do not stage or commit.

## Progress

- [x] Review findings and trace Axum 0.8.9 Router::route_layer, Router::layer, Router::reset_fallback, Router::merge semantics in the pinned registry source.
- [x] Change the HTTP assembly contract at the owning router boundary.
- [x] Add failure-path HTTP regressions and a feature-inventory negative control.
- [x] Update rustdoc, integration/status/testing/validation contracts and the Bead.
- [x] Run focused checks, both required toolchains, HTTP smoke and Jig work gates; record exact evidence.

## Surprises & Discoveries

Axum 0.8.9 Router::route_layer excludes fallback_router; Router::layer includes it. Router::merge panics when both routers have explicit fallbacks. Router::reset_fallback removes all fallback routing on that Router, including nested fallback entries. These operations are in pinned local Axum source, not assumptions about a later version.

## Decision Log

The protected input Router is a complete protected service, including its custom root and nested fallbacks. Use whole-router layering. The public probe Router is a collection of explicitly opted-in routes; erase its fallbacks before merge. Preserve a custom protected root fallback; retain the default unguarded 404 when none is supplied. Do not add a wrapper that merely renames Router while retaining its hidden fallback behavior.

The isolated feature check uses Cargo metadata as the feature inventory and an exhaustive expectation mapping as its validation oracle. A newly declared unmapped feature fails immediately instead of receiving accidental coverage claims.

## Outcomes & Retrospective

The complete business Router is now guarded, including custom root, nested and
method fallbacks. Public probe fallbacks are discarded before merging. The
feature runner derives its feature inventory from Cargo metadata and refuses
an unmapped declaration. The focused Runlimit suite passed (31 HTTP, 14 quota,
one unit and eight doctests), as did 23 runner controls and all eight isolated
feature subsets. `verify.sh` passed on Rust 1.98.1 and 1.94.0. Ten HTTP process
smokes and both quota example runs passed. A fresh disposable agent consumer
used public APIs only and reproduced the expected 401/200/429 boundaries.
Jig `work check` passed with fresh `api:test` receipt
`receipt_01M2T599HYFBB1H3JT85ZD7TME`; `work evidence` and `work gates`
reported no unresolved gates. Exact commands and limitations are in
`docs/validation.md`. No new public type, dependency, or backend behavior was
needed. Axum still owns route-conflict validation for arbitrary duplicate
registered paths; this change closes the fallback bypass and merge panic.

## Context and orientation

crates/batter-runlimit/src/http.rs owns protected and public router composition. crates/batter-runlimit/tests/http/response_contract.rs contains HTTP boundary regressions. scripts/check_runlimit_features.py owns independent optional-feature checks. docs/integrations.md and docs/testing.md state consumer and verification contracts.

## Plan of work

First replace route_layer with whole-router layer on business input and reset public-probe fallback before merge; remove the final forced fallback. Then exercise nested and root custom fallbacks, public fallback-only probes, and two-custom-fallback preparation through the existing in-process client and events. Derive features from metadata with exact mapping check and a synthetic unknown-feature negative control. Update docs/status/validation after executed checks.

## Validation and acceptance

Run focused batter-runlimit HTTP tests and feature-oracle unit tests. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh. Build and execute the HTTP smoke profiles in docs/testing.md, then Jig work check/evidence/gates with final api:test receipt. A passing response from the nested protected fallback must require valid auth and report quota; a probe fallback must not become a public handler; a business fallback and probe fallback must prepare without panic.

## Idempotence and recovery

Tests and checks may be rerun. Preserve existing user changes and Cargo.lock generation. On a failed check, repair only the responsible source or test, then rerun the affected gate. Do not claim completion from stale Jig receipts.

## Interfaces and dependencies

No new dependency or public type is planned. Axum is pinned to 0.8.9 in Cargo.lock. Native Runlimit ownership and the foundation/adapters dependency direction remain unchanged.
