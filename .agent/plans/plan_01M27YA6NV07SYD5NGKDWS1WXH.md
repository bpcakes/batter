# Host the upstream worker with witnessed startup and conservative drain

Owning Bead: `batter-0cp`. Beads owns delivery scope, acceptance, priority and
dependencies; this file supplies the restartable implementation details required
by `.agent/PLANS.md`. Audited baseline: `d82f5bf`, 2026-09-11. Do not commit,
publish or deploy without explicit user authorization.

## Purpose / Big Picture

After this change the unpublished reference service hosts the pinned Runledger
0.12.0 supervisor as one Batter critical component. Startup synchronizes and
executes a dedicated control job through the actual registered handler before
that component acknowledges startup. The ordinary delivery job type remains
absent from the worker registry, so delivery commands remain pending and the
HTTP root deliberately stays in `Starting` until `batter-8q8.2` supplies and
witnesses the real provider handler.

Drain requests Runledger shutdown before Batter's forced-cancellation phase and
awaits the owning driver with a ten-second cooperative allowance plus Runledger's
documented abort-cleanup allowance. A separate application termination gate
allows dependent PostgreSQL cleanup only before Runledger can spawn or after a
verified cooperative stop. Returned timeout, panic, wrapper abort and dropped
ownership remain unproven and produce retained nested `UnsafeTaskExit` cleanup
evidence while preserving the original upstream failure.

## Progress

- [x] (2026-09-11) Audit repository guidance, Bead dependencies, worktree/Jig
  state, prior readiness audit, typed worker settings, Batter lifecycle/startup
  behavior and the exact pinned Runledger catalog/claim/shutdown implementation.
- [x] (2026-09-11) Establish that the task is ready: all four blocking Beads are
  closed, the selected revisions are present, the live harness exists, and the
  termination/readiness design was already incorporated by the owning audit.
- [x] (2026-09-11) Add explicit successful-startup handoff without automatic application
  readiness approval, with failure-contract tests and documentation.
- [x] (2026-09-11) Add the probe-only catalog, fresh per-attempt operation boundary, witnessed
  Runledger driver, termination gate and separately driven dependent cleanup.
- [x] (2026-09-11) Wire the production root without registering a delivery handler; add live
  startup, drain, timeout/error, claim-race, retry/attempt and configuration tests.
- [x] (2026-09-11) Update integration/testing/status/validation contracts and the owning Bead;
  complete two-toolchain, HTTP smoke, PostgreSQL 18 and Jig verification.

## Surprises & Discoveries

- Observation: `Startup::drive` currently calls `handle.mark_ready()` on every
  successful initializer, so the reference root cannot honestly withhold
  application approval by using its current constructor alone.
  Evidence: `crates/batter/src/startup/driver.rs`; the 2026-09-09 readiness audit
  explicitly requires the staged worker root to remain unavailable.
- Observation: Runledger's `SupervisorBuilder::build` validates all returned
  error conditions before the first `spawn_on`, then starts intent promoter,
  worker, scheduler and reaper loops. The termination gate can restore
  `NotStarted` for a returned build error, but must remain `Unproven` across a
  build panic.
  Evidence: pinned `runledger-runtime/src/supervisor.rs` at revision `0f464b4`.
- Observation: the worker computes claimable types from
  `registry.registered_static_types()`. Synchronizing the delivery definition
  does not make it claimable when the probe-only registry omits its handler.
  Evidence: pinned `runledger-runtime/src/worker.rs` and current handler-free
  delivery definition in `examples/reference-service/src/schema.rs`.
- Observation: handler errors are durable job values, while loop exits and join
  failures are supervisor errors. A pending controlled handler can force the
  real `run_until_shutdown` timeout path without manufacturing a panic or
  changing Batter's task classification.
  Evidence: pinned worker execution, task group and supervisor sources.
- Observation: requesting Runledger's cloneable shutdown handle does not resolve
  the external future supplied to `run_until_shutdown`, so that request alone
  never starts its ten-second timeout clock.
  Evidence: the first real held-handler timeout test remained pending until
  Batter aborted the wrapper; owning and resolving a second one-shot external
  trigger made the same test retain `RuntimeError::ShutdownTimeout` after the
  documented native bound.
- Observation: making the first caller directly drive nested cleanup would let a
  cancelled outer-hook waiter drop the only finalization future.
  Evidence: the cleanup owner now starts one retained publication driver; a
  regression aborts the first waiter while its finalizer is held, releases it,
  and receives the same successful report from a later waiter.

## Decision Log

- Decision: add an opt-in `Startup` constructor mode that hands off a successfully
  initialized running supervisor without calling `ShutdownHandle::mark_ready`.
  Keep the existing constructor behavior unchanged.
  Rationale: component acknowledgement and application admission are separate
  lifecycle facts. A fake never-acknowledging component would obscure that
  contract and make the provider cutover harder to reason about.
  Date/Author: 2026-09-11 / Codex.
