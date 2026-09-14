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
- [x] (2026-09-14T20:54:48Z) Slice 2 separated public lifecycle control,
  read-only status, readiness-gated operation admission and component shutdown
  observation from private coordination authority; every workspace consumer and
  contract was migrated without a broad compatibility shim.
- [x] (2026-09-14T20:54:48Z) Slice 2 passed both supported `verify.sh`
  matrices, all ten fresh HTTP process smokes, the historical archive's seven
  tests on both toolchains, and all five current-tree Jig targets. Its fresh
  `api:test` receipt is `receipt_01M2GTNF6KZNW11CMHVHN6KZVV`.
- [x] (2026-09-14T20:54:48Z) Slice 2 converged after four complete
  comprehensive Claude/Codex passes, two ordinary repair rounds and one
  supporting-work closure batch. Both terminal reviewers reported no actionable
  finding or material test gap, and the parent and native reviewer matched final
  complete fingerprint
  `edc0ed55db27bd2ac30f0a57579ecf63386fa798b956c21d33b8c96aa136006f`.
- [x] Slice 2: purpose-qualify lifecycle status, admission, and shutdown control,
  migrate callers, validate, converge the review loop, and commit.
- [x] (2026-09-14T22:53:02Z) Slice 3 replaced clone-wide application readiness
  mutation with a single linear decision paired with lifecycle ownership.
  Ordinary start/run paths approve automatically; deliberately deferred spawned,
  caller-owned and Startup paths expose distinct non-cloneable typestates.
- [x] (2026-09-14T22:53:02Z) Slice 3 repair validation passed the locked full
  workspace test suite, 30 positive and 35 compile-fail foundation doctests,
  warning-denied workspace/foundation Clippy, workspace all-target compilation,
  formatting, diff hygiene and repeated native file-budget checks.
- [x] (2026-09-14T22:53:02Z) Slice 3 converged after three complete
  comprehensive Claude/Codex passes, two ordinary repair rounds and one bounded
  supporting closure with its permitted correction. Both focused terminal
  reviewers marked the abandonment obligations satisfied, reported no remaining
  defect, and matched terminal fingerprint
  `1e8357ff4f3418b8ccfcbcf5207153063748672f2340506e300cd5ec8887a151`.
- [x] Slice 3: replace clone-wide deferred application approval with a one-shot
  capability, migrate callers, validate, converge the review loop, and commit.
- [x] (2026-09-15T01:05:28+02:00) The final source tree passed both supported
  `verify.sh` matrices, all ten toolchain-specific HTTP process profiles, 12
  HTTP example tests and both runnable foundation examples per toolchain, and
  the immutable historical consumer archive on both toolchains.
- [x] (2026-09-15T01:05:28+02:00) All five Jig targets executed successfully on
  the pre-review evidence inputs. The target-validation receipt is
  `receipt_01M2H2N5T82ZJXA0T7EAHCTSPN`; fresh `api:test` receipt
  `receipt_01M2H2N569BC58AS7KNXR44NY7` matches the current worktree and the
  required gate has no unresolved condition.
- [x] (2026-09-15T01:05:28+02:00) The first comprehensive Slice 4 review matched
  complete fingerprint
  `4a43db1f419ebd95ded8f4f167bafe2256d56609341f4bcd68403d5239e05ab0`.
  Both reviewers found no production defect and agreed on one stale tracker
  statement; one reviewer found two additional evidence-scope overstatements.
  The repair reconciles all three and makes the executed-example wording exact.
- [x] (2026-09-15T01:11:37+02:00) The second comprehensive pass matched complete
  fingerprint
  `9b6de5d22bbeb19a97c267b5c662b569d93ce0f594bd8fd72d2bbb1f942bd9fa`.
  Codex found no issue. Claude found the passing whole-repository file-budget
  receipt had become stale after the evidence repair. A work-check refresh
  reused unchanged Rust receipts and reran `repo:contract` and
  `repo:file-budget`; target-validation receipt
  `receipt_01M2H3AANXVB5GH5ZW40SC8T2M` passed.
- [x] (2026-09-15T01:17:28+02:00) The third comprehensive pass matched complete
  fingerprint
  `4389f9a6edc9917fd113294ed2b8dae0da97e2adb24169cf795ef2fb4489dcc2`.
  Both reviewers found no actionable finding or test gap. The Slice 4 loop
  converged after three complete passes and two supporting-only repair rounds;
  its one supporting closure batch is consumed to record convergence before
  focused verification.
- [x] (2026-09-15T01:23:42+02:00) Both focused closure reviewers marked the
  validation and Bead chronology obligations satisfied with no collateral or
  substantive defect. Their reports and the parent matched complete terminal
  closure fingerprint
  `e9f72b6e13e4046c0f75331c523711fa738b95cc457f3498acb2a50df657b30b`.
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

