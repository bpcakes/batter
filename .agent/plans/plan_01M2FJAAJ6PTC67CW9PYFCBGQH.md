# Make operational configuration purpose-qualified

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` current while implementing it.
Maintain it under `.agent/PLANS.md`. The owning delivery record is Bead
`batter-k8m`; Beads owns scope, acceptance, priority, status, and dependencies.

## Purpose / Big Picture

After this change, configuration that is valid only for an offline database
maintenance command cannot be passed to the serving runtime or HTTP router.
Serving configuration is parsed into a concrete serving type whose fields are
already qualified for serving: a password-bearing PostgreSQL endpoint, a bearer
authenticator, a validated native worker configuration, and validated operational
budgets and capacities. The application then consumes that value to create one
inert, non-cloneable `PreparedServing` owner before any listener, connection, or
task starts. `runtime::run` accepts only that prepared owner. Offline maintenance
uses a separate type and preparation path that intentionally accepts local
trust-authenticated PostgreSQL without acquiring serving capabilities.

The flaw surfaced in the unpublished reference service, but the reusable cause is
also addressed in the owning Batter boundaries. `BulkheadCapacity`,
`ProcessCapacity`, and the Axum adapter's `ResponseConstructionBudget` retain the
fact that raw values passed validation. Operational constructors consume those
witnesses without repeating validation or accepting interchangeable `usize` and
`Duration` values. This is a direct pre-production cutover. It does not create a
generic configuration framework, move application schemas into Batter, or claim
that types validate remote PostgreSQL or worker effects.

The visible result is demonstrated by public rustdoc, normal configuration tests,
compile-fail type checks, the runnable HTTP example, the reference runtime, and the
existing retirement and live-suite entrypoints. A missing serving password,
authentication value, worker identity, or operational bound fails during parsing
or inert preparation, before startup can acquire a pool, migrate a schema, bind a
socket, or register work.

## Progress

- [x] (2026-09-14) Preserved the pre-existing Beads review record in baseline
  commit `3b4364f` and confirmed the checkout was clean.
- [x] (2026-09-14) Opened and claimed P1 bug `batter-k8m`; linked it to the
  historical typed-configuration delivery and made `batter-lp2.4` depend on the
  cutover because that task's old `RootSettings` instruction would preserve the
  defect.
- [x] (2026-09-14) Started Jig work as
  `plan_01M2FJAAJ6PTC67CW9PYFCBGQH` against baseline `3b4364f`.
- [ ] Slice 1: add Batter- and Axum-owned validated capability types, migrate all
  workspace consumers, update their contracts/examples/tests, validate narrowly,
  and converge the comprehensive low-severity review/fix loop before committing.
- [ ] Slice 2: replace `ConfigMode`/`RootSettings` with purpose-specific settings
  and inert prepared serving/maintenance owners; narrow runtime/router/pool/worker
  boundaries, migrate every reference consumer, validate, review to convergence,
  and commit.
- [ ] Slice 3: complete cross-cutting contracts, status, validation evidence,
  compile-fail/failure-path coverage, and fresh consumer assessment; run final
  two-toolchain, HTTP, live-suite/Jig verification, review to convergence, and
  commit.
- [ ] Close `batter-k8m` only after all acceptance evidence is delivered. Do not
  publish or push.

## Surprises & Discoveries

- Observation: the reference parser validates `ConfigMode::Serve` more strictly
  than `ConfigMode::Setup`, but both branches return cloneable `RootSettings`.
  Evidence: `examples/reference-service/src/config.rs` stores optional
  authentication and optional worker identity, while
  `examples/reference-service/src/runtime.rs::run` accepts the erased root type.
- Observation: router construction currently provides a late authentication
  failure, after the runtime has already begun PostgreSQL acquisition and schema
  initialization.
  Evidence: `runtime.rs` calls `register_pool` and `initialize_schema` before
  `http::router`; `RouterBuildError::Authentication` reconstructs the missing
  serving requirement.
- Observation: the foundation already uses inert ownership transfer for startup
  and native supervisors, so application preparation can follow an established
  contract instead of introducing a new framework.
  Evidence: `StartingSupervisor`, Runledger `PreparedSupervisor`, cleanup
  reservations, and their compile-fail/runtime tests.
- Observation: the raw capacity and duration checks are owned by the operational
  constructors today. Application settings therefore validate by constructing and
  discarding a dummy `RequestPolicy`, while raw values remain interchangeable until
  later calls.
  Evidence: `RootSettings::from_sources`, `Bulkhead::new`,
  `Supervisor::with_process_capacity`, and `RequestPolicy::new`.

## Decision Log

- Decision: use concrete purpose types rather than `Validated<T, Mode>`, const
  generic modes, marker traits, or a schema DSL.
  Rationale: the serving and maintenance command capabilities are materially
  different. Concrete structs make the permitted operations discoverable and
  avoid pushing a generic typestate protocol onto coding-agent consumers.
- Decision: keep source overlay, field names, defaults, secret requirements, and
  PostgreSQL URL policy in the reference application. Batter owns only reusable
  operational value witnesses and the existing source/bounds/redaction mechanics.
  Rationale: this preserves application -> adapter -> native dependency direction
  and the established `batter::settings` boundary.
- Decision: make `Bulkhead::new`, `Supervisor::with_process_capacity`, and
  `RequestPolicy::new` consume opaque validated values in a coordinated direct
  cutover. Do not retain raw overloads or implicit conversions under equivalent
  names.
  Rationale: unpublished packages and agent-only consumption make the breaking
  change cheap; duplicate raw paths would leave the impossible state and confuse
  the canonical path.
- Decision: expose `new(raw) -> Result<witness, ConfigurationError>` on each
  witness, plus only the traits needed for ordinary ownership (`Copy`, `Clone`,
  `Debug`, equality). Keep the raw field private and do not expose unchecked
  constructors.
  Rationale: parsing can validate once, operational construction becomes
  infallible, and the type itself carries the boundary proof.
- Decision: serving settings contain native `JobsConfig`, not a worker wrapper
  whose identity remains optional. Offline maintenance never parses authentication
  or worker fields.
  Rationale: validation should remove optionality rather than promise a later
  fallible conversion.
- Decision: preparation may read SQLx's ambient PG defaults only at the explicit
  `from_process`/preparation boundary and must reject supported `PG*` ambiguity as
  today. It constructs `PgConnectOptions`, `PgPoolOptions`, `Supervisor`, and the
  prepared HTTP policy without starting async work.
  Rationale: SQLx 0.9 native construction has ambient behavior, while lazy pool
  construction, listener binding, migration, and task registration belong inside
  owned startup.
- Decision: `PreparedServing` is `#[must_use]`, non-`Clone`, and consumed by
  `runtime::run`. `PreparedHttp` is a narrower opaque value consumed by the router.
  A maintenance preparation value has no conversion into either type.
  Rationale: consumption records ownership transfer and prevents a broad settings
  aggregate from crossing serving sub-boundaries.
