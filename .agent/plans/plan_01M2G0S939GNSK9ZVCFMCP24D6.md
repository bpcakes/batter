# Compile PostgreSQL verification policies before execution

This ExecPlan is a living document. The sections `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept current
as implementation proceeds. Maintain this file according to `.agent/PLANS.md`.
The owning delivery record is Bead `batter-310`. The exact Git baseline is
`8da44f97892021b4a538b1de9e8ed743dd056cf1`.

## Purpose / Big Picture

An application must be able to construct and retain a PostgreSQL verification
policy knowing that every later execution can structurally succeed. At the
baseline, public `AuthorityPolicy`, `MigrationPolicy`, and `VerificationPolicy`
values can contain invalid object/privilege combinations, duplicate or
contradictory declarations, oversized input, and invalid migration checksums.
The operation discovers many of those failures only after its future is polled.
`VerificationRequest::new()` and `Default` additionally construct a request that
is always rejected. This makes deterministic configuration validity compete with
runtime cancellation and startup deadlines.

After this work, potentially incomplete construction happens only in a pure
builder. `AuthorityPolicy` and `MigrationPolicy` are opaque, immutable, compiled
values. A non-empty `VerificationPlan` combines already compiled components and
is the sole input to the shared executor. Applications observe policy errors at
construction and only database, report-capacity, rollback, or interruption
errors while executing. The runnable verification example and the public
documentation demonstrate the new boundary.

## Progress

- [x] (2026-09-14T13:12:40Z) Created and claimed Bead `batter-310` after checking
  graph triage, ready work, and the existing verification/adoption inventory.
- [x] (2026-09-14T13:14:41Z) Inspected the policy validator, exact-role compiler,
  executor, offline failure contracts, public docs, current Git baseline, and
  open pinned-consumer adoption task.
- [x] (2026-09-14T13:49:00Z) Introduced opaque compiled authority and migration
  policies with a pure authority builder, exhaustive validation, and
  deterministic normalization.
- [x] (2026-09-14T13:54:00Z) Replaced
  `VerificationRequest`/`VerificationPolicy` with a non-empty
  `VerificationPlan` and made the executor accept only that plan.
- [x] (2026-09-14T14:10:00Z) Migrated package tests, live fixtures, the runnable
  example, rustdoc, and repository contracts to the new public boundary.
- [x] (2026-09-14T14:18:00Z) Ran focused package tests and repaired behavioral
  and API regressions; the final focused library run passed 126 tests.
- [x] (2026-09-14T14:36:00Z) Ran both supported Rust verification matrices and
  all required HTTP smokes; both Rust 1.98.1 and 1.94.0 matrices and all ten
  smoke profiles passed.
- [x] (2026-09-14T14:42:00Z) Recorded the final Jig and PostgreSQL preflight evidence. The PostgreSQL
  live suite preflight executed and stopped before inventory because all three
  endpoint variables were absent; no live PostgreSQL execution is claimed.
- [x] (2026-09-14T15:13:00Z) Ran final Jig gates after the review repairs. The first full Jig check
  passed test, Clippy, formatting, and contract targets but exposed line-debt
  growth in `policy.rs`; extracting the compiler cleared the targeted file-budget
  gate. The terminal check passed all five targets with fresh evidence.
- [x] (2026-09-14T15:13:00Z) Ran the user-required `review-fix-loop` over the final working-tree scope
  with comprehensive fixes, low minimum severity, and all reviewers; continue
  beyond the ordinary bounded limit under the user's explicit authorization
  until the loop converges. The fourth focused closure pass converged with no
  actionable findings from Claude, Codex, or Cursor at fingerprint
  `2786f67770508ba3590e8c6e320a264e92719426d5e1c9674e1928d085378e87`.
- [x] (2026-09-14T15:13:00Z) Reconciled `batter-r2a` evidence with the breaking
  revision and updated `batter-310`. The delivery Bead remains in progress because
  its required PostgreSQL 18 live evidence and refreshed pinned-consumer adoption
  could not be executed in this environment. No commit or push occurred.
- [x] (2026-09-15) With explicit user authorization, provisioned dedicated
  loopback-only PostgreSQL 18.4 primary and observer containers with SCRAM,
  `track_counts=on`, `autovacuum_naptime=1s`, and
  `max_prepared_transactions=10`. Preflight passed and the exact 61-case SQLx
  live inventory passed on Rust 1.98.1 and 1.94.0 (11 disposition, 14 pool
  ownership, and 36 verification cases per toolchain).
- [x] (2026-09-15) Reopened `batter-lp2.5` with user authorization and repaired
  its ordinary-start ordering race by applying readiness approval before the
  coordinator is spawned. Both complete Rust matrices, all HTTP process profiles,
  both foundation examples, and the refreshed two-toolchain PostgreSQL closure
  matrix passed. The dependent fresh-agent evaluation also converged; final Jig
  and tracker closure remain.
- [x] (2026-09-15) Completed the final acceptance audit and Jig follow-up after
  the dependent lifecycle repair. All five targets passed under
  target-validation receipt `receipt_01M2JBKSFB23E3GFJQ52HG5JCM`; the required
  verify gate is fresh with no unresolved condition. The Bead was then closed
  with its evidence comment and acceptance reason recorded atomically.

## Surprises & Discoveries

- Observation: generic authority validation checks object/privilege compatibility
  only for required privileges. Impossible allowances such as relation EXECUTE
  remain inert rather than being rejected.
  Evidence: `validate_authority` calls `valid_privilege` only from its
  `required_privileges` loop in
  `crates/batter-sqlx/src/verification/policy/discovery.rs`; evaluator allowance
  lookup considers only catalog-valid privileges supplied by each object reader.

- Observation: duplicate semantics differ across public construction paths.
  Generic allowance lookup uses any matching entry and PUBLIC indexing merges
  grant options with boolean OR, while `ExactRoleManifest` normalizes identical
  duplicates and rejects conflicting declaration options.
  Evidence: `authority/privileges.rs`, `authority/privileges/public.rs`, and
  `verification/manifest/compile.rs`.

- Observation: the empty-request regression deliberately lets prior cancellation
  win over invalid policy validation. Moving structural validation out of the
  operation removes that ordering question rather than choosing a different
  runtime precedence.
  Evidence: `interrupted_protected_wrappers_do_not_acquire_or_validate_first`
  and `empty_protected_request_fails_before_acquisition` in
  `crates/batter-sqlx/tests/offline.rs`.

- Observation: one real consumer is already pinned to immutable Batter commit
  `b062f92b7df6928fb7b954a57c31ccdee77c93f0` through open Bead `batter-r2a`.
  The revision can remain usable, but its adoption evidence cannot certify this
  new API until that consumer is migrated and reverified.

- Observation: schema discovery can materialize more exact object entries than
  the 10,000-entry bound applied to retained caller input. Reapplying that input
  bound after catalog expansion rejected an otherwise bounded discovery result.
  Evidence: the discovery-scale contract failed until the compiler separated
  caller-input capacity from the catalog/evaluation work and report limits.

- Observation: exact PUBLIC overrides already select verifier query targets;
  copying them into the legacy PUBLIC grant list created two representations of
  one allowance and made normalization semantics ambiguous.
  Evidence: the shared PUBLIC index now consumes overrides directly, while
  discovery expansion no longer synthesizes a second legacy grant.

- Observation: Jig's file-budget gate rejected the initial 1,108-line
  `policy.rs` because its line debt grew by 308 lines from baseline. Moving pure
  policy compilation and normalization into `policy/compiler.rs` reduced the
  model module to 743 lines and the targeted gate passed without a waiver.

- Observation: authorized PostgreSQL execution removed the database blocker,
  but the first subsequent full repository matrix exposed a lifecycle readiness
  ordering race outside this SQLx implementation. The race is owned by the
  previously closed `batter-lp2.5` capability cutover; closing this Bead while
  the required current matrix is red would overstate repository evidence.
  Evidence: `crates/batter/src/lifecycle/driver.rs::Supervisor::start` calls
  `start_unapproved()` before `approve_readiness()`, and the spawned coordinator
  can acknowledge a fast component between those operations.

## Decision Log

- Decision: perform a breaking canonical cutover instead of adding a permanent
  `ValidatedAuthorityPolicy` alongside mutable executable policies.
  Rationale: all packages are unpublished at 0.1.0, invalid and valid values
  must not look like equivalent execution inputs, and a dual path would preserve
  the agent-consumption ambiguity this change is intended to remove.
  Date/Author: 2026-09-14 / Codex.

- Decision: keep application serialization schemas downstream. Expose a native
  Rust `AuthorityPolicyBuilder` whose unfinished state is not accepted by I/O;
  do not add serde, a DI container, or configuration framework.
  Rationale: Batter owns verification invariants, while applications own dynamic
  configuration vocabulary and source parsing.
  Date/Author: 2026-09-14 / Codex.

- Decision: keep existing object-specific declaration structs as builder input,
  but make fields on the executable `AuthorityPolicy` private outside the crate.
  Validate every declaration kind and normalize its bounded collections before
  returning the policy.
  Rationale: invalid draft declarations may exist transiently, but no invalid
  value can cross the executor boundary. This preserves a native, inspectable API
  without multiplying public typestate parameters for every PostgreSQL object.
  Date/Author: 2026-09-14 / Codex.

- Decision: use one non-empty borrowed `VerificationPlan` with a required first
  component and fallible `with_*` composition for mutually exclusive components.
  Rationale: private optional fields express the executor's real shape, while a
  required root constructor prevents the empty state. Borrowing avoids cloning
  potentially large policies and matches existing startup use.
  Date/Author: 2026-09-14 / Codex.

- Decision: retain `verify_authority`, `verify_migrations`,
  `verify_exact_role`, and `verify_sqlx_migrations` only as safe convenience
  wrappers that construct a `VerificationPlan`; the private executor itself
  accepts only a plan.
  Rationale: these names express useful single-component coverage without
  reintroducing raw execution or a second executor.
  Date/Author: 2026-09-14 / Codex.

- Decision: remove the ordinary public `CompiledExactRole::authority_policy`
  escape hatch. Internal evaluation may borrow its compiled authority, but
  consumers use `verify_exact_role` or place the compiled role in a plan.
  Rationale: the old accessor allowed a clone to shed the ownership safeguard
  and made the lower-level route appear equivalent to protected verification.
  Date/Author: 2026-09-14 / Codex.

- Decision: distinguish bounded external drafts from internally expanded catalog
  policies. External `AuthorityPolicyBuilder::build` applies retained-input
  capacity before normalization; crate-private discovery expansion revalidates
  semantics but relies on existing catalog, evaluation-work, and report bounds.
  Rationale: the caller cannot inject the expanded entries, and imposing the
  retained-input count on derived catalog state would make valid bounded
  discovery configurations structurally unexecutable.
  Date/Author: 2026-09-14 / Codex.

- Decision: after implementation and ordinary validation, run the review-fix
  loop with `--fix-mode comprehensive --min-severity low --all-reviewers` over
  the working tree. Continue additional fully accounted rounds if the normal
  limit is insufficient, because the user explicitly authorized continuation
  until convergence.
  Rationale: the user required all three independent reviewers and repairs for
  every actionable severity before handoff.
  Date/Author: 2026-09-14 / Codex.

## Outcomes & Retrospective

The working tree now has one executable-input boundary: mutable authority data
must compile through `AuthorityPolicyBuilder`, migration and schema inputs are
validated at construction, and every `VerificationPlan` is non-empty with one
value per semantic axis. The shared executor accepts only those compiled values.
Exact-role compilation produces the same authority representation while its
ownership safeguards cannot be discarded through the public API.

Both complete Rust 1.98.1 and 1.94.0 verification matrices passed, including
128 `batter-sqlx` library tests and 19 positive plus seven compile-fail SQLx
doctests. All ten HTTP process profiles, 12 `http_service` tests per toolchain,
and the two runnable foundation examples per toolchain passed. The terminal Jig
check passed `api:test`, `api:clippy`, `api:fmt`, `repo:contract`, and
`repo:file-budget` with fresh evidence.

The required all-reviewer loop used two complete comprehensive rounds followed
by four focused closure passes. The first complete round found and repaired a
catalog-dependent validation error introduced during discovery expansion; the
second found no substantive defect. Focused passes strengthened mixed-axis
report assertions, routed all capacity-sensitive executable fixtures through
public compilation, covered the distinct PUBLIC normalization branch, and
retained Clippy/file-budget compliance. Claude, Codex, and Cursor reported no
actionable findings in the terminal pass, and the parent confirmed the unchanged
complete fingerprint
`2786f67770508ba3590e8c6e320a264e92719426d5e1c9674e1928d085378e87`.

The earlier PostgreSQL preflight blocker was cleared on 2026-09-15 with explicit
authorization. Dedicated PostgreSQL 18.4 primary and observer containers passed
preflight, and the full 61-case SQLx live inventory passed on Rust 1.98.1 and
1.94.0. Refreshed immutable consumer adoption is recorded on `batter-r2a`.

The previously contradictory lifecycle evidence was repaired under reopened Bead
`batter-lp2.5`. Both complete repository matrices and the authorized two-toolchain
PostgreSQL inventory now pass on the same source tree, and the bounded fresh-agent
consumer evaluation passed after retaining its first compile failure and two
review-directed initial repairs. The final Jig follow-up passed all five targets
under `receipt_01M2JBKSFB23E3GFJQ52HG5JCM`, with a fresh required gate and no
unresolved condition. No hosted CI, new Linux run, commit, push, publication, or
deployment is claimed.

## Context and Orientation

`crates/batter-sqlx/src/verification/policy.rs` defines PostgreSQL identifiers,
migration input, authority declarations, and policy errors. Its child modules
under `verification/policy/` enforce capacity and required-privilege rules.
`crates/batter-sqlx/src/verification/manifest.rs` defines the higher-level exact
role manifest and `manifest/compile.rs` normalizes that manifest into the current
low-level authority graph. `crates/batter-sqlx/src/verification/request.rs`
defines already-validated SQLx migration and schema-inspection components plus
the currently empty-capable request. `verification/executor.rs` owns one SQLx
lease, transaction reset, read-only snapshot, inspection, rollback, and lease
disposition. Public functions and re-exports live in `verification.rs`.

A compiled policy in this plan means a value whose private representation has
already passed all structural checks and cannot be mutated through the public
API. Compilation is pure: it does not poll a future, acquire a connection, or
consume an `OperationContext`. A verification plan is a non-empty selection of
compiled inspection components. It remains historical configuration; it does
not certify future database state.

The root `AGENTS.md` requires generic scenario names, updates to the relevant
contract/status/testing/validation evidence and owning Bead, rustdoc and an
example for new public APIs, two supported Rust toolchains, HTTP smokes, and Jig
contract v9 evidence. `crates/batter-sqlx/AGENTS.md` additionally requires the
offline feature package and explicitly selected PostgreSQL live suite. SQLx
stays in the optional adapter; PostgreSQL provisioning remains external.

## Plan of Work

First, change `AuthorityPolicy` into an immutable executable value. Add
`AuthorityPolicyBuilder`, initialized with an explicit `DiscoveryScope`, with
methods for discovery defaults, role policy, exact object declarations, database
policy, PUBLIC allowances, required privileges, and required unsupported
surfaces. The builder owns the mutable graph. Its `build` method applies the raw
10,000-entry capacity limit before normalization, validates every privilege list
against its object kind, sorts and deduplicates identical declarations, rejects
conflicting duplicates, and validates required privileges against the final
allowance graph. Only then does it return `AuthorityPolicy`. Keep read-only
getters needed for diagnostics and internal evaluation.

Apply the same boundary to generic migration policy. `MigrationPolicy::new` and
its additional-migration constructor must be fallible and return an immutable
value with private fields. Duplicate versions and oversized checksums fail there.
Remove `VerificationPolicy`, because the new plan owns composition.

Refactor `ExactRoleManifest::compile` to finish through the same authority
compiler rather than constructing a separately trusted mutable policy. Preserve
its current deterministic duplicate and contradiction semantics. Keep
`CompiledExactRole` fields private and remove its public raw-policy accessor;
internal executor methods may borrow the canonical authority.

Replace `VerificationRequest` with `VerificationPlan`. Its public root
constructors are `authority`, `migrations`, `exact_role`, `sqlx_migrations`, and
`schema_inspection`. Each creates a valid non-empty plan. `with_*` methods add
independent components and return a typed `PlanError` when an authority,
migration, or schema component was already selected. The plan does not implement
`Default` and has no empty constructor.

Change the private executor to accept only `VerificationPlan`. Remove structural
validation from its async block and remove `VerificationError::InvalidPolicy`.
Public single-component helpers construct a plan and delegate to the same
executor; public `verify` accepts a plan directly. Preserve never-polled I/O,
already-cancelled operation, transaction, rollback, report, and lease-retirement
semantics.

Migrate every integration test and example that currently uses an exhaustive
authority struct literal through the builder. Unit tests inside the crate may
construct raw internal fixtures only when the test is specifically proving the
compiler rejects them; evaluation tests should consume compiled policy. Replace
runtime invalid-policy tests with pure construction tests and retain the pool-size
oracle showing construction performs no acquisition. Update the README,
rustdocs, `docs/guarantees.md`, `docs/integrations.md`, `docs/status.md`,
`docs/testing.md`, and `docs/validation.md` so only the compiled plan is taught as
the canonical path.

Finally, reconcile open Bead `batter-r2a`: record that its immutable old pin is
historical for this API, and require refreshed consumer evidence at the new
revision before using it to close adoption. Do not edit the private consumer or
claim its verification without explicit repository access and executed commands.

## Concrete Steps

Work from `/Users/aa/Documents/batter`.

1. Edit policy and manifest modules using `apply_patch`, then run:

       cargo test -p batter-sqlx --features test-support --locked

   Expect all offline/unit tests to pass and all explicitly external PostgreSQL
   tests to remain ignored. New pure compiler tests must fail against the baseline
   because impossible ordinary allowances currently pass validation.

2. Migrate public call sites and documentation. Run:

       cargo fmt --all -- --check
       cargo clippy -p batter-sqlx --features test-support --all-targets --locked -- -D warnings

   Expect no formatting diff and no warning.

3. Run the repository verification matrices:

       bash scripts/verify.sh
       RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh

   Expect both commands to exit zero. Record exact rustc versions and outcomes
   in `docs/validation.md`.

4. Build and execute the HTTP smoke binary as required by `docs/testing.md`, in
   every currently documented mode. Reuse a fresh passing receipt only if its
   exact source, command, toolchain, and prerequisites remain applicable.

5. If `DATABASE_URL`, `BATTER_SQLX_AUTH_ACCEPT_URL`, and other documented live
   prerequisites are present, run:

       bash scripts/test_sqlx_live.sh

   Expect the exact registered inventory to pass. If any prerequisite is absent,
   run the preflight, record the nonzero missing-prerequisite result, and do not
   claim PostgreSQL execution.

6. Inspect and execute Jig evidence:

       scripts/jig work evidence --plan-id plan_01M2G0S939GNSK9ZVCFMCP24D6
       scripts/jig work gates --plan-id plan_01M2G0S939GNSK9ZVCFMCP24D6
       scripts/jig work check --plan-id plan_01M2G0S939GNSK9ZVCFMCP24D6

   Expect required `api:test`, formatting, Clippy, contract, and file-budget
   targets to pass for the final worktree. If evidence inspection reports
   `collection_limit`, repeat the evidence or gate read with
   `--freshness-timeout-ms 30000` rather than rerunning checks blindly.

7. Run the review-fix loop with these effective controls:

       --scope working-tree --fix-mode comprehensive --min-severity low --all-reviewers

   Normalize the controls with the skill's `loop-options.mjs`, pin one complete
   working-tree fingerprint, launch Claude, Codex, and Cursor reviewers in
   parallel, repair every verified eligible finding, validate, and repeat
   same-scope reviews until convergence. The ordinary default is four repair
   rounds plus one supporting-work closure batch; the user's explicit instruction
   authorizes further accounted rounds until convergence if needed. Preserve the
   pre-existing index and all unrelated work throughout.

## Validation and Acceptance

Acceptance is proven when an external Rust caller cannot construct an
`AuthorityPolicy` or `MigrationPolicy` with public fields, cannot construct an
empty `VerificationPlan`, and cannot pass a builder or declaration directly to
`verify`. Rustdoc examples must show a policy construction failure before any
async code and a successful compiled plan execution.

Pure tests must enumerate every PostgreSQL object kind against all fourteen
`ObjectPrivilege` variants. Each valid pair compiles and each invalid pair
returns `PolicyError::InvalidObjectPrivilege`. Separate tests prove exact
duplicate normalization, conflicting grant-option rejection, duplicate-object
conflict rejection, PUBLIC/default/required precedence, migration duplicate and
checksum bounds, and exact capacity edges. A compile-fail doctest or external
fixture must prove fields and empty construction are inaccessible, not merely
that runtime tests avoid them.

Executor tests must prove an already-cancelled valid plan acquires no connection,
an unpolled valid plan does no application work, and policy errors cannot appear
from execution. Existing report and live cases must preserve their supported,
unsupported, violation, interruption, rollback, and retirement outcomes.

The complete repository acceptance additionally requires both Rust toolchains,
HTTP smokes, applicable Jig gates, updated contract/status/testing/validation
documents, and a current Bead. PostgreSQL live behavior and refreshed pinned
consumer adoption are separate required evidence; unavailable external inputs
must be reported rather than silently skipped or inferred from compilation.

## Idempotence and Recovery

All source edits and pure tests are repeatable. `scripts/jig work check` records
receipts but does not alter application semantics. Tracker commands append audit
state; use `br show batter-310 --json` before each update and `br sync
--flush-only` after final mutations. Never overwrite `.beads/issues.jsonl` by
hand because it already contained user-owned changes at the starting baseline.

If the cutover stops mid-migration, use compiler errors from `cargo check -p
batter-sqlx --all-targets --features test-support --locked` as the call-site
inventory. Do not restore the old raw executor to make intermediate tests green.
If an external consumer cannot be accessed, finish the repository implementation
and leave the adoption acceptance explicitly incomplete; do not fabricate or
weaken the requirement.

## Interfaces and Dependencies

At completion, `batter_sqlx::verification` exposes these conceptual interfaces;
exact spelling may change only if recorded in the Decision Log:

    pub struct AuthorityPolicy { /* private canonical fields */ }

    pub struct AuthorityPolicyBuilder { /* private mutable draft */ }
    impl AuthorityPolicyBuilder {
        pub fn new(discovery: DiscoveryScope) -> Self;
        pub fn build(self) -> Result<AuthorityPolicy, PolicyError>;
        // Typed setters/adders for every existing policy component.
    }

    pub struct MigrationPolicy { /* private validated fields */ }
    impl MigrationPolicy {
        pub fn new(
            ledger: QualifiedName,
            required: impl IntoIterator<Item = MigrationExpectation>,
        ) -> Result<Self, PolicyError>;
        pub fn with_additional(
            self,
            additional: AdditionalMigrations,
        ) -> Result<Self, PolicyError>;
    }

    pub struct VerificationPlan<'a> { /* private, always non-empty */ }
    impl<'a> VerificationPlan<'a> {
        pub const fn authority(policy: &'a AuthorityPolicy) -> Self;
        pub const fn migrations(policy: &'a MigrationPolicy) -> Self;
        pub const fn exact_role(role: &'a CompiledExactRole) -> Self;
        pub const fn sqlx_migrations(manifest: &'a SqlxMigrationManifest) -> Self;
        pub const fn schema_inspection(policy: &'a SchemaInspectionPolicy) -> Self;
        // Fallible with_* methods prevent duplicate semantic components.
    }

    pub async fn verify(
        pool: &sqlx::PgPool,
        context: &batter::operation::OperationContext,
        plan: VerificationPlan<'_>,
    ) -> Result<VerificationReport, OperationError<VerificationError>>;

No new crate or third-party dependency is required. `batter-sqlx` continues to
depend downward on `batter` and native SQLx; `batter` remains independent of the
adapter. PostgreSQL provisioning, migration selection, configuration decoding,
and application schemas remain outside this workspace.

Revision note (2026-09-14): Replaced the initial one-line Jig body with a
self-contained implementation plan after inspecting the current policy,
executor, exact-role compiler, tests, tracker, and pinned adoption evidence.

Revision note (2026-09-14): Added the user's required comprehensive,
low-severity, all-reviewer review-fix loop and explicit permission to continue
until convergence.
