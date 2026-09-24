# Testing and failure-contract coverage

Checked owned-completion consumers run through
`cargo test -p batter --test checked_completion_consumer --locked` and the
temporary external package created by `scripts/check_facade_features.py`.
Eight driven cases cover `anyhow` and `BoxError` propagation, process exit
status, successful and failed count summaries from one checked observation,
coordinator failure without a fabricated report, and worker readiness success,
rejection, deadline and failed shutdown with awaited cleanup. The external
package also rejects forged success evidence and raw-report disposal for the
specific privacy and `unused_must_use` diagnostics. These generic controls do
assert that direct `ShutdownFailure` formatting omits a coordinator panic marker
while `anyhow` chain formatting can reveal it. They do
not establish compatibility of a pinned downstream application graph; that
requires a separate candidate rehearsal with Cargo metadata and locked tests.

Transaction timeout profiles (`batter-qhps`) have offline precision/range and
override controls in `crates/batter-sqlx/tests/profile.rs`. The explicit live
inventory adds six PostgreSQL 18 cases in `atomic_live/profile_timeouts.rs`,
`profile_timeout_drift.rs` and `profile_timeout_expiry.rs`: startup defaults versus
session settings, explicit zero, legacy compatibility, all pool hook paths,
atomic/snapshot application and drift, retained poison and recovered inspection
errors, and independently observed server termination/lock release. The expiry
controls retain the Rust lease and assert that the local pool slot stays occupied.
Total expiry spans multiple successful short statements; idle expiry waits for
an independent backend observation. Existing setup-redaction cases use complete
profiles. Older PostgreSQL versions and fresh-agent usability tasks are not
executed by these cases.

Atomic runner regression coverage (`batter-gzh`) lives in
`crates/batter-sqlx/tests/atomic_live`: acknowledged output/rejection, retained
uncertainty, caught inner cancellation, inherited session reset, completion
retirement/advisory-lock release, and original snapshot-guard cleanup. The explicit
`scripts/sqlx_live.py` inventory includes these PostgreSQL 18 cases. Rustdoc
compile-fail tests reject separate completion, escaping scopes, and passing a
write helper to a read-only inspector. The paired Runledger branch additionally
rejects enqueue-then-record at compile time and tests intent/queue/app atomicity.