- Decision: review each slice in working-tree scope with comprehensive repair,
  inclusive low severity, Claude Opus plus native Codex, at most four ordinary
  rounds, and the protocol's one supporting-only closure allowance. Commit only a
  converged slice.
  Rationale: this is the user's explicit delivery constraint.

## Context and Orientation

`crates/batter/src/admission.rs` owns `Bulkhead` and its Tokio semaphore limit.
`crates/batter/src/lifecycle/process.rs` constructs the finite-process channel and
semaphore; `crates/batter/src/lifecycle.rs` exposes the supervisor constructor.
`crates/batter-axum/src/lib.rs` owns the response-construction deadline held by
`RequestPolicy`. Their tests and runnable examples contain the current raw
constructor calls and must all migrate together.

`examples/reference-service/src/config.rs` currently defines `ConfigMode` and
`RootSettings`. Its `endpoint`, `pool`, and `worker` submodules parse PostgreSQL,
SQLx pool, and Runledger settings. `src/runtime.rs` owns startup; `src/http.rs`
owns router assembly; `src/main.rs` is the serving command.
`src/retirement.rs`, `examples/retire_startup_controls.rs`, configuration tests,
and live-test support use the permissive setup path. These are application
consumers of the new command-specific types, not reusable schema owners.

The foundation contract is documented in `docs/guarantees.md`; architecture and
integration placement are in `docs/architecture.md` and `docs/integrations.md`;
implemented state is in `docs/status.md`; exact verification belongs in
`docs/validation.md`. `docs/testing.md` defines the five HTTP smoke profiles and
the reference live suite. `examples/reference-service/AGENTS.md` and the crate
guides route future agents to the protected path.