- Decision: construct and continuously drive the Runledger supervisor during
  initialization, race its owning driver against the control-handler witness,
  then transfer its shutdown handle and join handle into one Batter component.
  Rationale: build completion is not readiness, and an unpolled watcher would
  miss early upstream loop exits even though Runledger's loops were spawned.
  Date/Author: 2026-09-11 / Codex.
- Decision: keep the PostgreSQL finalizer in an application cleanup owner shared
  with, but not captured solely by, the outer Batter cleanup hook. The owner
  stores one nested cleanup report and can be finalized after the outer report
  if Batter skipped that hook.
  Rationale: a wrapper `Err` is joined direct work and does not make Batter infer
  hidden-child uncertainty; wrapper abort can instead skip the hook. Both cases
  need durable, inspectable evidence without unsafe pool close.
  Date/Author: 2026-09-11 / Codex.
- Decision: register only a typed control handler in this task. It creates a new
  `OperationContext` from each Runledger execution deadline and uses host-owned
  trusted witness/behavior dependencies. The delivery definition remains
  synchronized but unregistered.
  Rationale: this proves the reusable attempt boundary and worker host without
  fabricating provider success that belongs to `batter-8q8.2`.
  Date/Author: 2026-09-11 / Codex.
- Decision: give the probe preparation caller an explicit witness allowance and
  require both actual handler invocation and persisted `SUCCEEDED` before
  returning the registrable host.
  Rationale: a controlled PostgreSQL rejection after handler return can now prove
  the negative readiness path quickly, while the production root retains its
  documented 20-second allowance.
  Date/Author: 2026-09-11 / Codex.

## Outcomes & Retrospective

Implemented the staged probe-only worker host without adding delivery-provider
behavior. `Startup::without_readiness_approval` separates successful owned
handoff from application admission while preserving the existing default. The
reference root now proves actual control-handler dispatch and durable success,
owns native stop and join, retains original driver failures, and gates separately
driven PostgreSQL cleanup on explicit termination evidence.

Five new hosted-worker live cases plus the converted configured-concurrency case
cover registry omission, failed durable witness, normal/in-flight drain, real
native timeout, retry/attempt accounting and native settings effects. The full
47-case inventory passed on Rust 1.98.1 and 1.94.0; both repository verification
matrices, all ten HTTP process smokes and every connected Jig target passed.
The two implementation defects found by the async audit—an unstarted native
timeout clock and cleanup coupled to a cancellable waiter—have dedicated
regressions. No dependency or Cargo.lock change was needed.

## Context and Orientation

`examples/reference-service/src/runtime.rs` owns the process root. It currently
opens the SQLx pool, applies Runledger plus application migrations, registers the
HTTP server and Unix signals, and automatically becomes ready. Add worker-host
logic in a focused application module and keep Runledger supervision out of the
foundation crate.

`examples/reference-service/src/config/worker.rs` is the only worker constructor:
`WorkerSettings::builder(&PgPool)` passes the validated native `JobsConfig` to
the pinned explicit builder. Do not call an environment builder or reconstruct
settings. `examples/reference-service/src/schema.rs` already synchronizes the
handler-free `records.delivery.execute` definition.

Runledger's build call spawns its internal loops. `run_until_shutdown` consumes
the supervisor, watches every direct loop, requests shutdown when its supplied
future resolves, and applies the selected timeout. On timeout it aborts direct
loops and attempts to drain them for up to `min(timeout, 1 second)` beyond the
cooperative deadline. It returns only the first observed loop failure and logs
later drain failures. The application must preserve that visibility limit.

Batter treats the host as one critical component. That component calls
`mark_started` only after the startup job was claimed, dispatched to the actual
control handler and durably completed. On `ShutdownSignal::draining`, it requests
Runledger shutdown immediately and awaits its driver. It does not supervise or
restart individual Runledger loops.

The termination gate has three states. `NotStarted` means no upstream task can
exist and ordinary partial-startup cleanup is permitted. `Unproven` is armed
immediately before `build`; it remains on panic, timeout, dropped host, join
failure or returned runtime failure. `CooperativelyStopped` is set only after
`run_until_shutdown` returns `Ok(())`. Dependent cleanup closes normally only in
the first or last state; `Unproven` turns its nested `CleanupStack` into an
`UnsafeTaskExit` report without invoking the pool finalizer.

## Plan of Work

First extend `batter::startup::Startup` with an opt-in withheld-approval mode,
keeping `Startup::new` source and behavior compatible. Add a unit/failure-path
test showing successful handoff plus acknowledged components stays `Starting`
until the caller explicitly approves, and document the distinction.

