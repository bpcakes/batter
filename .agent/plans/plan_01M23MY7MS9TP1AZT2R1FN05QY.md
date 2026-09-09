# Encapsulate lifecycle tasks and HTTP observation

Owning Bead: batter-bu2. Baseline: ecfc4aa01dc47564b8b164883a4a373a0a4cbb09.
Implement the two approved architecture suggestions without public API changes.

## Progress

- Both private modules and narrow coordinator operations are implemented.
- All 87 existing focused integration tests passed before and after extraction.
- Four new private-boundary regressions passed; both full toolchain matrices,
  all five HTTP smokes and the Jig verify profile passed.
- Documentation/tracker finalization and final gate freshness check precede closure.

## Surprises & Discoveries

The coordinator reads TaskSet fields directly. Merely moving the struct would
not establish the requested boundary: joins must be recorded by TaskSet itself.
The HTTP observer already has an independent closure-based composition entry.
Its extracted guard and composition body are text-identical apart from internal
visibility. The first full run caught Clippy async_yields_async in the new test;
an outer holder retains the pending future without returning an awaitable from
the async fixture. Assertions were unchanged and full verification was repeated.

## Decision Log

Use private lifecycle/tasks.rs for TaskSet, task metadata, join classification
and an owned final summary. Keep readiness transitions in state.rs and shutdown
phase policy in lifecycle.rs. A next-exit operation records a ready join before
returning; it has no additional await after consuming the join. Preserve select
priority and task-collection destruction before cleanup. Do not change error
types, factory laziness, tracing targets/levels/fields/parents, budgets, or runtime
ownership. Source-file/module metadata necessarily follows the moved code.

Move observe_response and HttpObservation unchanged into private observation.rs.
Keep all public adapter items at their current crate-root paths. The observer's
only internal composition entry accepts a native request and response future;
it must not acquire lifecycle/admission policy.

## Outcomes & Retrospective

Both approved boundaries are implemented, public APIs/manifests/lockfile remain
unchanged, and executed evidence is in docs/validation.md. The lifecycle root
went from 746 to 541 lines and the adapter root from 461 to 346; moved code and
four added regressions remain in their owning private modules. This is an
encapsulation change, not a net line-reduction claim. No commit or push authorized.

## Execution and validation

1. Extract the two private modules with apply_patch; migrate coordinator calls
   to narrow TaskSet methods. Keep dependency manifests and lockfile unchanged.
2. Add regressions at the new internal boundaries for recorded task failure and
   cancellation of observation, preserving existing integration assertions.
3. Update owning guides, docs/architecture.md, guarantees.md, testing.md and
   status.md. Record results, platform, toolchain and limitations in validation.md.
4. Run cargo test -p batter --test lifecycle --test shutdown_causes --test
   process_ownership --test scoped_owned_tasks --locked and cargo test -p
   batter-axum --test observation --test scoped_dispatch --locked, plus new unit
   tests. Run bash scripts/verify.sh and RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh.
5. Rebuild cargo build -p batter-axum --example http_service --locked. Run
   scripts/smoke_http.py with --binary target/debug/examples/http_service in
   default, --signal SIGINT, --deadline, --warn-filter, and --warn-filter
   --deadline modes. All must pass without relaxing semantics.
6. Run scripts/jig work check, inspect work evidence and work gates, close the
   Bead, flush its export and finish this plan after all required gates pass.

If checks fail, retain diagnostics and repair only this extraction. Do not
revert unrelated work or claim proposed behavior from compilation alone.