## Milestone 1: Retain reusable operational validation witnesses

Add public `BulkheadCapacity` beside `Bulkhead`. Its checked constructor rejects
zero and values above `tokio::sync::Semaphore::MAX_PERMITS`; `Bulkhead::new`
accepts the witness and returns `Bulkhead` directly. Add public `ProcessCapacity`
in the lifecycle process module with the same finite-process field diagnostics;
re-export it from `batter::lifecycle`. `Supervisor::with_process_capacity`
accepts it and returns `Supervisor` directly, while private `ProcessHandle::new`
also consumes it. Add public `ResponseConstructionBudget` in `batter-axum` using
the existing nonzero, one-year/`Instant::checked_add` rules; `RequestPolicy::new`
accepts it and returns `RequestPolicy` directly.

Give every new public type rustdoc showing checked creation followed by its
canonical operational constructor. Update existing API examples and all package,
test, and runnable-example call sites. Add boundary tests for zero, maximum,
first-over-maximum, and valid handoff. Tests must show constructor handoff is
infallible after a witness exists; do not merely rename the old fallible path.
Update `docs/guarantees.md`, `docs/integrations.md`, `docs/status.md`, and owning
crate guidance where the public contract changes.

Run focused tests and doctests for `batter` and `batter-axum`, plus relevant
reference configuration compilation. Then run the normalized review/fix loop on
the complete working-tree slice. Fingerprint before and after each read-only pass;
triage every low-or-higher report, apply comprehensive causal repairs and
proportional prevention, rerun affected checks, and obtain a fresh complete pass.
If it does not converge within the pinned protocol, stop and report the retained
working state. Otherwise update this plan and the Bead, then commit the slice.

Milestone 1 acceptance is observable when no public operational constructor in
scope accepts a raw interchangeable capacity/budget, every current consumer
compiles, invalid raw values fail only while constructing the witness, and both
selected reviewers return a complete finding-free final pass (or supporting-only
closure completes under its distinct allowance).

## Milestone 2: Split command schemas and consume inert preparation

Replace `ConfigMode` and `RootSettings` with concrete settings types. The serving
loader recognizes the full serving schema and requires all serving credentials.
It stores a password-qualified endpoint, a concrete `BearerAuthenticator`, a
native validated `JobsConfig`, `BulkheadCapacity`, `ProcessCapacity`,
`ResponseConstructionBudget`, bind address, and pool settings. The maintenance
loader recognizes only the database fields needed by offline consumers, permits a
missing or empty password, and cannot expose HTTP, worker, bind, or supervisor
construction.

Keep common overlay/parsing functions private. Do not parse a permissive superset
and discard irrelevant fields: dedicated files and overrides reject fields outside
their command schema, while captured environment retains the documented reserved
prefix policy. Split the endpoint representation so only the serving endpoint can
be created with a nonempty password. Preserve redacted `Debug`/`Display` on every
aggregate and the existing explicit native-cause limits.

Create opaque prepared values. `ServingSettings::prepare` consumes the serving
settings and fixed shutdown budget input, constructs inert SQLx options, supervisor,
and `PreparedHttp`, and returns `PreparedServing`. `PreparedServing` owns the bind
address and native jobs input as well as those values; it is `#[must_use]` and not
cloneable. Provide only narrow consuming decomposition required within the package.
`ServingSettings::prepare_http` may exist as a consuming or borrowing helper only
if external router tests need the public application boundary; it must return an
opaque `PreparedHttp`, never raw optional fields. Maintenance has its own inert
native endpoint preparation and no conversion/promotion into serving.

Change `runtime::run` to accept and consume `PreparedServing`. Pool registration
accepts prepared SQLx inputs. Router construction accepts `PreparedHttp` by value,
so `RouterBuildError::Authentication` and configuration branches disappear; make
router creation infallible unless a remaining operation genuinely can fail.
Worker registration consumes the already validated `JobsConfig`. Keep listener
binding, pool connectivity, schema work, and task registration inside protected
startup, and retain current readiness withholding and cleanup order.