Then add the reference worker module. Define the private control job contract,
typed handler, per-attempt observation, trusted behavior dependencies, catalog
construction, witness submission, hosted driver and termination gate. Build via
`WorkerSettings::builder`, start the consuming driver, observe actual handler and
durable completion, and return one registration object to the root.

Add the application cleanup owner around a nested stack holding the pool close.
The outer cleanup hook calls it during ordinary shutdown; the runtime retains a
second owner and calls it after the outer report so a skipped hook still yields
nested evidence. Return a sanitized composite failure that retains both the
outer shutdown/startup cause and nested report where necessary.

Wire runtime startup in this order: pool acquisition and cleanup ownership,
schema initialization, HTTP bind, Unix signals, then probe catalog
synchronization, witness enqueue, Runledger build/driver/witness, component
registration and successful handoff without application approval. Keeping the
spawning build last avoids fallible asynchronous acquisition after native work
may exist. A delivery row
submitted through the real command path before the host begins must remain
pending because its type is absent from the registry.

Extend the live suite with controlled cases for successful witness and normal
drain, early registry/witness failure, ordinary-job omission, changed concurrency,
retryable then successful control execution with one durable attempt per handler
invocation, drain-versus-in-flight claim classification, and real upstream
shutdown timeout with skipped dependent cleanup and preserved native error.
Prefer acknowledged channels/semaphores and database state over sleeps.

Update `docs/guarantees.md`, `docs/integrations.md`,
`docs/reference-compatibility.md`, `docs/testing.md`, `docs/status.md`, package
guidance and `docs/validation.md`. Update `.jig.toml` and
`.agent/jig-contract.json` together only if a new input root is introduced.

## Concrete Steps

All commands run from `/home/aa/Documents/batter`.

Claim and start the work identity:

    br update batter-0cp --status in_progress --json
    scripts/jig work start --title "Host the upstream worker" \
      --body-file .agent/plans/batter-0cp.md --base d82f5bf --json

Focused development checks:

    cargo test -p batter --lib startup --locked
    cargo test -p batter-example-reference-service --all-targets --all-features --locked
    cargo clippy -p batter-example-reference-service --all-targets --all-features --locked -- -D warnings
    cargo fmt --all --check

Explicit live validation on each supported toolchain:

    RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh

Final repository verification:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
    scripts/jig work check --plan-id <plan-id>
    scripts/jig work evidence --plan-id <plan-id>
    scripts/jig work gates --plan-id <plan-id>

Repeat the build and five smoke profiles with `RUSTUP_TOOLCHAIN=1.94.0`. Record
actual commands, versions, environment prerequisites and outcomes in
`docs/validation.md`; never turn absent live prerequisites into a skip.

## Validation and Acceptance

Successful offline tests prove the new Startup mode does not change existing
automatic approval and that explicit withheld approval separates successful
handoff from admission. Worker unit tests prove fresh independent contexts and
the exact registry contents without requiring PostgreSQL.

The PostgreSQL suite must prove actual control-handler dispatch and durable
success before component acknowledgement; ordinary delivery stays unclaimed;
normal drain joins the upstream driver; changed native concurrency affects held
handler execution; retry produces matching durable attempt and invocation counts;
and a real returned shutdown timeout preserves the upstream error while the pool
finalizer is not invoked and nested skipped cleanup is inspectable.

The final diff must contain no delivery provider implementation, alternate job
queue, per-loop supervision, from-environment worker construction, false
linearized no-claim promise, or automatic cleanup after unproven termination.

## Idempotence and Recovery

Schema and definition synchronization remain safe on repeated startup. Control
job identity must be unique per startup so old terminal jobs cannot satisfy a
new witness. If startup fails before Runledger build, the `NotStarted` gate closes
the pool normally. If build may have spawned, preserve `Unproven`, request native
shutdown when possible, and retain skipped dependent cleanup rather than retrying
construction.

All tests use disposable databases from the existing harness and close owned
drivers before fixtures. If a test reports unproven termination, retain its
fixture owner and diagnostics for the existing recovery path; do not delete or
drop leases to manufacture success. Re-running focused tests and Jig checks is
safe. Tracker mutation is flushed only after meaningful status/notes changes.

## Interfaces and Dependencies

Keep the upstream revisions already pinned in `examples/reference-service/Cargo.toml`.
Use `runledger_runtime::catalog::JobCatalog`,
`runledger_runtime::SupervisorBuilder::with_catalog`,
`runledger_runtime::Supervisor::run_until_shutdown`, and the cloneable native
shutdown handle. Use `JobExecution::deadline` for each fresh
`batter::operation::OperationContext`; never inherit request cancellation.

The reference worker module may expose only the application-owned host
constructor and report types needed by integration tests and `batter-8q8.2`.
Every new public item requires rustdoc and a no-run example or an executable test.
Do not alter Runledger internals or make Batter depend on Runledger.