- Observation: historical generated-consumer evidence cannot remain both
  immutable and compiled against an intentionally breaking current workspace
  API.
  Evidence: the archive source records the API introduced by revision
  `034ce0085220044dcf5f3561b00a0bfce96a801f`; pinning both archive dependencies
  to that public revision preserves all seven historical tests on Rust 1.98.1
  and 1.94.0 without rewriting the evidence.

- Observation: operation admission is a concurrent phase decision rather than
  a static token getter.
  Evidence: the deterministic state table and a barrier-controlled race prove
  that admission concurrent with drain yields only rejection or a context that
  remains active through drain and is cancelled by forced shutdown.

- Observation: a caller-owned future cannot both stay pinnable across polls and
  later consume a linear outer readiness typestate if the future itself is pinned
  in place.
  Evidence: the first Slice 3 review caught discarded approval, and the next
  review showed that splitting the capability back out reintroduced detached
  authority. Pinning only the inner exceptional driver leaves the combined outer
  `UnapprovedDriver` movable and consumable after polling.

- Observation: an innocently named standalone constructor can recreate an
  unapprovable admission path even after mutation methods are removed.
  Evidence: review traced `ShutdownHandle::new()` to `Shared::new(false)` with no
  approval token. Removing `new`/`Default` forces callers to choose explicitly
  between permanently Starting `new_unapproved` and paired approval construction.

- Observation: post-drop state assertions do not prove destructor ordering.
  Evidence: the first focused closure verification demonstrated that cancellation
  after captured-value destruction would still satisfy the initial assertions;
  the corrected test observes drain and cancellation from inside those destructors.

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

- Decision: expose `LifecycleStatus`, `OperationAdmission`, and
  `ShutdownSignal` as separate cloneable projections while keeping
  `LifecycleCoordinator` private and `ShutdownHandle` at the composition root.
  Rationale: consumers receive only the observation or downward-cancellation
  authority their role needs; state mutation remains centralized without
  duplicating the lifecycle state machine or introducing typestate that cannot
  model concurrent phase changes.
  Date/Author: 2026-09-14 / Codex.

- Decision: remove public raw process-token extraction and require transient
  root work to enter through `OperationAdmission::admit`.
  Rationale: a freely cloned raw token bypassed readiness and made the admission
  obligation caller-owned. The admitted `OperationContext` preserves the
  established drain-versus-force semantics and parent-bounded deadlines.
  Date/Author: 2026-09-14 / Codex.

- Decision: pin the historical `batter-gi4` consumer archive to the immutable
  revision that created it instead of porting the archived source.
  Rationale: rewriting generated evidence would erase its historical claim;
  current-version compatibility remains covered by live workspace examples and
  tests rather than this explicitly historical oracle.
  Date/Author: 2026-09-14 / Codex.

- Decision: make ordinary `Supervisor::start`, `Supervisor::run_until`, and
  successful Startup handoff consume application approval automatically, while
  explicitly named unapproved methods and builder types retain the sole pending
  decision.
  Rationale: the canonical agent-generated path has no coordination instruction
  to forget, while exceptional delayed policy remains visible in the type and
  method name and cannot be cloned or repeated.
  Date/Author: 2026-09-14 / Codex.

- Decision: represent caller-owned delayed approval as `UnapprovedDriver` with a
  pinned boxed inner driver and a movable, consuming outer typestate; do not expose
  an `into_parts` capability split.
  Rationale: policy can poll before deciding without ever representing approval
  authority detached from its driver. Only the exceptional caller-owned path pays
  the allocation; ordinary `run_until` retains its allocation-free wrapper.
  Date/Author: 2026-09-14 / Codex.

- Decision: remove `ShutdownHandle::new` and `Default`; retain an explicitly named
  `new_unapproved` constructor and the paired standalone constructor.
  Rationale: migration cannot silently produce an admission source that remains
  Starting forever. Cancellation-only uses remain possible but must state their
  permanent lack of readiness approval.
  Date/Author: 2026-09-14 / Codex.

## Outcomes & Retrospective

Slices 1 and 2 are implemented, validated and independently converged. Slice 1's
first review
exposed an internal cross-lifecycle pairing still representable by separate
registration and spawn inputs; the durable repair carries one complete
`ComponentStartup` from registration into task execution. Its second review
identified only evidence and documentation obligations, which were completed
with full supported-toolchain, process-smoke, Jig, rustdoc, and drop-behavior
coverage. Slice 2 removes ambient root authority from policies, probes,
components and transient-operation entrypoints. Its review rounds made the
private coordinator boundary explicit, closed race and compile-fail gaps, and
preserved the historical consumer archive through an immutable dependency pin.
The terminal reviewers found no actionable issue. Slice 3 removes clone-wide
application readiness mutation, pairs the sole decision with spawned,
caller-owned or startup handoff ownership, and makes the ordinary agent path
automatic. Review exposed and eliminated both a permanently unapprovable
caller-owned driver and a detached capability escape hatch. The final slice now
owns final-tree two-toolchain, process-smoke, Jig and tracker evidence.

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