The native adapter's offline contracts run with `cargo test -p batter-runledger
--locked`: local initialization without a database, zero task starts on rejected
or unstarted registration, original preparation errors with owned startup cleanup,
and compilation rejection of live-supervisor/closure arguments. The managed and
process ownership targets cover retained settlement and delayed native stop
observation. Library lifecycle controls verify wakeups in all three process phases
and an earlier update arriving during callback execution. Native complete-report
tests tighten active graceful and abort waits while preserving failure causes and
uncertain descendants. Review closure is tracked separately; older hosted-control sections below describe
the previous implementation.

Finite command completion controls in `crates/batter-core/tests/command/completion.rs`
place cancellation or paused-clock expiry in the final poll versus future
destruction, preserving both returned success values and application errors.
Managed initialization controls distinguish native stop, native settlement and
process drain before acknowledgement, and check retained settlement and cleanup.
These sources are covered by the existing exhaustive Jig source/test inputs.

Cargo discovers foundation, adapter, example and generic test-support tests, plus
library doctests. This guide describes behaviors and verification commands.
Foundation fixture-dispatch entries are inert in the parent run. The non-yielding
and scheduling suites cover native process controls, deterministic policy checks,
controlled-clock regressions and Python subprocess ownership. Linux orphan-probe
entries are excluded on other Unix targets.
Windows is unsupported and not planned; see [platform scope](adr/007-unix-platform-scope.md).
Source presence and package-integrity checks are not type checking.

The SQLx `atomic_live::validation` controls count native SQLx query events for
first, repeated and post-recovery operations: four statements unprofiled, five
profiled (both timeout constructors), one additional statement on SQL rejection,
and two for final commit. `validation_drift` checks every declared setting,
role/session-authorization changes and restored settings after savepoint recovery.
Independent-session schema removal and USAGE revocation must prevent callback
invocation. A PostgreSQL 18 `EXPLAIN` of the exact production query, with
sequential scans disabled, requires an indexed namespace-name condition; this
detects both a cast on the catalog column and an `EXISTS` rewrite that scans the
whole catalog. These cases run through the exact `scripts/test_sqlx_live.sh` inventory;
ordinary workspace tests discover them but leave them ignored.

Policy-based atomic runners are covered by `atomic_live::policy` and
`atomic_live::policy_cancellation`: fixed native `?` inference, recovered errors,
acknowledged rejection, caught terminal failure, abandoned work, paired recovery
causes, setup failure, and commit/rollback uncertainty. Native
`runledger-postgres/tests/atomic_runner/policy.rs` exercises one consumer error
across SQL, intent recording, queue operations and required-conflict rollback.
The context unit suite cancels from policy mapping of a closed-pool begin error
through both `_with_in` wrappers. Cancellation policies retain an `OperationOwner`;
the wrappers receive only its `OperationContext`. `atomic_live::policy_context`
does the same with a deferred commit failure, asserting the provisional output and native
SQLSTATE/constraint survive as `OperationError::Failed`. The native policy test
also covers intent observation and resource enqueue: real CHECK violations pass
through `From<runledger_postgres::Error>`, retain their SQLx causes, and permit
subsequent application writes and named operations to commit after recovery.
Two later native policy cases use real PostgreSQL failures: required-intent
storage conversion retains `RequiredIntentError::Storage` and allows a later
write after savepoint recovery; termination of the transaction backend before a
named required-intent call produces a terminal scope error, retains the same
cause in `ScopeLost`, and does not commit an earlier provisional write.
Focused runs of the two later native cases are recorded in `batter-rpgk`.
The `batter-runledger` crate-level doctest implements all five failure-policy
handlers using only adapter imports, covering the direct consumer export surface
including `PgTransactionError`.
The earlier `batter-rpgk` snapshot had an exact SQLx live inventory of 44 atomic
tests (107 total). On that earlier snapshot, all 107 passed on both Rust 1.98.1
and 1.94.0 on macOS arm64 with PostgreSQL 18.6, along with both full
verification scripts and five HTTP smoke modes per toolchain. Its final Jig
test rerun passed; initial suite failures and their unchanged reruns are
recorded in the owning Bead.

The original `batter-a0qb` evidence records 105 passing cases on macOS arm64
with Rust 1.98.1 and 1.94.0 and PostgreSQL
18.6. Both full verification scripts and five HTTP smokes per toolchain passed;
required Jig gates passed. The first minimum-toolchain workspace run encountered
a Docker port-resolution failure in the existing `job_read_scope` fixture. That
target and the full rerun passed unchanged. See `batter-a0qb`.

## Verification commands

Outcome-aware attempt acceptance uses an explicit disposable PostgreSQL 18:
`DATABASE_URL=... cargo test -p batter-runlimit --features postgres --test attempts_live -- --ignored`.
Ten cases cover committed failure audit, success reset preserving audit, native
denial without factory invocation, stale claim before application writes,
operational rollback, final replay rejection after successful verification,
one parent budget, verification cancellation, acknowledged commit with immediate
parent cancellation, lock-protected claim across lease expiry, and unconfirmed
commit without replay (some cases cover multiple invariants). The ordinary suite
discovers these cases but leaves them ignored; absence of the explicit command is
not live evidence. SQLx library tests separately control simultaneous deadline
and acknowledgement readiness, retained errors and preflight factory inertness.
Rustdoc compilation enforces typed application decisions and inaccessible receipt
completion. See the adapter README's ADR-010 assessment for remaining policy limits.

The native quota adapter's `cargo test -p batter-runlimit --all-features --locked`
uses real memory-store atomic decisions and scripted pending/uncertain failures.
Its real memory-store regression also drops an unpolled `Quota::run` future and
proves that the next call still gets the sole grant. Direct native single and
batch trait-call controls now prove that dropping an unpolled memory check leaves
the grant available, while polling consumes it.
An enforced storage-capacity denial retains its exact typed retry duration in
the public result without calling work. Allowed results expose native validated
allowances, while enforced and shadow denials retain the native validated batch
size; the two-check atomic-denial regression proves the original size survives
the adapter. Protected HTTP checks both native
denial reasons and whole-second `Retry-After` rounding from the typed delay.
It covers total budgets, factory inertness, denial-before-body, async authentication,
direct-peer identity, protected-by-default route assembly, literal-only public
probe registration, inner raw-principal
collision, narrowed allowed/denied/interrupted results, native serving, and
retained single observations after timeout
or drop. Axum unit tests and compile-fail doctests cover the consuming quota
writer's unstarted, started, and terminal phases, including rejection of a second
terminal write or a nonterminal finish. PostgreSQL error/type checks are offline,
not database acceptance; they explicitly cover the current lost-confirmation and
commit-timeout variants as possibly consumed.
`cargo run -p batter --features runlimit-memory,runlimit-axum --example quota_service --locked`
executes native admission and an HTTP 200/429 sequence.
The public example now compiles without subject-selector argument annotations.
An external disposable Cargo consumer additionally checks a valid assembled
service and confirms wrong auth and selector closure signatures fail at
`HttpQuota::new`.

`python3 scripts/check_runlimit_features.py` compiles all eight independent feature
subsets in a disposable consumer workspace with the current toolchain. It checks
normal dependency reachability, refuses external source/version drift from the
repository lock, and requires the specific disabled-HTTP import error without
`axum`. This runner is part of `test_matrix.py`, including Jig's test action.
It reads the declared feature inventory from Cargo metadata and fails if a
feature is added without a graph-membership expectation. A runner control
injects an additional declared feature and requires that failure. Protected
HTTP regressions cover nested custom and method fallbacks through authentication
and quota, custom protected root fallbacks with and without public probes, and
a public probe's unsupported method returning the default 405 without polling
its request body. A focused preparation test pins Axum's startup panic when a
public GET probe duplicates a protected GET route. Real one-use memory tests
recheck the same subject after domain work failure and post-grant cancellation;
both require a later native denial and no second work invocation. Constructor tests reject empty and mixed-mode policy sets;
native memory certainty tests cover all pinned error variants directly.
Existing exhaustive source/test/example and scripts globs in both Jig contract
representations include the new package and runner; no new input root is used.

The `batter-sqlx` offline library suite includes the pure exact-role manifest
compiler and grant renderer. Its cases compare differently grouped inputs,
idempotent and conflicting duplicates, required versus allowed-only authority,
PUBLIC precedence, relation/column separation, row-type and SECURITY DEFINER
options, structural scalar/array routine identities, identifier quoting,
database rendering context, invalid privilege/object pairs and both retained
input and expanded capacity bounds. These tests perform no database I/O; live
provisioning remains external. Protected SQLx ledger, schema-setting, ownership,
and pre-cancelled wrapper controls are registered in the same package and live
inventory; credentialed PostgreSQL cases remain ignored in ordinary gates.

```sh
cargo test -p batter-sqlx --features test-support --locked
```

The component and real HTTP/1.1 ownership suites run in normal Cargo discovery:

```sh
cargo test -p batter-core --test component_ownership --locked
cargo test -p batter-axum --test http_lifetime --locked
```

The component suite gates initialization and child joining and contrasts a hidden
child that answers after wrapper completion and cleanup. It also retains task and
cleanup failures and checks conservative panic/abort cleanup. Every comparison
has a ten-second paused-Tokio-time bound, with a pending-future rejection control.
This detects yielding deadlocks, not non-yielding execution; the existing matrix
process owner also bounds test phases independently of Tokio. Existing
non-yielding, finite-descendant, abandonment and scheduling regressions remain
part of the workspace matrix.

The HTTP ownership target has 23 tests: eleven loopback scenarios, a deliberate-runtime-stall
control, startup-timeout, missing-event and forced-teardown diagnostic controls, a terminal
handler-count control, lock-poisoning and rejection-trace regressions, a native
acknowledgement parser control, three launch/completion controls, and its inert dispatch entry. Shared std-only machinery lives in workspace
`test-support/process/`; HTTP wrappers attach launch/completion controls while the foundation owns
its non-yielding fixture and Linux Python helper. Each HTTP scenario
has an eight-second parent deadline, kill/reap on expiry, the existing PID-bound
stdin authorization, parent-death handling and ten-second emergency exit. A killed
ordinary case fails even if it emitted partial evidence. The stall control proves
observed SIGKILL/reaping and rejects success validation. The focused command has
these bounds even outside `scripts/test_matrix.py`.

Test-owned channels, pending body polls and resource Drop events establish entry
and ordering. Full client disconnect is SHUT_RDWR followed by socket close;
destruction must occur before test release within one second. A pending body is
inspected while the runtime remains alive after wrapper abortion, then released
and independently reconciled with message framing and socket closure. Exercise
task errors/panics and teardown errors are retained together. Construction and
readiness share a one-second startup deadline. The running server owner is
retained before readiness waiting, so a readiness timeout still drives teardown
and retains its report; a construction failure before ownership reports that no
running owner exists, with captured state/trace diagnostics. Yielding exercises
have a two-second diagnostic timeout followed by separately bounded 3.5-second
teardown; the failure includes the actual shutdown report, event and trace snapshots.
The missing-event control uses a 500 ms exercise bound. Shared fixture constants
assert room for both diagnostic phases plus startup/unwind margin inside the
parent deadline, which must remain below the emergency exit. The teardown bound
is checked against the actual shutdown budget's `total_allowance()`, including
cleanup/reaping, not a parallel phase sum. Event/wait progress is
also written to the parent's bounded capture, including if a non-yielding runtime
prevents those inner timeouts. Ordering assertions release the event lock before
panicking so later destructor evidence remains available. The companion
admission case withholds graceful notification so a second established-connection
request must reach admission and return 503 with no handler entry.

Both HTTP lifetime targets use `tests/support/http_graceful.rs` to wait for the
exact Axum 0.8.9 native connection-task event with `info,axum::serve=trace` capture.
The helper requires a current-thread runtime: the native event immediately precedes
synchronous `graceful_shutdown()`, which returns before the test can resume.
The producer's `graceful-signal-ready` and the accept-loop event are insufficient;
a parser control rejects them, and a missing native event fails after one second.
Cooperative handler/body cases keep release withheld for a further 50 ms, assert
live resources and unfinished server/cleanup, then release explicitly. The
instrumented target additionally checks that the wire read remains pending.
The native event establishes ordering; the subsequent finite observation window
does not establish indefinite survival. See the [versioned source rationale](references.md#native-connection-graceful-acknowledgement-2026-09-10).

The existing matrix includes `--workspace --all-features --all-targets`; no new
runner command is required. Jig's existing `**/*.rs`, manifest and lockfile inputs
cover both the new fixtures and reused control modules, so source changes stale
the test receipt. All five HTTP executable smoke profiles run in the independent
`api:http-smoke` target of the complete verification profile.

The startup rustdocs are executable: the legacy example acquires a native capacity permit, starts
a channel service, waits for acknowledged readiness, handles a request, and
awaits shutdown/resource release. The protected example reserves cleanup and
registers a critical component without supervisor access. `tests/protected_startup.rs`
checks typed error retention before destructor panic, observer identity, direct
reservation across await, real request handling and join-before-finalization.
`tests/registration.rs` checks sealed reborrowing plus invalid/duplicate ordinary
and managed rejection without factory invocation. `tests/startup_composition.rs` independently
checks unsuccessful empty supervision with successful cleanup and successful
finite-work-only supervision. `finite_command` is explicitly declared with
`test = true`; its six example tests run in the ordinary all-targets workspace
matrix. They check native loopback work, separate/simultaneous work and cleanup
failures, and cleanup after post-acquisition cancellation or deadline. The command
library contracts in `tests/command.rs` additionally cover owner/waiter loss,
downward cancellation, original result and panic retention, absolute total reserves,
cleanup timeouts and report observation across runtime destruction. The scoped
task suite checks work and cleanup destruction under the originating subscriber.

```sh
cargo test -p batter-core --locked --test command --test startup_composition
cargo test -p batter-core --doc --locked startup::Startup
cargo test -p batter --doc --locked
cargo run -p batter --example finite_command --locked
```

The command's `--fail-work`, `--fail-cleanup`, `--fail-both`, `--cancel` and
`--deadline` modes must exit nonzero; interruption must still report successful
cleanup. See [usage](usage.md#finite-commands-and-owned-cleanup).
These tests require no database. Owned command finalization requires a live runtime;
it cannot promise remote effect reversal or termination of arbitrary spawned tasks.

The optional SQLx adapter's offline contracts run with ordinary workspace gates.
Its live tests are explicitly ignored even with all features/targets. With
`DATABASE_URL` identifying an externally provisioned disposable PostgreSQL
database, `BATTER_SQLX_AUTH_ACCEPT_URL` identifying a known-good password-
authenticated endpoint, and `BATTER_SQLX_ADMIN_URL` identifying a PostgreSQL
superuser connection on a dedicated disposable cluster, run
`bash scripts/test_sqlx_live.sh`. The runner rejects any missing
configuration and case-inventory mismatches, then executes the two-case native
migration target and the exact eleven-case
disposition target, fourteen-case pool-ownership target and thirty-six-case restricted-
login verification target serially under the existing Unix process watchdog.
The identity controls compare cross-schema multirange grants with native
has_type_privilege and retain exact alias policies. The notice control runs all
three verifier entrypoints and requires their expected idle reset warning. Pure
production-snapshot tests cover cooperative cancellation/deadline, combined
role/object scale, near-capacity parameter assembly and missing-object lookup
with counted catalog visits and name lookups, the checked captured-parameter uniqueness boundary,
and explicit work/report/policy limits. A full `AuthorityPolicyBuilder::build` path at
the 10,000-entry aggregate limit covers relation, nested-column, sequence, schema,
routine, type, parameter and database allowances with an explicit logical-work
counter independent of `HashMap` internals. Every keyed kind has hundreds of
distinct required lookups, including two independently necessary columns per
relation; singleton success cannot stand in for scale coverage. The fixture
builds the complete draft before applying the independent logical-work observer,
then rebuilds mutated drafts to reject one-entry overflow and a late column
denial. The parameter scale oracle also
retains hidden, custom-placeholder, absent-object and late ACL/grant-option
verdicts without using elapsed wall time or standard-library hashing counts.
Its observer follows the private catalog into the full production evaluator;
each traversal visit and index lookup counts, so repeated builds and per-item
scans remain visible. Catalog assembly counts each deduplication decision and
catalog visit through the same observer. The observer is inert outside tests.
Two additional full-evaluator fixtures each capture 10,000 parameters and fill
the policy budget. One exercises 3,000 required parameters (including ordinary
current-role/inherited ACL checks); the other exercises 4,000 PUBLIC grants and
3,001 overrides. They retain known-absent versus uncaptured identities, hidden
metadata, custom requirements, denied required privileges, PUBLIC denial and
grant-option findings. All five indexed parameter caller sites have independent
scan-mutation evidence. The original 29,993-operation declared-parameter
regression is retained unchanged.
Pure construction controls enumerate every supported authority object kind
against every `ObjectPrivilege` variant, normalize identical privilege entries,
reject conflicting grant-option declarations, and reject duplicate required or
exact object identities. They also reject relation/sequence collisions across
exact, PUBLIC, column-parent and required declarations. Plan tests cover both
composition orders when a migration ledger conflicts with a sequence identity,
while report tests require canonical unique coverage ordering and catalog
identity tests retain exact typed causes. Full snapshot regressions require both
directions of valid-policy/catalog kind drift to remain `MissingObject`
violations while the observed object uses its own discovery defaults. They cover
relation, sequence and column PUBLIC/required targets plus the stricter
no-default behavior of declared-only scope. Duplicate catalog schema, relation,
column, type and routine identities are rejected during checkpointed expansion;
a malformed routine argument type retains its typed catalog-identity cause, and
an impossible snapshot containing both relation kinds retains the distinct
catalog-expansion error. The large discovery regression exercises the same
one-pass internal policy constructor, so no post-expansion global
canonicalization phase is outside its watchdog.
Generic and SQLx migration constructors
stop consuming input at the fixed row boundary; generic required and allowlisted
rows are also bounded together. Compile-fail doctests prove callers cannot use
executable authority fields, detach exact-role authority from its safeguards, or
create an empty/default `VerificationPlan`. Pure construction tests prove invalid
drafts cannot produce an executable value; already-cancelled valid plans still
acquire no connection.
Private work-counted map/set boundaries also measure traversal in scan-mutation
controls; production callers cannot obtain a raw collection iterator. The policy
tests additionally fill the aggregate limit with PUBLIC grants, overrides,
explicit discovery schemas and required privileges. They check late conflicts,
unselected schemas and the `UserSchemas` exclusion of system schemas. A separate
collection control forces growth and distinguishes one lookup from visiting
every entry. These counters cover the named indexing paths, not every operation
in policy validation or native hash-table implementation work.
Python controls reject missing, skipped,
duplicated, summary-only, or newly unlisted cases. It requires up to six simultaneous
server sessions, visibility of its own `pg_stat_activity` rows, advisory locks
and CREATE-on-database privilege. It does not create databases. The restricted-login
verification fixture creates uniquely named roles and schemas, cleans them on
normal/error/panic paths and uses finite credentials. Its parameter ACL names
are unique per fixture, so cleanup cannot revoke an operator's pre-existing
PUBLIC grant. PostgreSQL roles and parameter ACLs are cluster-wide; process
death can leave disposable residue until expiry or external cleanup, which is
why the administrative endpoint must belong to a disposable cluster.

The pool-ownership target proves Command and legacy Startup query composition,
reservation rejection before construction, native option/callback preservation,
a successful query followed by exact `28P01` after changing only that connection's
password, and cancelled-acquisition cleanup, later error
and panic cleanup, two-pool LIFO order, and held-checkout success/timeout behavior.
The held-checkout control requires the owning report to remain unfinished before
release. Each successful pool requires zero native size and a later acquisition
returning `PoolClosed`. Positive and negative cases call the same cleanup-plus-
native-close oracle. One negative closes the native pool but omits the cleanup
record; the other supplies the claimed successful record while a held checkout
keeps native size nonzero, then passes that unchanged oracle after release and
close completion. This is local client evidence only, not remote session
termination or rollback acknowledgement.

Each interrupted-case observer first witnesses the acquired backend waiting for
a lock. Cancellation, deadline, application error, panic and outer-future drop
must retire the client. Independent SQL and pool close each have a two-second
local allowance before unlocking; all tracked backends must still be blocked.
After unlock, a separate five-second observation requires their disappearance
before test-control connections close. Three repeated interruptions must retain
three distinct server identities despite a one-slot pool. Successful query and
acknowledged commit/rollback reuse the same backend; a rejected deferred-constraint
commit preserves its SQLSTATE and retires. The ordinary SQLx return control
demonstrates the blocked-capacity failure mechanism. These checks do not establish
remote cancellation acknowledgement, ambiguous-commit rollback or a server-side
concurrency bound. The full process has a 180-second watchdog; OS scheduling and
process death do not establish cleanup guarantees.

Private-boundary regressions in
[`lifecycle/tasks/tests.rs`](../crates/batter-core/src/lifecycle/tasks/tests.rs) cover
cancelled join waiters, exactly-once recording of early success/error/panic,
closed admission before returning a failure cause, and conservative unjoined
summaries. [`observation/tests.rs`](../crates/batter-axum/src/observation/tests.rs)
checks unchanged failure responses without admission and destruction under the
saved dispatcher after the observation waiter is cancelled. These complement
the existing scheduling, shutdown-cause, HTTP composition and dispatch suites.


```sh
bash scripts/verify.sh --bootstrap  # Initial formatter + dependency lock + checks.
bash scripts/verify.sh             # Fresh complete Jig verification with receipts.
bash scripts/verify.sh --plan-id <id>  # Same required profile, reusing fresh plan evidence.
```

Choose one of the non-bootstrap commands above. Both delegate to Jig; the profile
builds the HTTP example and runs all five process smoke modes, checks rustdoc,
and retains both workspace/all-feature and native Runlimit/default-feature Clippy.
There is no separate full-matrix or HTTP rerun after a successful profile.

Run local verification once with the current release pinned in
`rust-toolchain.toml` (1.98.1). Exact Rust 1.94.0 verification is CI-only unless
explicitly requested to reproduce an MSRV failure. Upgrade the local pin and
pinned CI entries together; a weekly CI run checks floating `stable`.

Python 3.9 or newer is required by the scheduling subprocess tests, including plain
`cargo test`; the verification script checks that prerequisite before running Rust.
Workspace tests also require permission to create Unix subprocesses and bind and
connect IPv4 and IPv6 loopback TCP sockets. IPv6 loopback (`::1`) must be enabled
on the host or container running the matrix: the ordinary reference configuration
target performs a native PostgreSQL protocol handshake on a test-owned IPv6
listener, without an external database. Enable that prerequisite before running
the gate; this semantic test is mandatory and does not silently skip when IPv6
is unavailable. The example readiness tests run automatically
under `--all-targets`; they are not ignored and contact only test-owned listeners
on `127.0.0.1`. A sandbox denying those operations cannot run the complete gate.
HTTP process smokes are a required `api:http-smoke` sibling in the complete Jig profile.
The standalone commands below remain useful for focused troubleshooting.

HTTP text assertions check completion fields after the completion message; span
fields cannot satisfy the event-field oracle. `scripts/test_smoke_http.py` includes
negative controls for missing and conflicting event fields even when the span
contains every expected value. It runs in the existing Python discovery command.
Its operation-filter controls reject INFO completions with or without the
example formatter's timestamp, while allowing WARN deadline completions and
application INFO events. Nested-observer redaction checks cover the full capture,
including lines outside the HTTP completions.
The live readiness fixture awaits teardown before checking results and reports
the saved request, supervisor and server outcomes together if any failed.

The script checks isolated `-p batter-core --no-default-features` library
compilation and core tests, the no-default facade, and the bounded facade
feature runner. `scripts/check_facade_features.py` uses disposable external
workspaces to compile each declared feature, the standalone at-rest leaf, the representative Axum/SQLx and
Runlimit unions, and all-features; it checks the selected normal graph, nine
focused disabled-module failures, and one all-feature direct/facade identity
fixture. It runs under the invoking toolchain and leaves the repository lock
unchanged. The separate `scripts/check_runlimit_features.py` gate still covers
all eight native Runlimit feature combinations. The matrix then checks all
workspace targets, doctests, Clippy with warnings denied,
and rustdoc with warnings denied. The workspace run includes the SQLx executable
and compiles the reference compatibility cases as ignored. Since the Runledger
workspace import, ordinary verification also executes the native PostgreSQL 18
and producer/worker tests through Docker. The explicit live runner below still
provides separate Batter adapter/reference compatibility evidence. A Cargo.lock generated by
the actual resolver must be committed after the first successful run.

CI's `scripts/check-batter-at-rest-portability.sh` separately selects exact Rust
1.94.0, copies only the tracked `crates/batter-at-rest` candidate into a neutral
temporary root, rejects workspace/path/config leakage, runs detached tests and
the independent Node.js fixed-vector generator, runs all-target checks, creates
a verified package, repeats checks from the unpacked artifact, and executes
public-API tests in a separate consumer manifest depending only on that artifact.
The consumer uses default features and no leaf dev-dependencies. Its negative mutations prove the gate
fails when standalone metadata or dependency-source boundaries regress.

At-rest decoder tests exercise every truncation and every single-byte replacement
of a composite sample, requiring canonical re-encoding after successful decode.
Borrowed and owned decoders share structural parsing, with explicit expected-error
cases for nested lengths, version, key ID, body bounds and trailing bytes. Format
tests pad a valid encoded envelope beyond the header limit, so removing the size
guard changes the asserted error; the body test supplies an actual byte beyond
the maximum. Public consumer and independent Node fixture cases open both views
and check that the borrowed body points at the exact encoded subslice. Rustdoc
rejects retaining the borrowed payload after its encoding or its decrypt view
after the decoded header owner, with a compiling scoped counterpart. Parsing validates trailing
data before owned decode copies the body. These are bounded regressions and
pointer/lifetime evidence, not exhaustive fuzzing or a measured allocation claim. Crypto tests
cover failure at each seal randomness request, changed-key rewrap randomness
failure, and successful wrapper rotation with a corrupt body that still fails open.
Facade consumer graphs reject the leaf's `test-support` feature and check the
AES/GCM dependency family; shared hashes used by other adapters are not treated as
exclusive evidence of at-rest feature selection.

Regular CI targets MSRV 1.94.0 and the pinned 1.98.1 toolchain. The weekly
Linux verification job targets floating `stable`. CI requires the
checked-in lockfile and uses the strict verification path; missing lockfiles fail
before cache metadata resolution. [Run 35580602864](https://github.com/bpcakes/batter/actions/runs/35580602864)
passed the Linux verification matrix and the focused macOS jobs for commit
`56814038f2a9cf6a34688ee39cd9f0e433487a1e`. It does not validate the later
documentation, package-description, and rustdoc refresh. Both platforms run the
five HTTP smoke modes, including the two WARN-filtered profiles. The macOS job
also runs the adapter integration tests and live database-independent example
tests. This remains focused hosted evidence, not a full hosted macOS matrix or
hosted PostgreSQL validation.

The Rust workflow runs on pull requests, pushes to `master`, merge groups and
manual dispatch. Feature-branch pushes use their PR run instead of starting a
second complete matrix; branches without a PR can use manual dispatch. Linux
verification calls the same complete Jig profile, so its former standalone HTTP
step is removed. The focused macOS job retains its explicit smoke step.
Superseded runs for the same PR/ref are cancelled. Regular CI runs six matrix jobs:
two Linux verification jobs, two focused macOS jobs, and two native Runlimit
PostgreSQL jobs. Mondays at 04:23 UTC, the scheduled run executes only the Linux
verification job with `stable`, including all five HTTP smoke profiles. Its
concurrency group is separate from regular CI. Scheduled execution starts after
the workflow reaches the default branch; no hosted result for this change is
claimed. Existing test, lint, documentation and HTTP smoke commands remain
required, with no cache-hit condition bypassing them. Job timeouts are 45 minutes
for full Linux verification, 30 for macOS and 15 for native Runlimit PostgreSQL.

The pinned Rust cache action restores Cargo downloads and compiled dependencies
after toolchain installation. Its default keys separate jobs, compiler/host,
manifests, lockfiles and compiler environment. Only `master` saves caches, which
PRs can restore; this avoids per-PR cache churn. Workspace and incremental build
artifacts are excluded by the action, which disables incremental compilation.
Temporary isolated-consumer builds still run in their own target directories.

For the optimization baseline, [PR run 35697207609](https://github.com/bpcakes/batter/actions/runs/35697207609)
took about 26 minutes elapsed and 92 total runner-minutes across its seven jobs;
the same branch also started [push run 35697202800](https://github.com/bpcakes/batter/actions/runs/35697202800).
Eliminating that duplicate reduces two full Rust matrices to one per PR update.
The optimized workflow passed [PR run 35713923253](https://github.com/bpcakes/batter/actions/runs/35713923253)
at `19c3575`, before the at-rest merge; cache speedups remain unmeasured.
This public repository's standard hosted runner usage does not establish
a billed dollar saving. See [primary references](references.md#github-actions-scheduling-and-caching-reviewed-2026-09-22).

The workspace enables `clippy::cognitive_complexity` and `clippy::too_many_lines`
at warning level in all workspace packages. Root `clippy.toml` sets their thresholds to
20 and 100 respectively. The existing `-D warnings` verification step enforces
both limits across libraries, examples, and tests.

## Explicit reference compatibility probes

The [reference package](../examples/reference-service/README.md) has the required named
ignored cases in `tests/reference_live.rs`: the atomic command, upstream compatibility,
fixture lifecycle and configuration probes. `scripts/reference_live.py::CASES` owns the exact inventory. The command case applies both migration histories twice,
uses the production authenticated router, and checks exact replay, canonical
payload conflict, foreign-owner isolation, replacement-generation fencing,
discarded-response reconciliation, pending state and exact command/delivery/job
counts. It also checks unresolved native terminal states and missing-effect
corruption through both authenticated read routes, preserving uncertainty and
owner isolation. The production-root case rechecks readiness beyond the health
freshness window, disables connections to its disposable database, requires
unready, restores connections, and requires readiness recovery. Its retained
control connection restores database availability before returning an error.
The readiness scenario owns its database-fixture entrypoint and derives its
completion-observation budget from both signal cases' actual phase limits,
process bind/reap limits, two announcement-failure child reap bounds and explicit
fixture headroom (currently 182.4 seconds).
The generic 30-second fixture budget is not suitable for these serial phases.
Offline process controls assert the serial allowance; a separate seven-state,
two-writer table verifies terminal-source rejection before SQL acquisition.
The live state probe additionally supplies stale unresolved arguments against
all four retained terminal states and a different unresolved state; both writers
must return an invariant failure and preserve the complete row, including
provider identity, uncertainty and timestamps.
The observation budget does not guarantee termination of arbitrary database I/O
or claim cleanup completion after a timeout.
The native provider-timeout regression uses a loopback peer that withholds
headers or stalls a declared response body. A native reqwest read timeout must
retain `delivery.provider_timeout` and release provider capacity; the enclosing
operation budget is deliberately longer. The admission-interruption live case
also checks that `delivery.admission_interrupted` reaches the retained effect
row, not merely the pure diagnostic mapper.
Fixture cases cover template reuse/isolation, acknowledged
lock operations, returned body errors, partial/sibling acquisition, panic,
resumable wait cancellation, foreign-template rejection, simultaneous body/cleanup
errors, observer failure, cancelled native creation, abandoned producer errors
and shared-harness waiting. The owned runner preserves all teardown results;
independent catalog queries verify absence. Low-level finish controls additionally
check absence before deferred drain.
Additional cases prove native detach and adapter-retired backend retention past
pool close, real observer failure and explicit retry, distinct consuming/deferred
cleanup failure resources, handled pool error identity, assertion plus Script
exhaustion, live waiter loss and actual runtime destruction. The session cases
use an independent one-slot admin pool and five-second witness budgets, with a
two-second detached-session observation attempt and 30–40 ms cancelled/pending waits. They
require database presence and exact retired backend identities before releasing
the acknowledged lock. The runtime-loss test holds its checkout through native
Drop deletion, demonstrating the absence of a runtime-death completion guarantee.
The deferred fault test separately drains external mode and recovers both tagged
residual databases after releasing the catalog lock. Every new case belongs to
the checked Python inventory and its 180-second external process watchdog.
The retry follow-ups exercise the actual shared bounded completion helper:
a pending error returns in 30 ms with its run/control intact, repairs a closed
observer pool and retains the original body and observation causes. Another case
blocks two observer connection-initialization queries behind an acknowledged
PostgreSQL advisory lock, sends a retry while both three-second attempts are active,
then recovers its pending completion owner and checks both ordered failure histories,
native error identities and successful recovery. The same oracle runs on current-thread
and two-worker Tokio runtimes. Detached-session diagnostics have their own
pool, acknowledge the blocking relation before body exit, and witness backend
absence before requesting the bounded retry. The outer-helper regression holds a diagnostic checkout while the body waits
for a release signal. It requires a pending return before release, verifies that
the pool remains open, then resumes the same body/run and checks database absence.
Ordinary shared fixture helpers use a
30-second whole-run completion budget; their session attempts are 10 seconds for
`run` and five seconds for `observed_failure`.
The terminal-driver regression destroys an unpolled driver runtime, then closes
its admin pools on a surviving runtime. Real held checkouts force a bounded
pending return; release and resume must close both pools and return the original
cancelled task ID. The no-checkout branch requires closure before error return. A successful cached
report with a held session checkout must identify admin-close pending distinctly
and leave diagnostics usable after recovery. Offline report tests isolate handled
pool errors and recovered observation failures with all other branches successful,
and check native source identity, failure counts and source precedence.
Ten additional cases cover a real wrong-server/missing-target observation,
restricted observer logins seeing other users' database identities,
shared retries across runs, updates before the first attempt, coordinated and
premature shared-pool close, template-clone recovery, multithreaded in-flight
retry, SCRAM startup and a real autovacuum worker. The startup negative control
keeps a connection paused before database assignment: observation advances to
native lease cleanup while its `datname` is null; completion stays pending until
that connection closes. This is an executable limit, not a connection fence.
The autovacuum control witnesses the worker and retained database past the attempt
budget, disables further ordinary vacuum launches, observes session exit, then
explicitly retries. Neither case authorizes future producers during cleanup. A deliberate assertion
panic in the restricted-login test body is joined before pool closure and DROP
ROLE; a native JoinError remains inspectable after the role is confirmed absent.
Each login gets a PostgreSQL-generated random password expiring after five minutes;
the live probe checks that finite deadline. Expiry bounds password authentication
after a killed process but does not remove the role or terminate existing sessions.
The completion regression also holds an original admin checkout while a later
replacement pool starts closing, then resumes the same completion owner.

The native lifecycle cases now use `batter-runledger` and owned native preparation.
They exercise queue-independent initialization, in-flight work completing after
drain, retained settlement after owner loss, durable business failure without
process failure and a non-yielding callback that remains unjoined at report time.
The production-root child requires `/live` and eventual `/ready` 200, no
control-job rows, one confirmed provider effect per run and successful awaited
cleanup in sequential SIGTERM and SIGINT runs. Its parent settles both the
assertion task and child shutdown before combining them, retaining both errors if
they fail together; the offline `child_fixture` control checks that dual-failure
branch.

Two provider-effect cases use a parent-owned real loopback HTTP fixture and the
actual production binary. The crash case withholds an accepted POST response,
independently queries a leased native job plus `reconcile_needed`, SIGKILLs and
reaps that process, then starts the ordinary root again. It proves ordinary
confirmation and repeats the crash window with generation replacement between
attempts; restart must reconcile the accepted effect into manual resolution
rather than business denial. The outcome case checks same-key replay and mismatch
conflict, three bounded native handlers behind one provider permit, business
denial, structured known non-dispatch, opaque-text lookup-before-replay, bounded
exhaustion, generation replacement after provider acceptance, retention
expiry/manual resolution and later work after terminal outcomes. A retained
already-expired local resolution window reaches manual resolution without a
provider lookup or dispatch, even when the fixture would report authoritative
absence. A retained
near-expiry uncertainty plus authoritative absence proves that the following
fresh POST receives a newly anchored 24-hour reconciliation deadline. Its admission
phases prove that fresh work waits without immediate attempt loss, terminal
redelivery completes without provider admission, and an admission deadline while
`reconcile_needed` preserves acceptance possibility and the resolution deadline.
The same outcome case holds the authoritative job-row lock after provider
acceptance, lets the handler reach its confirmation fence, revokes the native
lease before releasing the lock, and requires the stale handler to leave the
effect in `reconcile_needed`. Offline transport cases require the identical
canonical request echo for POST and GET, classify only connector failure as
known non-dispatch, and keep missing/mismatched responses conservative.
Runledger still owns the already-claimed attempt; provider waiters are bounded by
its configured global handler concurrency.
The outcome case independently corrupts the persisted provider payload and key,
requires invariant rejection from both owner-scoped reads and worker loading,
and witnesses zero provider requests. With a future dispatch lower bound, keyed
GET must still run: accepted lookup confirms without POST; authoritative absence
preserves the complete effect row and schedules native retry no earlier than the
retained lower bound. A pure planner case separately pins reconciliation before
dispatch eligibility and keeps expired reconciliation terminal.
The crash/restart probe has a 75-second fixture-completion bound. The composite
outcome probe has a separate 180-second bound because its sequential phase
bounds, including the roughly 30-second locked native retry schedule, exceed 75
seconds before cleanup. Running production children select port zero themselves,
send the bound loopback address once to a private Unix datagram receiver bound
by the parent before spawn, and must exit zero with empty stdout and stderr after
checked SIGTERM/SIGINT shutdown; the separately held
acquisition controls still require the documented startup-failure exit.
Seven offline retirement cases separately exercise legacy disable, quiescence,
preservation and uncertain outcomes. The current inventory is listed below;
the earlier example-owned witness/lease protocol was removed.

The private `provider_state_lock_and_retry_boundaries` probe also witnesses both
confirmation/replacement lock orders using `pg_blocking_pids`: replacement-first
requires manual resolution; confirmation-first blocks replacement until the
confirmation commit. An expired lease during the record wait must leave retained
uncertainty untouched, and a NOWAIT job-row acquisition proves the record wait
does not hold the heartbeat lock. Offline Unix announcement tests exercise absent
configuration, exact address payload, missing receiver and saturated receiver
settlement; configuration tests reject relative, NUL-containing and oversized
paths. These are native Unix cases, not a general stdout cancellation guarantee.
The production-root case additionally starts a typed assertion child with no
announcement receiver, requiring the protected startup report at `http.bind`,
the retained `io::ErrorKind::NotFound` cause and successful `postgres.pool`
cleanup. It then runs the actual executable with a missing receiver and requires
natural exit1, empty stdout and the exact sanitized stderr diagnostic. Each child
has the existing eight-second reap bound; watchdog termination is failure, not
successful settlement evidence. Its ordinary receiver/readiness phases remain
the positive control, and the live inventory remains 66 cases.

Ordinary reference HTTP tests separately pin the composition boundaries: a bare
in-process `/live` request succeeds without `ConnectInfo`, while an authenticated
business request with malformed JSON, an invalid UUID path, and a body exceeding
the configured limit return Axum's native plain-text 400/413 responses plus the
outer generated response-ID header, not an application problem envelope. The
database delivery fixture therefore captures raw status/header/body first and
applies JSON envelope and matching body/header identity assertions only for
routes whose contract promises that envelope.

Their [API manifest](reference-compatibility.md) states the exact scope and pins.

```sh
cargo check -p batter-example-reference-service --all-targets --all-features --locked
POSTGRES_TEST_ADMIN_URL='postgres://postgres:fixture@127.0.0.1:5432/postgres?sslmode=disable' \
POSTGRES_TEST_OBSERVER_URL='postgres://postgres:fixture@127.0.0.1:5433/postgres?sslmode=disable' \
  bash scripts/test_reference_live.sh
```

Select two dedicated disposable local PostgreSQL 18 servers. The primary needs
superuser authority for temporary restricted-role controls, SCRAM host authentication
for the startup handshake, and both autovacuum and track_counts enabled with
`autovacuum_naptime <= 5s`, plus `max_prepared_transactions > 0` for offline
retirement acceptance. Use `postgres -c autovacuum_naptime=1s -c
max_prepared_transactions=10` when provisioning it. The secondary must
be a different cluster and allow `pg_control_system()` for identity preflight;
PostgreSQL 18.4 permits this by default. If that access was revoked, grant
`EXECUTE ON FUNCTION pg_catalog.pg_control_system()` to the observer login in
its selected administrative database. No target fixture databases are created
there. Matching cluster identities are
rejected before compiling or running fixtures, including endpoint aliases. The runner
rejects missing endpoints and insufficient primary authority before fixtures.
Failure controls briefly lock that shared system catalog;
keep other workloads off this endpoint. The runner executes cases serially.
This entry point deliberately executes the complete inventory; individually selected
Cargo probes do not establish complete live-inventory verification.
The runner invokes the `reference_preflight` Rust example, which shares endpoint
validation with fixture acquisition and authenticates through the same explicit
SQLx options as the configured startup probe. It rejects PG* environment entries,
missing/remote/TLS-required endpoints and unsupported query parameters before
fixture creation. Both endpoints are validated before either connection opens.
The full suite requires SCRAM on the primary; use explicit URL credentials.
The database-only maintenance constructor also supports separately selected passwordless
endpoints for focused probes; passfiles are not consulted. The runner
checks the server and ignored-case inventory, and requires every named case to run.
The existing Unix process owner allows 300 seconds for preflight compilation/run,
300 seconds for compile/inventory and 180 seconds for live execution, plus bounded
signal escalation/reaping. For each endpoint, native acquisition and the
prerequisite query each have a ten-second budget; its pool closes before the
result is interpreted or the next endpoint opens.

The inside-target observer case now uses the shared completion helper for the
wrong pool and corrected retry. It recovers the pending owner, explicitly closes
the wrong native pool through its observer_pools accessor, witnesses backend exit
and then retries. No external wrong-pool clone is needed. Replacing
a pool alone does not close its database sessions. Redacted report controls
distinguish successful cleanup with retained failures from failed consuming cleanup.
The Rust controls reject failed prerequisite results and equal cluster identities,
including signed 64-bit extrema; Python controls require successful native preflight
before inventory and exact execution. Preflight does not certify SCRAM for a future disposable
database: pg_hba rules can differ by database. The raw startup case checks the
actual authentication exchange for that database and fails on trust authentication.

The autovacuum oracle records the actual vacuum_probe relation OID and selects
its worker through pg_stat_progress_vacuum while most heap scanning remains. It
rechecks that same pid/database/relation after the observation timeout. A transient
launcher visit or vacuum on another table cannot satisfy that witness. The dedicated
serial server and bounded cooperative scheduling assumptions still apply.

The startup probe witnesses the actual target DROP backend waiting on
`ProcSignalBarrier`, rechecks that same backend after the bounded pending wait,
then closes the paused socket and awaits completion. Startup identity uses before/
after backend sets on the dedicated serial server: container proxies can rewrite
the client's port, so a frontend socket tuple is not a portable server-side identity.
No unrelated connection producer may run during that identity window.
Watchdog termination is failure and does not claim application/database cleanup.
The earlier serial sixteen-case suite took 5.54–5.61 seconds in the round-one
Linux measurements after compilation, leaving over thirty times that measured
duration within the 180-second bound. This is a workload backstop, not a promise
to complete arbitrary stalled native operations. Arbitrary slower environments remain unverified.
Compile time has its separate inventory bound.
`scripts/test_reference_live.py` checks zero-test, skipped, missing-case and native preflight
failure/ordering behavior in the ordinary test matrix.

Live test failures print only known redacted report counts and combined failure
branches. The adapter does not format native error contents or arbitrary source chains;
upstream harness cleanup diagnostics and the default panic hook can still print. The separate offline `fixture_diagnostics` target verifies actual
panic output for simultaneous body/observer failures and unknown-error redaction;
it does not change the exact ignored live inventory. Fixture report doctests
reject discarded owned and borrowed observations with `unused_must_use` denied.
Fingerprint controls vary migration identity and kind independently of SQL.
Warm-cache reuse permits zero initializations; cold initialization is established
only by fresh-cluster execution, not by that count alone.

The focused `cargo test -p batter-example-postgres-lifecycle --test native_sqlx
--locked` target checks native pool/connection/transaction composition without
requiring Runledger or a database. Its unpolled-factory test makes no live SQL claim.

## Jig verification

Facade consumer checks reuse artifacts under
`<cargo-target-directory>/facade-features/<compiler-digest>`. The target directory
comes from root Cargo metadata, honoring `CARGO_TARGET_DIR` and Cargo configuration;
the digest covers `rustc -vV`, including its version and host. Temporary manifests,
source files and reconciled lockfiles remain independent. Every graph check,
positive compilation, expected negative diagnostic and external runtime test
still executes. `scripts/test_facade_features.py` exercises an enabled at-rest
consumer followed by disabled imports against the same cache, and requires the
negative-test oracle to reject a deliberately enabled import. Its final disabled
import also guards against the preceding successful consumer masking rejection.

The workspace test profile optimizes only `batter-at-rest` at level 2, with debug
assertions and integer overflow checks explicitly enabled. This retains the full
64 MiB seal/open boundary test and every existing test selection. Level 2 avoids
the unoptimized shared generic instantiations that limited the measured level-1
package override. Other packages retain their existing profiles, including the
native release-mode regression. Detached at-rest package checks retain Cargo's
default profile because the override belongs to this workspace, not the leaf
manifest. See [Cargo profile semantics](references.md#cargo-test-artifacts-and-profiles-2026-09-24).

The repository pins Jig v0.5.0 at commit
`a328c17910c40603327c73329e5158a42c37417d` with contract v8 for scoped
target freshness. The earlier unreleased contract v9 epoch was migrated to v8;
historical Jig state remains append-only, and new checks establish v8 evidence.
The Rust check targets declare `source_state = "worktree"` because they read working
files; tracker-only staging or commits can reuse their passing receipts.
Use `scripts/jig info` to inspect the source SHA and contract epoch. Use
`scripts/jig update --recopy` to retain this revision. Plain
`scripts/jig update` advances to the upstream default branch.

On a fresh checkout, run `scripts/jig doctor` before starting an MCP client.
The first invocation builds the repository-local runtime using Cargo, Git,
Bash, and Python 3. The runtime profile disables Jig's optional dev proxy.
MCP startup requires an already installed runtime.

`scripts/jig doctor` checks harness readiness; `scripts/jig check` runs the
configured Clippy, formatting, locked core/all-feature/doctest, contract, and
file-budget gates. Both test aliases and `verify.sh` use `python3
scripts/test_matrix.py`. It checks minimal core compilation, the process-runner
regressions and SQLx smoke controls first, then overlaps the core and workspace runtime test commands,
then runs doctests if both passed. Both dependency feature configurations remain
covered. `verify.sh` delegates to this profile rather than invoking the matrix
again. The independent `api:docs` and `api:http-smoke` targets require rustdoc with
warnings denied and all five process profiles against the Cargo-reported build
artifact. Local verification uses the pinned toolchain once; CI owns the MSRV matrix.

For final backend verification, a fresh passing `api:test` receipt from the current
plan's gate/profile run also satisfies the final-test requirement. Inspect
`scripts/jig work evidence --plan-id <id>` and `scripts/jig work gates --plan-id <id>`
before deciding to run `scripts/jig check test` again. Reuse requires unchanged
check inputs, command/configuration, toolchain and relevant environment/prerequisites,
and no later unresolved failure. Toolchain and external-state identity are not
established by Jig's fingerprint alone. `bash scripts/verify.sh --plan-id <id>`
uses `work check` to reuse those receipts and execute missing or stale required
targets. Without a plan ID, `verify.sh` uses `jig check --profile verify` for a fresh
run with receipts and a policy comparison against `origin/master` (which CI
fetches). The complete profile, rather than `api:test` alone, includes
rustdoc and HTTP smokes. CI's MSRV verification remains separate.

The `verify` profile requires independent Clippy, formatting, tests, rustdoc, HTTP
smoke, contract and file-budget targets. Keep these as siblings; `depends_on` is for actual execution
prerequisites. The Rust targets, including the rustdoc and HTTP smoke siblings, declare
`inputs_policy = "exhaustive"`: workspace
manifests and lockfiles, toolchain/Cargo/lint configuration, package source,
examples, tests, benches and migrations, and shared test sources. Test targets
also cover the Python helpers and shell entrypoints under `scripts/`. When a
check starts consuming another path, update both `.jig.toml` and the resolved
`.agent/jig-contract.json` inputs. Use scoped prefixes rather than `**/*.rs`:
exhaustive globs must not intersect ignored build or cache directories.

Root tracker exports and root documentation are outside the Rust targets' input
scopes. Closing a Bead does not invalidate their receipts. Contract and file-budget
retain conservative whole-repository scope; after tracker closeout, run
`scripts/jig work check --plan-id <id>` to refresh these inexpensive policy checks
and reuse the original Rust passes. Configuration changes intentionally invalidate
previous evidence. A file-budget policy edit also invalidates that native target.
This is per-target freshness, not a global exclusion of tracker data.

If inspection reports `unknown` with reason `collection_limit`, increase its
read-only budget before deciding that checks need execution:

```sh
scripts/jig work evidence --plan-id <id> --freshness-timeout-ms 30000
scripts/jig work gates --plan-id <id> --freshness-timeout-ms 30000
```

The default inspection budget is two seconds; this checkout can exceed it.
These commands validate existing evidence and do not run checks. A remaining
unknown result needs diagnosis; increasing the budget is not a freshness bypass.

For targeted native recovery use
`scripts/jig check repo:file-budget --plan-id <id>` and let Jig derive the
comparison from the work plan. `work check --tool jig.file_budget` produces legacy
evidence and cannot satisfy a native target gate. Supplying explicit comparison
provenance can also differ from the gate's work-plan comparison authority.

The private `parallel_process.py` runner owns at most four direct command groups,
keeps bounded output while continuing to drain overflow, and returns all outcomes
in command order. Any unsuccessful, incomplete, overflowing or interrupted outcome
fails verification. SIGINT/SIGTERM stop further launches and request SIGINT in
owned groups; repeated signals do not reset the grace period before kill/reap.
Inherited ignored signals remain ignored. An exited group leader is retained
until pipe EOF or termination; a reaped leader never authorizes another group
signal. Matrix commands have a 1,500-second per-command watchdog, ten seconds of
grace and five seconds per final reap/output observation. These are local process
bounds, subject to OS scheduling and process creation; they do not establish
cleanup of arbitrary detached descendants. Test output appears in grouped logs
when commands finish, with each outcome and elapsed time. Each matrix command
retains at most 8 MiB of source output: half for the initial reads and half for
rolling tails, shared fairly between stdout and stderr with unused tail space
available to the other stream. Truncated streams receive explicit omission markers
(at most two additional marker lines); overflow still fails verification. Logs
within the limit remain exact. Machine-readable scheduling and mutation capture
keeps its prefix-only policy.
The matrix executes seven batches with command counts `[4, 4, 4, 3, 1, 4, 3]`.
Runner controls inject a failure at every one of those 23 command positions and
require that no later batch starts; surplus mocked outcomes cannot stand in for an
unexecuted runtime or doctest batch.

Jig's database tooling is disabled because SQLx currently appears only in an
example package. There are no migration or prepared-query metadata gates.
Workspace checks execute the startup/shutdown error-retention unit tests and
bounded subprocess checks of portable exit paths and redacted configuration
failures. Explicit live query and pool-closure checks are separate.
Agent bootstrap requests only the Rust and ExecPlan plugins. No frontend,
development app, or external status provider is configured. Vault scope metadata
is retained for Jig compatibility; these checks do not need a vault passphrase.

The existing `ci.yml` owns the Rust/toolchain/HTTP matrix. `repo-policy.yml`
adds Jig installation, contract, guide, file-budget, and integration regression
checks. `batter-at-rest-portability.yml` separately verifies detached source,
packaging and an external consumer on Rust 1.94.0 when its crate, gate script or
workflow changes; merge groups and manual dispatch also run it. Its gate creates
a fresh Cargo home and target directories, independently of the workspace cache.
The policy workflow caches only installed runtime directories, keyed by runner OS and
architecture plus the source/configuration, contract, toolchain, installer, and
workflow contents. A cache miss builds the selected revision; a hit still passes
Jig's compatibility and source-stamp checks. Cache reuse on hosted runners has
not yet been exercised.

`scripts/check_file_budget.sh` retains the exact event base for pull requests,
pushes, and merge groups. Manual dispatch compares against `origin/master`,
which exists in GitHub's full checkout even when local `master` does not.
Missing exact bases remain errors; push-before zero SHAs retain Jig's explicit
empty-tree handling.

Run the Jig integration and scheduling process regressions
after installing the Jig runtime:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

The Jig tests execute the CI helper with the real Jig runtime in disposable Git repositories,
verify budget enforcement and missing-base behavior, exercise a restored runtime
with Cargo blocked, check actual Git merges, and inspect generated ZIP contents.
Freshness controls retain the real target scopes and native policy checks, replace
expensive commands with execution counters, and check tracker/documentation reuse
across dirty, staged and committed states. Source, shared fixtures, migrations and
new or changed Python helpers must invalidate test evidence; a targeted native
file-budget refresh must preserve the original Rust receipts.
The Python tests use the standard library and do not run or provision PostgreSQL.
The scheduling target additionally supplies its compiled Rust binary to run six
launch-protocol controls; standalone Python discovery runs the process controls.
Markdown plans use normal text merging so contradictory edits conflict;
append-only JSONL records retain union merging.

Repository customizations omit the generated duplicate Rust/agent-map workflows and the
checkout helper. Review `jig update` output before accepting it: a full template
refresh can restore these defaults and replace managed guide/ignore blocks.
Keep the three-workflow split, regression checks, runtime cache, and Rust-only
settings when refreshing the harness. The plan merge override sits outside the
managed attributes block so a regenerated union rule cannot silently replace it.

The two budget-exhaustion observation tests run in their own integration-test
executable, isolating scoped log capture from concurrent subscriber-free cleanup
tests. Their report, event, invocation, and capture-order assertions are unchanged.
Static current-document link inspection includes active maintenance guides and
`.agent/PLANS.md`; it excludes append-only historical Markdown under
`.agent/plans` and `.agent/reviews`.
Source archives include the executable Jig launcher, contract, and durable work
records; local caches, runtime data, scratch files, and the deprecated adoption
receipt are excluded.

## Authored coverage map

| Contract | Tests |
| --- | --- |
| Deadline clamping, preflight rejection and downward child cancellation | [operation.rs](../crates/batter-core/tests/operation.rs) |
| Scope cancellation on success/drop, owned-future drop on timeout | [operation.rs](../crates/batter-core/tests/operation.rs) |
| Finalization reserve validation, sibling cancellation and original deadline | [operation.rs](../crates/batter-core/tests/operation.rs) |
| Concrete errors, borrowed futures, panic separation | [operation.rs](../crates/batter-core/tests/operation.rs) |
| Replay prohibition, fresh futures, attempt counts, classifier stop | [retry.rs](../crates/batter-core/tests/retry.rs) |
| Same-poll attempt cancellation before legacy success acceptance; same-poll input cancellation retains an unclassified returned error | [retry.rs](../crates/batter-core/tests/retry.rs) |
| Provider lower bounds, shared budget, capped backoff | [retry.rs](../crates/batter-core/tests/retry.rs) |
| Injected jitter endpoints, seed reproducibility, provider floor and reserve composition | [retry.rs](../crates/batter-core/tests/retry.rs) |
| Interrupted backoff/later attempt retains previous failure | [retry.rs](../crates/batter-core/tests/retry.rs) |
| Per-attempt cap formula, total/same-poll cancellation precedence, current and earlier retained errors, destruction and panic | [retry_attempt_deadlines.rs](../crates/batter-core/tests/retry_attempt_deadlines.rs) |
| Retry attempt telemetry matches application failure, same-poll cancellation and success without logging error contents | [telemetry.rs](../crates/batter-core/tests/telemetry.rs) |
| Permit exhaustion/release, deadline wait, close/cancellation | [admission.rs](../crates/batter-core/tests/admission.rs) |
| LIFO, all errors, async and synchronous-factory panic observation | [cleanup.rs](../crates/batter-core/tests/cleanup.rs) |
| Timeout/reap before dependent hook, total budget, explicit skips | [cleanup.rs](../crates/batter-core/tests/cleanup.rs) |
| Every budget-skipped hook logged/reported once; native capture-drop LIFO | [cleanup_observation.rs](../crates/batter-core/tests/cleanup_observation.rs) |
| Dropping an active cleanup driver aborts its hook and does not start dependencies | [cleanup.rs](../crates/batter-core/tests/cleanup.rs) |
| Inert registration, monotonic readiness, early success as failure | [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs) |
| Error/panic observation, drain/cancel distinction, abort reports | [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs) |
| Dependency health freshness, exhaustive observation-to-readiness classification, unrepresentable healthy-as-failure state, 2,000 concurrent read-only observations, sequential probes, combined acquisition/query timeout, recovery, writer loss, drain/abort/destruction and safe publication | [health.rs](../crates/batter-core/tests/health.rs), foundation [readiness](../crates/batter-core/src/readiness.rs), and [ownership](../crates/batter-core/tests/health/ownership.rs), [publication](../crates/batter-core/tests/health/publication.rs) |
| Owned startup waiter/owner loss, constrained registration, acquisition-registration barriers, LIFO failures, initialization deadline, returned-error/destruction panic, simultaneous drain/destruction classification and readiness/handoff | [startup.rs](../crates/batter-core/tests/startup.rs), [protected_startup.rs](../crates/batter-core/tests/protected_startup.rs), [registration.rs](../crates/batter-core/tests/registration.rs) |
| Protected synchronous signal install, policy precedence, reserved identity, retained injected IO/destructor/cleanup causes, deterministic same-poll failure/success reception, TERM/INT during start and running, cleanup-owned observation of delayed repeated signals without deadline restart, and unconfigured/unstarted default-disposition controls | [startup_signals.rs](../crates/batter-core/tests/startup_signals.rs), injected driver controls in [driver.rs](../crates/batter-core/src/startup/driver.rs), and lower-level controls in [unix.rs](../crates/batter-core/src/lifecycle/unix.rs) |
| Cleanup after task stop, partial startup, failed finalization | [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs) |
| Completion/drain classification regression | [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs) |
| Never-polled ordinary/unapproved caller-owned driver drop signals readiness/cancellation before captured values are destroyed; unapproved spawned-owner drop requests drain before its driver poll; borrowed non-Send shutdown | [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs), [lifecycle_state.rs](../crates/batter-core/tests/lifecycle_state.rs) |
| Component and application startup acknowledgement, deterministic caller-owned driving before and after approval, dropped unacknowledged capabilities, deferred/default startup handoff, last-owner and waiter drop, retained driver failure; compile-fail rejection of read-only, cloned, repeated, and post-transition approval | [lifecycle_state.rs](../crates/batter-core/tests/lifecycle_state.rs), [process_ownership.rs](../crates/batter-core/tests/process_ownership.rs), [startup.rs](../crates/batter-core/tests/startup.rs), `ComponentStartup`/`ReadinessApproval`/`ShutdownSignal` rustdoc in [capability.rs](../crates/batter-core/src/lifecycle/capability.rs), caller-owned typestate rustdoc in [caller_owned.rs](../crates/batter-core/src/lifecycle/caller_owned.rs), and owned-driver typestate rustdoc in [driver.rs](../crates/batter-core/src/lifecycle/driver.rs) |
| Completion observers require an owned driver; immediate last-owner drop before first poll retains success or coordinator panic; a new observer after publication retains its report after owners/runtime drop; destroying an unpublished monitor's runtime causes the documented observer panic | `ShutdownHandle` compile-fail rustdoc in [capability.rs](../crates/batter-core/src/lifecycle/capability.rs) and [driver_observer.rs](../crates/batter-core/tests/driver_observer.rs) |
| Finite capacity/receipt ownership, descendants, typed failure vs normal business denial | [process_ownership.rs](../crates/batter-core/tests/process_ownership.rs) |
| Initial finite errors/panics use `FiniteTaskExit`; critical errors/panics/early success retain `ComponentExit`; later finite failures preserve `Requested` | [process_ownership.rs](../crates/batter-core/tests/process_ownership.rs), [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs), and the `ProcessHandle::try_spawn` rustdoc example |
| Mixed critical/finite ownership, first observed failure retained across task kinds, descendant cause labels, ready requests preceding unobserved finite/critical failures, and shutdown aborts preserving `Requested` | [shutdown_causes.rs](../crates/batter-core/tests/shutdown_causes.rs) |
| Shutdown, task, cleanup and record Debug/Display redact retained error contents while the causes remain reachable through explicit fields | [report_redaction.rs](../crates/batter-core/tests/report_redaction.rs) |
| Permanent admission closure before startup, unpolled driver drop/abort, dropped unstarted supervisor, post-start abort and completed shutdown; rejected factories stay inert | [terminal_admission.rs](../crates/batter-core/tests/process_ownership/terminal_admission.rs) |
| Startup permutations, irreversible transition table, concurrent request/completion, admission precedence, nonblocking readiness snapshot and each waiter notified outside the transition lock, allowing redundant wakes | [state/tests.rs](../crates/batter-core/src/lifecycle/state/tests.rs) |
| Read-only status waits and operation admission across Starting/Ready/Draining; admitted-context drain/cancel behavior; abandonment notification, guard transfer and component/cleanup capture destruction order; extracted cleanup independent of process cancellation; inert factories and invalid-name/startup/capacity/closure precedence | [lifecycle_state.rs](../crates/batter-core/tests/lifecycle_state.rs) and [lifecycle.rs](../crates/batter-core/tests/lifecycle.rs) |
| Multi-thread admission/drain and startup/drain races | [process_ownership.rs](../crates/batter-core/tests/process_ownership.rs) |
| Seeded root/descendant contention, cancellation/readiness/drop schedules, complete failure accounting, replay and watchdog controls | [scheduling.rs](../crates/batter-core/tests/scheduling.rs) and its [profile](../crates/batter-core/tests/scheduling/profile.rs) |
| Delayed observation of completed success/error/panic; abort only unfinished work | [process_ownership.rs](../crates/batter-core/tests/process_ownership.rs) |
| Non-yielding direct work, ordinary-start approval visible immediately after component acknowledgement, unjoined reports, skipped cleanup, blocked runtime destruction/timers; watchdog kill/reap and failure cleanup | [non_yielding.rs](../crates/batter-core/tests/non_yielding.rs) and its [fixture](../crates/batter-core/tests/non_yielding/fixture.rs) / [watchdog](../crates/batter-core/tests/non_yielding/watchdog.rs) |
| Axum context/probes/gate/deadline/sanitized responses; budget validation | [http.rs](../crates/batter-axum/tests/http.rs) |
| Configured envelope/status/headers, original trusted metadata on timeout/cancellation | [http.rs](../crates/batter-axum/tests/http.rs) |
| Ordinary INFO completion, scoped context and error redaction | [core telemetry](../crates/batter-core/tests/telemetry.rs) |
| Actual HTTP status and request/error redaction | [HTTP telemetry](../crates/batter-axum/tests/telemetry.rs) |
| Independent HTTP observation and complete-router coverage | [Composition](../crates/batter-axum/tests/observation/composition.rs): startup/ready/drain, probes, fallback, rejection, application errors, new routes, trusted correlation and redaction. |
| HTTP event fields independent of span filtering | [Event fields](../crates/batter-axum/tests/observation/event_fields.rs): direct event visitors check typed fields at all severities, WARN/ERROR with INFO spans disabled, admission rejection, unmatched routes and future destruction under another ambient subscriber. |
| Observation filtering and middleware order | [Composition edges](../crates/batter-axum/tests/observation/composition_edges.rs): DEBUG/TRACE overrides suppressed by INFO with a WARN positive control, outer status/severity rewriting after observation, and retained overrides read by each nested observer. |
| HTTP context ownership and handler unwinds | [Correlation](../crates/batter-axum/tests/observation/correlation.rs): mixed target filters, interleaved completion/drop under other spans/subscribers, retained parent lifetime, untouched application fields, no replacement of an absent parent, propagated task panic, cancelled admitted context and sanitized dropped observations. |
| Foundation decision and example readiness policy over real HTTP | Foundation unit/rustdoc tests exhaust every lifecycle/health classification, deterministically require dependency sampling before lifecycle, and reject `Dependency(Healthy)`; Axum rustdoc makes the old reason import fail loudly, while [operational tests](../crates/batter-axum/tests/operational/readiness.rs) preserve all valid typed decisions, reusable status/severity mappings, partial default severity delegation and drain precedence; [example tests](../crates/batter/examples/http_service/tests.rs) cover the actual router's Starting/Ready/Draining/Stopped statuses and event levels with generated request correlation. A separately owned listener stays available through all phases; this does not prove the binary's connection shutdown timing. |
| Explicit HTTP observation severity | [Severity](../crates/batter-axum/tests/observation/severity.rs): all five event levels, preserved defaults/status/outcome/identity/response data, expected readiness vs unrelated unguarded failure, custom rendering, middleware override replacement/removal and ignored request-side hints. Rendering tests include forced cancellation with an ERROR override on a 429; lifetime tests keep dropped futures at WARN despite an annotated response constructed but never returned. |
| Split/legacy rendering and request lifetime | [Rendering](../crates/batter-axum/tests/observation/rendering.rs) and [lifetime](../crates/batter-axum/tests/observation/lifetime.rs): actual custom status, original metadata, timeout, forced cancellation, inert unpolled futures, cross-subscriber abort/destruction, context cancellation and post-response body drop. |
| Axum middleware placement and observation ownership | [Placement](../crates/batter-axum/tests/observation/placement.rs): existing vs late-added routes, pre-routing missing metadata, observation-free admission and explicit duplicate observations when wrapping the legacy middleware. |
| Operation abort; public wrapper capture at call, unpolled destruction, borrowed/non-Send work | [core dispatch](../crates/batter-core/tests/scoped_dispatch.rs) |
| HTTP abort under another subscriber, including nested span destruction | [HTTP dispatch](../crates/batter-axum/tests/scoped_dispatch.rs) |
| Unpolled !Unpin future capture and nested span destruction under its saved dispatcher | [private wrapper](../crates/batter-core/src/scoped_dispatch.rs) |
| Critical/finite abort, dropped cleanup driver, hook timeout; submitter vs driver diagnostics | [scoped_owned_tasks.rs](../crates/batter-core/tests/scoped_owned_tasks.rs) |
| Filtered task spans retain enabled application parents during execution and normal/aborted destruction | [filtered.rs](../crates/batter-core/tests/scoped_owned_tasks/filtered.rs): critical components, finite tasks and cleanup hooks under `info,batter=warn`, on current-thread and two-worker runtimes, with a separate ambient subscriber and parent. |
| Subscriber callbacks precede finite admission locking | [subscriber.rs](../crates/batter-core/src/lifecycle/state/tests/subscriber.rs): actual submission under enabled/filtered task spans; `try_lock` assertions cover `new_span`, `current_span` and `clone_span`, with callback counts rejecting a vacuous pass. The fixture never starts a coordinator or application factory. |
| Dual body/cleanup failures and deterministic scripted outcomes | [support.rs](../crates/batter-test-support/tests/support.rs) |

Timer tests use Tokio's paused time. This controls the Tokio clock, not system
wall time or a PostgreSQL server's clock. Paused time is appropriate for the
retry and lifecycle timing model; it cannot prove real transport cancellation,
database transaction behavior, or non-yielding task preemption.

## Seeded scheduling exploration

Run the discoverable target with `cargo test -p batter-core --test scheduling --locked`.
It launches separate two- and four-worker Tokio profiles, each with seeds 0–31,
64 workload supervisor lifecycles and 4,096 completed finite tasks. Every seed
also runs all eight required families: shared capacity, admission closure,
operation boundaries, readiness, failure retention, ownership, escalation and
the sustained workload. Missing families or workload minima fail the profile.

Each workload cycle has eight rounds of four roots and four descendants. All
eight are held at a gate while root and descendant over-capacity submissions are
rejected, then released so capacity can be reused. A test-owned ledger records
acceptance, factory invocation and returned numeric IDs, including dropped
receipts, and reconciles all 64 results with the shutdown report. Factory-return
accounting is not a join: a later submission may still see Full until the wrapper
releases its actual permit. The held gates establish exact saturation checkpoints;
a separate held-driver scenario checks queued, never-polled work. The deterministic
capacity-one ancestor/descendant rejection closes the gap found in the task audit.

Controlled orderings check allowed and forbidden admission, simultaneous operation
branches, readiness prerequisites and every driver/receipt ownership distinction.
Seeded yields explore competing submissions, scope expiry, drain, force, failures,
completion and cancellation without changing Tokio. Concurrent task and cleanup
errors are compared by complete identity/name sets; a scoped subscriber must
exclude synthetic returned-error contents. Panic fixtures use generic messages
and retain Rust's default panic hook. Existing paused-time delayed-coordinator
regressions in `process_ownership.rs` remain the deterministic false-abort oracle.

Live cooperative escalation cases may complete or miss a phase deadline. Success
must reconcile the completed-task count; any recorded abort request still skips
dependent cleanup, even when completion wins the race. With no abort request,
success requires finalized cleanup. Termination must retain the named abort and
JoinError and skip dependent cleanup. Two additional
current-thread tests use paused Tokio time to prove the precise outcomes: a real
thread stall beyond the 25 ms allowance preserves cooperative success, while a
Tokio sleep beyond the phase deadlines produces an observed abort. Exact phase
assertions therefore do not depend on live workers running within 25 ms. The live
two/four-worker cases keep their original deadlines and generated choices.

The [runner](../scripts/stress_scheduling.py) accepts the compiled test executable.
Resolve it without relying on Cargo's changing filename hash:

```sh
cargo test -p batter-core --test scheduling --no-run --locked --message-format=json > /tmp/batter-scheduling-build.jsonl
scheduling_binary="$(python3 -c 'import json; print(next(r["executable"] for line in open("/tmp/batter-scheduling-build.jsonl") if (r := json.loads(line)).get("executable") and r.get("target", {}).get("name") == "scheduling"))')"
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 4 --seed 17
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule capacity-after-drain
```

A single seed runs two workload cycles and all families; it does not claim the
full-corpus minima. Named schedules `capacity-before-drain` and
`capacity-after-drain` replay an acknowledged sequence: ancestor holds the only
permit, optional drain returns, then a descendant must be Full. Repeat a full
corpus in at least three fresh invocations per worker count when validating
changes to these tests. A failure is evidence to investigate, not a reason to
retry until the command turns green.

`SCHEDULE` records include version, seed, workers, family, case and action. The
version-1 generator uses a fixed SplitMix64 stream for yield counts and receipt
choices. A seed reconstructs these choices, **not the Tokio/OS scheduler**.
Concurrent event records establish only their observed checkpoint order. Use
named controlled schedules for exact action replay, and reduce any newly found
race defect to a deterministic regression before claiming reproduction or repair.
Changing generator consumption or schedule meanings requires a version change.

Every family is bounded by an independent five-second Tokio timeout and the whole
profile by 120 seconds. An external Python watchdog requests process termination
at 140 seconds by default and maximum, with one five-second reap/output
EOF allowance. These observed process deadlines are not an OS scheduling SLA.
Rust elapsed assertions consume the process owner's structured observation, which
starts immediately before target child creation. Launch of the Python process owner
and host scheduling before `run_process` remain outside that deadline and inside
the enclosing Cargo/Jig command bounds.
The workload ledger has 128 slots, shared finite capacity eight, and bounded
channels; each case discards its data before the next. Capture retains at most
1 MiB of raw stdout/stderr combined and continues draining after overflow, which
fails validation. The [process owner](../scripts/scheduling_process.py) separately
records direct-child status, pipe EOF, overflow and I/O failures. It kills the
owned Unix process group on a deadline, then observes exit and output under one
cleanup allowance. While either pipe remains open, it defers polling/reaping the
leader so that its numeric process-group identifier stays reserved until the
signal decision. Cached reaped status forbids a later group signal. An escaped
process may retain a pipe after the direct child
was reaped; the outcome then explicitly records incomplete EOF and preserves the
captured checkpoints. No implicit context-manager wait can extend that allowance.
Process creation and OS scheduling are not preemptible by this Python deadline.
The tool requires the default SIGCHLD disposition so another handler or ignored
child status cannot be mistaken for observed success; it leaves SIGCHLD unchanged.

Ordinary Cargo discovery stays inert even with an ambient fixture flag. Exact
child arguments plus a PID-bound launch record authorize the profile. The native
five-second startup timeout and the 149-second emergency thread are armed before
reading stdin. Launch records are capped at 128 bytes. The watchdog maximum plus
reap allowance finishes before the emergency backstop. Parent-stdin EOF is another
fallback process exit; neither fallback runs application finalizers. The previous
149–150-second watchdog settings now fail argument validation, preventing those
settings from being preempted by the emergency exit.

Process controls reject exit zero without a completed profile oracle, output
overflow even with a success marker, and a blocked runtime that cannot repoll an
already-armed 50 ms timer. The blocked fixture uses a three-second external
watchdog; its test requires observed SIGKILL/reap no earlier than three seconds.
The outer bound is twelve seconds: three for observation, five for cleanup and
four for Python/process startup overhead. This margin changes no exit-status,
checkpoint, missing-success-marker or runtime-timer assertion.
A separate finite non-yielding fixture checks the unjoined task name, absence of
a fabricated joined outcome, a still-pending receipt and skipped finalizer before
the watchdog terminates its blocked runtime destruction. Its deadline includes
four seconds of startup slack, the complete five-second case allowance, and three
seconds of hang observation: twelve seconds total. Both unjoined parent tests
measure elapsed time independently of the configured watchdog: at least twelve
seconds and less than twenty-one, including five seconds for cleanup and four for
Python/startup overhead. A synthetic late-duration control rejects the default
140-second profile deadline without waiting for it. The `unjoined-delayed-start`
replay delays runtime creation by two seconds and requires the same report,
pending-receipt and skipped-cleanup checkpoint before the kill. This regression
rejects the former shared three-second deadline. These negative
fixtures intentionally exit unsuccessfully; their parent tests pass only when
the required rejection evidence is present.

Replay the delayed startup directly with
`python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule unjoined-delayed-start --watchdog 12`.
Expect runner exit 1, the reconciled unjoined report checkpoint, and child status
`-9`; run its Cargo parent test to validate those expected failure facts.

Challenge the actual admission oracle in a disposable copy:

```sh
python3 scripts/check_scheduling_mutation.py --output-dir validation/local/scheduling-mutation
```

Use a new output directory for each run. The command builds the unchanged source
and a variant giving descendants independent permits. For each variant it runs
the recorded `capacity-after-drain` schedule three times per worker count. All six
original runs must pass; all six mutant runs must fail the specific shared-capacity
assertion, with neither a compiler failure nor a watchdog timeout accepted as
mutation evidence. Logs, the patch, compiler/lock identity and exact commands are
retained in the output directory. Build/version commands share the same process
owner: builds allow 180 seconds with 16 MiB capture, version inspection 10 seconds
with 64 KiB capture, each followed by at most five seconds of cleanup observation.
The copied subject includes the repository Cargo configuration and the scheduling
control entrypoint with its sharding and parallel-process dependencies. An isolated
entrypoint regression checks this production copy list. Build and replay
outcomes are saved before classification, including partial timeout/error logs.
The mutation checker consumes structured scheduling outcomes directly. The CLI
still emits a standalone `WATCHDOG_RESULT` JSON line after its bounded diagnostics,
even when the final captured line was truncated. Production sources are never
edited in place.

Two paused-clock descendant closure tests exercise forced cancellation and a
returned task failure. Prompt observation requires clean completion; a 1.25-second
observation delay must terminate the held ancestor after its one-second cancellation
allowance. Both retain strict post-closure admission rejection and reconcile named
receipt/report outcomes, original error identity and dependent cleanup. Ordinary
seeded cases retain the same generated choices and accept either permitted outcome.

The scheduling target runs 30 Python controls: real children cover normal exit,
blocked work, EOF before exit, inherited and escaped pipe writers, output overflow,
spawn failure, partial build/replay evidence and invalid child-status ownership.
An exited leader with an inherited pipe must remain unreaped at the group signal;
a separate real-child control rejects any signal after reaping. The
escaped-pipe fixture uses two directly owned children in separate process groups
sharing a test-created pipe. The outer test retains the writer's process handle
through bounded kill/reap, including observation exceptions; no orphan or PID
file is needed to demonstrate incomplete EOF from an outside-group writer. The
ordinary Python controls allow three seconds for startup and fixture work, with cleanup
and scheduling slack included in their elapsed-time bounds. A one-second delay
before the first checkpoint exercises startup slower than the former 300 ms
budget. These margins are not OS scheduling guarantees.
Narrow injected read/close failures and an unobserved-exit wrapper exercise paths
that cannot be induced deterministically through an ordinary child. A partial
capture setup control injects an I/O error or interruption at the second stream's
nonblocking setup or selector registration. It retains a real selector and requires
its underlying descriptor to be closed before garbage collection, while also
requiring child termination, reaping and the original error category. Six controls
use the real Rust fixture for ambient flags, missing/malformed records, parent EOF
and the watchdog ceiling, plus a real replay through a non-interactive background
shell. The process owner requires the main thread, default Unix SIGCHLD and one
of Python-default, SIG_DFL or SIG_IGN for SIGINT; custom or unknown ownership
fails before launch. Inherited SIG_IGN stays ignored in both owner and child,
preserving background-job policy. Either default uses a scoped SIGINT handler
that records a request instead of raising between arbitrary instructions.
Normal observation stops at its next polling checkpoint, while cleanup continues
under its original deadline. Signals during setup, observation, cleanup and close
retain interrupted evidence and descriptor/reap checks; repeated cleanup signals
cannot restart its deadline. The exact prior SIGINT disposition is restored after
resource release, including callback exceptions. Controls cover all three standard
dispositions across success, spawn failure, callback failure and watchdog expiry;
a background-shell control sends real SIGINT to both owner and child while ignored.
Tests that require active interruption explicitly establish and restore their own
signal policy, so the full controls also run from a background shell. This is a private synchronous tool
contract, not a library-global handler or a general Python exception guarantee.
Direct full-suite execution uses four separate Python processes by default;
`--jobs 1` selects one serial worker. Every worker discovers the same selected
suite and runs its deterministic partition. Parent-side validation requires one
successful completion record with exactly the assigned executed test IDs; a
zero exit without that record, duplicate or missing IDs, a skip, partial output
or failed worker cannot pass. Scheduling cost hints only balance partitions;
newly discovered tests are always assigned, even without a hint. Ordinary
`unittest discover` and explicit unittest selectors retain serial behavior.
Each shard gets a 45-second watchdog, five seconds of graceful interruption and
one second of final reap/output observation. At most four final observations
precede the unchanged outer backstop; all individual fixture deadlines and
assertions remain unchanged. Run `python3 scripts/test_scheduling_process.py`
for the faster standalone process controls, or add `--binary <scheduling-test>`
to include the six launch controls.

The standalone control command and each shard have a separate 60-second
emergency exit so a regression in the process owner fails its Cargo parent visibly.
Run it directly with:

```sh
python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"
```

This suite explores bounded schedules and rejects selected faults; it does not
prove exhaustive concurrency correctness, task preemption, detached-descendant
termination or cleanup after a watchdog kill. The hosted macOS jobs ran the
Rust `scheduling` target on both supported toolchains for commit
`56814038f2a9cf6a34688ee39cd9f0e433487a1e`. They did not run
`scripts/test_scheduling_process.py --binary`; those binary-backed launch
controls retain local evidence only.

## Non-yielding subprocess tests

Run the focused suite with:

```sh
cargo test -p batter-core --test non_yielding --locked -- --nocapture
```

These tests also run in the ordinary isolated-core and workspace matrices. Each
synchronous parent launches this test executable's exact `child_fixture` entry,
then sends a private stdin record containing the new child's actual PID and
scenario. Ambient environment variables cannot activate the fixture. The parent
retains the pipe writer through process waiting. A child OS thread observes EOF
when that owner disappears and terminates the whole child with exit code 74.
A separate OS thread, armed before launch input is read, exits with code 75
after ten seconds even if the pipe remains open or launch never completes.
Neither emergency path runs Rust destructors or claims application cleanup.

The parent uses `std::time::Instant` and polls process status every 10 ms.
Ordinary scenarios request a kill and wait for a child still alive after five
seconds. The blocked current-thread scenarios instead allow five seconds from
spawn to observe `drain-requested`, then a separate three-second window: the
fixture's two-second observation plus one second for scheduling/capture. Missing
startup evidence fails at the startup deadline and unwinds through kill/reap.
Only complete, exact protocol records count. The reader timestamps each record
under the capture mutex; startup evaluates the evidence and clock under that
same mutex. A record captured at or before the deadline remains valid if the
parent polls late, including after later child-panic diagnostics arrive. A
captured startup event establishes timing only; final validation still rejects
the panic. With no startup record, a known panic fails promptly. Capture
overflow fails every startup decision. A genuinely late record fails. The observation window starts
at the captured drain timestamp, so late polling cannot extend it. A real-process
regression exercises both blocked scenarios and compares the deadline actually
passed to `wait()` with captured drain time plus the observation allowance;
fast startup cannot conceal reverting to a fixed spawn deadline. A second live
control waits for the deliberate panic to be captured before its first startup
resolution, then requires the original observation deadline and final panic
rejection. Deterministic cases cover the same ordering before and after the
startup deadline, plus missing/late startup and overflow accompanied by panic.
The synchronization bound comes from the scenario policy's maximum wait;
an exact-instant regression includes panic after the latest accepted startup.
The live wiring control rejects a kill delayed by a full observation allowance
past its recorded deadline. This tolerance does not guarantee OS scheduling.
Event waits join capture when the child exits before evaluating its final
evidence; exited-child controls cover both present and absent events.
Both phases fit within eight seconds, before the independent ten-second child
emergency exit, subject to the OS timing limits below. The kill request and
observed exit status are separate facts. Watchdog assertions
require SIGKILL; a natural exit before the kill cannot masquerade as that signal.
They also require the recorded first kill-request time to reach the selected deadline and the scenario's
complete milestones. There is no alternate numeric-exit-code fallback.
Emergency codes 74/75 and ordinary failures cannot satisfy watchdog assertions.

A dedicated reader continuously drains merged stdout/stderr, retaining at most
64 KiB of diagnostic bytes and metadata for at most 64 distinct protocol events.
Event names reference those retained bytes; duplicate records keep their first
timestamp. It keeps draining after either limit is exceeded, preventing pipe
backpressure. Overflow fails startup and final validation because a preview
cannot prove absence of later failures. An unfinished protocol record also fails
final validation: termination may have interrupted forbidden evidence mid-write.
Diagnostics include the scenario, actual status, first kill-request time,
elapsed time and selected deadline. Capture joining cannot turn an early kill
into valid deadline evidence. No temporary capture files exist. On errors or parent unwinding,
the guard kills/reaps the child before joining the capture reader. Scripted
`Read` inputs exercise this same reader loop: an interrupted read must retry,
EOF must preserve the evidence, and I/O errors or panics after partial output
must make `finish()` fail after joining reader destruction. The I/O error kind
and message survive; a reader panic becomes the fixed harness I/O diagnostic.
Partial output remains available for diagnosis, without becoming a successful
finish result.
Overflow diagnostics retain the first detected cause and the captured event
count. The event-overflow fixture emits more than 64 distinct records before
flooding the pipe and exiting: ordinary exit proves continued draining, while
validation must still reject it for event overflow even after the byte cap.

The cooperative control waits for forced cancellation, drops its direct task,
runs the dependency finalizer, and exits normally. The non-yielding task parks
its OS thread forever inside one Tokio poll; it has no self-release. With two
runtime workers, an external OS thread requests drain after confirmed task
entry. The owned driver must retain exactly that task in both `abort_requested`
and `unjoined`, record no invented joined outcome, and skip dependency cleanup
without invoking its factory. The child verifies the task is still live, then
blocks in runtime destruction until the parent kills it.

The current-thread case arms and polls a one-second Tokio timer before starting
the direct task. After the task enters and the external thread requests drain,
neither the timer nor shutdown report can finish on the blocked runtime. The
external thread also records two seconds elapsed after requesting drain,
exceeding both the timer and the total configured shutdown allowance. The parent
requires those milestones and rejects completion/cleanup markers. An expected
timeout without its required milestones or with a captured default panic-hook
diagnostic fails the test. Controls also exercise natural success/failure before
a later kill, an early kill, forbidden cleanup evidence, output beyond the
preview, spawn failure, ambient scenario state, a mismatched launch PID,
incomplete launch, parent-pipe closure, and both kinds of child panic.
Deterministic tests exercise the same parser and wait policy used by the process
watchdog with explicit monotonic instants: delayed startup, exact deadline
arrival, late polling, genuinely late/missing evidence, split and unterminated
records, misleading substrings, duplicates, overflow and panic evidence. They
replace the four-second launch sleep and its narrow scheduling margin. Additional
controls reject a kill that precedes its selected deadline even if output capture
finishes much later, other signal statuses, and overflow on the watchdog-kill
path. Real subprocess controls still cover startup that never completes,
extended-deadline early kills, natural-exit races and the actual blocked runtime.

The Linux parent-death regression requires Python 3; absence is a test failure.
Its isolated [probe](../crates/batter-core/tests/non_yielding/parent_death.py) becomes
a Linux child subreaper, waits for a blocked fixture's report and runtime-drop
entry, then sends SIGKILL only to the owner PID. It adopts and waits for the
orphan's exit code 74, rejecting cleanup/destruction evidence. Its own timeout
and failure cleanup contain the probe processes. Cargo's process-wide reaping
state is untouched. Evidence checks use explicit failures that remain active
under Python optimization. Both the real fixture test and a negative control run
the probe with `PYTHONOPTIMIZE=1`; the control supplies children that complete the
launch/EOF protocol but exit with code 7 or emit forbidden cleanup evidence
before exiting 74. Each must fail with its specific diagnostic and without the
success marker.
The probe's single five-second startup budget covers both owner PID discovery
and fixture readiness. Its hard SIGALRM is a last-resort bound and can bypass
`finally`; owner/child fallback deadlines and the OS adopting reaper then own
containment. This is not a guarantee of Python cleanup after hard termination.

The lifecycle fixtures themselves spawn only runtime/OS
threads; the parent-death probe adds an owner process. The unwind control
requires cleanup and capture-reader joining to finish within five seconds of
child startup, before the child's ten-second emergency exit. This detects a
destructor that merely waits instead of killing. On Linux it also checks
`/proc/<pid>` is absent, including absence of an unreaped zombie.

`--nocapture` exposes the deliberately caught parent assertion panic through
Rust's default panic hook. It is expected in the passing unwind test. No panic
hook or global tracing subscriber is replaced. OS process termination contains
these fixtures; it does not prove application cleanup or give Batter preemption.
OS scheduling delays/suspension and OS process-control failures remain outside
the test's timing model. If the real parent dies, the OS's adopting reaper owns
the terminated child's wait; only a surviving owner can explicitly reap it.
The hosted macOS job type-checks every workspace target, builds and smoke-tests
the HTTP example, and runs the focused subprocess, scheduling, ownership,
report-doctest, database-independent lifecycle, Axum and readiness suites on
Rust 1.94.0 and 1.98.1. Those jobs passed for commit
`56814038f2a9cf6a34688ee39cd9f0e433487a1e`; they do not cover the later
documentation, package-description, and rustdoc refresh or claim a full hosted
macOS matrix. Separately, both full verification matrices and the rebuilt HTTP
smoke modes passed on a macOS arm64 host over SSH. The environment-variable
control is intentional: it protects the
previously removed ambient launch path. Only the emergency fallback control
deliberately waits ten seconds; the parent-death probes return on evidence.
Keeping that control in ordinary matrices validates the real configured fallback.

## Browser credential transport

`cargo test -p batter-axum --test browser --locked` runs the public browser
transport contract. Its table-driven cases cover HTTPS and explicit loopback
origin construction (including special-scheme recovery forms and raw paths
erased by URL dot-segment normalization), cookie name/value syntax and redaction,
every Cookie field, duplicate targets, strict versus target-only malformed input,
fixed-scope secure and local Set-Cookie output, sibling append preservation, and
matching removal. Same-name set/set, set/removal, and removal/set attempts must
return the sanitized duplicate-name error without mutating the first field;
independent names still append in order.

The same target covers non-empty mutation-policy construction, exact field
multiplicity, compatible and strict Fetch Metadata modes, exact JSON media type,
Fetch-normalized marker configuration, RFC 9110 parameter edge cases, sanitized
status/code mapping, and deterministic Origin/Fetch/marker/content precedence.
Every marker policy automatically requires exact same-origin Fetch Metadata; a
regression uses the user-agent-added `Upgrade-Insecure-Requests` navigation
field to prove that the marker alone and a cross-site value both fail closed.
Real Axum Router cases prove private headers cover inner success,
application errors, rejection middleware and fallback, preserve response data,
do not cover an outer short-circuit, and compose inside one `observe_http` event
without request-header leakage. The exact `Referrer-Policy: same-origin` value
pins the specification-level composition with exact-origin HTML form mutations;
no browser request constructor is simulated. Adapter doctests compile the public
consumer shapes. No browser process, application session store, CORS policy, or
real credential protocol is simulated; the target proves the HTTP header
contract.

## Axum operational defaults

`cargo test -p batter-axum --test operational --locked` exercises the actual
composition of server identity, observation and guarded-route admission. It
covers forged headers/Tower/adapter extensions, concurrent IDs and nested
operations, custom renderer precedence, missing typed identity, probes/fallback/
unsupported methods, deadline/forced cancellation and drop under another
dispatch with INFO spans disabled. Captures use `test-support/dispatch.rs` and may
be dropped per case; only tests explicitly comparing multiple captures retain them
together. HTTP field and correlation assertions inspect event-local fields.

Readiness tests use the real HealthMonitor with controlled polling/time to cover
Unknown, Failed, TimedOut, Healthy, Stale and stopped writer, plus lifecycle
Starting/Ready/Draining/Stopped. Reads start no probes; custom severity retains
body/status/reason/outcome. Native loopback tests cover owned Startup handoff,
registration/abandonment listener release, and a stream retained after request
budget and direct-wrapper abort. The test explicitly releases and awaits that
body after observing unsuccessful shutdown and skipped dependency cleanup.
These tests establish the stated limits, not general streaming ownership.

## HTTP/1.1 instrumented lifetime observations

Run `cargo test -p batter-axum --test http_lifetime_observations --locked` from the root.
The 27 discovered tests comprise twelve real loopback cases, an inert child entry,
eight failure controls, three launch/completion controls, two unwind/Drop evidence
regressions and the native acknowledgement parser control. The normal workspace
all-targets pass in `scripts/test_matrix.py` and both Jig test aliases include it;
no ignored-test switch or external wrapper invocation is required. The configured
macOS adapter all-targets job also includes it. That job passed on both supported
toolchains in run 35580602864 for commit
`56814038f2a9cf6a34688ee39cd9f0e433487a1e`.

Both HTTP targets use `tests/support/http_process.rs` and the existing native Unix
launcher/watchdog in `test-support/process/`. Every case has an eight-second parent
bound with kill/reap and captured output EOF; child launch requires exact argv and
a PID-bound stdin record. A ten-second emergency thread and parent-pipe EOF also
contain non-yielding fixture work. Ordinary discovery ignores ambient scenario
settings. Successful exit additionally requires a complete event naming the exact
case, emitted after exercise, teardown and runtime destruction. Controls reject
zero selected tests, exit without completion, a wrong case, and ambient `stall`
launches. The deliberate runtime stall requires observed SIGKILL/reaping and
cannot pass ordinary success validation. Process creation, OS scheduling and
hard termination of the watchdog are not general software-time guarantees.

The driver owns the running supervisor before awaiting readiness. Both HTTP
targets now import complete-phase limits from `tests/support/http_process.rs`:

| Phase | Allowance and ownership |
| --- | --- |
| Startup | 1 s shared by construction and readiness; a readiness failure retains the running owner for teardown. |
| Exercise | 2 s for wire/resource checkpoints, inside its spawned task; the driver joins that task before teardown. Terminal report waits are not part of exercise. |
| Teardown | 3.5 s shared by report waiting and subsequent body/socket reconciliation through one absolute deadline. |
| Parent reserve | 1 s startup/unwind/scheduling margin; the sum is 7.5 s, checked below the unchanged 8 s watchdog. |

Observation event and wire reads have no private timer. Their owned wait guards
retain missing names or markers, partial bytes and event snapshots on cancellation;
the driver retains those diagnostics and tracing capture after joining the task. The
observation fixture's shutdown allowances are also checked against the complete
teardown budget, including cleanup and abort observation. Its normal drain is
2 s; forced drain remains 100 ms, blocked-body cancellation remains 100 ms, other
cancellation is 300 ms, abort observation 300 ms, and cleanup total/reap 500/100 ms.
These are test fixture policies; runnable service defaults are unchanged.

Both suites move terminal report checks and report-dependent ordering assertions
into teardown. Scenario access exposes only the deliberately named abort-report
checkpoint, restricted to the blocked-body case; that checkpoint must remain in
exercise to prove body ownership after wrapper abort and before release. The old
independent four-second report timeout is removed. A delayed-success control in
each suite uses a real 2.2 s finalizer, proves exercise completion precedes cleanup,
and validates the report in teardown. Its dedicated cleanup allowance is 2.7 s;
drain is 400 ms, cancellation and abort observation are 100 ms each, and cleanup
reap is 100 ms. The complete 3.4 s shutdown allowance still fits teardown. Production policies and
ordinary case allowances are unchanged.

Missing-reconciliation controls with fast and delayed reports require the governing
teardown timeout, the actual report, the named interrupted wait and subsequent
resource Drop. Exercise event/wire and shorter disconnect controls also require
retained diagnostics. Both suites recheck handler-entry counts after the terminal
report; late-entry injection controls prove an EOF-only assertion cannot pass. A slow-failure control times out an exercise and then reconciliation after a real
shutdown report. It requires both timeout results, the retained report, and
exercise destruction before finalizer completion before reconciliation destruction,
with exit 101 and no watchdog kill. A startup control withholds component readiness
acknowledgement and requires timeout, skipped exercise and owned shutdown/cleanup.
The existing dual-failure control still retains an exercise panic and cleanup error.
A report remains outside reconciliation, so later assertions/timeouts cannot erase
it. Timing out a JoinHandle would detach its task; here the timeout owns the
exercise future inside the task, and the handle is awaited. These diagnostic bounds
require yielding work; the independent stalled-runtime control remains necessary.

Shared event storage releases its mutex before assertions and timeout diagnostics; regressions
drop resources during unwinding after failed ordering and missing-event waits.
These paths require loopback and Unix process permissions, with no Python wrapper.

Fixtures acknowledge handler entry, pending upload/body polls, response construction
and observation, headers, body completion/destruction, complete Content-Length or
chunk framing, client EOF/error, native server socket destruction, direct task
outcome and cleanup. The client uses raw HTTP/1.1 on ephemeral native loopback
sockets. A test-owned listener forwards native IO; server success is retained
independently of client/body errors. Exercise-task errors are retained while the
fixture releases its controls and awaits/inspects the running supervisor.

[ADR-009](adr/009-http-lifetime-observations.md) maps every case to its expected
outcome. The ordinary second-request case permits 503 or transport closure; its
companion withholds graceful delivery until an actual 503 admission rejection is
witnessed. The blocked-body abort case inspects the report while the body, socket,
and runtime remain live, then releases the body and separately reconciles framing,
EOF and destruction. It tests a 50 ms pending-read checkpoint, not indefinite
survival. Disconnect cases require positive body/handler and socket destruction
within one shared second after the close/EOF checkpoint, before any release/drain; full `Shutdown::Both` plus socket
drop differs from write-side shutdown followed by reading EOF. No test invents
another HTTP completion after response headers.

## Fixture infrastructure regression and mutation checks

`cargo test -p batter-core --test tracing_dispatch --locked` isolates first callsite
registration in a fresh process. An unsubscribed thread first creates a disabled
span; the registered subscriber must subsequently create an enabled span at that
same callsite. A filtered-capture regression also requires output storage to be
released after the capture is dropped (allowing brief concurrent cache borrowers).
The obsolete static vector of real dispatchers has been removed. The private test
constructor uses an explicitly OFF-filtered inert registry. A separate bootstrap
regression checks that global maximum filtering remains OFF before the real
dispatch rebuild, so concurrent macros cannot seed stale interest in the first
registration window. This works around the reproduced pinned upstream cache paths without installing a global subscriber or weakening
admission-lock callbacks or filtered-event assertions. Raw subscriber arguments to
`WithSubscriber::with_subscriber` also construct a dispatcher implicitly and must
use this helper; already-built dispatcher clones need no second construction.

`python3 scripts/check_http_graceful_mutation.py --output /tmp/http-mutation-evidence`
requires Python 3.11+, cached locked dependencies and a new output directory
outside the checkout. Optimized Python is rejected so assertions cannot disappear. It
resolves Axum 0.8.9 through Cargo metadata, copies the workspace and dependency,
and uses a private build directory. Original native behavior must pass all four
cooperative cases. Immediate native connection exit must fail their pending-work
assertions; removing the connection event alone must fail acknowledgement despite
the producer event. Build failures, zero-test runs and unrelated assertions do not
count. The same per-variant replacement definitions drive source edits and
`evidence.json`; each variant records its exact replacements and patched-source
SHA-256, including the unchanged baseline. Logs and evidence go in the selected directory. Root/registry sources
remain unchanged, and the copied lock may change only Axum's path identity.

## Provider lease and retry boundary probes

The reference live runner additionally requires the ignored library test
`delivery::worker::state::live_tests::provider_state_lock_and_retry_boundaries`
after its 66-case integration inventory and maintenance-session probe. It uses a
disposable harness database, synthetic native lease identity and no heartbeat to
isolate the SQL authority boundary: lock-only waits on the job and effect rows
cross expiry and must leave application state unchanged. It also discards the
native completion value after outcome persistence and checks both dispatch
authorization paths reject future eligibility, then accepts expired eligibility.
The ordinary library matrix covers returned storage-error delay retention and
pure retry planning. The integration outcome case blocks native completion after
the application commit, kills the real worker, and checks recovery issues no POST
and schedules at or after the retained provider boundary. Synthetic SQL authority
coverage and real native recovery coverage are distinct evidence.

## HTTP process smoke test

The example defaults its logging filter only when `RUST_LOG` is absent.
Malformed or non-Unicode filters fail startup with a sanitized configuration
message. Example unit tests check default/explicit selection, rejection and
retention of the original error without printing environment contents.

After verification builds the example:

```sh
cargo build -p batter --features axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
cargo test -p batter --features axum --example http_service --locked
cargo run -p batter --locked --example process_owned
cargo run -p batter --locked --example operation_budget
```

The smoke script starts its own local process and probes `/live`, `/ready`,
`/work`, and an unmatched fallback. It requires exactly one HTTP completion with
the correct route/status for each tested server-generated request ID, including
probes and fallback; raw unmatched paths/queries must not appear in telemetry. Normal mode verifies an application 500; deadline mode verifies the
middleware 503. Both check the shared error envelope, nonempty generated
correlation ID/header, rejection of untrusted correlation input, and default INFO
completion/status/latency logs. It sends the selected signal and requires exit 0.
The `--warn-filter` profile sets `RUST_LOG=info,batter=warn,batter::request=info`,
requires INFO HTTP and operation completions to be absent, and requires operation
warnings to correlate to observed response IDs (including the deadline completion
for `/work` in deadline mode). It requires each 5xx WARN event to retain its request ID
and all HTTP fields with the HTTP span disabled. The example test separately
drives acknowledged lifecycle transitions over a live loopback listener to prove
the exact Starting/Draining INFO and Stopped WARN readiness policy under both INFO
and mixed-target filtering. It checks generated identities per response, expected
absence of filtered INFO events, and the event-local identity on Stopped WARN events with all adapter INFO spans
disabled.
The readiness route also requires a fresh successful cached dependency sample.
A controlled router test covers unknown, failed, recovered, expired and stopped
observations without additional probe calls. The runnable monitor uses an explicit
simulated dependency, not a real database availability check.
Readiness now acknowledges installed signal listeners, so there is no extra
sleep before signalling. It is intentionally
not a streaming/disconnect/load suite. It never contacts a user deployment.
A temporary local port is selected before launch; another local process can race
that port reservation, in which case the smoke test should fail visibly.

## Limits of current coverage

The operational `connect_info` cases use protected startup and two real TCP
connections, comparing middleware and handler output against each client's
independently observed local address and port. Forged Forwarded, X-Forwarded-For
and X-Real-IP headers cannot select the reported peer. Registration failures
require exact invalid/duplicate-name errors and rebinding of only rejected
listeners; dropping the unstarted owner releases the accepted listener. Shared
serving scenarios also execute the peer variant through startup waiter/owner
abandonment and a streaming body surviving forced wrapper abort. These checks
do not establish application authentication or proxy-trust correctness.

The reference package separately exercises its actual business-boundary function
with the production `operational_http`, direct-peer metadata, bearer
authentication and request-admission order. Twelve overlapping requests use
distinct injected native peer IPs and forged forwarding, trace, request-ID and
owner-extension values; each handler and nested `OperationContext` returns its
own generated correlation and expected peer, and the configured owner always
wins. The concurrency case gives its operation a ten-second budget while a
three-second wall-clock guard owns failure, avoiding a hidden load-sensitive
deadline. Authentication and domain-failure cases require the selected peer to
remain absent from their response bodies. Additional cases cover absent optional headers, missing `ConnectInfo`
failing closed, unauthenticated and domain-invalid production routes, and forced
request cancellation retaining the correct response/header identity while the
separate operation context becomes cancelled. These ordinary tests substitute a
deterministic handler only after applying the same production boundary and need
no PostgreSQL. Adapter real-socket tests independently establish ConnectInfo
provenance by comparing the observed value with each client's own socket address;
the reference policy tests establish the exact SocketAddr-to-IP step. The database-backed live delivery helper uses the opaque
`InProcessRequestClient`, which cannot be served or expose its inner router and
requires the helper to select a synthetic peer for every request. It requires every parsed JSON
`request_id` to equal the generated response header.

The application-owned `http::register_in` function fuses router construction
with native peer-aware registration. An ordinary non-database real-socket case
starts that exact operation and requires authenticated-boundary 401 rather than
the missing-peer 500 control. The production-root live case supplies the final
evidence layer: after the real listener becomes live, it sends an unauthorized request to the matched
`/delivery-commands/transport-probe` route and requires 401
`authentication_required` plus matching header/body identity. Replacing native
ConnectInfo registration with plain registration would instead hit the retained
missing-peer 500 unit control. `MockConnectInfo` is not used because this
middleware reads request extensions directly while the mock affects extractor
fallback. No proxy trust mode, durable metadata, quota backend, or spawned-work
inheritance is claimed.

The SQLx example's `tests/diagnostics.rs` launches the actual executable with
missing, non-Unicode and malformed database URLs, requiring a nonzero exit and
sanitized stderr. Its example-local Python parent uses the shared process owner
with a 10-second observation budget, 64 KiB combined capture and a 2-second
kill/reap allowance. A hung-child control requires watchdog expiry, SIGKILL,
reaping and output EOF. The watchdog runs outside the example runtime; process
creation, OS scheduling and watchdog-parent destruction remain outside its
deadline guarantee. These cases do not connect to a database. Receipt regressions
in `process_ownership/error_sources.rs` check concrete error identity through
both receipt and report source chains. Compile-fail doctests on `CleanupReport`
and `ShutdownReport` deny ignored awaited reports using `unused_must_use`.
`SharedShutdownReport` adds compile-fail examples for discarded owned reports
after `?` and `unwrap()`, plus a compiling inspection/propagation example.
`process_ownership/report_usage.rs` requires the precise `unused_must_use` lint
on raw `CleanupStack::close` and `Supervisor::run_until`, and all six combinations
of owner wait, shutdown and observer wait with `?` or `unwrap()`: denying
unfulfilled lint expectations prevents unrelated compiler
errors from satisfying these controls. Its runtime regression retains both
task and cleanup failures through clones and observers after driver-owner drop,
checks report/source identity, and verifies Display omits error contents.
It also renders the error chain and requires one task/cleanup summary, with a
distinct shared-owner context and the same concrete report source.

The SQLx example's portable unit tests execute its production completion and
exit functions: success waits for cleanup, startup failures retain both causes,
and task or finalizer failures remain nonzero exits after coordination. A failed
diagnostic writer cannot convert a failure into exit success. Another control
passes an error whose Debug, Display and source access panic, proving the exit
handler does not implicitly inspect unknown failures. The subprocess
credential checks require exactly `Error: process failed` on stderr and exclude
the marker from both output streams. The missing-configuration control also
requires empty stdout. The process wrapper retains concrete sources and prints
only its known, redacted Display; the exit handler never formats unknown causes.

The `batter-sqlx` adapter's separate `scripts/test_sqlx_live.sh` runner requires
an exact 84-case inventory: twenty-one owned atomic/snapshot cases, two migration
cases, eleven PostgreSQL lease/disposition and read-only-verification cases,
fourteen owned-pool cases and thirty-six authority-
and-protected-verification cases. Its verification controls use committed uniquely named
fixture objects, exercise an explicitly allowed later migration,
missing/checksum/unsuccessful rows, bounded oversized-ledger rejection,
read-only ledger preservation, role and
PUBLIC authority, two policies and required unsupported coverage. It does not
claim SECURITY DEFINER body behavior, durable application history, provisioning,
or profile-grant equivalence; those remain in the consuming suite.

Three live tests are explicitly ignored during ordinary runs, because database
provisioning is external. With `DATABASE_URL` configured for a test database:

```sh
cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored
cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle --signal SIGINT
```

The selected live target fails when configuration is absent or connection fails;
it never silently returns success. Each test creates its pool inside protected
startup through the example's `pool_in` helper, rather than registering a pool
acquired outside ownership. Tests execute real `SELECT 1` and
division-by-zero failures, then verify that startup failure and unsuccessful
shutdown retain their causes and await native pool close. Pool size becomes
zero and later acquisitions return `PoolClosed`. The executable smokes wait for
actual initialization acknowledgement before signalling and require successful
pool-cleanup evidence plus exit 0. Both the smoke and configuration checks reuse
`scripts/scheduling_process.py`; the smoke bounds startup to 15 seconds,
shutdown after signalling to 25 seconds, combined capture to 64 KiB, and
kill/reap cleanup to 5 seconds. Only a complete line ending in the readiness
marker permits SIGTERM or SIGINT; overflow, incomplete observation and missing
cleanup cannot pass.

Run `python3 scripts/test_smoke_postgres.py -v` without a database. Its thirteen
tests include real child processes for both signals, inherited SIGINT ignore,
split/ANSI readiness, absent or partial readiness, early exit, output overflow
before/after readiness, ignored shutdown signals, nonzero exits and missing
cleanup. The Python fixture uses short timed waits rather than `signal.pause()`:
a signal arriving before the native wait must not strand its deferred Python
callback until a second signal. Signal receipt, cleanup, exit and watchdog
assertions remain unchanged. Forced shutdown is checked with both a runnable child and one stopped
by SIGSTOP after readiness: it requires the real SIGTERM request and watchdog
kill/reap, without depending on a child callback before the short deadline.
Other controls interrupt the parent during readiness and reject a complete line
whose marker is followed by extra fields. Controls require reaping and output EOF; captured direct-child PIDs
also require `waitpid` to report no remaining child. Synthetic cleanup text
tests the smoke's acceptance protocol, not SQLx pool closure. These controls
run through `scripts/verify.sh` and Jig's test actions; their Python inputs are
part of the Jig receipt digest. macOS CI includes the portable
example exits, report tests/doctests, and smoke controls; live checks remain a
separate externally provisioned invocation.

The suite does not establish full real-transport behavior, non-yielding task
termination, transaction commit/cancellation or upstream worker compatibility, or HTTP handler-panic
recovery. Basic Problem JSON tests do not establish comprehensive route-contract
or RFC conformance. Bounded multi-thread regressions are not exhaustive race
exploration.

The audited missing-test requirements and their dependencies live in
[Beads](roadmap.md), not in a second checklist here. Database prerequisites must
never be silently skipped, and non-yielding tests must not hang the test runner.

## Reporting convention

Record command, toolchain, dependency lock hash, platform, result, and skipped
prerequisites in the owning Bead. A source review is not a passing test.
A passing build is not an operational audit. A deterministic unit test is not
proof that a database commit or an arbitrary external effect is cancellation-safe.

## Typed settings verification

The native preflight uses signed 64-bit cluster identifiers and rejects failed
prerequisite rows or equal cluster identities. Offline controls preserve both
signed extrema and require invalid secondary endpoints to fail before connection.

`crates/batter-core/tests/settings.rs` covers injected sources, exact-path bounded
literal reading, redacted aggregates, retained typed errors and numeric limits.
The HTTP example is explicitly marked `test = true`, so its actual router and
configuration child-process tests run under the normal all-targets matrix.
Configured capacities 1/3 hold real request futures; 5/100 ms source policies
change the demonstration work outcome. Child entrypoint failures require useful
sanitized stderr and a nonzero exit, using the shared Unix watchdog/capture.

`examples/reference-service/tests/configuration.rs` covers all native worker/pool
fields, signed/unsigned conversion edges, purpose-specific serving/maintenance
schemas, separate finite-task and Bulkhead capacities, response deadlines, real file acquisition/cleanup
failures, tracing redaction, PG* rejection and fake passfile isolation in bounded
children. A cleared-environment child also proves that maintenance ignores known
serving-only process variables without parsing their invalid or secret values,
while dedicated maintenance sources reject the same names. A loopback IPv6 native PostgreSQL handshake verifies decoded startup
and password bytes independently of SQLx's URL formatter. This is a protocol
fixture, not PostgreSQL server or TLS verification.

The same cleared-environment child constructs complete `PreparedServing` outside
a Tokio runtime while its configured TCP address is already bound. Successful
preparation therefore proves that this boundary neither spawns a Tokio task nor
binds the listener. Compile-fail rustdocs separately reject maintenance at the
runtime and router signatures and reject cloning the serving preparation. These
checks do not prove that native SQLx construction avoids all process-local work or
that a remote database will accept the resulting options.

The exact live inventory includes `configured_command_root_bounds`,
`configured_pool_capacity_and_acquire_timeout`,
`configured_worker_concurrency` and `configured_startup_pool_close_before_lease`.
The command-root case holds real pool work to distinguish configured pool timeout
from request deadline, holds one production router request to reject at the
configured Bulkhead, and exhausts configured finite-process admission.
The first holds the sole checkout through a second acquisition's native timeout
and then proves reuse. The second holds real handlers at limits 1/2 through
the native adapter, observes handler starts, releases and joins the managed
component, then independently requires SUCCEEDED rows.
The third retains an external native lease until owned startup failure and pool
close are observed, then awaits cleanup/drain and queries independent absence.
These cases are ignored only in ordinary discovery. The explicit runner requires
every configured case to execute, and missing live prerequisites fail its target.


### Configuration review regressions

The settings fixture rejects existing temporary directories and symlinks before
writing and checks private Unix permissions. The reference configuration target
executes the same report checker as the live pool-close probe: actual Startup
failures prove retention of primary, cleanup and channel causes on evidence
mismatches, with sanitized formatting. Neither regression needs PostgreSQL.

Native connection-option tests always run through the configuration suite's
cleared-environment subprocess boundary, including actual IPv6 and worker-builder
assertions. `test_matrix.py` additionally runs the entire configuration target with
PGDATA, PGUSER and PGPASSWORD deliberately set in its parent process. This catches
new tests that accidentally depend on an empty developer environment. Native
constructor tests must use that boundary rather than mutating process globals.

The `reference_preflight` example's normal tests invoke its actual entrypoint in
bounded child processes, requiring sanitized rejection before live work for
missing, PG-contaminated, remote and unsupported-query inputs. The configuration
and preflight targets share only private Unix process mechanics in
`tests/support/configuration_process.rs`; scenario assertions stay with their
owning target. A successful fake runner response is never database evidence.

`live_handoff_preserves_credentials_across_native_parsers` runs in a cleared
configuration child. It compares the preflight options and the handed-off URL
through SQLx 0.9.0 and the harness's locked tokio-postgres 0.7.18 parser against
explicit expected password/application bytes. It covers query `+`/`%20`, literal
and encoded plus signs, reserved characters, Unicode and an empty password.
This parser regression needs no database and does not establish live authentication.
The endpoint-policy regression additionally follows the harness's `Url`
normalization and database-path replacement before native parsing. Encoded DNS
and IPv4 loopback inputs must preserve the validated host, explicit port and
username in fixture options; the generated fixture database remains distinct
from the admin database. Uppercase TLS-mode inputs and leading database slashes
must agree across both native parsers. The fake-passfile child also reparses the
live handoff, proving that an explicit empty password survives. IPv6 literals
are rejected by both the shared live policy and actual preflight entrypoint
before connection work; the separate direct native IPv6 wire test remains.

## Historical probe preparation and early signal regressions

This section records the former production witness implementation. Its hosted
worker tests have been replaced by native initialization/settlement and offline
retirement probes under `batter-gi4`; do not use these historical names as the
current live inventory.

The former reference live inventory included hosted_preparation_cancellation_releases_lease,
hosted_preparation_leased_and_terminal_reconciliation,
hosted_preparation_late_control_commit_reconciliation,
hosted_worker_blocked_reconciliation_preserves_lease_monitor and
startup_signals_during_schema_and_control_preparation. They held actual PostgreSQL
locks, await blocked queries, then cancel a caller/parent, publish a late committed
control, commit a racing terminal transition, or send SIGTERM to an authorized child. Preparation cancellation must
retain confirmed unlock before immediate successor preparation. Child tests require
owned cleanup reports rather than merely an exit code. The offline
startup_signal_during_pool_acquisition case withholds native authentication on a
loopback socket. Private Unix launch/watchdog machinery bounds every child.

The former unit tests exercised overflow and an incompatible retry/poll witness before
acquisition, the exact 4,999/5,000 ms Serve boundary, both release errors, shared release
timeouts, native-plus-release failures, a later lease failure during
reconciliation-triggered settlement, the composed stop allowance and a late
cleanup failure after the outer hook times out. The composed clock test models the
pinned native maximum, reconciliation close and the real release helper; real native timeout
was covered by hosted_worker_timeout_skips_dependencies. No fault test claims
that SQLx close witnesses backend exit. The foundation child test completes received()
then registers those sources and requires drain without a second signal.

That runner revision discovered 56 reference target entries and executed with
--include-ignored: 54 database probes, the offline acquisition-signal test and
its private child entry. It required zero ignored/filtered entries
and exact per-name success; that ordinary workspace run left all 54 live probes
ignored. The runner controls reject missing startup/cancellation cases too.

## Native lifecycle and offline retirement acceptance

The current reference target has 66 entries: 61 live database probes, two
offline synthetic-acquisition signal controls, two offline executable-composition
signal controls and the private child dispatch entry. Two exact legacy aliases
were removed in the hard cutover instead of being counted as independent evidence.
The runner classifies each entry explicitly and builds both process executables
with the invoking toolchain before discovery. A Python control reads Cargo
metadata and requires `default-run` to select `batter-example-reference-service`,
preserving the documented bare package `cargo run` command after adding the
fixture binary. Seven retirement cases verify history/migration/sequence preservation and
additive catalog disable, target identity, restricted session visibility, late
native enqueue, prepared native enqueue, and retained commit failure through
readback cancellation, plus actual lost COMMIT acknowledgement after durable
cancellation. The latter discards the server's cancellation COMMIT response,
observes `CANCELED` through a separate direct connection and requires exactly one
cancellation event without changing the failed primary report. The runner
additionally executes the ignored library case
`retirement::session::tests::maintenance_session_replacement_is_refused`; it kills
only its own observed maintenance backend, then requires physical replacement to
fail with the original bounded acquisition error and completed cleanup. Exact
execution checks prevent either target's cases being skipped or counted from a
summary alone. The primary preflight requires prepared transactions enabled.

Native lifecycle cases independently cover queue-free local initialization,
in-flight work after drain, owner-drop settlement, durable business failure without
process failure, and an unjoined callback preventing dependency cleanup. Production
startup no longer enqueues a control job. Its process-level case checks liveness,
withheld readiness and successful SIGTERM and SIGINT cleanup across actual HTTP
requests, with no durable control job before or after either shutdown. A normal
child exit additionally requires the checked shutdown report to contain exactly
one successful cleanup-stack `postgres.pool` record for the close hook registered
by the application pool's `pool_in` call.
Full workspace and fresh agent/review acceptance remain separate requirements;
local checks use the pinned compiler and CI owns MSRV verification.
Owning Bead `batter-lp2.4` records what actually executed for this
scope.

### Protected startup consumer process cases

`batter-lp2.4` adds eight named `reference_live` wrappers over the existing private
`child_fixture` dispatch. Children capture stdout and stderr separately with the
shared bounded capture; assertion children must exit successfully with empty
stderr, print the static cleanup proof only after typed checks, and disclose
neither the endpoint nor the fixture token. Rust test-harness banners may share
stdout with `batter-fixture:` events.

The ordinary `child_fixture` also runs two offline broadcast controls. Each child
installs both Tokio listeners, emits `signal-listeners-ready`, and only then may
the parent request TERM or INT. Success requires both that readiness event and the
later `startup-signal-observed` event, so the control cannot pass through the
signal's default disposition or by notifying only one listener.

- `protected_startup_acquisition_sigterm` and `_sigint` run actual `runtime::run`
  against a loopback listener whose accepted handshake is withheld. After accept
  the parent sends the signal; the child requires `ProtectedRuntimeStartupFailure`
  at `postgres.acquire`, `StartupCause::Draining`, no destructor panic, no skipped
  hooks and the required successful `postgres.pool` record. Cleanup registration
  rejects duplicate names. A failed startup never
  reaches the running handoff, so Ready is structurally unreachable under the
  startup contract; this child does not observe readiness directly. The parent
  requires the OS signal request to succeed before the child is observed exited;
  the final typed report proves startup classified the request as drain. These are
  offline cancellation controls, not database queries.
- `protected_startup_schema_sigterm` and `_sigint` block actual schema SQL behind
  an `ACCESS EXCLUSIVE` fixture lock. An independent fixture connection requires a
  lock wait whose `pg_blocking_pids` contains the blocker before signalling; the
  same typed report is required at `postgres.schema`. After the OS accepts the
  request, the child-owned listener acknowledges reception before the parent
  releases the lock. Tokio broadcasts the notification to every installed
  listener before either receiver can emit that acknowledgement. Lock release
  still precedes child cleanup so a dropped SQLx checkout cannot make
  return-to-pool and pool close depend on that lock. This child uses a two-worker
  runtime to exercise that concurrent ordering.
- `protected_startup_waiter_loss` and `_owner_loss` use a test-owned protected
  composition, because `runtime::run` exposes no startup owner. It uses validated
  root native options, `pool_in`, a real `SELECT 1`, a dependent finalizer and
  startup-owned signals, then parks at `test.hold`. Waiter loss drops only a
  borrowed wait, proves startup is still pending, and receives TERM; observer and
  resumed owner must share the same Draining report. Owner loss drops the owner
  with no OS signal. Both require the dependent hook, observed while the pool is
  open, before one successful pool close, zero size with later `PoolClosed`, and
  a readiness waiter that never observed Ready.
- `protected_startup_executable_sigterm` and `_sigint` first launch Cargo's
  dedicated `signal_witness_fixture` binary at the withheld handshake. Its second
  Tokio listener emits one static, signal-specific stdout acknowledgement only
  after the requested signal is observed and `runtime::run` has settled, so an
  output failure cannot discard runtime cleanup. The same case then launches
  `CARGO_BIN_EXE_batter-example-reference-service` at a fresh withheld handshake
  and requires the production entrypoint's empty stdout. Both processes must exit
  with status 1, not by signal, and stderr exactly
  `Error: reference service failed` plus newline. The production entrypoint has no
  witness mode or test environment switch. An offline negative control rejects
  the generic status/diagnostic pair as signal evidence without the fixture
  acknowledgement. Internal report fields come from the paired typed cases, not
  from these process exits.

Ordinary workspace runs execute the four offline cases; the four database cases
are ignored and belong to the explicit runner. On the earlier 66-entry tree, a
deliberate mutation trial (a wrong executable diagnostic and a wrong cleanup-record
count) made its five offline acquisition/executable entries fail before the oracles
were restored. That historical trial includes the since-removed acquisition alias;
it is not mutation evidence for the current four-case inventory.

Verifier semantic-boundary controls cover temporary namespace rejection through
combined, migration-only and authority-only entrypoints; exact PUBLIC relation
overrides against column defaults and explicit exceptions; and concurrent
schema grants after snapshot capture. The last test also demonstrates why a
native privilege inquiry is not a snapshot-equivalent replacement: a freshly
prepared inquiry sees the grant while the repeatable-read ACL row remains old.

Exact-role manifest controls separately require an explicit parent relation for
column groups, reject special/reserved role targets before rendering, and assert
column-specific `GRANT` text remains role-targeted even when PUBLIC delivery is
allowed for verification. These are pure compiler/renderer cases and do not claim
that PostgreSQL executed the output. Boundary controls also cover the exact
generated-policy capacity limit, mixed required/allowed purpose on one target,
multi-column/multi-privilege rendering and parent/column PUBLIC contradictions.

The late-ledger-attachment live control holds a namespace catalog barrier after
explicit snapshot capture, deletes a required row from an unrelated table and
attaches it atomically. Both combined and migration-only verification must retain
the missing migration. A fresh check rejects both inherited parents and children
with Incomplete, preserves serving identity and reuses the acknowledged session.


### Owned Runledger database integration

With `DATABASE_URL` pointing to a dedicated disposable PostgreSQL 18 database,
run `SQLX_OFFLINE=true cargo test -p batter-runledger --locked --test database_live -- --ignored`.
This executes native schema verification and atomic application-write/intent
commit and rollback through consuming transaction ownership. SQLx offline
compilation uses the committed native query metadata; a fresh fixture is not a
compile-time schema source. The live SQLx runner also requires migration history,
checksum-failure retirement, and cancellation before a server lock is released.

The atomic suite also catches and discards terminal operation errors deliberately.
It requires the runner to retain the original shared boundary/recovery cause with
the callback's output or replacement rejection. Repeated calls after poison must
not invoke SQL or replace that cause; caught cancellation remains explicitly
`OperationAbandoned`. Compile-fail rustdoc rejects the removed session transaction
API and wrapping known rejections as uncertainty or terminal storage failures.

## Native Runledger workspace verification

The full workspace matrix now includes five native Runledger packages. Their
default database and container-lifecycle tests require Docker and PostgreSQL 18
(the default image is `postgres:18`). Provisioning belongs to the distinct native
`runledger-test-support` package; Batter's optional SQLx fixture/live commands
retain their external database prerequisites. Missing Docker must fail these
native tests rather than skip them. Root Cargo configuration selects committed
SQLx offline metadata for compilation, which does not replace database tests.

`python3 scripts/check_runledger_workspace.py` checks actual local Cargo package
identity, publishing restrictions, dependency direction, and migration/cache
synchronization. Its negative controls run in `test_matrix.py`. The facade
feature matrix separately compiles native/direct/facade type identity without
Runledger patches. The Jig exhaustive inputs include all native source roots,
package builds, tests, examples, migration copies and SQLx cache files.

Imported files above the existing 800-line budget have per-file ceilings fixed
at their exact imported size. They remain measured and cannot grow under those
ceilings; ordinary new Rust files keep the 800-line limit. This avoids mixing a
native source reorganization into the behavior-preserving import.

## Runledger standalone sources and cache maintenance

`python3 scripts/check_runledger_consumer.py` runs in the normal test matrix. It
copies export-eligible sources to a temporary directory without `.git`, rejects
local dependencies outside that copy and external dependency drift, compiles and
runs direct/facade identity checks in a separate consumer workspace with no patch
table, then executes the copied native producer/worker PostgreSQL test. Its
temporary lock is seeded from the root lock, which must remain unchanged. Docker
is required for the worker round trip. The repository's active Rust toolchain
(or explicit `RUSTUP_TOOLCHAIN`) is retained outside its directory. The single
worker test runs serially with output capture, so application stdout cannot
interrupt the required success record. `scripts/test_runledger_tools.py` exercises
source omissions, forbidden dependency locations, version drift, missing/mismatched
README snippets and rejected migration/refresh states.

`python3 scripts/refresh_runledger_sqlx.py` requires SQLx CLI 0.9.0, `psql` and an
explicit `DATABASE_URL` for PostgreSQL 18 with the canonical migrations already
applied. Applied versions and successful status must exactly match the canonical
inventory; a database ahead of the checkout is rejected even though SQLx's
`migrate info` omits its extra versions. It never applies migrations. Preparation happens in a disposable source
copy; only after successful offline compilation are the original native cache and
migration copies synchronized. The failed-prepare and failed-offline-build controls
assert byte-for-byte preservation of original assets. The canonical migrations
are inputs, never rewritten by this command. Review the resulting source diff and
avoid editing sources concurrently with refresh. This command is a developer tool,
not application database provisioning or a registry publication path.

With Docker, `psql` and SQLx CLI 0.9.0 installed, run
`RUNLEDGER_REFRESH_LIVE=1 python3 -m unittest discover -s scripts -p test_runledger_tools.py`
to include the live database-ahead regression. It applies an extra migration to an
owned disposable PostgreSQL 18 container, removes that migration from its source
fixture, and proves refresh rejects it before copying or preparing sources.

## Native Runlimit workspace verification

Root verification includes native default and all-feature tests, both native lint
configurations, doctests, existing facade/adapter isolation, source graph and asset
controls, and the release-mode memory corruption fail-closed regression.
`python3 scripts/check_runlimit_workspace.py` checks the actual Cargo graph and
immutable imported SQL/license digests. The source archive and mutation-workspace
inventories retain `runlimit/`, its SQL assets and both licenses.

Native PostgreSQL tests stay explicit and ignored in ordinary tests. Against a
disposable database, run both feature configurations:

```sh
RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql://... cargo test -p runlimit-postgres --tests --locked -- --ignored --test-threads=1
RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql://... cargo test -p runlimit-postgres --tests --all-features --locked -- --ignored --test-threads=1
```

CI retains upstream PostgreSQL 16 and executes both commands on Rust 1.94.0 and
1.98.1. A workflow definition is not hosted execution evidence. The native test
fixtures own isolated schemas; database provisioning remains external. Existing
Runledger PostgreSQL 18 tests are separate.

The pool-budget regression polls admission and cleanup into a pending pool
acquisition while holding the only connection. Each wait exceeds the entire
operation budget before releasing the connection, so starting that deadline
before acquisition must fail. The fresh two-second work budget leaves headroom
for the retained 100 ms SQL triggers and commit; success still requires the
exact quota and cleanup outcomes (`batter-qhps`).

The GCRA replenishment regression drives the persisted database clock through
0, 199, 200, 399 and 400 ms after exhausting a two-unit burst. It checks exact
denial delays, one-unit replenishment and no full burst reset at the period
boundary. A separate test brackets the admission watermark with real PostgreSQL
clock samples. Neither check requires the runner to schedule requests inside a
subsecond wall-clock window. The former sleep-based test failed when a delayed
request correctly received newly replenished quota (`batter-jtnk`). An injected
220 ms pause reproduces that failure and passes with the repaired clock fixture;
mutations replacing continuous time with period buckets or the database sample
with zero are rejected by the respective tests.

`python3 scripts/check_runlimit_consumer.py` copies eligible sources outside the
checkout and runs the retained native smoke plus `runlimit/smoke/facade_consumer.rs`
from a sibling standalone workspace. The latter proves shared facade/direct/native
identities, successful work admission and denial without invoking work. All five
native packages compile. The checker rejects duplicated, missing, remote or
outside-copy required identities and changed external dependency versions. It
supplies a public fixture key, needs no database or consumer patches, and checks
execution markers only after successful process exit. Both fixtures are Jig inputs.

The source workspace's active compiler is passed explicitly to Cargo; an explicit
`RUSTUP_TOOLCHAIN` takes precedence. CI selects `RUSTUP_TOOLCHAIN=1.94.0`
for the minimum compiler; local checks use the pinned default. The root lock remains unchanged; the temporary lock may
only prune unused packages. `python3 -m unittest discover -s scripts -p
test_runlimit_consumer.py -v` executes independent graph, completion and asset-copy
failure controls. Both commands are part of the bounded root test matrix. The same
checker can run from an extracted source ZIP; it does not inspect Git history.
