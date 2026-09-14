# Make lifecycle capabilities unrepresentable by construction

This ExecPlan is a living document. The sections `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must remain current
as implementation proceeds. Maintain this file according to `.agent/PLANS.md`.
The owning delivery record is Bead `batter-lp2.5`. The exact Git baseline is
`150d16df364379b4ae64a5af4eb82a68b08a0ff0`.

## Purpose / Big Picture

Batter currently gives both ordinary shutdown observers and registered critical
components the same public `ShutdownSignal` type. Only a privately modified
instance can acknowledge component startup, so whether `mark_started()` works is
hidden provenance represented by `Option<Arc<AtomicBool>>` and a Boolean return.
An agent can therefore write code that compiles while attempting an unauthorized
or repeated acknowledgement, and review must rediscover the mistake.

After this work, `ShutdownSignal` only observes drain and forced cancellation. A
registered critical component receives one non-cloneable `ComponentStartup` and
consumes it to acknowledge actual initialization, receiving the ordinary
`ShutdownSignal` for its running phase. Successful registration creates the one
pending acknowledgement token stored with that component; spawning no longer
mutates a read-only signal. Purpose-qualified lifecycle status, admission, and
application-approval capabilities prevent adapters from retaining root control
they do not use. Canonical examples demonstrate these boundaries, compile-fail
tests reject invalid compositions, and existing runtime tests continue to prove
readiness, drain, cancellation, ownership, and cleanup behavior.

The work is delivered in independently committed slices. Each slice leaves a
coherent working tree, receives a working-tree `review-fix-loop` with
`--fix-mode comprehensive --min-severity low`, and is committed only after that
loop converges. The loop uses its default Claude and Codex reviewers, four
ordinary repair rounds, and one separate supporting-work closure allowance.

## Progress

- [x] (2026-09-14T18:31:22Z) Confirmed the Git checkout was clean at the exact
  baseline, so no preservation commit was necessary.
- [x] (2026-09-14T18:31:22Z) Created and claimed Bead `batter-lp2.5` and made it
  block `batter-tmx.1`, preventing facade extraction from freezing the old API.
- [x] (2026-09-14T18:31:53Z) Started Jig plan
  `plan_01M2GK0TJJSGNBFPNG0SEADTNF` and recorded the implementation and review
  contract in this ExecPlan.
- [x] (2026-09-14T18:46:00Z) Implemented the vertical component-start
  capability cutover, migrated every workspace consumer, removed all legacy
  `mark_started` calls, and updated the owning contracts and test inventory.
- [x] (2026-09-14T18:48:00Z) Slice 1 passed locked workspace tests including
  doctests and compile-fail controls, all-target compilation, formatting, diff
  hygiene, and warning-denied workspace Clippy on Rust 1.98.1.
- [x] (2026-09-14T20:17:00Z) Slice 1 passed both supported `verify.sh`
  matrices, all ten HTTP process smokes, and the current-tree Jig verify gate;
  its final `api:test` receipt is `receipt_01M2GNWWTSBKT8E21703XDB0TC`.
- [x] (2026-09-14T20:25:00Z) Slice 1 converged after three complete
  comprehensive Claude/Codex passes and two repair rounds. Both terminal
  reviewers reported no actionable finding or test gap, and every pre/post
  check matched complete fingerprint `457d67fc0389b2b6b3960a107c6030df820dfa1cc87a051b1f2ab0876f9b78bf`.
- [x] Slice 1: pair component registration with one linear startup capability,
  migrate every workspace caller, validate, converge the review loop, and commit.
- [ ] Slice 2: purpose-qualify lifecycle status, admission, and shutdown control,
  migrate callers, validate, converge the review loop, and commit.
- [ ] Slice 3: replace clone-wide deferred application approval with a one-shot
  capability, migrate callers, validate, converge the review loop, and commit.
- [ ] Slice 4: update contracts, status, examples, exhaustive Jig scopes and final
  evidence; run the full two-toolchain and HTTP verification, converge, and commit.
- [ ] Complete the Bead and Jig work only after a requirement-by-requirement audit
  proves every requested slice, review, commit, contract, and validation exists.

## Surprises & Discoveries

- Observation: the current runtime accounting itself is correct despite the
  public invalid state.
  Evidence: `Shared::mark_started` serializes the pending-count decrement, and
  the focused `readiness_waits_for_every_component_acknowledgement` regression
  passed at the baseline.

- Observation: the broad handle problem extends beyond startup acknowledgement.
  Evidence: Axum `RequestPolicy` and `ReadinessPolicy` retain
  `ShutdownHandle` although they only read readiness and derive downward
  cancellation; managed lifecycle code recovers root control through the private
  handle stored inside `ShutdownSignal`.

- Observation: a foundation-only public API commit would leave adapters and
  examples uncompilable.
  Evidence: the first workspace check failed in `batter-axum::serving` because
  its registered factory received `ComponentStartup`, while the planned adapter
  migration was still assigned to the next commit.

- Observation: storing only the private pending-start token in `Component` left
  an internal cross-lifecycle state representable because task spawning accepted
  a separate shutdown handle.
  Evidence: the first Codex review demonstrated that safe internal code could
  acknowledge supervisor A while returning supervisor B's observer. The repair
  stores the complete paired `ComponentStartup` at registration and removes the
  spawn-time handle parameter.

- Observation: the type-level cutover needed process-level evidence, not only
  unit and doctest evidence, because Axum serving acknowledges startup through
  the same changed boundary.
  Evidence: both toolchains passed all five HTTP smoke modes after fresh example
  builds, and the second review's requested drop-without-acknowledgement scenario
  now has a dedicated process-ownership regression.

## Decision Log

- Decision: perform a coordinated breaking cutover with no deprecated
  `ShutdownSignal::mark_started` shim or source-compatible alias.
  Rationale: every package is unpublished, migrations are pending, and retaining
  the method would preserve precisely the invalid state this delivery removes.
  Date/Author: 2026-09-14 / Codex.

- Decision: use a non-cloneable `ComponentStartup` whose
  `acknowledge_started(self)` operation returns `ShutdownSignal`; keep the
  underlying pending token private.
  Rationale: ownership makes repeated acknowledgement a compile error while the
  returned observer supports the component's normal running and drain phases.
  A public freely separable acknowledgement token would enlarge the API and make
  the obligation easier to lose.
  Date/Author: 2026-09-14 / Codex.

- Decision: registration, not task spawning, creates the pending-start token and
  stores it in the registered component.
  Rationale: one accepted registration and one pending acknowledgement then share
  structural ownership instead of relying on a counter increment followed by a
  later unrelated `Option` mutation.
  Date/Author: 2026-09-14 / Codex.

- Decision: preserve late acknowledgement during drain as an accepted component
  event that cannot revive readiness.
  Rationale: this is the current race contract. The capability proves authority
  and exactly-once use; it does not claim the process is still eligible to become
  ready.
  Date/Author: 2026-09-14 / Codex.

- Decision: keep concurrent lifecycle facts in the private mutex-protected state
  machine rather than introducing generic typestate for every runtime phase.
  Rationale: the demonstrated defect is authority distribution. The centralized
  state writer already represents necessary concurrent combinations and has
  transition and scheduling evidence.
  Date/Author: 2026-09-14 / Codex.

- Decision: deliver source, adapter, authority-hardening, and evidence slices
  separately, running the requested comprehensive low-severity review/fix loop
  on each working-tree diff before committing it.
  Rationale: this preserves reviewable failure boundaries while still reaching
  the systemic end state rather than a compatibility-oriented partial remedy.
  Date/Author: 2026-09-14 / Codex.

- Decision: make Slice 1 a vertical component-start capability cutover across
  the complete workspace, then separate status/control and application-approval
  authority into Slices 2 and 3.
  Rationale: every committed slice must compile as a coherent workspace before
  it can honestly converge. A package-only source cutover would intentionally
  commit broken adapters and examples.
  Date/Author: 2026-09-14 / Codex.

- Decision: carry `ComponentStartup` intact from registration through direct and
  managed task construction; derive task-exit classification from its paired
  private observer.
  Rationale: the representation must prohibit cross-lifecycle pairing inside the
  foundation too, not merely hide that pairing from public callers.
  Date/Author: 2026-09-14 / Codex.

## Outcomes & Retrospective

Slice 1 is implemented, validated, and independently converged. Its first review
exposed an internal cross-lifecycle pairing still representable by separate
registration and spawn inputs; the durable repair carries one complete
`ComponentStartup` from registration into task execution. Its second review
identified only evidence and documentation obligations, which were completed
with full supported-toolchain, process-smoke, Jig, rustdoc, and drop-behavior
coverage. The terminal review found no actionable issue. The broader capability
cutover remains in progress through Slices 2-4.

## Context and Orientation

`crates/batter/src/lifecycle.rs` defines public lifecycle handles, component
registration, the supervisor, and critical component factories.
`crates/batter/src/lifecycle/state.rs` is the sole writer of readiness and
admission facts. `crates/batter/src/lifecycle/tasks.rs` turns registered
components into Tokio tasks. At the baseline, registration increments
`pending_startups`, while task spawning later constructs an atomic flag and puts
it into an optional field on `ShutdownSignal`.

`crates/batter/src/registration.rs` exposes constrained registration to protected
startup and supported adapters. `crates/batter/src/health.rs` has both a canonical
registered path and a lower-level standalone driver; the standalone path must
receive shutdown observation without startup authority. The managed runtime under
`crates/batter/src/lifecycle/managed/` acknowledges a native initialization
witness and also needs explicit private shutdown-control and stop-clock authority.
It must not recover those mutations from a value documented as read-only.

`crates/batter-axum/src/serving.rs` registers a bound listener as a critical
component. `crates/batter-axum/src/lib.rs` and `readiness.rs` currently retain the
broad lifecycle handle for admission and probe decisions. Foundation, Axum,
PostgreSQL lifecycle, and reference-service examples are consumer contracts and
must compile against the final public API. `docs/guarantees.md` owns behavior,
`docs/architecture.md` owns the boundary description, `docs/usage.md` owns the
canonical consumer path, `docs/status.md` owns implemented facts, and
`docs/validation.md` owns executed evidence.

A capability in this plan is an ordinary Rust value whose methods grant a
specific authority. A linear capability is non-cloneable and consumed by the
operation it authorizes. These types do not prove that an application truthfully
identified initialization, eventually acknowledges, yields to Tokio, or joins
hidden descendants. Existing runtime contracts and failure tests continue to own
those claims.

## Plan of Work

Slice 1 changes the component boundary across the whole workspace. Add a public non-cloneable
`ComponentStartup` and make `ShutdownSignal` a cloneable observer with no startup
field or acknowledgement method. Add a private pending-start token created by
successful registration and stored directly in `Component`. Consuming
`ComponentStartup::acknowledge_started` decrements the pending count exactly once
and returns its observer. Change `Supervisor::register`, reserved registration,
and constrained `Registration::register` factories to accept
`ComponentStartup`. Migrate all foundation-owned registered components, managed
initialization, health registration, Unix signals, Axum serving, SQLx coverage,
workspace examples, fixtures, and tests. Add compile-fail rustdoc proving that a
read-only signal cannot acknowledge, the startup capability cannot clone, and
acknowledgement cannot repeat. Preserve race and missing-ack runtime tests. Run
focused tests and workspace Clippy, then the requested review/fix loop and commit
only after convergence.

Slice 2 removes adjacent ambient status and control authority. Introduce narrow public
status and admission projections used by readiness and request policies, keep
shutdown requests on an owner/control capability. The exact final names may be
refined during compilation, but the end state must ensure that read-only policies
cannot request shutdown or approve readiness.
Managed internals receive explicit private coordinator authority rather than
reaching through `ShutdownSignal`. Preserve direct-supervisor composition without
reintroducing a framework, DI container, or opaque DSL. Migrate all callers, run
focused lifecycle/Axum/reference tests, converge, and commit.

Slice 3 replaces clone-wide Boolean application approval with a one-shot,
purpose-qualified approval path for staged startup. Unauthorized or repeated
approval must be rejected structurally while direct-supervisor composition stays
available through an explicit owner capability. Migrate all callers, preserve
readiness races and deferred approval behavior, run focused tests, converge, and
commit.

Slice 4 updates every owning contract and evidence surface. Update public rustdoc,
README and usage examples, architecture, guarantees, implemented status, testing
inventory where test names or counts change, validation evidence, Bead details,
and both `.jig.toml` and `.agent/jig-contract.json` if any exhaustive input scope
changes. Run both supported Rust matrices, build and execute every required HTTP
smoke profile, inspect Jig evidence/gates, run the final Jig work check, and run
the slice review/fix loop. Commit only after convergence. Finally audit every
acceptance criterion, close the Bead, flush the tracker export, finish the Jig
plan, and commit the tracker/evidence closure if that closure itself converges.

## Concrete Steps

Work from `/Users/aa/Documents/batter`. Before each slice, require `git status`
to show only the plan/tracker state intentionally carried into that slice. Use
`apply_patch` for source and documentation edits. Format with Cargo rather than
manual bulk rewrites.

For each slice, validate narrowly first. Representative commands are:

    cargo test -p batter --locked
    cargo test -p batter --doc --locked
    cargo clippy -p batter --all-targets --locked -- -D warnings
    cargo test -p batter-axum --locked
    cargo test -p batter-axum --doc --locked

After validation, normalize and run the review loop with:

    node scripts/loop-options.mjs --fix-mode comprehensive --min-severity low --scope working-tree

from the installed `review-fix-loop` skill directory. Capture a working-tree
scope fingerprint, launch fresh isolated Claude and Codex reviewers concurrently,
triage and repair every verified low-or-higher finding within the bounded runtime,
validate repairs, and repeat until the skill reports `converged`. Stop and report
the exact ledger if it does not converge. Commit the converged slice with a
specific message; never push, publish, or deploy.

For final verification, run the repository-required commands:

    bash scripts/verify.sh
    RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

Then freshly build the HTTP example on each fixed toolchain and execute every
smoke profile described in `docs/testing.md`. Inspect `scripts/jig work evidence`
and `scripts/jig work gates` for this plan before rerunning expensive gates; run
`scripts/jig work check --plan-id plan_01M2GK0TJJSGNBFPNG0SEADTNF` with the
required freshness handling and require a final successful `api:test` receipt.

## Validation and Acceptance

Compilation must reject calling startup acknowledgement on a value from
`ShutdownHandle::signal` or `ProcessScope::signal`, cloning the one-shot component
startup capability, acknowledging it twice, and using read-only status/admission
views for root mutations. Positive doctests must show delayed initialization,
one acknowledgement, and continued drain observation.

Runtime tests must show that application approval, a running driver, and every
registered component acknowledgement are all still required for `Ready`; a
component that exits or remains pending without acknowledgement cannot admit root
work; acknowledgement racing drain cannot revive readiness; managed native
initialization acknowledges only after its existing final-poll, destruction, and
context checks; standalone health never changes component readiness; and drain,
forced cancellation, task failures, retained reports, and conservative cleanup
retain their prior behavior.

The complete workspace must pass Rust 1.98.1 and 1.94.0 verification, required
doctests and Clippy, all HTTP smoke profiles, the Jig contract and file budget,
and every selected review loop. Each loop must have complete same-fingerprint
Claude and Codex evidence and no outstanding low-or-higher substantive or
supporting finding before its slice commit.

## Idempotence and Recovery

All source edits and tests are local and repeatable. Never reset or discard
working files. If validation creates a generated file, retain it only when the
repository contract owns it and include it in the next review fingerprint. If a
review fingerprint changes unexpectedly, stop as `scope changed` and preserve
the complete working state. If a review does not converge within its allowed
rounds, do not commit that slice; report the ledger and leave the changes for
inspection. If a commit succeeds but a later slice fails, preserve all earlier
commits and continue from the last converged boundary rather than rewriting
history.

## Interfaces and Dependencies

At the end of Slice 1, `batter::lifecycle::ShutdownSignal` is cloneable and offers
only drain/cancel observation. `batter::lifecycle::ComponentStartup` is public,
non-cloneable, has a read-only shutdown accessor, and consumes itself through an
acknowledgement method returning `ShutdownSignal`. All ordinary component
registration functions accept factories over `ComponentStartup`.

At the end of Slice 3, lifecycle readiness/status and request admission consumers
accept only narrow read/downward-cancellation capabilities. Root shutdown request
and deferred application-readiness approval remain purpose-qualified and are not
recoverable from those views. The private shared state remains the sole transition
writer. No new normal dependency, allocation-heavy abstraction, global state,
application framework, Windows path, or adapter-to-foundation reverse dependency
is introduced.

Plan revision note, 2026-09-14: expanded the Jig-created stub into the complete
four-slice implementation, review, commit, and verification plan required by
Bead `batter-lp2.5` and the user-authorized delivery.