Migrate the serving binary, retirement command, live preflight, fixture support,
configuration diagnostics, HTTP/delivery tests, and startup failure tests. Add
compile-fail rustdoc proving maintenance values cannot call `runtime::run` and
cannot construct the production router. Add executable tests that serving
requirements fail before preparation, maintenance accepts passwordless endpoints,
serve-shaped maintenance inputs cannot promote the type, preparation starts no
Tokio tasks or sockets, and prepared router construction has no stored-config
failure. Preserve existing native-option and secret-redaction tests.

Run focused package tests and compile all reference targets before the second
working-tree review/fix loop. Apply the same convergence rule and commit only after
the final verified pass.

## Milestone 3: Contract and evidence closeout

Audit every public/documented use of `ConfigMode`, `RootSettings`, and the old raw
constructors. Update architecture, guarantees, integrations, operations, testing,
status, reference compatibility, README/example guidance, rustdoc, and source maps
to describe concrete command capabilities and honest limits. Record this repair as
a foundation/adapter design response to an example-contract symptom under ADR-010.
No text may claim that local validation proves database authentication, remote
availability, migration success, or worker execution.

Run a fresh agent implementation/modification assessment using only the public
requirements and guidance if the available workflow can provide it; label any
unexecuted assessment explicitly. Update `.jig.toml` and
`.agent/jig-contract.json` together only if new source roots or fixtures expand an
exhaustive Jig input scope.

Inspect Jig evidence and gates before rerunning expensive checks. Execute focused
format/Clippy/tests as needed, then both complete matrices:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

For each toolchain, rebuild the Axum example and run all five profiles against that
binary:

    cargo build -p batter-axum --example http_service --locked
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
    python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline

Run `bash scripts/test_reference_live.sh` on both toolchains only with the two
documented externally provisioned PostgreSQL 18 endpoints. If prerequisites are
absent, run its read-only preflight/compile inventory as documented and record the
failure honestly; do not provision a database or call a missing endpoint success.
Run `scripts/jig work check --plan-id <id>`, inspect `work evidence` and `work
gates`, and satisfy current `api:test` evidence under the freshness rules.

Run the final working-tree review/fix loop over the complete closeout slice. After
convergence, update validation chronology, plan outcomes, and the Bead; close the
Bead and commit the slice. Confirm the checkout is clean. Do not push or publish.

## Validation and Acceptance

The implementation is accepted only when these facts hold together:

1. Raw invalid values cannot be passed to Batter's three changed operational
   constructors; checked witnesses are required and their handoff is infallible.
2. No public type returned by maintenance parsing implements an implicit or
   explicit conversion to serving settings, prepared serving, or prepared HTTP.
3. `runtime::run` has no overload for settings or maintenance values and consumes
   the prepared serving owner.
4. Serving parsing stores no optional authentication/worker/password requirement,
   and router/startup do not rediscover those omissions.
5. Preparation is inert and all actual acquisition remains under owned startup and
   registered cleanup.
6. Compile-fail, boundary, failure-path, redaction, startup, retirement, HTTP, and
   live inventory tests prove the relevant behavior without weakening old oracles.
7. Each slice has a recorded complete comprehensive review at low severity and its
   own commit; final matrices, smokes, Jig evidence, and available live evidence
   are current and accurately documented.

## Idempotence and Recovery

All source edits are ordinary text changes and may be reapplied after inspecting
the current diff. Cargo formatting and checks may update only ordinary build output
outside the tracked source tree. Do not remove or overwrite concurrent files. Keep
Beads exports and Jig state append-only through their supported commands.

Review passes are read-only and use a pinned working-tree scope fingerprint. If a
fingerprint changes outside recorded repairs or validation artifacts, stop as
`scope changed`. If validation exposes a repair-caused defect, use only the bounded
mechanical/behavioral corrections and counted recovery permitted by the review
runtime. Preserve every failed attempt and do not weaken tests to obtain green.

Commits are slice checkpoints explicitly authorized by the user. Before each
commit, review the exact diff and confirm convergence. If a slice cannot converge,
leave its changes uncommitted and report the reviewer ledger, failing checks, and
last verified scope. Never reset the checkout, discard pre-existing work, push, or
publish.

## Outcomes & Retrospective

Not yet complete. Record delivered APIs, causal repair, prevention evidence,
review rounds, exact verification outcomes, live/platform limits, commits, and
remaining work here as slices finish.
