# Validation evidence

Latest evidence: 2026-09-10. Earlier sections retain their historical scope.

## Upstream integration with the fixture workspace: 2026-09-10

User-requested pull and complete verification, tracked by `batter-s03`. Fetched
origin/master through `0f486036ec13d48d760665a8e41997ccd14cf7cd` and integrated
three upstream commits:

- `33b6e025d7ff68866980afa265ff8112a1365cc8`: retain tracing parents when task spans are filtered.
- `ecfc4aa01dc47564b8b164883a4a373a0a4cbb09`: record the daily scan configuration.
- `0f486036ec13d48d760665a8e41997ccd14cf7cd`: extract private lifecycle-task and HTTP-observation ownership.

Local merge commit `fa18aeab96cb84a94b03ba04fd4408222de670eb` has parents
`662f2ba7271ba332f6ffdab0114d45ab59688569` and the fetched upstream tip. The
existing two local commits and uncommitted fixture work are preserved. The
verification below covers that merge **plus the restored fixture working tree**;
it does not claim that the merge commit alone contains the fixture feature.
No push or scheduled-scan execution was performed.

The lifecycle conflict retained both the new private `tasks` module and local
Unix signal registration. Testing and validation conflicts retained both complete
histories, including the local SQLx/startup/health evidence at the document's end.
An initial reconciliation helper incorrectly assumed identical history tails;
the resulting incomplete local merge was immediately corrected and amended
before restoring the fixture work or running verification. Conflict-marker checks
passed afterward. Backup hashes confirmed all saved files were restored, and
all prior append-only workflow records remained present. Tracker import added
three upstream issues without updating or deleting local issues.

Cargo regenerated workspace resolution with `cargo update --workspace --offline`,
reporting zero package-version changes. External package identities/checksums
match the pre-pull lockfile exactly; only the foundation's test-only
`tracing-core` dependency edge was added. The harness and job-runtime remote
heads still match their existing pins. Final Cargo.lock SHA-256:
`ed37786b14f40d59b12b0155889bc2f6d7c191d1a578bf35dc5dca144ec8b2f5`.

Executed locally on Linux 7.0.11-76070011-generic x86_64 with Rust/Cargo 1.98.1
and 1.94.0. Both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed: **575 test/doctest
executions, zero failures and 29 explicitly ignored live cases across 65 summaries
per toolchain**. Totals include repeated profiles. Formatting, Clippy and
warning-denied rustdoc passed, as did 22 process controls, 13 PostgreSQL-smoke
controls and four reference-runner controls. This includes upstream filtered-parent,
subscriber-callback and private-boundary regressions together with local
startup, health, SQLx and fixture tests. All-target/all-feature compilation also
passed before the full runs. No semantic assertions were relaxed.

A fresh dedicated UTF-8 PostgreSQL 18.6 cluster
(`Ubuntu 18.6-1.pgdg24.04+2`, `max_connections=100`) served loopback TCP on
`127.0.0.1:33449`, role `aa`, without TLS. The following commands passed on each
toolchain; select the second with `RUSTUP_TOOLCHAIN=1.94.0`. DATABASE_URL was
scoped to live commands and was absent from the ordinary workspace verification.

```sh
POSTGRES_TEST_ADMIN_URL='postgresql://aa@127.0.0.1:33449/postgres?sslmode=disable' bash scripts/test_reference_live.sh
DATABASE_URL='postgresql://aa@127.0.0.1:33449/postgres?sslmode=disable' bash scripts/test_sqlx_live.sh
DATABASE_URL='postgresql://aa@127.0.0.1:33449/postgres?sslmode=disable' cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
DATABASE_URL='postgresql://aa@127.0.0.1:33449/postgres?sslmode=disable' python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle
DATABASE_URL='postgresql://aa@127.0.0.1:33449/postgres?sslmode=disable' python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle --signal SIGINT
```

Each reference invocation executed all sixteen cases with zero ignored/filtered
cases (10.49 and 9.56 seconds). Each SQLx invocation executed all ten cases with
zero ignored/filtered cases (7.39 and 7.55 seconds). The PostgreSQL executable
selection ran all three live cases; its nine ordinary cases were deliberately
filtered from that live selection and already ran in the ordinary matrix.
All five HTTP and both PostgreSQL signal smokes passed on both toolchains.
Thus all 29 cases ignored by the ordinary matrix were executed explicitly.

Independent catalog snapshots after each toolchain contained the same three
retained compatibility templates and no disposable leases or fault-test templates.
The dedicated server was stopped with awaited fast shutdown. Logs, commands,
restoration manifest and safety backups are retained in ignored
`.agent/tmp/batter-upstream-sync-20260910/`. Final Jig evidence/gates and closure
belong to `plan_01M251A96JMV6KR8AHWGXK6SW8`. This is new Linux execution evidence;
earlier macOS entries remain scoped to their recorded snapshots. No hosted,
TLS, runtime-death or detached-session guarantee is added.

## Fixture report observation loop, round 2: 2026-09-09

Bead `batter-4jz`, baseline `662f2ba7271ba332f6ffdab0114d45ab59688569`
plus working changes, on the same Linux host and dedicated PostgreSQL 18.6
endpoint described in round 1. Cargo.lock remains
`81119b89b9789a15d81d14d25a5fdaf0e0ccb12034b565c65ebee998a53ddded`.

After the final round-two source changes, both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed: 563 Rust test/doctest
executions, zero failures and 29 explicitly ignored live cases across 65 summaries
per toolchain. These totals include repeated profiles. Formatting, Clippy,
warning-denied rustdoc, 22 process controls, 13 PostgreSQL-smoke controls and
four reference-runner controls passed.

The new borrowed-report discard doctest initially compiled successfully despite
`FixtureReport` being must-use: a bare reference does not inherit that warning.
Returning `FixtureReportRef` corrected the diagnostic boundary. Both owned and
borrowed discard controls now fail compilation as required, alongside a passing
usage example. Two separate offline diagnostic tests exercise actual terminal
panic text, including combined failure branches, all report failure counts and
unknown/native-content redaction. Migration identity and kind are independently
varied in the existing fingerprint test. No failure assertions were weakened.

On Rust 1.98.1 and 1.94.0, the same explicit reference, SQLx, HTTP build and five
HTTP smoke commands listed in round 1 all passed against port 33349. Each live
runner executed its exact inventory: sixteen reference cases and ten SQLx cases,
zero skips/filters. Serial reference durations were 5.68 and 5.63 seconds;
SQLx durations were 7.41 and 7.45 seconds. The unchanged 180-second reference
watchdog remains over thirty times the measured serial workload. The catalog
retained exactly the same three compatibility templates and no disposable leases
or fault-test templates after the runs.

Logs and the validation script are in ignored `.agent/tmp/batter-fixture-loop-2/`.
Independent Codex and Cursor (Grok 4.6 xhigh/fast) reviews then both completed
with no actionable findings and no blocking questions. Initial and final complete
scope fingerprints matched
`0563528e8ef9fc7e84786d34119aa3cab3b3ab9567d89032c45bf9d5fde9d3ea`;
`.agent` was excluded by `662f2ba7271ba332f6ffdab0114d45ab59688569:.reviewignore`.
The review snapshot precedes this completion record and tracker closure; source,
tests and dependency inputs are unchanged. Final Jig receipts belong to the plan.
Residual review limits include explicit owner discard and detached-driver report
loss, which remain outside the demonstrated failure-observation boundary. The
live role rejection was executed manually; the retained automated prerequisite
control mocks the command result. Catalog fault tests require a dedicated server
and serial invocation, as documented.
Manual initializer panic/cancellation, initializer-owned live connections, forced
producer abortion, runtime destruction and detached sessions do not acquire new
cleanup-completion guarantees. Cold initialization evidence remains the original
fresh-cluster run; this round verifies reuse on that retained cluster. No macOS
or hosted execution is claimed.

## Fixture acquisition ownership loop, round 1: 2026-09-09

Bead `batter-4jz`, baseline `662f2ba7271ba332f6ffdab0114d45ab59688569`
plus the working changes, Linux 7.0.11-76070011-generic x86_64. The Cargo-generated
lockfile remains `81119b89b9789a15d81d14d25a5fdaf0e0ccb12034b565c65ebee998a53ddded`;
no upstream pin or dependency edge changed in this correction.

Both `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
passed after the final code/test changes: 558 test/doctest executions, zero
failures and 29 explicitly ignored live cases across 64 summaries per toolchain.
These are profile execution totals, not distinct-test counts. Formatting, Clippy,
warning-denied rustdoc, 22 process controls, 13 PostgreSQL-smoke controls and four
reference-runner controls passed. Producer unit controls preserve native join
panic and delivered/report cause identity; a handled producer failure still makes
a successful-body report fail. Offline fixture controls cover zero pool maxima,
budget overflow, ordered fingerprints and redacted complete reports. A compile-fail
doctest requires the low-level owner discard warning.

A dedicated fresh UTF-8 PostgreSQL 18.6 (Ubuntu 18.6-1.pgdg24.04+2) cluster served
local non-TLS TCP on `127.0.0.1:33349`, role `aa`. Its first launch needed a
user-writable Unix socket directory; the successful launch used the cluster's
private directory. On **both** Rust 1.98.1 and 1.94.0 these commands passed:

```sh
POSTGRES_TEST_ADMIN_URL='postgresql://aa@127.0.0.1:33349/postgres?sslmode=disable' bash scripts/test_reference_live.sh
DATABASE_URL='postgresql://aa@127.0.0.1:33349/postgres?sslmode=disable' bash scripts/test_sqlx_live.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Select the second toolchain with `RUSTUP_TOOLCHAIN=1.94.0`. DATABASE_URL was scoped
to the independent SQLx runner. Every run executed all sixteen reference cases
and all ten SQLx cases, with zero skipped/filtered cases, plus all five HTTP
profiles. The native-creation cases cancel acquisition after acknowledged catalog
blocking and require the report to remain pending until creation completes. Empty
creation, cloning and template preparation are covered. Disposable database absence
is checked before fallback recovery. Stable fault-test templates are reclaimed
through upstream tagged-resource cleanup; repeated complete runs retain only the
same three compatibility templates.

The new abandoned-initializer panic regression initially failed: an initializing
template remained outside deferred drain. Passing the initializer task's native
join failure through upstream's returned-error/awaited-abort path fixed it; both
returned-error and panic cases now require absence before recovery. Shared
admission and pending-versus-completed pool-error cases passed. A real temporary
nonsuperuser CREATEDB role was rejected by the explicit runner with exit 1 and a
catalog-privilege diagnostic before inventory/fixtures; the role was then removed.
Offline controls also require prerequisite failure to stop before later commands.

Logs and the validation script are in ignored `.agent/tmp/batter-fixture-loop-1/`.
This evidence establishes the correction's executed scope; independent review
convergence and final Jig completion are recorded separately for the active plan.
Server shutdown is caller-owned, native admission can wait on other lease owners,
and detached-session/runtime-death guarantees remain outside this delivery. No
macOS or hosted execution is claimed for this correction.

## Fixture ownership review follow-up: 2026-09-09

Bead `batter-4jz`, baseline `662f2ba7271ba332f6ffdab0114d45ab59688569`
plus the working changes. Linux 7.0.11-76070011-generic x86_64, local execution.
Cargo-generated lockfile SHA-256:
`81119b89b9789a15d81d14d25a5fdaf0e0ccb12034b565c65ebee998a53ddded`.
Only fixture feature edges changed; upstream pins remain unchanged. Normal
`cargo tree` checks confirm the default adapter/core/generic leaf exclude the
harness, while the fixture feature selects the harness without Runledger.

Both `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
passed: 555 Rust test/doctest executions, zero failures and 25 explicitly ignored
live cases across 64 summaries per toolchain. Counts include repeated profiles,
not 555 distinct tests. Formatting, Clippy and warning-denied rustdoc passed,
as did 22 process controls, 13 PostgreSQL-smoke controls and three reference-runner
controls. The four offline fixture tests include concrete multi-error report
retention/redaction; rustdoc includes a borrowed-scope escape compile-fail control.
Runner controls now explicitly reject incorrect success-summary counts.

A fresh dedicated UTF-8 PostgreSQL 18.6 (Ubuntu 18.6-1.pgdg24.04+2) cluster served
local non-TLS TCP on `127.0.0.1:33249`, administrative role `aa`. On **both** Rust
1.98.1 and 1.94.0, these commands passed (select the second with
`RUSTUP_TOOLCHAIN=1.94.0`; do not globally export DATABASE_URL):

```sh
POSTGRES_TEST_ADMIN_URL='postgresql://aa@127.0.0.1:33249/postgres?sslmode=disable' bash scripts/test_reference_live.sh
DATABASE_URL='postgresql://aa@127.0.0.1:33249/postgres?sslmode=disable' bash scripts/test_sqlx_live.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Each toolchain executed all twelve reference cases and all ten independent SQLx
cases, with zero skipped/filtered cases, and passed all five HTTP profiles.
Reference tests prove partial multi-pool and sibling acquisition cleanup, panic
retention, close ordering while a checkout is held, cancelled/resumed waiting,
actual simultaneous body/consuming-cleanup errors, observer failure retention,
foreign-template and capacity rejection, and wrong-blocker timeout. The injected
catalog-lock failure is serialized and its residual is reclaimed through upstream
owner-aware cleanup after unlock. Low-level finish failure paths remain covered.

Cold and warmed development runs passed. The final two-toolchain runs reused
exactly the same three template database identities (two schema templates and one
stable rejection control); independent before/after catalog inventories matched,
with no disposable databases remaining. This supersedes the prior section's
per-invocation UUID workaround. SQLx close still does not prove detached-session
termination; remaining owner-loss/deferred-failure/retired-session cases stay with
`batter-kjl`. Runtime death is outside the driver guarantee.

The dedicated server was stopped with awaited fast shutdown. Logs and inventory
comparisons are in ignored `.agent/tmp/batter-fixture-review/`. Final Jig evidence
is recorded against the follow-up plan; the earlier plan's receipts alone do not
validate this correction. No macOS or hosted execution is claimed for this change.

## Reusable isolated PostgreSQL fixtures: 2026-09-09

Bead `batter-4jz`, baseline `662f2ba7271ba332f6ffdab0114d45ab59688569`
plus the working changes. Linux 7.0.11-76070011-generic x86_64, local execution.
Cargo-generated lockfile SHA-256:
`71517f9f16b7e57801068b6634154c0d0776d79718af56d205617dc348b275a1`.
The upstream pins remain unchanged. `cargo tree -p batter-sqlx --edges normal
--no-default-features --locked` excludes the harness; adding `--features
test-support` selects it without Runledger. Core and generic leaf normal graphs
exclude both libraries.

Both `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
passed after the final code changes: 552 Rust test/doctest executions, zero
failures, 20 explicitly ignored live cases across 63 summaries, plus 22 process
controls, 13 PostgreSQL-smoke controls and three reference-runner controls.
Counts include repeated profiles, not 552 distinct tests. Formatting, Clippy and
warning-denied rustdoc passed. Three new offline fixture tests cover connection
arithmetic, ordered fingerprints and redacted formatting with native causes.

A dedicated PostgreSQL 18.6 (Ubuntu 18.6-1.pgdg24.04+2) disposable cluster was
initialized with `--no-locale --encoding=UTF8` and served local non-TLS TCP on
`127.0.0.1:33149`, with administrative role `aa`. On **both** Rust 1.98.1 and
1.94.0, the following passed (set `RUSTUP_TOOLCHAIN=1.94.0` for the second run):

```sh
POSTGRES_TEST_ADMIN_URL='postgresql://aa@127.0.0.1:33149/postgres?sslmode=disable' bash scripts/test_reference_live.sh
DATABASE_URL='postgresql://aa@127.0.0.1:33149/postgres?sslmode=disable' bash scripts/test_sqlx_live.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Each live run executed all seven reference cases and all ten independent SQLx
cases, with zero skipped/filtered cases. The three new reference cases demonstrate
retained template reuse, changed-SQL readback, concurrent isolated writes, an
observed one-slot lock operation, pre-acquisition budget rejection, and retained
body error through awaited cleanup. Catalog absence is checked before deferred
drain; every producer has finished before the drain begins. Five HTTP profiles
passed on each toolchain. Removing the respective environment variable made each
explicit live runner fail with exit 1, as required.

Development failures were repaired without weakening assertions: the first
SQL_ASCII test cluster rejected UTF-8 migration text; a repeat invocation exposed
persisted templates bypassing the cold initializer-count oracle; and globally
exporting DATABASE_URL made upstream SQLx macros query an uninitialized admin
database during the minimum-toolchain build. The final cluster uses UTF-8, the
count oracle gets a fresh per-invocation revision shared by its comparisons,
and DATABASE_URL is scoped only to the independent SQLx live runner. A test
function was split to satisfy the existing cognitive-complexity limit.

The disposable server was stopped after the live runs. Logs are retained in
ignored `.agent/tmp/batter-4jz/`. Jig verify passed all five configured targets,
including a successful final `api:test` receipt. Jig completion evidence is
associated with `plan_01M23V4ZGPRT9FHJQWAZF27RPY`; its append-only records identify
the final receipts. No new macOS or hosted execution is claimed. Fixture finish
is explicitly awaited and is not cancellation-shielded; owner-loss and retained
teardown delivery remain `batter-kjl`. No commit, publication or deployment is
part of this task.

## Private task and HTTP observation boundaries: 2026-09-09

Bead `batter-bu2`, baseline `ecfc4aa01dc47564b8b164883a4a373a0a4cbb09`,
implements the two accepted architecture suggestions. Private lifecycle tasks
own join recording and task collections; the coordinator uses narrow operations
and a final owned summary. Private HTTP observation retains its original
composition function and guard implementation. Public paths, dependency versions,
factory laziness, shutdown priorities, error retention and conservative cleanup
policy are unchanged. Tracing targets, levels, fields and context are retained;
source-file/module metadata follows the moved implementation.

All 87 existing focused integration tests passed before and after extraction.
Four new unit regressions cover cancelled join waiters and exactly-once failure
recording, conservative unjoined summaries after releasing the task owner,
unchanged HTTP failure responses without admission, and observer destruction
under a saved dispatcher. The first full 1.98.1 run passed runtime tests and
doctests but Clippy rejected the new fixture returning an awaitable from an
async block. Storing the pending observation outside that block fixed the
fixture without changing its assertions. The complete matrix was repeated.

Executed on macOS 26.6.2 (`25G83`) arm64 with Python 3.14.7:

```sh
cargo check --workspace --all-targets --all-features --locked
cargo test -p batter --test lifecycle --test shutdown_causes --test process_ownership --test scoped_owned_tasks --locked
cargo test -p batter-axum --test observation --test scoped_dispatch --locked
cargo test -p batter --lib lifecycle::tasks::tests --locked
cargo test -p batter-axum --lib --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Both final verification runs passed on Rust 1.98.1 (`48a229cea`) and 1.94.0
(`4a4ef493e`), including minimal-core and workspace runtime configurations,
22 runner controls, 13 SQLx smoke controls, doctests, formatting, Clippy and
warning-denied rustdoc. All five HTTP smokes passed against the rebuilt 1.98.1
example. The three live database tests remain explicitly ignored without
provisioning; this change has no new live PostgreSQL, Linux or hosted CI evidence.
Cargo.lock is unchanged at SHA-256
`3a85b3e9dcbf632ab66488f6652638154792511c3c7b351ff6ad6b8ca2958de2`.

`scripts/jig work check --plan-id plan_01M23MY7MS9TP1AZT2R1FN05QY` passed
the verify profile, including final `api:test` receipt
`receipt_01M23NEB04BB79RGS6GAW7NXP5`. Documentation and tracker evidence were
then finalized; no test input, toolchain, configuration or prerequisite changed.
The final gate status and evidence refresh are recorded on that Jig plan.

## Filtered-parent push reconciliation: 2026-09-09

Before pushing `batter-cpb` and `batter-cpb.1`, origin/master advanced to
`7601916`. The local change was rebased onto that commit; production code
merged without conflicts. Tracker and validation conflicts were resolved by
retaining both histories, including all five issues in the conflicting tracker
block. Existing append-only Jig records were preserved.

On the same macOS arm64 host and toolchain versions recorded below, both
`bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
passed again on the combined source (`bacf0c2` before this evidence update).
These runs include the upstream 13 SQLx smoke controls, the 22 runner controls,
runtime tests, doctests, formatting, Clippy and rustdoc. Live database tests
remain explicitly ignored without provisioning; no live PostgreSQL run is
claimed. The Rust 1.98.1 HTTP example was rebuilt with
`cargo build -p batter-axum --example http_service --locked`; all five
`scripts/smoke_http.py` commands listed below passed again. The lockfile hash
remains `3a85b3e9dcbf632ab66488f6652638154792511c3c7b351ff6ad6b8ca2958de2`.
This is local macOS execution evidence, not a hosted CI result.

The final `scripts/jig check test` passed with receipt
`receipt_01M23KQ7F6RTHA9P16KJEH01GJ`. Its first attempt failed in the unchanged
`event_wait_rejects_an_exited_child_without_the_event` watchdog control: the
panic reported the missing-event deadline instead of the expected exited-child
diagnostic. Both full matrices and the unchanged full-gate retry passed that
control. No assertion was relaxed; the intermittent failure's cause remains
unresolved. Only validation prose and generated receipts changed afterward.

## Subscriber callback lock-order regression: 2026-09-09

Bead `batter-cpb.1` addresses the comprehensive-review coverage finding for the
filtered task-parent change above Git baseline `e518b6b`. A synchronous test
submits real finite work with enabled and filtered task spans. Its subscriber
checks admission mutex availability during `new_span`, `current_span` and
`clone_span`, and asserts that the expected callbacks actually execute. No
coordinator competes for the mutex and the application factory must stay
uncalled. This follow-up does not change production behavior.

The focused regression passed. Deliberately moving `or_current()` beneath the
admission lock made it fail immediately at `current_span` (exit 101), without
waiting for a deadlock watchdog. Correct ordering was restored before the
passing checks below. Focused library/test Clippy also passed with warnings
denied.

Executed on macOS 26.6.2 (`25G83`) arm64 with Python 3.14.7, against the baseline
plus the accumulated filtered-parent fix and this follow-up:

```sh
cargo test -p batter --lib subscriber_callbacks_run_before_finite_admission_lock --locked
cargo clippy -p batter --lib --tests --locked -- -D warnings
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Both full verification commands passed on Rust 1.98.1 (`48a229cea`) and 1.94.0
(`4a4ef493e`), including minimal-core and workspace runtime configurations,
22 runner controls, doctests, formatting, workspace Clippy and rustdoc. All five
HTTP smoke profiles passed against the rebuilt Rust 1.98.1 example. The final
`scripts/jig check test` runtime/doctest matrix passed with `api:test` receipt
`receipt_01M23KDPRE4E8YMYEJEGJ26BXQ`. Only documentation and tracker evidence
changed afterward; test inputs, configuration, toolchain and environment were
unchanged.

The test-only direct tracing-core dependency uses the already resolved 0.1.36;
Cargo generated the added dependency edge without changing package versions.
Cargo.lock SHA-256 is
`3a85b3e9dcbf632ab66488f6652638154792511c3c7b351ff6ad6b8ca2958de2`.
This follow-up has no new Linux, hosted CI, exporter or live PostgreSQL
execution evidence.

## Filtered task parent retention: 2026-09-09

Bead `batter-cpb` fixes the async concurrency review finding at Git baseline
`e518b6b`. Critical components, finite process work and cleanup hooks now retain
the available application parent when their own INFO span is disabled. Finite
work selects the fallback outside the admission mutex. The existing dispatcher
wrapper still protects complete future destruction; ownership, budgets and
dependency versions are unchanged.

Two new runtime regressions exercise `info,batter=warn` on current-thread and
two-worker Tokio runtimes. They check callback execution, normal destruction,
critical/finite abortion and destruction of an aborted cleanup hook, retaining
separate driver/request parents and subscribers despite an unrelated ambient
context. They also check actual shutdown outcomes and that filtered task spans
remain absent. Both regressions failed on missing parent context before the
production fix; all six scoped-owned-task tests passed afterward, including the
existing enabled-span controls. Focused Clippy passed with warnings denied.

Executed on macOS 26.6.2 (`25G83`) arm64 with Python 3.14.7, against the baseline
plus this working-tree change:

```sh
cargo test -p batter --test scoped_owned_tasks --locked
cargo clippy -p batter --test scoped_owned_tasks --locked -- -D warnings
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
scripts/jig check test
```

Both full verification commands passed on Rust 1.98.1 (`48a229cea`) and 1.94.0
(`4a4ef493e`), including minimal-core and workspace runtime configurations,
22 runner controls, doctests, formatting, workspace Clippy and rustdoc. All five
HTTP smoke profiles passed using the rebuilt Rust 1.98.1 example. The final Jig
`api:test` runtime/doctest matrix passed with receipt
`receipt_01M23HQSW29YERH6KB9Y9P2RE6`. Cargo.lock
remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
This change has no new Linux, hosted CI, exporter or live PostgreSQL execution
evidence.

## Reference and operational-helper reconciliation: 2026-09-09

User-requested integration follow-up to `batter-4t6`, on main commit
`5bf943526274ff349561648133f4ed06fb17837d` plus the reconciled working changes.
Main's SQLx, startup, health and executable Rust implementations are unchanged.
The reference package/probes are retained alongside them. Shared matrix wiring
now runs four prerequisites: core compilation, process-runner controls,
PostgreSQL-smoke controls and reference-runner controls. Failure in any one
prevents later batches; both runtime profiles remain separately checked.

Cargo regenerated the lockfile; SHA-256 is
`99abe5d8ef161c7193f40dffaf249e066f08490d55bafccfdbc9d5ae09b38472`.
`cargo metadata --locked --format-version 1` resolves SQLx/core/PostgreSQL 0.9.0
only. All six workspace packages are unpublished and retain Rust 1.94.

On Linux x86_64, both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed: 548 Rust test/doctest
executions, zero failures, 17 explicitly ignored live cases across 62 summaries,
plus 22 process controls, 13 PostgreSQL-smoke controls and three reference-runner
controls. Counts include repeated profiles, not 548 distinct tests. Formatting,
Clippy and warning-denied rustdoc passed.

A dedicated disposable PostgreSQL 18.6 Debian server listened on
`127.0.0.1:33049`, with a local administrative role and no TLS. The following
live commands passed on both Rust 1.98.1 and 1.94.0 (set
`RUSTUP_TOOLCHAIN=1.94.0` for the second run):

```sh
POSTGRES_TEST_ADMIN_URL=postgres://postgres@127.0.0.1:33049/postgres bash scripts/test_reference_live.sh
DATABASE_URL=postgres://postgres@127.0.0.1:33049/postgres bash scripts/test_sqlx_live.sh
```

Each run executed all four reference and all ten SQLx-adapter cases with no
failures or ignored cases. On default Rust 1.98.1, the three selected
`postgres_lifecycle` live cases also passed. Rebuilt `http_service` passed
default, SIGINT, deadline, WARN and WARN/deadline smokes; rebuilt
`postgres_lifecycle` passed SIGTERM and SIGINT smokes. The commands and flags
are listed in [testing](testing.md). The server contained no remaining harness
databases after the live runs.

Logs are retained in ignored `.agent/tmp/reconcile-5bf9435/`. Final Jig
check/evidence/gates and session closure belong to
`plan_01M23TBC9TJ6S2G7XQTDVDYP9Q`; its append-only records carry final receipt
identities. Earlier receipts below retain their original source scope and are
not reused as evidence for this combined tree. No new macOS or hosted execution,
Runledger adapter, commit, publication or deployment is claimed.

## Native reference compatibility: 2026-09-09

Bead `batter-4t6`, baseline `e518b6b52a88a460edfb6f7c41107e71932b6c70` plus
the working-tree change. Linux 7.0.11-76070011-generic x86_64; local execution,
not hosted CI. The new reference package uses the exact upstream Git sources and
contracts in [the compatibility manifest](reference-compatibility.md).

Cargo-generated lockfile SHA-256:
`b9ababa4b3955c6c7c850080e687484b893c2be6c2e311343a779cfc89e73a0d`.
Cargo added the selected upstream graph; metadata resolves one SQLx/core/PostgreSQL
version, 0.9.0. All five workspace packages retain Rust 1.94 and `publish = false`.
Only the reference package depends on Runledger or the external-only harness.

| Toolchain | rustc / Cargo | Executed result |
| --- | --- | --- |
| 1.98.1 | rustc `48a229cea` (2026-09-01); Cargo `797e8a9bc` (2026-08-05) | Full `verify.sh` passed: 455 Rust test/doctest executions, zero failed, four intentionally ignored live cases across 53 result summaries; Clippy and rustdoc passed with warnings denied |
| 1.94.0 | rustc `4a4ef493e` (2026-03-02); Cargo `85eff7c80` (2026-01-15) | Same full verification outcome and counts |

These counts include repeated foundation profiles, not 455 distinct tests.
Each matrix also passed 22 existing process/matrix runner controls and three
new reference-runner controls. The latter reject missing/remote/TLS-required
endpoints, incomplete inventories, zero executed cases, skipped cases and
filtered results. Ordinary matrix invocation supplied no live database URL;
the four live cases were compiled and explicitly reported as ignored.

```sh
cargo metadata --locked --format-version 1
cargo tree -p batter-example-reference-service --duplicates
cargo check -p batter-example-reference-service --all-targets --all-features --locked
RUSTUP_TOOLCHAIN=1.94.0 cargo check -p batter-example-reference-service --all-targets --all-features --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Live probes used a dedicated disposable PostgreSQL **18.6** Debian server on
loopback, with a local administrative role and no TLS. The external harness
created and deleted its leases; cleanup-on-start was disabled. No existing
application database or upstream repository was changed.

```sh
POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:33006/postgres' bash scripts/test_reference_live.sh
RUSTUP_TOOLCHAIN=1.94.0 POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:33006/postgres' bash scripts/test_reference_live.sh
```

Both explicit invocations passed all four required cases, with zero failures,
ignored or filtered cases: initialized-schema upgrade; fresh migrations and
transactional enqueue; controlled worker startup/shutdown; lease cleanup/defer/Drop.
This includes dropping a never-polled consuming cleanup future, exact conflict
and isolation codes, original request snapshot readback, owner identity,
rollback of application/job changes, shared migration history and preservation
of an application row during upgrade. Polled cleanup-waiter cancellation is
source-inspected only, with broader fixture failure delivery owned by `batter-kjl`.

The following five HTTP profiles all passed with process exit 0 after rebuilding
the example on default Rust 1.98.1:

```sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Development failures were repaired without dropping assertions: a harness project
name exceeded its 16-character limit; the status readback needed the PostgreSQL
enum cast and uppercase value; SQLx 0.9 migration fixture methods require an
explicit history-table argument; four probe functions needed smaller helpers to
meet the existing complexity limit. The initial matrix also caught its exact
batch-layout test needing to include the newly added third control command; the
test now asserts that command's identity as well as retaining both Rust profiles.

Jig initially rejected authored/resolved test-input drift. A regular recopy
reported customized managed-file conflicts and made no changes. The same pinned
Jig release rendered in a disposable Git worktree; only its generated contract
input changes were transferred back, preserving all repository customizations.
Contract, agent-map and all five package-guide checks passed. Final
`scripts/jig work check --plan-id plan_01M23KB4PN20JRGB7DJA5M12VB --json`
passed all five targets in `run_01M23N5PHMDW9GBPY27H9FT1XF`. Its successful
`api:test` receipt is `receipt_01M23N6K8WMJVQN882T48SNTH0`. Subsequent work
evidence reports the required verify gate passed with fresh target receipts.
No Rust/test/configuration change followed this run. An independent final server
query found zero remaining disposable databases; `max_connections` was 100.

Scratch command logs are in `/tmp/batter-4t6-validation-5lZJ0W/`; the durable
evidence is this section and the plan/receipts. No macOS or hosted execution of
the new graph is claimed. No business reference service, stable adapter,
publication or deployment is established by these compatibility probes.

## Smoke deadline review fix and upstream reconciliation: 2026-09-09

Bead `batter-a63`; baseline `e518b6b52a88a460edfb6f7c41107e71932b6c70`.
The shutdown-deadline control previously required a Python signal callback to
print within 150 ms. It now installs OS-level signal ignore before readiness,
records the actual SIGTERM request, and requires shutdown watchdog expiry,
SIGKILL, reaping and output EOF. Both a runnable child and a child paused by
SIGSTOP after readiness pass. Dedicated signal-delivery controls still require
handler output. Two further controls cover parent interruption while waiting
for readiness and rejection of a complete marker line followed by extra fields.
All thirteen smoke controls use synthetic children; they do not prove pool close.

Before committing, origin/master had advanced through `f367bc2` and `e518b6b`.
The combined source preserves the upstream bounded concurrent matrix runner,
rolling failure-output tails and typed redacted SQLx errors. Readiness observation
uses the same byte-retention path as ordinary capture. Smoke controls join the
matrix's prerequisite phase, with all three failures preventing later phases.
The SQLx executable retains an explicit ExitCode boundary and now renders the
known process wrapper as exactly `Error: process failed`. All upstream source
identity, cleanup-order and redaction assertions remain in a sibling test module;
the original missing-configuration subprocess assertions now run under the
external watchdog. Tracker reconciliation retained both sets of issues, and
both validation histories and append-only work records remain present.

Executed on Linux x86_64, Python 3.12.3, Rust 1.98.1 (`48a229cea`) and
Rust 1.94.0 (`4a4ef493e`):

```sh
python3 scripts/test_smoke_postgres.py -v
python3 -m unittest discover -s scripts -p test_parallel_process.py -v
cargo test -p batter-example-postgres-lifecycle --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
# Run the build and all five following smokes once per toolchain.
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Each full verification passed **473 Rust test/doctest executions**, zero failures
and three explicitly ignored live database tests, plus **22 runner controls and
13 smoke controls**. Both runs passed formatting, minimal-core compilation,
Clippy and warning-denied rustdoc. All five HTTP smoke profiles passed on each
separately rebuilt binary. An initial compile caught a redundant dereference
introduced during conflict resolution; fixing it restored the concrete source
borrow. The first matrix run caught two outdated wiring assumptions: the old
two-command prerequisite count and an overflow fixture accidentally running the
new smoke suite. The corrected fixtures include the third prerequisite and its
failure propagation, while retaining the original output-size bound. Final runs
passed without weakening the process or output assertions.

Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Logs are retained under ignored `.agent/tmp/batter-a63/`. Plan
`plan_01M23JFB7A1AJ7Y5EEWEZ727QM` owns the final Jig check/evidence/gates and
completion records. This combined source has new local Linux evidence only;
prior macOS and PostgreSQL evidence below retains its original source scope.
Hosted CI has not been executed here. OS scheduling, process creation and
watchdog-parent destruction remain outside the subprocess deadline guarantee.

## Lifecycle error-boundary review fixes: 2026-09-09

Bead `batter-7r3.3.2` addresses the three low-severity findings from the
Claude/Codex comprehensive review of the preceding change. The executable now
wraps every returned error in a fixed `ProcessFailure` diagnostic while retaining
the concrete error as its source. Startup failure drives a real cleanup stack and
retains its `CleanupReport`; unsuccessful owned shutdown retains the complete
`Arc<ShutdownReport>` instead of its text. Source-identity assertions compare
addresses without trait-object metadata.

Three unit regressions cover an early concrete error, a failed startup with two
real LIFO cleanup outcomes, and a component plus cleanup failure in a real owned
shutdown report. A separate subprocess test removes `DATABASE_URL` and asserts
the executable exits unsuccessfully with exactly `Error: process failed` on
stderr and no stdout.

Executed against Git baseline `f367bc2425d395f4aa9127eb188681680ab7538b` plus
the working-tree changes on macOS 26.6.2 (`25G83`) arm64 with Python 3.14.7:

```sh
cargo test -p batter-example-postgres-lifecycle --all-targets --locked
cargo clippy -p batter-example-postgres-lifecycle --all-targets --locked -- -D warnings
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
scripts/jig check test
```

The first focused run passed two tests and failed the shutdown source-identity
assertion: the derived error exposed its `Arc` wrapper rather than the retained
report allocation. An explicit `Error::source` implementation corrected that
mapping; the final focused run passed all three unit tests and the subprocess
test. Focused Clippy passed with warnings denied.

Both full verification commands passed on Rust 1.98.1 (`48a229cea`) and 1.94.0
(`4a4ef493e`), including both runtime configurations, 22 runner controls,
doctests, formatting, workspace Clippy and rustdoc. The rebuilt HTTP example
passed all five smoke profiles, and the final Jig `api:test` target passed its
runtime and doctest matrix. Cargo.lock remained unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
`DATABASE_URL` remained absent, so these tests exercise owned report construction
and process rendering without claiming a live PostgreSQL connection or cleanup.

## Initial partial-startup error retention: 2026-09-09

Bead `batter-7r3.3.1` corrects the SQLx example's startup-error boundary. The
example now returns its typed `StartupFailure` through `BoxError`, so the original
startup cause remains its error source and the complete `CleanupReport` remains
available to a trusted sink. Its own `Display` and `Debug` projections contain
only fixed text and the cleanup outcome counts. The focused regression retained
a failed cleanup record, its concrete error, a skipped record and the original
startup cause while rejecting both sensitive test details from those projections.

Executed against Git baseline `f367bc2425d395f4aa9127eb188681680ab7538b` plus
the working-tree fix on macOS 26.6.2 (`25G83`) arm64 with Python 3.14.7:

```sh
cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
cargo check -p batter-example-postgres-lifecycle --all-targets --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
scripts/jig check test
```

The focused command passed its single regression on Rust 1.98.1. Both full
verification commands also exercised it and passed on Rust 1.98.1 (`48a229cea`)
and 1.94.0 (`4a4ef493e`), including both runtime configurations, 22 runner
controls, doctests, formatting, Clippy with warnings denied and rustdoc. The
rebuilt HTTP example passed all five smoke profiles.
The final Jig `api:test` target passed the same configured runtime and doctest
matrix. Cargo.lock remained unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
`DATABASE_URL` was absent, so no live PostgreSQL connection or cleanup was
executed; the new regression covers the error-object boundary without claiming
database integration evidence.

## Local verification review fixes: 2026-09-09

Bead `batter-o5f` addresses the two actionable Claude findings on `batter-tip`.
Matrix output now retains a bounded prefix and fair per-stream rolling tails,
with explicit omission markers. The 8 MiB source-byte budget, continued draining,
and rejection of overflow remain intact. Scheduling and mutation machine evidence
keeps prefix-only capture. The mutation subject copy now includes both
`scheduling_controls.py` and `parallel_process.py`.

On macOS 26.6.2 arm64 with Python 3.14.7, all **60 Python discovery tests** passed
in 36.362 seconds. The **22 runner regressions** include five new controls for
exact under-limit output, separate streams at the shared limit, both streams'
final failures after overflow, actual matrix rendering above its 8 MiB limit,
and isolated execution of the production mutation copy's control entrypoint.
An initial global-tail implementation failed the two-stream regression because
pipe read order could evict the other stream's failure details. Fair per-stream
tails fixed the reproduced failure without changing the test's assertions.

A separate temporary mutation-script copy ran its full binary-independent control
entrypoint in Python isolated mode: all **24 controls across four shards** passed
in 6.353 seconds. Only the copied script directory was added to the import path;
repository imports could not supply missing dependencies.

Both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed **445 Rust test/doctest
executions**, 22 runner regressions, minimal core compilation, formatting, Clippy
and rustdoc. Their wall times were **28.394 / 27.307 seconds** respectively with
warm Cargo caches. The Rust 1.98.1 HTTP example rebuilt successfully and all five
smoke commands listed in the next section passed. Agent-map and all four package
guides also passed their Jig checks. Exact commands, statuses, timings and logs
are retained in `/tmp/batter-o5f-validation/` (`timings.jsonl`); these are local
measurements, not statistical or cold-build benchmarks.

Plan `plan_01M23DDBZWFRVWZ9WYTJYT0QYX` connects final Jig profile receipts and
completion. No fixture deadline, assertion, dependency graph or application API
changed. Linux and Python 3.9 execution of the new capture mode remain unverified;
previous platform evidence retains its original scope.

## Concurrent local verification: 2026-09-09

Bead `batter-tip` preserves the two runtime test configurations while overlapping
core and workspace passes. Jig's two test aliases and `verify.sh` use the same
`test_matrix.py` entrypoint. The scheduling target's 30 Python controls run in
four separate processes with exact assigned/executed-ID checks. Fixture assertions,
individual deadlines and the independent 60-second emergency backstop are unchanged.
The final backend policy accepts fresh passing test evidence from a gate/profile
run under the documented source/configuration/toolchain/environment conditions;
it no longer requires an automatic duplicate invocation.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7, Rust 1.98.1 and 1.94.0:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

All 55 Python discovery regressions passed in 32.025 seconds. This includes 17
new runner/partition controls: real command overlap, peer and spawn failures,
output overflow with continued draining, timeout/kill/reap, partial capture setup,
interruption before a second launch, repeated signals without deadline reset,
preserved ignored SIGINT, partial-output read failure, and rejection of signalling
a reaped group. Discovery controls cover both binary-independent and binary-dependent
suites, new test inclusion without cost hints, duplicate discovery, invalid bounds,
custom signal-owner rejection and missing/duplicate/partial/failed completion.
Matrix wiring controls preserve both feature configurations, prerequisite ordering,
locked commands and failed-peer propagation. Each matrix invocation also runs these
17 regressions before its Rust runtime passes.

Both toolchains passed all **445 Rust test/doctest executions**, plus formatting,
minimal core compilation, Clippy and rustdoc. All five HTTP smoke profiles passed
against the rebuilt Rust 1.98.1 example. Full logs and per-command wall times are
in `/tmp/batter-tip-validation/`; `timings.jsonl` records exact commands and statuses.

The Rust 1.98.1 verification script took **24.723 seconds** with warm Cargo caches.
Its concurrent core/workspace runtime passes took 22.263/22.305 seconds, compared
with the previously measured serial pair's combined 78.897 seconds: about **72% less
runtime-matrix waiting**. Both scheduling targets took about 12.1 seconds, with the
unchanged twelve-second unjoined-work controls now determining their duration;
previously each scheduling target took about 29.2 seconds. Rust 1.94 verification
took 33.576 seconds and included compilation after switching toolchains. These are
single local measurements, not idle-host statistical benchmarks or cold-build claims.
The pre-change full Jig profile took 81.718 seconds; final Jig receipts remain the
authority for this change's complete profile outcome and timing.

The temporary investigation and production runner preserve both dependency graphs:
workspace feature unification additionally enables Tokio bytes/fs/io-util and tracing
log features. Dropping a runtime pass would discard distinct coverage. Ordinary
Python discovery remains serial; sharding applies to direct full-suite execution,
including invocation from the Rust scheduling tests. Per-shard output is capped
at 128 KiB and matrix command output at 8 MiB; overflow fails evidence.

Plan `plan_01M23B60659DFS30Q2JMSQVDKE` connects final Jig checks/evidence/gates and
completion. The passing test receipt from that profile is sufficient for backend
completion when fresh; there is no additional automatic full-test invocation.
No dependency graph, application API, remote CI configuration, or publication change
was made. New runner execution on Linux, other Unix systems and Python 3.9 is
unverified; Python 3.9 syntax parsing alone is not runtime validation. Process
creation/scheduling and arbitrary detached descendants remain outside the runner's
containment claims. Earlier platform evidence below retains its original scope.

## Report exit boundaries and bounded subprocess coverage: 2026-09-09

Owning Bead: `batter-xzq`; baseline `fbe77addaafc8709c95d7ecf4982dd3ceea3f41d`
with the existing working changes preserved. The report abstraction retains its
concrete errors: the defect was incomplete guidance at the application exit
boundary. Rust's Result termination path prints Debug, so the shared-report
rustdoc now includes an explicit ExitCode main. A portable example regression
passes an error whose Debug, Display and source access panic, verifying that
production exit reporting emits only the selected fixed diagnostic.

The raw-report compile-fail examples lacked an exact diagnostic oracle.
`process_ownership/report_usage.rs` now pairs the ignored raw cleanup/shutdown
expressions with `expect(unused_must_use)` and denies unfulfilled expectations.
In a disposable source snapshot, `cargo check -p batter --test process_ownership
--locked` passed unchanged on Rust 1.98.1. Removing only CleanupReport's
must-use attribute failed with exit 101 and an unfulfilled expectation at line
16; restoring it and removing only ShutdownReport's attribute failed identically
at line 18. Neither mutation was applied to the working checkout.

The subprocess gap was duplicated ownership and missing orchestration tests.
The SQLx smoke and executable configuration tests now reuse the existing bounded
Unix process owner. Eleven Python controls exercise the smoke's output checks
and real readiness/signal/overflow/timeout/exit paths. Direct-child PIDs retained
in capture also require `waitpid` to return ECHILD after observation. Two Rust
configuration tests exercise the real executable's three invalid-input cases
and a deliberately hung child under an external watchdog. These controls are
database-independent and cannot establish pool closure. The shared process
owner's 24 standalone controls also passed after the readiness-phase addition.

Executed on Linux x86_64 with Python 3.12.3; Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

```sh
python3 scripts/test_smoke_postgres.py -v
python3 -m unittest discover -s scripts -p test_scheduling_process.py -v
cargo test -p batter-example-postgres-lifecycle --test diagnostics --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Rust 1.98.1 (`48a229cea`, 2026-09-01; Cargo `797e8a9bc`) and Rust 1.94.0
(`4a4ef493e`, 2026-03-02; Cargo `85eff7c80`) verification each passed: 469
Rust test executions, including repeated feature configurations, zero failures,
and three explicitly ignored live database tests; the eleven Python smoke
controls also passed on each run. Formatting, no-default-feature compilation, Clippy and warning-denied
rustdoc passed. The initial sandboxed attempt reached the HTTP example tests
and failed because loopback binding returned EPERM. The complete rerun with
loopback permissions passed; no test or assertion was relaxed.

Rebuilt `http_service` separately with each Rust toolchain and passed all five
HTTP smoke profiles on each: default SIGTERM, SIGINT, deadline, WARN filter, and
WARN filter plus deadline. Exact invocations:

```sh
# Repeat the build with RUSTUP_TOOLCHAIN=1.94.0, then run the same five smokes.
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Jig work check passed all five required targets: Clippy, formatting, tests,
contract and file budget. It retained the existing file-size warnings for
`lifecycle.rs` (746 lines) and `process_ownership.rs` (779 lines); there were no
file-budget errors or waivers. Command:

```sh
scripts/jig work check --plan-id plan_01M23D8KGX9YHDSCGHV9H9QKFD --json
```

Jig's test actions now execute the smoke controls and include both smoke files
and the configuration Python helper in their input digests. The contract was
regenerated with `scripts/jig update --recopy --no-input --force` in a disposable
Git checkout; only the generated contract's test-input changes were retained,
preserving the repository's customized managed guides. Logs and disposable
snapshot locations are under ignored `.agent/tmp/batter-xzq/`.

This change has no new macOS, hosted CI or live PostgreSQL execution evidence.
The CI definition includes the controls but is not execution evidence. OS
scheduling, process creation and loss of the watchdog parent are outside the
subprocess deadline guarantee. No dependencies, public Rust APIs, process
lifecycle contracts or database schema changed in this follow-up.

## Finite-cause and shared-report reconciliation: 2026-09-09

Owning Bead: `batter-bzr`. Local `master` fast-forwarded from `61a025f` to
`fbe77addaafc8709c95d7ecf4982dd3ceea3f41d`, then restored the existing working
changes from recovery stash `7bbb97f910fc24ee9d5356a77d5fb18c2d36480c`.
The expanded concrete-source test in `process_ownership/error_sources.rs`
retains the upstream `FiniteTaskExit("shared-error")` assertion alongside
receipt/report error identity. Both documentation histories and every original
local/upstream Beads and append-only work record were retained. Additive tracker
reconciliation imported `batter-299` without updating or deleting existing issues.

The first direct file-budget check rejected the combined `lifecycle.rs` at 811
lines. Moving the unchanged `ShutdownReport` type and implementations to private
`lifecycle/report.rs` reduced it to 746 lines and passed the existing gate.
Its public path, fields, methods, error/display behavior, must-use diagnostic and
compile-fail example remain available. No source limit or semantic assertion
was relaxed.

Executed on Linux x86_64 (kernel `7.0.11-76070011-generic`), Python 3.12.3:

```sh
cargo test -p batter --locked --test process_ownership --test shutdown_causes --test lifecycle
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 -m unittest discover -s scripts -p test_smoke_postgres.py -v
```

Both Rust 1.98.1 and 1.94.0 full matrices passed 467 test/doctest executions,
with zero failures and three explicitly live database tests ignored per run.
Formatting, isolated core compilation, Clippy and rustdoc passed. After each
toolchain's `cargo build -p batter-axum --example http_service --locked`, all
five `python3 scripts/smoke_http.py --binary target/debug/examples/http_service`
profiles passed: default SIGTERM, `--signal SIGINT`, `--deadline`,
`--warn-filter`, and `--warn-filter --deadline`. All four PostgreSQL smoke
evidence controls passed without a database connection. Cargo.lock is unchanged
at SHA-256 `3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Logs are `/tmp/batter-finite-reconcile-focused.log`,
`/tmp/batter-finite-reconcile-verify-{1.98.1,1.94.0}.log` and
`/tmp/batter-finite-reconcile-http-{1.98.1,1.94.0}.log`. Final Jig receipts belong
to `plan_01M23AKCPCYF368ZTE28XH9PT3`. This reconciliation adds no macOS, hosted
CI or live PostgreSQL execution evidence. Earlier evidence retains its original
scope. All working changes remain uncommitted; the recovery stash is retained.

All five Jig work gates and the final `scripts/jig check test` passed. Their
logs are `/tmp/batter-finite-reconcile-jig-check.json` and
`/tmp/batter-finite-reconcile-final-backend.log`. Bead closure changes Jig's
input digest; refreshed completion receipts retain the same validated source.

## Finite shutdown cause review follow-up: 2026-09-09

The `batter-299` review follow-up corrects the abort-trigger documentation and adds
`shutdown_causes.rs`. Its six paused-clock tests cover finite error/panic causes
with a healthy critical component, the selected error retained in both task-kind
orders, a descendant failure's own label, ready requests taking priority over
unobserved finite/critical errors, and a finite shutdown abort preserving
`Requested`. Each complete scenario has a ten-second Tokio-time watchdog;
these tests contain cooperative futures and do not claim wall-clock preemption.
The classification implementation is unchanged from the preceding section.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7:

```sh
cargo test -p batter --locked --test shutdown_causes
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 445 test/doctest
executions plus formatting, compilation, Clippy and rustdoc. All five HTTP smoke
profiles passed against the rebuilt Rust 1.98.1 example. Full verification output
is retained in `/tmp/batter-299-precedence-verify-1.98.1.log` and
`/tmp/batter-299-precedence-verify-1.94.0.log`; the smoke output is
`/tmp/batter-299-precedence-http.log`. The earlier non-yielding test failure did not
recur, and its missing original assertion cannot be reconstructed from the saved
preview. No diagnosis is claimed. Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Jig work and review completion evidence belongs to
`plan_01M236PZDSB8V98CSH9BH4WW1N`. Linux execution of this change, hosted CI and live
PostgreSQL remain unverified.

## Finite task shutdown causes: 2026-09-09

Bead `batter-299` adds `ShutdownCause::FiniteTaskExit` for an admitted finite
task initiating shutdown. `ComponentExit` remains specific to registered critical
components. The private task-recording helper preserves that distinction when
selecting the cause; shutdown ordering, error retention and cleanup decisions
are unchanged. Downstream exhaustive matches need the new variant.

With the variant declared but the old classification still in place, three
extended finite-failure tests failed with `ComponentExit` instead of
`FiniteTaskExit`: an unobserved error, a typed receipt error and a factory panic.
After the fix, all 46 focused lifecycle/process-ownership tests and four foundation
doctests passed. Existing fixtures now also assert critical-component causes,
normal finite completion followed by `Requested`, and preservation of `Requested`
when later finite errors/panics are observed after the shutdown allowance.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7:

```sh
cargo test -p batter --locked --test lifecycle --test process_ownership
cargo test -p batter --doc --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 433 test/doctest
executions, formatting, compilation, Clippy and rustdoc. All five HTTP smoke
profiles passed against the example rebuilt with Rust 1.98.1. Cargo.lock remains
unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Logs are `/tmp/batter-299-verify-1.98.1.log`,
`/tmp/batter-299-verify-1.94.0.log` and `/tmp/batter-299-http.log`.
Jig receipts are tracked under `plan_01M2355C4C45KVAGFESYDE2GNA` in the append-only
`.agent/state` records. Linux execution of this change, hosted CI and live
PostgreSQL remain unverified.

The first Jig work check failed its file-size gate and the workspace
`non_yielding` test target. Its retained stdout preview omits the failing
assertion, so this run does not establish a diagnosed subprocess defect.
An isolated `cargo test -p batter --all-features --locked --test non_yielding`
rerun passed all 46 tests; its log is
`/tmp/batter-299-non-yielding-recheck.log`. To satisfy the existing file limit,
the same runnable example moved to `ProcessHandle::try_spawn`, with a link from
the new variant, and the unobserved-error fixture now binds and drops its receipt
in separate statements. All regression assertions are retained. The two full
verification runs above preceded these documentation/fixture-layout changes;
the relocated example passed doctest rechecks on Rust 1.98.1 and 1.94.0. A second
Jig work run was invalidated because the agent updated this validation document while
the read-only checks were running. Its effect-policy rejection is retained in
the receipts. A subsequent unchanged Jig work check passed all five gates with
fresh evidence. The final `scripts/jig check test` also passed 433 test/doctest
executions on the final code and fixture layout. The subprocess failure did not
recur in either successful full run. Final logs are
`/tmp/batter-299-jig-final-check.json` and
`/tmp/batter-299-final-backend-test.log`.

## Owned-observer and shared-report reconciliation: 2026-09-09

Owning Bead: `batter-qgu`. Local `master` fast-forwarded from `9aa2f80` to
`61a025f5ed3699405034c1407942cdb4c280297c`, preserving all uncommitted report,
diagnostic and SQLx exit changes. Documentation conflicts retain both histories;
the tracker and append-only work records retain every local and upstream record.
Importing the merged Beads export added three upstream issues without updating
or removing the existing records.

Upstream constructs completion channels only in `Supervisor::start` and exposes
observers through `RunningSupervisor`. The merged driver retains that boundary
and the local `SharedShutdownReport` result. Initial compilation rejected two
upstream `Arc::ptr_eq` assertions on shared reports. They now compare borrowed
`ShutdownReport` pointers with `std::ptr::eq`, preserving the identity oracle.
Coordinator-error assertions still compare their `Arc<JoinError>` values.
The focused observer, lifecycle-state and process-ownership suites passed all
38 tests; the four PostgreSQL smoke-evidence controls also passed.

On Linux x86_64, both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed with 454 test/doctest
executions, zero failures, three explicitly live database tests ignored, and no
compiler warnings per run. Formatting, core compilation, Clippy and rustdoc
passed. After each version's HTTP example rebuild, all five smoke profiles
passed: SIGTERM, SIGINT, deadline, WARN filter and WARN filter with deadline.
Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

The first final Jig run passed functional checks but rejected the combined
rustdoc additions at 801 lines in `lifecycle.rs`, one over its existing budget.
The report paragraph was tightened without changing its meaning or the limit.
Formatting and all eight foundation doctests passed again on both toolchains;
no executable code changed after the full matrix and HTTP smoke runs.

Logs are `/tmp/batter-reconcile-focused.log`,
`/tmp/batter-reconcile-verify-{1.98.1,1.94.0}.log` and
`/tmp/batter-reconcile-http-{1.98.1,1.94.0}.log`. Final Jig receipts belong to
`plan_01M239QGHN0FFTW7H3A8TX62QY`. This reconciliation adds no new macOS, hosted
CI or live PostgreSQL execution evidence; earlier sections retain the exact
scope of those runs. No application commit or publication was performed.

## Distinct report chains and lifecycle exits: 2026-09-09

Owning Bead: `batter-9s6`. Baseline: `9aa2f8006000d460011638a7c9f12ace75e907ca`.
Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

The rendered-chain regression failed before the Display correction. It now
requires distinct owner context, one task/cleanup summary, no error markers,
and the same concrete report source. Five portable SQLx example tests exercise
the production completion and exit helpers, covering awaited successful cleanup,
startup plus cleanup failure, task failure, finalizer failure, and an unavailable
diagnostic writer. Configuration subprocess tests still exclude credential
markers from both streams. Four Python smoke-evidence controls passed.

Executed `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` on both hosts:

| Host | Rust versions | Passed per full run | Failed | Explicit live tests ignored in ordinary run |
| --- | --- | --- | --- | --- |
| Linux x86_64, Python 3.12.3 | 1.98.1, 1.94.0 | 443 | 0 | 3 |
| macOS 26.6.2 (`25G83`), arm64, Python 3.14.7 | 1.98.1, 1.94.0 | 439 | 0 | 3 |

Both versions passed formatting, core compilation, Clippy and rustdoc with no
warnings. Linux includes two extra subprocess cases in each foundation profile.
The ignored cases are separately selected live database tests, not missing
portable execution. All five HTTP smoke profiles passed after each host/version
build: SIGTERM, SIGINT, deadline, WARN filter, and WARN filter with deadline.

Three live tests passed on each host/version against a dedicated, externally
provisioned PostgreSQL 18.3 server: success, startup database error plus failed
cleanup, and database-task failure during shutdown. They retain SQLSTATE `22012`
and await actual pool closure; size reaches zero and acquisition returns
`PoolClosed`. Selecting these tests with DATABASE_URL absent failed all three
with a prerequisite diagnostic, rather than silently passing.

```sh
cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored
cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle
python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle --signal SIGINT
python3 -m unittest discover -s scripts -p test_smoke_postgres.py -v
```

The actual SQLx executable passed both signal smokes on each host/version,
requiring initialized components, successful pool cleanup and exit 0. macOS used
a loopback SSH forward to the dedicated test database. The forward closed when
the run finished, and the task-owned container was stopped and removed. No
database provisioning code or schema changes were added to the repository.

The first macOS attempt reused the existing build directory and received
SIGKILL on the scheduling binary before its tests started. Subsequent signature
verification and test listing succeeded; the cause of that kill is not established.
Complete reruns passed with a fresh isolated target directory and four Cargo
build jobs, without changing test assertions or test-thread settings.

After the full runs, the worker and HTTP example's direct summary formatting
was adjusted to borrow the underlying report. Focused worker/HTTP example tests,
HTTP rebuilds and all five HTTP profiles passed again on both versions/hosts.
All 111 Rust, Python, shell and Cargo/CI configuration files matched between the
final local tree and remote snapshot; manifest SHA-256
`28a3a8fc05bada7cff82ab6d419cc47567e8ee2300d448187a4cc5ab286b7f8e`.

Logs use `/tmp/batter-exit-verify-linux-{1.98.1,1.94.0}.log`,
`/tmp/batter-exit-verify-macos-{1.98.1,1.94.0}.log`, corresponding
`live-*`, `http-*`, and `final-examples-*` files. Jig completion receipts belong
to `plan_01M237XW8BBGZSC5DW4QN7MN8W`. The macOS CI definition now includes these
portable report/exit tests, but hosted CI execution remains unverified. Live
tests establish query failure and pool-close behavior, not transaction commit
ambiguity, cancellation rollback, or external worker compatibility.

## Shared owned shutdown reports: 2026-09-09

Owning Bead: `batter-66g`. Baseline: `9aa2f8006000d460011638a7c9f12ace75e907ca`.
Executed on Linux x86_64 with Python 3.12.3. Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Before the wrapper change, `cargo check -p batter --test process_ownership --locked`
failed on six unfulfilled `unused_must_use` expectations: owner wait, shutdown
and observer wait, each followed by `?` or `unwrap()`. These compile controls
now pass. The targeted process-ownership suite passed 29 tests; foundation
rustdoc passed seven examples, including two new compile-fail examples and a
compiling inspection/propagation example. The runtime regression verifies
shared report/source identity and retained task/cleanup failures after owner
drop, plus Display that omits the original error contents.

Initial full verification identified remaining explicit Arc references and
two implicit report disposals in scheduling fixtures. The callers now borrow
reports for identity checks and explicitly drop reports already checked by
their helpers. Their existing outcome and cleanup assertions were retained.

Fresh complete runs of both commands passed:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 438 test/doctest
executions, including repeated foundation profiles, with zero failures,
ignored tests or warnings. Formatting, isolated core compilation, workspace
Clippy and rustdoc also passed on both toolchains.

After each toolchain's explicit `cargo build -p batter-axum --example http_service --locked`,
all five `python3 scripts/smoke_http.py --binary target/debug/examples/http_service`
profiles passed: default SIGTERM, `--signal SIGINT`, `--deadline`, `--warn-filter`,
and `--warn-filter --deadline`. The 1.94.0 build used `RUSTUP_TOOLCHAIN=1.94.0`.

Local logs use `/tmp/batter-shared-report-verify-1.98.1.log`,
`/tmp/batter-shared-report-verify-1.94.0.log`, and
`/tmp/batter-shared-report-http-{1.98.1,1.94.0}.log`. Jig gate receipts belong to
`plan_01M236778ZMDZ5JBTBPB87WDVE`.

Jig verification run `run_01M236QXDC3G3BKKF696QDZBJ0` passed Clippy, formatting,
tests, contract and file-budget gates. The final `scripts/jig check test` also
passed (1/1 target, exit 0). The file-budget gate retains its nonblocking size
advisories for `lifecycle.rs` and `process_ownership.rs`. Tracker closure changes
Jig's input digest; the plan retains the refreshed completion receipts.

The public owned-driver success type changed from `Arc<ShutdownReport>` to
`SharedShutdownReport`; [usage](usage.md) records the source migration. The lint
is an advisory diagnostic, not evidence of inspection. There is no new macOS,
hosted CI or live PostgreSQL execution evidence and no dependency change.

## Error handling review corrections: 2026-09-09

Owning Bead: `batter-ey0`. Baseline: `9aa2f8006000d460011638a7c9f12ace75e907ca`.
Executed on Linux x86_64 with Python 3.12.3. Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

The concrete receipt source regression failed before the manual Error
implementation: downcasting its source to `std::io::Error` returned None. It
now proves that receipt, shared Arc and report all point at the same error.
The SQLx executable regression also failed before the exit-handler change;
it now checks absent, non-Unicode and malformed URLs without a database.
A prior synthetic non-Unicode URL reproduced credential output on stderr.

Targeted commands passed:

```sh
cargo test -p batter --locked --test process_ownership error_sources
cargo test -p batter-example-postgres-lifecycle --locked --test diagnostics
cargo test -p batter-axum --example http_service --locked
cargo test -p batter --doc --locked
```

The HTTP example tests include absent/explicit logging configuration, rejected
invalid directives, non-Unicode input, retained native sources and redacted
Display/Debug. Both report types have compile-fail doctests for ignored awaited
results under `deny(unused_must_use)`. Existing shutdown, cleanup, replay and
HTTP behavior assertions were retained.

`bash scripts/verify.sh` passed on Rust 1.98.1: 433 test/doctest executions,
zero failed or ignored, zero warnings, including repeated core profiles.
Formatting, isolated core compilation, workspace Clippy and rustdoc also passed.
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` also passed all 433 executions,
formatting, core compilation, Clippy and rustdoc with no warnings.

After the Rust 1.98.1 build, all five `scripts/smoke_http.py` profiles passed:
default SIGTERM, SIGINT, deadline, WARN filter, and WARN filter with deadline.
All five also passed after rebuilding with
`RUSTUP_TOOLCHAIN=1.94.0 cargo build -p batter-axum --example http_service --locked`.
Additional executable checks supplied invalid and non-Unicode RUST_LOG values
and required exit code 1, empty stdout and exactly the sanitized configuration
message on stderr. Each subprocess had a ten-second external timeout.

Jig work verification passed Clippy, formatting, tests, contract and file-budget
gates in `run_01M2351BAY8CXNTR50TGVYX97J`. The required final
`scripts/jig check test` also passed (1/1 target, exit 0).

These changes have no new macOS or hosted execution evidence. No live PostgreSQL
server was contacted, no dependency was upgraded, and no commit, deployment or
publication was performed. Default panic-hook output remains outside the
sanitized returned-error exit path.

## Observer review follow-up: 2026-09-09

Bead `batter-w3o` addresses the observer review's documentation findings and test
gaps. Rustdoc now names the panic when runtime destruction drops an unpublished
completion monitor; the forwarding wait/shutdown methods link to that boundary.
Unreleased records the removed shutdown-handle observer API and its replacement.
Production behavior, dependencies and the retained outcome type are unchanged.

Three new regressions cover the missing boundaries. A current-thread paused test
drops the last owner before first poll, then retains the coordinator panic caused
by a skipped cleanup capture. It verifies one capture drop, no finalizer-factory
call and identical retained JoinError Arcs across observers. Another test creates
an observer from an owner clone after successful publication, drops all owners
and the owning runtime, and reads the identical report on a second runtime. The
runtime-loss test destroys an unpolled monitor while retaining its driver owner,
then requires the documented panic when observing elsewhere. New observation
waits have five-second watchdogs; a timeout has a different panic message and
cannot satisfy the expected library-panic assertion.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7:

```sh
cargo test -p batter --locked --test driver_observer --test process_ownership
cargo test -p batter --doc --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Focused validation passed 31 integration tests and three foundation doctests.
Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 432 test/doctest
executions, with formatting, compilation, Clippy and rustdoc passing. All five
HTTP smoke profiles passed against the example rebuilt with Rust 1.98.1.
Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Logs are `/tmp/batter-w3o-verify-1.98.1.log`,
`/tmp/batter-w3o-verify-1.94.0.log` and `/tmp/batter-w3o-http.log`.
Jig gate and final backend-test receipts are tracked under
`plan_01M2343T3C1G7MAGHKWCQT4N0V` in the append-only `.agent/state` records.
Linux execution of this follow-up, hosted CI and live PostgreSQL remain unverified.

## Completion observers require owned drivers: 2026-09-09

Bead `batter-50z` removes `ShutdownHandle::observer` and creates the completion
channel only in `Supervisor::start`, with the sender owned by its monitor.
Observers are obtained from `RunningSupervisor`; migrate former control-handle
calls after startup. Unstarted supervisors and caller-owned drivers no longer
offer an observer whose missing publisher could cause a pending wait or panic.
The retained result type, waiter cancellation and shutdown protocol are unchanged.

Before removing the old method, the new `ShutdownHandle` compile-fail doctest
failed with exit 101 because the invalid observer construction compiled:

```sh
cargo test -p batter --doc --locked lifecycle::ShutdownHandle
```

After the change, that doctest passes. The new current-thread runtime test
obtains and clones an observer immediately after `start`, drops the final owner
without yielding, then proves the component and finalizer each ran once and
both observers retain the same successful report. The old abandonment test's
observer assertion is replaced by the compile-time API restriction; its readiness
and cancellation assertions remain. The existing tests still retain coordinator
panics and prove that cancelling a shutdown waiter does not cancel cleanup.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7:

```sh
cargo test -p batter --locked --test driver_observer --test lifecycle_state --test process_ownership
cargo test -p batter --doc --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The focused commands passed 33 integration tests and three foundation doctests.
Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 426 test/doctest
executions, including repeated foundation execution, the scheduling/subprocess
corpora and live loopback readiness tests. Formatting, compilation, Clippy and
rustdoc passed on both toolchains. The HTTP example was rebuilt with Rust 1.98.1;
all five smoke profiles exited 0. Cargo.lock is unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Local logs are `/tmp/batter-50z-verify-1.98.1.log`,
`/tmp/batter-50z-verify-1.94.0.log` and `/tmp/batter-50z-http.log`.
Jig gate and final backend-test receipts are tracked under
`plan_01M2337YZR5WE7DC7SP2RZ6BX8` in the append-only `.agent/state` records.
Linux execution of this change, hosted CI and live PostgreSQL remain unverified.

## Extracted cleanup and abandonment review follow-up: 2026-09-09

Bead `batter-vtx` clarifies that extracting finalizers does not detach their
captured operation tokens from process cancellation. The contract and usage
guidance require independent teardown; production lifecycle behavior and public
signatures are unchanged.

The new public regression drops the supervisor before closing its extracted
stack. It proves that the process operation token is cancelled, drop performs no
teardown, and explicitly awaited cleanup completes with a fresh independent
`OperationContext`. The capture-order regression now observes both component and
cleanup captures for direct supervisor drop and unpolled driver drop. Separate
probes require each readiness/drain/cancellation waiter to be notified outside
the state lock, permitting redundant wakes and checking each future becomes ready.

Executed on macOS 26.6.2 (`25G83`), arm64, Python 3.14.7:

```sh
cargo test -p batter --locked --lib --test lifecycle_state
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The focused run passed 13 tests. Rust 1.98.1 and 1.94.0 each passed 423 test/doctest
executions, including repeated foundation execution and live loopback readiness
tests. Formatting, compilation, Clippy and rustdoc passed on both toolchains.
The HTTP example was rebuilt with 1.98.1; all five smoke profiles passed.
Logs: `/tmp/batter-vtx-verify-1.98.1.log`, `/tmp/batter-vtx-verify-1.94.0.log`,
and `/tmp/batter-vtx-http.log`. Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Linux execution of this follow-up, hosted CI and live PostgreSQL remain unverified.
Final Jig evidence, gates and backend test receipts are recorded in the finish
resolution for `plan_01M232EEV96Y284FZQYHWWY79H` under `.agent/state`.

## Upstream and local lifecycle reconciliation: 2026-09-09

Local `master` was advanced from `19aa9ff` to upstream `37888ff` while retaining
the uncommitted lifecycle work for `batter-8ot` and `batter-7dm`. The three
documentation conflicts were reconciled by preserving both validation histories,
combining platform evidence, and retaining the upstream behavior-focused testing
guide with the local lifecycle coverage rows. Local crate changes and upstream
adapter/script changes were each verified byte-for-byte against their original
trees. Append-only Jig records and both sides' changed Beads records were retained;
the merged Beads export was imported into the local database.

Executed on macOS 26.6.2 (`25G83`), arm64, with Python 3.14.7:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`) and 1.94.0 (`4a4ef493e`) each passed 421 test/doctest
executions, including repeated foundation execution and live loopback readiness
tests. Formatting, compilation, Clippy and rustdoc passed on both toolchains.
All 38 Python controls and all five HTTP smoke profiles passed. The HTTP example
was rebuilt with Rust 1.98.1. Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Local logs are `/tmp/batter-reconcile-verify-1.98.1.log`,
`/tmp/batter-reconcile-verify-1.94.0.log`, `/tmp/batter-reconcile-python.log`
and `/tmp/batter-reconcile-http.log`. This adds macOS evidence for the combined
source, without changing earlier results' historical scope. Linux execution of
the combined source, hosted CI and live PostgreSQL remain unverified.

## Lifecycle transition ownership and startup abandonment: 2026-09-09

Bead `batter-7dm` centralizes readiness/admission facts in the private
`lifecycle/state.rs` module. All readiness transitions, including coordinator
completion, and atomic snapshot publication now require the same mutex guard.
No caller can directly mutate those fields. The published snapshot retains
nonblocking readiness reads while native enqueue holds admission. Explicit
readiness/drain/cancellation wakeups run after releasing the guard.

Supervisor construction owns synchronous abandonment signaling, transferred
into the caller-owned driver. Both structs place that guard before captured
application values/the inner future. Abandonment reports Draining, signals
cancellation and wakes readiness waiters without invoking factories/finalizers
or fabricating a completion report. The private driver wrapper uses the existing
pin-project-lite dependency without allocation or public signature changes.

On macOS 26.6.2 (`25G83`), arm64, Rust 1.98.1, the new public ownership tests were
run before the implementation:

```sh
cargo test -p batter --locked --test lifecycle_state
```

It exited 101: the registered readiness waiter remained at Starting and captured
value destruction observed no abandonment signal. The owned-unpolled-driver
control passed. The initial all-mutex implementation then failed
`cargo test -p batter --locked --lib readiness_reads_remain_available_while_admission_is_held`
with Timeout; retaining a privately published atomic snapshot fixed that regression.
The test releases admission and joins the reader even on failure, using a
two-second scheduling watchdog rather than claiming a wall-clock latency bound.

The final focused command passed 58 tests:

```sh
cargo test -p batter --locked --lib --test lifecycle --test process_ownership --test lifecycle_state
```

Seven new unit tests cover all six driver/approval/component-ack startup orders,
the 20-entry readiness transition table, late acknowledgements, concurrent
request/completion, root/active/expired-scope admission with open/closed queues,
readiness reads during admission and three wakers checking the mutex is unlocked.
Four public tests cover abandoned-owner notification/no fabricated report, direct
and unpolled-owner capture destruction, ownership transfer, and invalid-name,
startup, capacity and terminal rejection precedence with inert factories.
The concurrent test releases request and completion together; deterministic
serialized transition cases cover their possible orders under the shared mutex.
This is not exhaustive exploration of the Tokio scheduler.

A temporary mutation making drain unconditional was rejected by the transition
table with `Stopped, event 2`: actual Draining, expected Stopped (exit 101).
The source was restored byte-for-byte before full verification.

Both complete verification matrices passed on the same macOS arm64 host with
Python 3.14.7:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Each matrix passed 184 foundation / 205 workspace entries and two doctests,
including all 15 scheduling and 46 macOS non-yielding entries. Formatting,
compilation, Clippy and rustdoc passed. The HTTP example was rebuilt with
Rust 1.98.1 (`48a229cea`, 2026-09-01); all three smoke modes exited 0.
Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
The first Jig profile passed Clippy, formatting, tests and the contract check,
but failed file-budget: the six earlier admission regressions pushed
`process_ownership.rs` above 800 lines. Those tests and their helper were moved
unchanged into `process_ownership/terminal_admission.rs`; no limit or assertion
was relaxed. Both matrices and HTTP smoke modes were repeated after this move.
Linux execution of this follow-up, hosted CI and live PostgreSQL remain
unverified. Final Jig gate receipts and closure are recorded under plan
`plan_01M22KZAHKTX0287A51AW4GHH0` in the append-only `.agent/state` records.

## Terminal process admission precedence: 2026-09-09

Bead `batter-8ot` corrects rejection classification when permanent closure
precedes the coordinator's first poll. Admission now checks forced cancellation,
task failure, expired ancestors, root drain and a closed coordinator queue before
startup errors. Name validation still runs first; active descendants retain their
drain exception and the shared capacity bound. No dependency or public signature
changed.

On macOS 26.6.2 (`25G83`), arm64, Rust 1.98.1, the six new tests were run before
the repair:

```sh
cargo test -p batter --locked --test process_ownership admission_is_closed
```

It exited 101: shutdown before startup, unpolled driver drop, abort before first
poll, and direct unstarted supervisor drop each returned `NotRunning` instead
of `Closed`. The normal completed shutdown and post-start abort controls passed.
After the repair, this focused command passed all 46 entries:

```sh
cargo test -p batter --locked --test process_ownership --test lifecycle
```

All six closure tests check that rejected factories remain inert. Existing
startup/readiness, capacity, active-descendant drain, forced cancellation and
admission/drain race assertions also passed.

Both complete verification matrices exited 0 on the same macOS arm64 host with
Python 3.14.7:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

The default toolchain was Rust 1.98.1 (`48a229cea`, 2026-09-01). Each matrix
passed 173 foundation / 194 workspace entries and two doctests, including all
15 scheduling entries and 46 macOS non-yielding entries. Formatting, compilation,
Clippy and rustdoc passed. The HTTP example was rebuilt with 1.98.1; all three
smoke modes exited 0. `scripts/jig doctor` reported ready and
`scripts/jig check contract` passed.

The first `scripts/jig check test` ran every test successfully (command exit 0),
but Jig rejected the result because validation documentation changed during its
read-only run. The receipt retains the `effect_policy` worktree-fingerprint
failure; it is not a passing Jig check.
The same `scripts/jig check test` was rerun with the worktree unchanged for its
entire execution and passed (Jig exit 0), including the locked foundation,
workspace and doctest matrix. Only evidence and tracker records were updated
after that final check; the tested Rust sources are unchanged.

Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
This adds local macOS scheduling evidence; Linux execution of this repair and
hosted execution remain unverified. Historical Linux results below retain their
original scope. No live PostgreSQL, runtime-death finalization, or exhaustive
scheduling guarantee is established.

## HTTP redaction and filtered-operation assertions: 2026-09-09

Bead `batter-faj.7` addresses the two accepted review gaps. The nested-observer
test now checks its full captured output for secrets before inspecting individual
completion events. The WARN-filtered process smoke rejects INFO operation
completions, while allowing WARN deadline completions and application INFO events.
Three additional Python control entries cover rejected INFO events with and
without the example formatter's timestamp, permitted WARN/application output,
and absent operation events. The timestamped negative control failed against
the first parser version and passed after its timestamp handling was corrected.

Executed on Linux x86_64 with Python 3.12.3:

```sh
python3 -m unittest discover -s scripts -p test_smoke_http.py -v
cargo test -p batter-axum --test observation composition_edges --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The seven focused Python controls and three middleware-edge tests passed.
Rust 1.98.1 and 1.94.0 each passed the full verification script, including 391
test/doctest executions, formatting, Clippy and rustdoc. All five rebuilt-example
smoke profiles passed. The first full Python discovery attempt failed in existing
Jig subprocess controls because the sandbox denied signal-handler writes with
EPERM; the rerun outside that sandbox passed all 38 entries. The first Jig work
check ran its tests successfully but rejected all target receipts because this
validation document changed during the read-only run. A repeat must keep the
checkout unchanged; final gate/test outcomes are recorded in the task plan and
Jig receipts using these commands:

```sh
scripts/jig work check --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X
scripts/jig check test
scripts/jig work evidence --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X --json
scripts/jig work gates --plan-id plan_01M22SNAMJ0FC0BT7A3NX9M11X --json
```

Logs are under `.agent/tmp/batter-faj.7`; tracked work uses
`plan_01M22SNAMJ0FC0BT7A3NX9M11X`. Cargo.lock remains unchanged at SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Runtime APIs and dependencies are unchanged. macOS/hosted execution, live
concurrent-connection correlation and sink-specific exporter behavior remain
unverified by this follow-up.

## HTTP smoke oracles and test diagnostics: 2026-09-09

Bead `batter-faj.6` addresses the two accepted review findings. Every HTTP process
smoke now checks method, route, status, outcome and latency after the completion
message, independently of fields printed on its spans. The shared Rust text
assertion and live readiness assertions use the same event boundary. Four Python
negative-control entries cover successful events, missing/conflicting event fields
despite correct span fields, and a missing completion message. Existing Python CI
discovery includes them without a workflow change.

The live readiness fixture still awaits teardown before assertions. One assertion
now checks and reports the saved request, supervisor and server outcomes together.
A task-local fault-injection control drove real teardown with request, supervisor
and server failures: the old assertions hid the request error, while the revised
assertion included all three injected messages. The fixture was restored
byte-for-byte after each control, including a first injector compile error from an
unconstrained Result type. That injector annotation was corrected; both expected
failure controls then passed. The initial Python control also caught and rejected
an overly broad latency substring match before it was corrected.

Executed on Linux x86_64 with Python 3.12.3:

```sh
python3 -m unittest discover -s scripts -p test_smoke_http.py -v
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo test -p batter-axum --all-targets --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
python3 .agent/tmp/batter-faj.6/check_diagnostics.py
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Python discovery passed 35 entries. The focused adapter run passed 44 entries.
Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 391 test/doctest executions: 169 foundation, 218 workspace and four
doctests. Formatting, compilation, Clippy and rustdoc passed on both toolchains.
All five rebuilt-example HTTP smokes passed with shutdown status zero. No ordinary
test was failed or ignored in the final runs; the diagnostic controls deliberately
required failure and checked the resulting messages.

Logs and the one-off diagnostic control are under ignored `.agent/tmp/batter-faj.6/`.
Tracked gate and final backend evidence belong to
[the owning plan](../.agent/plans/plan_01M22Q37MQG1B1Z3533Y7PQMZV.md).
No runtime implementation, public API or dependency changed. Cargo.lock remains
SHA-256 `3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Hosted/macOS execution, live concurrent-connection correlation and broader
transport lifetime coverage remain unverified by this follow-up.

## HTTP verification wiring and composition edges: 2026-09-09

Bead `batter-faj.5` addresses the completed review's CI and prerequisite findings
and its bounded observation test gaps. Linux CI now includes both WARN-filtered
HTTP process smoke profiles. macOS CI adds adapter integration/live example tests
and all five HTTP smoke modes. The required all-targets gate continues to run the
live readiness tests; `scripts/verify.sh` and the testing guide now state their
loopback socket and Unix subprocess prerequisites. No test was ignored or removed.

Three new integration test entries cover DEBUG/TRACE overrides filtered under an
INFO subscriber (with a WARN positive control), outer status/severity rewriting
after observation, and retained overrides through nested observers. The existing
complete-router scenario now checks newly added routes and unsupported methods
while Starting/Ready/Draining, including admission 503 before method fallback.
The unwind test additionally requires the original identity on resource destruction.
The live example adds a mixed-filter profile over all four readiness phases: INFO
events are absent and the Stopped WARN event retains its generated response ID.
Both live profiles check unique response IDs and per-response event counts.
Contracts and primary-source notes describe these existing behaviors; no runtime
implementation, public API or dependency changed in this follow-up.

Executed on Linux x86_64 with Python 3.12.3 and unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
bash -n scripts/verify.sh
cargo test -p batter-axum --locked
cargo test -p batter-axum --all-targets --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 391 test/doctest executions: 169 foundation, 218 workspace (including
42 adapter integration tests and two live example tests) and four doctests. No
test failed or was ignored. Formatting, compilation, Clippy and rustdoc passed
on both toolchains. The example was rebuilt with 1.98.1; all five process smokes
passed with shutdown status zero. A Python/PyYAML check parsed the workflow and
confirmed both filtered profiles in each CI job. Shell syntax passed.

Focused checks and both matrices are retained under ignored `.agent/tmp/batter-faj.5/`
as `adapter-tests.log`, `adapter-all-targets.log`, `adapter-clippy.log`,
`verify-1.98.1.log`, `verify-1.94.0.log`, `http-build.log` and five `http-*.log`
smoke results. Required Jig evidence/gates and final backend results are recorded
with [the owning plan](../.agent/plans/plan_01M22ND6VP41F0KQ8FVQJ0HZFH.md).

The updated CI workflows were not run on hosted runners, and macOS runtime
execution of these changes remains unverified. Broader streaming, disconnect,
connection shutdown and exporter delivery remain outside this task's evidence.

## HTTP context ownership and handler unwinds: 2026-09-09

Beads `batter-faj.4` and `batter-4qc` address a local ownership error: the optional
HTTP INFO span was also the observer's correlation parent. With
`RUST_LOG=info,batter=warn`, an explicit disabled HTTP parent made the WARN event
rooted even when the application's request span remained enabled. Looking up a
fallback only during destruction could instead attach another request's identity.
Research against resolved primary sources and related consumer tasks preceded
implementation; decisions and filtering limits are in [references](references.md).

The observer now selects and retains its HTTP span or available application span
at first poll, reusing it for inner execution/destruction and explicit completion
parenting. The original HTTP span remains the only target of HTTP field recording.
Public APIs, response severity policy, dependencies and the foundation's dispatch
wrapper remain unchanged. The example exposes a private router constructor used
by both main and its readiness test, preserving the runtime composition.

Three correlation regression entries cover standalone/split/combined observation,
enabled/disabled HTTP spans, interleaved requests, completion/drop under another
span and subscriber, retained parent lifetime, untouched application fields and
an absent/filtered original parent. The handler-unwind case verifies a Tokio task
panic, cancelled admitted context and exactly one WARN dropped observation without
an HTTP status or panic payload in tracing. Rust's default panic hook is unchanged.
Two of these tests compiled and failed before the fix; the absent-parent test
already passed. A second compiled mutation, selecting `or_current` at event
emission, failed both the retained-parent and absent-parent assertions. The mutation
was removed before successful final verification.

A live loopback test executes the example's actual readiness router during
Starting, Ready, Draining and Stopped. It checks 503/200/503/503, respectively,
INFO/INFO/INFO/WARN event levels, outcome, route, generated request identity and
one HTTP event per response. A separately owned listener deliberately remains
available through Stopped; this proves response policy, not the binary's connection
shutdown window. Client I/O and teardown have bounds; lifecycle transitions use
acknowledgements and an explicit release channel rather than startup sleeps.
The initial sandboxed attempt failed at socket bind with EPERM. Its approved
rerun passed without changing the test.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3, with
unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
cargo test -p batter-axum --locked
cargo test -p batter-axum --example http_service --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 -m py_compile scripts/smoke_http.py
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 387 test/doctest executions: 169 foundation, 214 workspace (including
39 adapter integration tests and the live example test) and four doctests. No
tests failed or were ignored. Both scripts passed formatting, compilation, Clippy
and rustdoc. The example was rebuilt with 1.98.1; all five process smoke commands
passed with signal exit zero. The new `--warn-filter` profile verifies that each
5xx WARN event retains its application request ID and its own HTTP fields with
the HTTP INFO span disabled, while INFO HTTP events are absent.

Ignored logs under `.agent/tmp/batter-faj.4/` retain `context-before.log`,
`late-parent-mutation.log`, `adapter-tests.log`, `adapter-clippy.log`,
`readiness-http.log` (the socket denial), `verify-1.98.1-tail.log`,
`verify-1.94.0.log`, `http-build.log` and five `http-*.log` smoke results. The
1.98.1 tail starts after the initial 39 successful executions; its initial output
and the focused approved live-test success remain in the tool transcript.
Required Jig gate outcomes and the final backend test receipt are recorded with
[the owning plan](../.agent/plans/plan_01M22JCBCPYZWQRDZSBPKQ6CZZ.md).
The first final backend run passed all 387 Rust executions and exited zero, but
Jig rejected its receipt because an independently added `.reviewignore` changed
the worktree fingerprint during execution. That file was preserved; the rejected
receipt and output remain in `jig-final-test-first.log`. This was an evidence
freshness failure, not a Rust test failure.

macOS and hosted CI execution of this change remain unverified. These checks do
not establish arbitrary per-layer/exporter delivery, panic recovery, aborting
panics, damaged shared-state recovery, streaming/body panics after headers,
disconnect handling or transitive connection shutdown. The corresponding broader
transport and metadata/exporter tasks remain open.

## HTTP completion fields independent of spans: 2026-09-09

Bead `batter-faj.3` corrects the dependency of HTTP completion fields on an enabled
INFO span. The observer retains normalized method and the cloned matched route
template and emits method, route, optional numeric status, outcome and latency
on the completion event itself at every severity. Existing span fields remain
for nested context. This does not change response construction lifetime,
admission, response severity policy or the number of completion events. Filtering
of application correlation spans and event delivery remains application-owned.

Three new regressions use an event visitor that never reads span fields. They
cover every severity across standalone/split/combined observers, disabled INFO
parents for WARN/ERROR, success/client/server outcomes, normalized custom methods,
matched and unmatched routes, admission rejection, and dropped futures under a
different ambient subscriber. Paused time checks exactly 25 ms on completed
events. Dropped futures retain WARN, omit status, and reach the first-poll
subscriber. All three tests compiled and failed on the original implementation
because the event contained only a message; the fix made them pass. The initial
test build needed an explicit response return type on the unreachable handler;
that compile error is retained separately and is not regression evidence.
The testing guide now describes coverage and commands rather than duplicating
aggregate inventories; dated executed counts remain here.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3, with
unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
cargo test -p batter-axum --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 .agent/tmp/batter-faj.3/smoke_warn.py
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 383 test/doctest executions: 169 foundation, 210 workspace (including
36 adapter tests) and four doctests. No tests failed or were ignored. Formatting,
compilation, Clippy and rustdoc passed on both toolchains. The example was rebuilt
with 1.98.1 and all three standard HTTP smoke modes passed. An additional live
check with `RUST_LOG=warn` requested `/fail`, asserted one 500 completion with all
HTTP fields and no INFO span context, checked query redaction and exited through
SIGTERM with status zero. The first network attempt was blocked by sandbox socket
permissions before starting a server; the network checks then ran with approval
outside that sandbox.

Logs and the additional smoke script are under ignored `.agent/tmp/batter-faj.3/`:
`regression-before.log`, `test-initial-compile.log`, `adapter-tests.log`,
`adapter-clippy.log`, `event-fields.log`, `verify-1.98.1.log`, `verify-1.94.0.log`,
`http-build.log`, `http-default.log`, `http-sigint.log`, `http-deadline.log`,
`http-warn.log` and `http-sandbox-denial.log`. The final full scripts include the
later status-class and exact-latency assertions. Required repository gate results
and final backend test results belong to
[the owning plan](../.agent/plans/plan_01M22FQT8J3TFN9T5K1XVZQKRB.md), with
`jig-work-check.log`, `jig-final-test.log`, `jig-evidence.json` and `jig-gates.json`
in that log directory.

macOS and hosted CI execution of this change remain unverified. These checks do
not establish streaming-body, disconnect or panic recovery behavior, or delivery
by an arbitrary subscriber/exporter. The standard network smoke still does not
force requests into the example's Starting/Draining readiness window.

## Explicit HTTP observation severity: 2026-09-09

Bead `batter-faj.2` adds `HttpObservationLevel(tracing::Level)` as application-owned
response metadata. Standalone observation and the combined compatibility wrapper
read it through their shared helper. Unannotated responses retain WARN for 5xx
and INFO otherwise; futures dropped without returning a response retain WARN.
The level does not change status, outcome, fields, correlation, response headers
or body. Request extensions and client level headers do not select severity.
The runnable example explicitly marks Starting/Draining readiness responses INFO;
other application failures and stopped-process probes retain their defaults.

Five new test entries exercise all five tracing levels across standalone/split/
combined observation, default status classes, ignored request-side hints, explicit
readiness policy versus unrelated unguarded 503s, failure-renderer overrides,
and inner middleware replacing/removing an override. Existing rendering tests
now also assert an ERROR override on 429 after admission rejection, timeout and
forced cancellation, retaining the original response/correlation assertions.
Abort tests assert WARN despite an annotated response constructed but never
returned, with nested resource destruction under the first-poll subscriber.
Unpolled and body-lifetime tests remain intact. Focused verification passed
33 adapter tests and two doctests.

A temporary mutation that ignored the response extension compiled and failed
the event-level assertion with exit 101. The source was restored byte-for-byte.
An initial test compilation rejected incorrect Axum response tuple ordering;
that was corrected. The initial full 1.98.1 run passed its tests but failed
Clippy's complexity limit in the expanded tracing branches. Selecting separate
noncapturing emitters resolved the lint without an exemption. Both full scripts
were then executed successfully on the final Rust source.

Executed on Linux x86_64 with unchanged Cargo.lock SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Rust 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02)
each passed 380 test/doctest executions: 169 foundation, 207 workspace and four
doctests, with zero failures or ignored tests. Formatting, compilation, Clippy
and rustdoc also passed on both. The HTTP example was rebuilt with 1.98.1;
all three smoke modes passed their existing response, correlation, redaction,
one-completion-per-request and signal assertions. Smoke does not force requests
into the example's startup/drain window; readiness severity is exercised in the
router tests, not established by these network smoke cases.

Logs are under ignored `.agent/tmp/batter-faj.2/`: `verify-1.98.1.log`,
`verify-1.94.0.log`, `verify-1.98.1-initial.log`, `mutation-ignore-level.log`,
`http-build.log`, `http-default.log`, `http-sigint.log`, and `http-deadline.log`.
Final repository gate receipts belong to
[the owning plan](../.agent/plans/plan_01M22AY02A7BBJJHCNFX3RDQNN.md):
`scripts/jig work check --plan-id plan_01M22AY02A7BBJJHCNFX3RDQNN` and the final
backend `scripts/jig check test`, with `jig-work-check.log` and
`jig-final-test.log` in that log directory. Jig/Beads closure records retain
their outcomes without changing the source snapshot after verification.

macOS and hosted CI execution of this change remain unverified. No new body
streaming, real disconnect, handler panic recovery or log-delivery guarantee is
established. DEBUG/TRACE observations remain subject to subscriber filtering.

## Independent HTTP observation: 2026-09-08

Bead `batter-faj.1` adds stateless `observe_http` and policy-driven
`request_admission`, retaining `request_scope` as the combined compatibility
entry point. Both paths share the private observation/admission implementations;
`RequestPolicy` still couples lifecycle readiness and deadlines. The runnable
HTTP example assembles guarded routes, probes and fallback before observation,
with server identity outermost. Its identity future now also protects full
instrumented-future destruction with the public dispatch helper.

The new 12-test observation executable checks full-router event counts and
sanitized fields through startup, readiness and drain; application errors and
short-circuit/status-changing middleware; probe/fallback coverage; an added
application route; custom rendering after timeout/forced cancellation; retained
original correlation; context cancellation; standalone observation without an
execution policy; and subscriber retention during abort and nested destruction.
Directly discarded unpolled entry points emit no completion or application work.
Dropping a response body after construction produces no second HTTP event. This
last check uses an ordinary unconsumed body and does not establish streaming or
real disconnect behavior.

Placement tests make two Axum limits executable: routes appended after
`Router::layer` bypass observation, and a wrapper outside routing records
`<unmatched>` before matched metadata exists. Nesting outer observation around
legacy `request_scope` deliberately produces two HTTP observations; the supported
split composition emits one. There is no automatic deduplication. All 16 existing
adapter tests remain, with their assertions intact. The existing scoped-dispatch
capture helper is shared with the new executable; no global subscriber is installed.

Two targeted mutations failed event-count assertions with exit 101: removing
standalone observation, and having `request_admission` call the combined wrapper.
The middleware source was restored byte-for-byte after each mutation. Neither
failure was a compile error or a missing prerequisite.

Executed on Linux x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3:

```sh
cargo test -p batter-axum --locked
cargo clippy -p batter-axum --all-targets --locked -- -D warnings -D clippy::mod_module_files
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

All commands exited 0. Each full Rust 1.98.1 / 1.94.0 matrix passed 169 foundation
entries in isolation, 202 workspace entries and three doctests: 374 successful
entries including repeated foundation execution, zero failed and zero ignored.
The workspace counts 28 Axum tests, 169 foundation tests and five generic support
tests; the added doctest compiles the split router composition. Formatting,
Clippy, compilation and rustdoc also passed on both toolchains.

The normal executable was rebuilt with Rust 1.98.1. All three HTTP smoke modes
passed, now requiring exactly one HTTP completion with matching server-generated
request ID, route and status for each tested request, including probes and
unmatched fallback. Polling readiness may generate additional requests with their
own IDs; the assertions do not confuse them with the explicit test requests.
Untrusted IDs and raw unmatched path/query sentinels must be absent from output.
The application envelope, deadline and both native signal checks remain intact.

Logs are in ignored `.agent/tmp/batter-faj.1/`: `verify-1.98.1.log`,
`verify-1.94.0.log`, `http-build.log`, `http-default.log`, `http-sigint.log`,
`http-deadline.log`, `mutation-missing-observation.log` and
`mutation-duplicate-observation.log`. The owning
[execution plan](../.agent/plans/plan_01M21E97EVPQTVPJ6DDC79SR40.md) records Jig
receipts and final backend verification. `scripts/jig work check --plan-id
plan_01M21E97EVPQTVPJ6DDC79SR40` passed Clippy, formatting, tests, contract and
file-budget checks. Work evidence/gates reported fresh and passed. The final
`scripts/jig check test` also exited 0. Logs are `jig-work-check.log` and
`jig-final-test.log` in the same directory.

Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
There are no dependency or minimum-version changes. macOS and hosted CI execution
of this change remain unverified. No downstream repository was modified, and no
commit, push or publication was performed.

## Scenario-derived synchronization and overflow controls: 2026-09-08

Bead `batter-fvz` addresses the three-reviewer follow-up. The timing finding was
a private harness policy duplication: a five-second panic synchronization guard
ignored the deliberate two-second post-startup observation. It now derives the
maximum allowance from the scenario's wait policy. An exact-instant regression
includes panic after the latest accepted startup; no near-deadline launch sleep
or configurable timing protocol was added.

Event waits now observe process status before sampling capture and join the
reader on exit, so unread final output is not rejected as missing evidence.
Exited-child controls cover present and absent events. Overflow retains the first
detected byte/event cause and reports the captured event count. A live event-bound
control floods output after exhausting metadata, requires ordinary child exit
to prove continued draining, and still rejects evidence for event overflow after
the byte cap. The live deadline-wiring control also rejects a first kill request
delayed by a full observation allowance past the selected deadline.

The focused Linux suite passed all 48 entries. Three targeted mutations each
failed with exit 101 and were restored byte-for-byte: returning startup alone
from the maximum wait calculation; stopping capture on overflow; and delaying
the actual wait threshold by observation plus one second while recording the
original deadline. The last fails with `watchdog kill was late`. Clippy initially
reported the expanded fixture function at 101/100 lines; extracting the existing
invalid-output scenarios fixed this without a lint exemption.

Research answers are in [references](references.md#synchronization-review-research-2026-09-08).
The macOS hosted job is deliberately focused. The exported scenario test protects
the removed ambient launch path. The Python probe has one intentional startup
phase budget; its hard alarm can bypass `finally`, leaving containment to the
independent deadlines and adopting reaper. The ten-second emergency control
continues to test the actual fallback in ordinary matrices. No finite margin
guarantees progress on a suspended/starved OS. These are explicit limits, not
reasons to introduce another deadline configuration or signal-handler protocol.

Both Rust 1.98.1 and 1.94.0 verification matrices exited 0 on Linux x86_64
(kernel `7.0.11-76070011-generic`, Python 3.12.3) and the authorized macOS arm64
host (macOS 26.6.2 build `25G83`, Python 3.14.7). Each host ran:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

HTTP was rebuilt using 1.98.1; all three smoke modes passed on both hosts.
Linux passes 154 foundation / 175 workspace entries plus two doctests; macOS
passes 152 foundation / 173 workspace entries plus two doctests. The focused
suite contains 25 process controls, 19 pure evidence/policy tests, three reader
tests and one inert dispatch entry; two process controls are Linux-only.
Formatting, Clippy, compilation and rustdoc passed through the full matrices.
Static package inspection and diff checks also passed.

macOS executed `/tmp/batter-review.ttFRHr`, reusing only the previous target
cache through a symlink; Cargo rebuilt the changed test source. All 67 source/build
hashes match on both hosts. Snapshot archive SHA-256:
`9cce84cdd3006e20ec2074c099365acc83ed90a7b9c6dabfc923ed5e012e841d`.
Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Documentation and tracker records were finalized after source validation.

Current follow-up logs in ignored `.agent/tmp/batter-fvz-review/` are
`verify-linux-1.98.1.log`, `verify-linux-1.94.0.log`, `mutation-startup-only.log`,
`mutation-stop-draining.log`, `mutation-late-kill.log`, `http-build.log`,
`http-default.log`, `http-sigint.log`, `http-deadline.log`, `macos-manifest.log`,
and the copied `macos/` logs. Older differently named logs in that directory
belong to earlier work. Jig receipts and final backend-test evidence belong to
[this execution plan](../.agent/plans/plan_01M20N7KXZDVWH9WM6AA1V4303.md).
The updated hosted macOS job remains unexecuted: the successful remote runs
still refer to baseline `e5f2f04`, and no commit or push was performed. Library
APIs, dependencies and lifecycle behavior are unchanged.

## Startup evidence after a child panic: 2026-09-08

Bead `batter-fvz` fixes a reproduced polling-order failure and the stale
Linux-only evidence summary in `docs/status.md`. A complete drain record
captured on time now establishes the observation deadline even when the next
parent poll sees later panic diagnostics. Capture overflow remains an error
before event lookup. Missing startup still fails promptly on a known panic;
a genuinely late event remains a deadline failure. Final validation still
rejects all captured child panics. This separates timing evidence from final
outcome validation without changing the library or dependencies.

The fresh 41-entry baseline passed. Regressions added before the fix failed
four tests (exit 101): the on-time event plus later panic, a late event plus
panic, the pure late-poll deadline calculation, and the live deliberate-panic
fixture. The live control waited for captured panic output before its first
startup resolution and reproduced the original `child panicked` unwrap failure.
It now reaches final validation and verifies that the panic is rejected there,
with the unchanged drain-plus-observation deadline and observed watchdog kill.
No arbitrary startup delay or near-deadline sleep was added.

Two additional mutations were rejected with exit 101 and restored byte-for-byte:
removing final panic rejection fails
`evidence::tests::later_panic_keeps_captured_startup_but_invalidates_final_evidence`;
removing the startup capture bound fails
`evidence::tests::byte_overflow_invalidates_even_previously_observed_startup`.
The latter now includes both an on-time drain and panic diagnostics before
overflow. Missing-startup panic remains covered by an immediate-failure test.

Both supported toolchains passed on Linux x86_64 (kernel
`7.0.11-76070011-generic`, Python 3.12.3) and the authorized macOS arm64 host
(macOS 26.6.2 build `25G83`, Darwin 25.6.0, Python 3.14.7): Rust 1.98.1
(`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`, Cargo `85eff7c80`).
These commands each exited 0 on both hosts, with HTTP rebuilt using pinned
Rust 1.98.1:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Linux passes 150 foundation / 171 workspace entries and two doctests, including
44 focused entries. macOS passes 148 foundation / 169 workspace entries and two
doctests, including 42 focused entries. The focused executable has 22 process
controls, 18 pure evidence/policy tests, three reader-thread tests and one inert
dispatch entry; two process probes are Linux-only. Formatting, compilation,
Clippy and rustdoc also pass through the verification scripts.

macOS ran an isolated source snapshot at `/tmp/batter-panic.jg9qJ2`, reusing the
previous temporary Cargo target directory via a `target` symlink. Cargo rebuilt
the changed targets from this source before execution. Archive SHA-256:
`7e897c7214b16915da9fa6d922cbea1588afe231fc80f79af572bbade7404ad2`.
All 67 source/build hashes match on both hosts. Documentation and tracker
records were finalized afterward. Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Ignored `.agent/tmp/batter-fvz-panic/` retains the baseline, before-fix and focused
logs, both mutation logs/backup, host verification and HTTP logs, source manifest
and archive. Remote logs/environment are copied into `macos/`. Repository gate
and final backend-test evidence belongs to
[the execution plan](../.agent/plans/plan_01M20K6ESFXMBG1239ATKTGFF6.md).
The updated hosted macOS job remains unexecuted; SSH testing does not validate
GitHub Actions execution. Abrupt-owner-death adoption/reaping probes remain
Linux-only; macOS covers the shared parent-pipe EOF path. Existing OS-scheduling,
process-control and application-finalization limitations remain explicit.

## Deadline wiring and reader failure controls: 2026-09-08

The final three-reviewer pass identified a remaining test gap in Bead
`batter-fvz`: pure deadline calculations were tested, but a fixed deadline
could replace the computed argument to `wait()` without failing the fast-startup
process controls. The implementation already used the correct deadline.

A new real-process control now runs both blocked-runtime scenarios and asserts
that the deadline actually passed to `wait()` equals the captured drain instant
plus the three-second observation allowance. It also requires observed SIGKILL
and a first kill request at or after that deadline. This checks the driver and
policy connection without a near-deadline startup sleep.

Capture now accepts a private `Read + Send + 'static` input; process runs still
use their ordinary pipe reader. Three scripted-reader controls exercise the
same OS thread, read loop and join path: retry Interrupted and finish on EOF;
return the original BrokenPipe kind/message after partial output; and convert a
reader panic into the fixed harness I/O error. Each confirms retained evidence
and reader destruction before finish returns. Neither read errors nor panics
are accepted as successful complete capture. No library API, dependency or
application behavior changed.

The fresh baseline passed all 37 focused entries. The expanded suite passes 41
on Linux and 39 on macOS: 21 process controls, 16 pure evidence/policy tests,
three reader-thread tests and one inert dispatch entry, with two Linux-only
probe controls excluded on macOS. Three mutations each failed (exit 101), then
were restored byte-for-byte before the full matrices:

| Mutation | Rejecting test / evidence |
| --- | --- |
| Replace `wait(limit)` with `wait(EXIT_LIMIT)` | `watchdog::tests::blocked_runtime_wait_uses_the_captured_drain_deadline`; all child milestones were present, but the selected deadline differed from drain plus observation (5.01-second run). |
| Replace read-error propagation with EOF | `capture::tests::reader_io_error_is_preserved_after_partial_output_and_join`; finish incorrectly returned Ok. |
| Replace reader-panic propagation with success | `capture::tests::reader_panic_becomes_an_io_error_after_partial_output_and_join`; finish incorrectly returned Ok. |

Both Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`,
Cargo `85eff7c80`) passed the following full matrices on Linux x86_64 (kernel
`7.0.11-76070011-generic`, Python 3.12.3) and macOS arm64 (26.6.2 build `25G83`,
Darwin 25.6.0, Python 3.14.7). Every command below exited 0; HTTP was freshly
built with pinned Rust 1.98.1 on each host.

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

Linux passes 147 foundation / 168 workspace entries and two doctests; macOS
passes 145 foundation / 166 workspace entries and two doctests. The verification
scripts also pass formatting, compilation, Clippy and rustdoc. The updated
hosted macOS CI job remains unexecuted; these are actual SSH host results.

macOS used the isolated `/tmp/batter-wiring.0G3Ftp` directory. The transferred
source archive has SHA-256
`200cbd184f11e91112d58b42ef6192d679d9bdad19899660ab96595cc9b0d721`;
all 67 source/build file hashes match on both hosts. Documentation and tracker
evidence were finalized afterward. Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Logs, restored-mutation backups and the source manifest/archive are retained in
ignored `.agent/tmp/batter-fvz-wiring/`, with remote environment and command logs
in `macos/`. Final Jig gate and backend-test evidence belongs to
[the execution plan](../.agent/plans/plan_01M20HPY9R5QVQ616B894HCAGK.md).
The prior platform, OS-scheduling and process-finalization limitations remain.

## Captured evidence and deterministic deadlines: 2026-09-08

Bead `batter-fvz` now separates diagnostic capture, event evidence and deadline
policy. The repeated timing findings exposed a private harness design weakness:
startup and final validation parsed different evidence, while polling time and
post-capture elapsed time stood in for event and kill-request times. They did
not establish a defect in Batter's lifecycle architecture. Research into Rust
sleep/process semantics and macOS runners preceded implementation; primary
sources are recorded in [references](references.md#subprocess-evidence-clocks-and-platform-research-2026-09-08).

A behavior-preserving extraction first passed the original 22-entry suite.
The subsequent fix gives complete protocol records one bounded owner with
capture timestamps. Recording and inspection sample time under the same mutex,
so a deadline decision cannot use stale evidence followed by a newer clock.
An on-time record remains valid when polling is late; a late record cannot
extend startup. Observation starts from captured evidence. Validation uses the
first actual kill-request time and observed SIGKILL, never capture-join delay.
Partial protocol records and byte/event overflow fail closed. This timestamp
measures parent capture, not an unobservable child write time.

The four-second sleep regression was replaced with deterministic boundary tests.
Sixteen pure tests cover exact/fragmented/duplicate records, incomplete final
records, panic/overflow, delayed polling, early/late/missing startup, retained
observation budgets, early kill requests and non-SIGKILL statuses. The real
subprocess controls still exercise blocked Tokio runtimes, missing startup,
emergency exit and kill/reap behavior. Linux has 37 focused entries (20 process
controls, 16 pure tests and one inert fixture dispatch); macOS has 35 because two
reaping/probe controls require Linux.

Three mutations each failed with exit 101, then were restored before full
verification: checking the poll clock before buffered evidence; resetting the
observation window from polling time; and approving a kill using elapsed time
after capture joining. The first two fail
`timing::tests::delayed_startup_keeps_its_full_observation_even_when_polled_late`;
the third fails
`watchdog::tests::delayed_capture_cannot_make_an_early_kill_look_timely`.

Final source passed on both hosts:

| Host | Environment | Verification entries per toolchain |
| --- | --- | --- |
| Linux | x86_64, kernel `7.0.11-76070011-generic`, Python 3.12.3 | 143 foundation, 164 workspace, two doctests; 37 focused |
| macOS via authorized SSH | arm64, macOS 26.6.2 build `25G83`, Darwin 25.6.0, Python 3.14.7 | 141 foundation, 162 workspace, two doctests; 35 focused |

Both hosts used Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0
(`4a4ef493e`, Cargo `85eff7c80`). These commands all passed with exit 0 on each
host, including formatting, compilation, Clippy, rustdoc and tests:

```sh
RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
```

HTTP was rebuilt with the pinned 1.98.1 toolchain on each host. macOS used an
isolated `/tmp/batter-evidence.06xIfN` directory, with no existing checkout
modified. The transferred source archive has SHA-256
`18012ec8a13218378321b7d9d4f2bdc2c4bc71cd3d5feb33e2e608bf448d47ee`;
a manifest confirmed identical hashes for all 66 source/build files on both
hosts. Documentation and tracker evidence were finalized afterward. Cargo.lock
is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Ignored logs are under `.agent/tmp/batter-fvz-evidence/`: `baseline.log`,
`extraction.log`, `focused.log`, the three mutation logs, `verify-linux-*.log`,
`http-linux-*.log`, `source-manifest.json` and `snapshot.tar`. The `macos/`
subdirectory retains environment, both verification and three HTTP smoke logs.
Final repository gates and completion evidence belong to
[the execution plan](../.agent/plans/plan_01M20F5AD3SH9MD9GM4YMNTKQZ.md).

The CI definition now compiles all workspace targets and runs focused macOS
tests on both supported toolchains. That updated hosted job has not executed;
the SSH results above are actual host evidence. Linux-only probes do not claim
macOS reaping coverage. Other Unix targets, OS suspension and process-control
failures remain outside the executed evidence. Library behavior, public APIs
and dependencies are unchanged. Process termination does not prove application
finalization.

## Separate startup and observation windows: 2026-09-08

Bead `batter-fvz` was reopened for the comprehensive-review timing finding.
The blocked current-thread scenarios now allow five seconds from spawn to
observe `drain-requested`, followed by the fixture's two-second observation
plus one second of margin. The combined eight-second allowance remains below
the child's independent ten-second emergency exit. Ordinary scenarios retain
their existing deadlines. Validation records and checks the selected deadline;
an extended observation cannot be approved using the old five-second floor.

Three new controls cover delayed launch, absent startup, and a SIGKILL between
the ordinary deadline and an extended deadline. The delayed-launch test first
failed against the original watchdog at 5.005940244 seconds with
`missing event blocked-observation-elapsed`. It holds the launch record for
four seconds, then requires all original blocked-runtime milestones and rejects
completion/cleanup evidence. With the fix, all 22 subprocess entries pass in
10.01 seconds. A separate mutation restored only the old validation comparison
`self.elapsed >= LIMIT`: the extended-deadline control failed (exit 101) in
5.00 seconds because that comparison accepted the premature kill. The watchdog
was restored byte-for-byte before full verification.

Logs and the mutation backup are retained under ignored
`.agent/tmp/batter-fvz-startup/`: `before-fix.log`, `focused.log`,
`old-deadline-mutation.log`, `verify-1.98.1.log`, `verify-1.94.0.log`, and
`http-*.log`. The focused command was
`cargo test -p batter --test non_yielding --locked`; the before-fix run selected
`-- --exact watchdog::tests::delayed_startup_preserves_the_blocked_runtime_observation_window`,
and the mutation selected
`-- --exact watchdog::tests::watchdog_rejects_a_kill_before_an_extended_observation_deadline`.

Executed on Linux `7.0.11-76070011-generic`, x86_64, Python 3.12.3:
Rust 1.98.1 (`48a229cea`, Cargo `797e8a9bc`) and Rust 1.94.0 (`4a4ef493e`,
Cargo `85eff7c80`). Cargo.lock is unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
These commands passed (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

Both full matrices pass 128 foundation entries, 149 workspace entries and two
doctests, including all 22 subprocess entries, plus formatting, compilation,
Clippy and rustdoc. The three rebuilt HTTP smoke modes pass. Jig gate receipts
and final completion status belong to
[the execution plan](../.agent/plans/plan_01M20DGZY1HF5V5TRAX9KQ0WT1.md).

This is a private test-harness fix; library behavior, APIs and dependencies are
unchanged. macOS execution remains unverified. Scheduling suspension and OS
process-control failures remain outside these test timing bounds.

## Prompt unwind cleanup regression: 2026-09-08

Bead `batter-fvz` was reopened for the unwind test's missing timing assertion.
The control now requires owner-thread unwinding and capture-reader joining to
finish within five seconds of child startup. The startup-relative measurement
prevents a late unwind from passing by waiting for the ten-second emergency
exit. The Linux `/proc/<pid>` absence check still establishes reaping.

An executed mutation replaced only `FixtureChild::Drop`'s `kill_and_reap()`
call with `self.child.wait()`. The command was:

```sh
cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::parent_unwind_kills_reaps_and_joins_capture_reader
```

Before the assertion, that mutation passed (exit 0) in 10.00 seconds. With the
assertion, the same mutation failed (exit 101), reporting
`unwind cleanup took 10.002227365s; expected completion before 5s`.
The original watchdog source was restored byte-for-byte after each mutation;
no destructor behavior change remains. Logs are retained under ignored
`.agent/tmp/batter-fvz-unwind/` as `original-wait-mutation.log` and
`fixed-wait-mutation.log`.

Execution uses the same Linux x86_64 platform, Python 3.12.3 and Rust/Cargo
versions documented in the optimization follow-up below. Cargo.lock is
unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
After restoring the destructor, these commands passed (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig check test
python3 scripts/check_package.py
git diff --check
```

Each matrix passes 125 foundation entries, 146 workspace entries and two
doctests, including all 19 subprocess entries, plus formatting, compilation,
Clippy and rustdoc. The three HTTP smoke modes and final Jig backend check pass.
The matrix logs are `verify-1.98.1.log` and `verify-1.94.0.log`; HTTP and Jig logs
are `http-*.log` and `jig-test.log` in the mutation log directory above.

macOS execution remains unverified. This change adds a private test assertion
and supporting documentation; library behavior and dependencies are unchanged.

## Optimized parent-death probe checks: 2026-09-08

Bead `batter-fvz` was reopened for the confirmed Python optimization finding.
The probe now uses explicit failure branches for deadlines, EOF, output bounds,
required evidence, owner/child status and forbidden events. The child command
accepts an argument vector so a Rust negative control can supply independent
protocol fixtures. Both the real orphan-reaping test and this control set
`PYTHONOPTIMIZE=1`. The control checks that a child exiting 7 and a child emitting
cleanup evidence before exiting 74 each cause probe exit 1, a specific failure
diagnostic, and no success marker. Their own alarm bounds the control children.

Before replacing the assertions, this command failed as intended (exit 101):

```sh
cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::optimized_parent_death_probe_rejects_invalid_child_evidence
```

The original optimized probe accepted the wrong exit code, returned 0, and
printed `batter-fixture:parent-death-reaped`. After the explicit-check fix, both
negative cases and the real parent-death case pass. This is an executed
regression check against the original failure, not just a source assertion.

Executed locally on Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu,
Python 3.12.3, Rust 1.98.1 (48a229cea 2026-09-01) / Cargo 1.98.1
(797e8a9bc 2026-08-05), and Rust 1.94.0 (4a4ef493e 2026-03-02) /
Cargo 1.94.0 (85eff7c80 2026-01-15). Cargo.lock remains unchanged, SHA-256
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

These commands passed (exit 0):

```sh
cargo test -p batter --test non_yielding --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

The focused harness passes 19 entries in 10.01 seconds. Both full matrices pass
125 isolated foundation entries, 146 workspace entries and two doctests with no
ignored tests, plus formatting, compilation, Clippy and rustdoc with warnings
denied. All three rebuilt HTTP smoke modes pass. Static package inspection
counts 146 authored entries and reports no failures. Full-matrix and HTTP logs
are retained under ignored `.agent/tmp/batter-fvz-python-optimize/`.
The final backend check, `scripts/jig check test`, also passed (exit 0); its log
is `jig-test.log` in that directory, with a receipt in `.agent/state/`.

This follow-up changes private test infrastructure and its documentation only.
No public API, dependency, or library behavior changed. macOS execution remains
unverified; no hosted CI, PostgreSQL, or production execution is claimed.
The earlier validation sections remain historical evidence for their snapshots.

## Non-yielding harness review fixes: 2026-09-08

Bead `batter-fvz` was reopened after comprehensive review. Follow-up execution
record: [.agent/plans/plan_01M20896B9AP88W2T4XZD2AGW6.md](../.agent/plans/plan_01M20896B9AP88W2T4XZD2AGW6.md).
The Git baseline remains `e5f2f04b2dbb349d08085caf177f662fcbc89812`.
The first implementation's evidence below is historical; its eight-entry
harness has been superseded by the 18-entry harness described here.

Research into GitHub cancellation, runner v2.337.0, Rust process/pipe semantics,
and Linux subreapers is recorded in [references](references.md#subprocess-review-follow-up-2026-09-08).
It did not establish unconditional runner process-group termination. The fix
adds a child-owned emergency deadline and a parent-owned stdin pipe that both
authorizes a PID-specific launch and ends the child on parent death. Capture
files are replaced by continuously drained, bounded memory. Kill requests and
actual wait status are separate facts. The platform follow-up also removes
non-Unix example signal fallbacks and records the Unix-only policy in guides,
package rustdoc and ADR-007. Library behavior, public APIs, manifests, Cargo.lock
and the resolved Tokio 1.53.1 dependency remain unchanged.

Executed locally on Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu,
with Python 3.12.3 and both supported toolchains: rustc 1.98.1
(48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05), rustc 1.94.0
(4a4ef493e 2026-03-02), Cargo 1.94.0 (85eff7c80 2026-01-15).
Cargo.lock SHA-256:
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

These commands passed after the final Unix-only platform cutover (exit 0):

```sh
cargo test -p batter --test non_yielding --locked
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

The focused suite passed all 18 entries in 10.01 seconds. Each full matrix
passed 124 isolated foundation entries, 145 workspace entries and two doctests,
with no ignored/filtered entries, plus formatting, compilation, Clippy and
rustdoc with warnings denied. All three HTTP smoke modes passed and exited 0.
Static package inspection found 145 authored entries and no link/package
failures; this is distinct from executed Rust evidence. The changed CI YAML
also parsed successfully locally.

The final platform cutover removed the Windows watchdog branch and CI target,
and all three non-Unix example signal fallbacks. Linux and macOS remain in
scope and use the same native Unix implementation. SIGINT/SIGTERM registration
still precedes readiness acknowledgement. A source/workflow audit found no
remaining Windows implementation branches, non-Unix signal fallbacks or Windows
CI targets. Final matrix logs are `verify-unix-1.98.1.log` and
`verify-unix-1.94.0.log`; rebuilt HTTP smoke logs are `http-unix-*.log` in the
directory below. The lockfile hash is unchanged.

The Linux regression killed only the fixture owner's PID after observing
unjoined work, skipped cleanup and runtime-drop entry. Its isolated subreaper
adopted and waited for the child's parent-disconnect exit (74), with no task
destruction or finalizer invocation. Unix controls verified the ten-second
emergency exit (75) with blocked work and incomplete launch, mismatched launch
PID rejection, harmless inherited scenario state, preserved natural statuses
after a later kill, early-kill rejection, forbidden events, output overflow,
spawn failure and parent-unwind cleanup. Post-run inspection found no fixture
processes or legacy capture directories.

Five temporary harness mutations each produced an executed test failure (101):
removing parent-EOF termination, accepting the wrong launch PID, accepting any
unsuccessful Unix status as a kill, ignoring forbidden events, and ignoring
capture overflow. Each used
`cargo test -p batter --test non_yielding --locked -- --exact watchdog::tests::<test_name> --nocapture`
with the corresponding regression in `watchdog_tests.rs`. All mutations were
restored byte for byte before both full matrices. The original report/cleanup
mutation evidence below remains applicable; its semantic assertions remain.
An initial compile check rejected moving a capture JoinHandle through
`catch_unwind`; the unwind control now joins a deliberately panicking owner
thread, avoiding an `AssertUnwindSafe` assertion or reuse of unwound state.

Jig's five configured targets passed, and its required `verify` evidence gate
was fresh and passed. Completion commands are:

```sh
scripts/jig work check --plan-id plan_01M20896B9AP88W2T4XZD2AGW6
scripts/jig work evidence --plan-id plan_01M20896B9AP88W2T4XZD2AGW6 --json
scripts/jig work gates --plan-id plan_01M20896B9AP88W2T4XZD2AGW6 --json
scripts/jig check test
```

Logs are retained under ignored `.agent/tmp/batter-fvz-review/`; completion
receipts, including the final backend test, belong to the linked plan. The macOS
focused CI job remains configured but has not been executed here; neither macOS
compilation nor hosted success is claimed. Windows is unsupported and not
planned; its CI job and exit-code branch have been removed. Process
termination proves neither application finalization nor Tokio preemption;
after real parent death, its adopting OS reaper owns waiting. No PostgreSQL,
external-service, hosted-CI or production evidence is implied. Nothing was
committed, published or deployed.

## Non-yielding subprocess tests: 2026-09-08

Bead `batter-fvz`, Git baseline `e5f2f04b2dbb349d08085caf177f662fcbc89812`.
Added a private foundation integration-test fixture and OS-process watchdog;
production source, manifests and dependencies are unchanged. Execution is local
Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu, with rustc 1.98.1
(48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05), rustc 1.94.0
(4a4ef493e 2026-03-02), and Cargo 1.94.0 (85eff7c80 2026-01-15).
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`;
the resolved Tokio version is 1.53.1.

Executed successfully on the final test source (exit 0):

```sh
cargo test -p batter --test non_yielding --locked -- --nocapture
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
scripts/jig work check --plan-id plan_01M205HP71C12K7EQ44H0JFW7G
scripts/jig work evidence --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
scripts/jig work gates --plan-id plan_01M205HP71C12K7EQ44H0JFW7G --json
scripts/jig check test
```

The focused executable passed all eight entries in 5.01 seconds: seven parent
tests plus an inert child-dispatch entry. Each full Rust matrix passed isolated
core compilation and 114 core entries, 135 workspace entries, two doctests,
all example/binary compilation targets, Clippy and rustdoc with warnings denied.
No entries were ignored or filtered in either full matrix. The three HTTP smoke
modes passed probes, work/deadline and envelope/telemetry assertions, then exited
zero after SIGTERM or SIGINT. Static inspection found 135 test entries and no
package/link failures; it is separate from the executed Rust evidence.
Jig's five configured targets passed (Clippy, formatting, tests, repository
contract and file budget), and its required `verify` evidence gate was fresh
and passed. The final backend `scripts/jig check test` passed the same locked
core/workspace/doctest matrix. No gate or semantic assertion was relaxed.

The cooperative child observed forced cancellation, joined its direct task,
ran the dependency finalizer and destroyed its runtime normally. The two-worker
child retained its blocked task as abort-requested and unjoined, produced no
fabricated termination record, skipped the finalizer factory, and remained
live during runtime destruction until killed. The current-thread child polled
its timer before task entry and externally acknowledged two seconds after drain,
beyond its timer and configured total allowance, without timer/report/cleanup
completion. Watchdog timeouts killed and reaped their children after the
five-second limit. Failure controls rejected missing milestones and child
panics, including a panic after expected stall milestones. Parent-unwind cleanup
removed capture files and left no `/proc/<pid>` entry (including no zombie).
A post-run process/capture scan found no fixture processes or temporary directories.

Two temporary production mutations challenged the report assertions using
`cargo test -p batter --test non_yielding --locked -- --exact non_yielding_child_reports_unjoined_work_and_cannot_drop_runtime --nocapture`.
Clearing the returned `unjoined` list failed the direct-join assertion; replacing
unsafe-exit skipping with cleanup invocation failed the live-task check and
cleanup-record assertion. Both executions failed as expected (exit 101), and
the parent killed/reaped the child after five seconds. Production source was
restored byte for byte; no weakened assertion or mutation remains.

The first fixture attempt stalled its two-worker driver after a drain request
from the blocking task, consistent with Tokio's documented non-stealable LIFO
wake slot. The final fixture requests drain from a separate OS thread after
task-entry acknowledgement. An initial compile error used the wrong cleanup
variant and was corrected to the existing `Succeeded` outcome. The default
panic hook prints the deliberately caught parent panic with `--nocapture`;
the test passes without replacing that hook or a global tracing subscriber.

Logs are retained locally under ignored `.agent/tmp/batter-fvz/`. This evidence
does not establish task preemption, application cleanup after OS kill, hidden
descendant termination, hosted CI, non-Linux behavior, live PostgreSQL or
production operation. No external service prerequisite was skipped. Nothing
was committed, published or deployed.

## Beads backlog migration: 2026-09-08

Audited delivery requirements against Git baseline `9654b2e`, including the
roadmap, integration/testing/operations docs, Effect reconciliation, capability
status, security policy and completed execution plans. Four planning review
rounds, a separate fresh-context fixture check and a final source-removal audit
resolved dependency sequencing and cleanup-ownership ambiguities. No runtime
code, dependency graph, upstream integration or publication behavior changed.

The tracker now contains 34 records: 29 tasks and five epics, comprising 28 open,
three closed and three deferred outcomes. The existing extraction issue was
reused; completed local-validation/finite-ownership milestones were migrated as
closed rather than rerun or reopened. All 12 former roadmap identifiers survive
in Beads provenance. Markdown task inventories were removed; contracts,
implemented capability facts and historical execution evidence remain.

Migration checks executed locally:

- Compared all 34 stored descriptions, acceptance fields, priorities, statuses
  and types through `br show` / `br list` with the reviewed issue descriptions.
- Compared each `br dep list <id> --json` result with the intended graph:
  66 edges, including parent grouping and blocking prerequisites.
- `br dep cycles --json`: no cycles.
- `br lint --status all --json`: no missing issue-template sections.
- `br ready --type task --json`: ten actionable delivery tasks; deferred items
  and tasks with unfinished prerequisites were excluded.
- `br --no-db list --all --deferred --json`: the JSONL-only view matched the
  database's descriptions, statuses and priorities. `br sync --flush-only`
  refreshed the tracked export, and `bv --robot-triage` saw all 34 records.
- `python3 scripts/check_package.py`: no TOML, internal-link or static source
  inspection failures. `git diff --check`: no whitespace errors.

An attempted `bv --beads-file` override was rejected by the installed CLI; no
such command was retained in documentation. Viewer source/count checks use its
reported `source_path` and the authoritative `br` result. Epic template lint
initially requested `Success Criteria` headings; those headings were corrected
and the clean result above is the final check. These are migration checks, not
new Rust, hosted-CI or live-database execution evidence. Current work is available
through [Beads navigation](roadmap.md).

## Original authoring evidence (historical)

The following table describes the original authoring environment only. Subsequent
local execution and dependency-upgrade evidence is recorded below.

The authoring environment did not contain `rustc`, `cargo`, or `rustfmt`.
The container could not resolve the Rust toolchain download host, and an attempt
to obtain the distribution through the download facility did not succeed.
No external compiler/CI execution was substituted or claimed.

| Check | Evidence status |
| --- | --- |
| Source files and 67 test cases authored | Present; inventory counted locally. |
| TOML manifest/toolchain/config parsing | Checked locally with Python tomllib. |
| Internal Markdown file links | Checked by scripts/check_package.py. |
| Rust delimiter lexical balance | Checked by scripts/check_package.py; NOT a Rust parser/type checker. |
| Shell/Python script syntax and CI YAML structure | Checked locally; not evidence of hosted CI execution. |
| ZIP extraction/CRC/source checksums | Checked during packaging. |
| Cargo dependency resolution / Cargo.lock | NOT RUN; no lockfile invented. |
| rustfmt / compiler / borrow checker | NOT RUN. Initial formatting must be normalized by bootstrap. |
| Unit/integration/doctests / Clippy / rustdoc | NOT RUN. No passing-test claim. |
| HTTP example / SIGTERM smoke / traffic tests | NOT RUN. |
| SQLx example against PostgreSQL | NOT RUN. |
| Runledger/Runlimit/harness compatibility | NOT IMPLEMENTED / NOT VERIFIED. |
| Intended MSRV 1.88.0 / current stable matrix | NOT VERIFIED. |
| Security audit / load / race / platform coverage | NOT PERFORMED. |

The machine-readable [package checks](package-checks.json) record the local static
inspection. These checks catch packaging mistakes, not Rust type/lifetime/runtime
errors. Source integrity hashes attest archive contents, not software correctness.

## Original first-run procedure

```sh
rustup show
rustc --version --verbose
cargo --version
bash scripts/verify.sh --bootstrap
# Review and commit the real Cargo.lock and formatter changes.
bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
```

Repeat the verification script with a current stable toolchain. The included CI
workflow describes the matrix but has not run here. Resolve any transitive MSRV
conflicts deliberately; do not casually raise the declared minimum or claim
reproducible builds while leaving the dependency graph unlocked.

## Recording subsequent evidence

Append the toolchain/target, platform, full command, resolved Cargo.lock SHA-256,
exit status, test counts, skipped prerequisites, and failure details. Change
source-status claims only after the corresponding command succeeds. Database
and upstream-integration claims need their own actual executions; successful
SQLx example compilation alone does not establish transaction behavior.

The initial validation milestone is retained as closed Bead `batter-okz`.
Its original Rust 1.88 baseline precedes the SQLx-driven minimum-version increase;
see [backlog access](roadmap.md) for tracker commands.

## Dependency and toolchain refresh: 2026-09-07

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu.
All runs below were local, not hosted CI. A Cargo-generated lockfile already
existed at the start of this session. Bootstrap formatted the authored sources;
`cargo +1.94.0 update` subsequently refreshed the dependency graph.

| Graph / toolchain | rustc | Cargo | Result |
| --- | --- | --- | --- |
| Original SQLx 0.8.6 graph / 1.88.0 | 1.88.0 (6b00bc388 2025-06-23) | 1.88.0 (873a06493 2025-05-10) | Full verification passed after the two repairs below. |
| Upgraded graph / minimum 1.94.0 | 1.94.0 (4a4ef493e 2026-03-02) | 1.94.0 (85eff7c80 2026-01-15) | Full verification passed. |
| Upgraded graph / pinned 1.98.1 | 1.98.1 (48a229cea 2026-09-01) | 1.98.1 (797e8a9bc 2026-08-05) | Full verification and HTTP SIGTERM smoke passed. |

The Rust release server identified stable as 1.98.1 on 2026-09-07. `rustup update
stable --no-self-update` installed that release, and `rustc +stable --version`
and `cargo +stable --version` match the pinned 1.98.1 versions above. The matrix
was run through the pinned toolchain; an identical stable-alias run was not repeated.
Default rustfmt: 1.9.0-stable (48a229ceae 2026-09-01).
Default Clippy: 0.1.98 (48a229ceae 2026-09-01).

Commands and outcomes (exit 0 unless explicitly described otherwise):

```sh
RUSTUP_TOOLCHAIN=1.88.0 bash scripts/verify.sh --bootstrap
cargo +1.94.0 update
cargo +1.94.0 update --verbose
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
bash scripts/verify.sh  # rust-toolchain.toml selects 1.98.1
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
```

The first 1.88.0 bootstrap run exited 101 at Clippy: `tests/cleanup.rs` wrapped
`pending::<Result<(), BoxError>>` in a redundant closure. The second exited 101
at rustdoc: `src/http.rs` used an unquoted `Extension<OperationContext>` that
rustdoc interpreted as an unclosed HTML tag. Removing the closure and adding
code formatting fixed these diagnostics. The third bootstrap run completed
successfully. No runtime logic or failure assertions were weakened.

Each successful verification script ran exactly these checks:

```sh
cargo fmt --all -- --check
cargo check --lib --no-default-features --locked
cargo test --workspace --no-default-features --lib --tests --locked
cargo test --workspace --all-features --all-targets --locked
cargo test --workspace --all-features --doc --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
RUSTDOCFLAGS=" -D warnings" cargo doc --workspace --all-features --no-deps --locked
```

Per toolchain: 59 tests passed without default features; 67 tests passed with
all features; one doctest passed. Zero failures, ignored tests, or filtered tests.
The eight HTTP tests are feature-gated and therefore absent from the core run.
All three examples compiled. Clippy and rustdoc completed with warnings denied.
The HTTP process smoke received successful responses from `/live`, `/ready`, and
`/work`, then sent SIGTERM and observed exit status 0.

Lockfile SHA-256:

- Original graph: `5c36cce6076c625bf9981a44220d4063d1adb8ee8e8c4995b08aca59216ac14a`.
- Upgraded graph: `79865e6518881639a9b8776dbd73d58d924ff80f385faf0d5fe483b093e87c26`.

All nine direct dependency versions in the lockfile match the crates.io latest
stable releases queried on this date; [references](references.md#dependency-refresh-2026-09-07)
records the old/new requirements and upstream constraints. A second verbose
Cargo update made zero changes. The only older compatible-line patches are
`matchit` and `generic-array`, held by exact upstream requirements.

Not executed: live PostgreSQL, Runledger/Runlimit/harness integrations, SIGINT,
streaming/disconnect/load/race suites, other platforms, or hosted CI. No database
was provisioned; no configured test silently skipped missing services. SQLx
example compilation does not establish database behavior or commit certainty.
No publishing or deployment occurred. This directory has no `.git` repository,
so Cargo.lock and formatting changes could not be committed here.

## Ownership, HTTP telemetry, and budget hardening: 2026-09-08

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu. Both toolchain
versions are unchanged from the preceding table. These are local checks, not
hosted CI or production validation. No dependencies were added or updated.

Cargo.lock SHA-256 remains
`79865e6518881639a9b8776dbd73d58d924ff80f385faf0d5fe483b093e87c26`.

Executed successfully on the final Rust source (exit 0):

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run --locked --example process_owned
cargo run --locked --example operation_budget
python3 scripts/check_package.py
```

Each complete verification run passed core compilation, **91 core tests**, **105
all-feature tests**, **one doctest**, all five example compilation targets,
Clippy, and rustdoc with warnings denied. No failures, ignored tests, or filtered
tests occurred. Twelve HTTP tests and two HTTP telemetry tests are excluded from
the core-only run. The matrix was rerun after the final ownership and
submission-context changes; scoped agent checks are not substituted for it.

The HTTP subprocess checks require successful probes, normal work, a sanitized
application 500, a middleware-deadline 503, consistent custom error envelopes and
nonempty generated correlation IDs, exclusion of untrusted correlation input,
default INFO completion/status/latency logs, and clean SIGTERM/SIGINT exit.
The deadline process checks the 503 independently: it does not require a separate
handler to beat a 1 ms budget. Startup acknowledgement replaces the old extra
sleep for signal-listener initialization. These remain short loopback smoke tests.

The finite-work example completed admitted work and dependency cleanup after
losing its receipt and final driver owner. The budget example completed both
work and finalization. New regressions cover completion/drain classification,
startup acknowledgement/failure, cancelled request and shutdown waiters,
coordinator/cleanup drop, retained coordinator panic, bounded descendant admission,
typed task failures versus normal business denial, two 64-iteration multi-thread
races, scoped subscriber continuity, finalization reserves, and reproducible jitter.
Review repaired a normal-stop/drain classification edge and propagated scoped
subscribers across owned spawn boundaries; assertions were not weakened.

Static package inspection found 105 authored tests and no broken internal links
or lexical/TOML failures. Its fixed snapshot date, `docs/package-checks.json`, and
`SHA256SUMS` belong to the original archive; they do not attest the modified source.
The actual compiler/runtime evidence is the matrix above.

Not executed: live PostgreSQL, upstream shared-library integration, hosted CI,
cross-platform execution, sustained load, full keep-alive/slow-body/streaming/
disconnect coverage, or non-yielding child-process tests. No database or external
resource was provisioned. The checkout now has Git metadata but remains
uncommitted; no commit, push, publication, or deployment was performed.

## Root-cause fixes for cancellation and shutdown: 2026-09-08

The four review findings were traced independently and reconciled before source
changes. They require internal lifetime/state corrections, not a replacement
runtime or new downstream cleanup protocol.

| Finding | Root cause and classification | Implemented correction |
| --- | --- | --- |
| Completed tasks reported as abort targets, suppressing cleanup | Local state-model error: JoinSet membership means unobserved, not unfinished; expired timers and cooperative polling expose the difference. | Harvest ready results at bounded phase boundaries; retain AbortHandles and request abort only for unfinished tasks. |
| Never-polled driver drop leaves readiness waiting | Ownership-boundary defect: the future takes ownership at construction, but its emergency guard was installed only on first poll. | Capture the guard eagerly in `run_until`; keep factories/readiness lazy and ordinary `.await` usage unchanged. |
| Scoped drop diagnostics are lost/misrouted; HTTP abort panics | Cross-cutting lifetime integration gap: subscriber propagation covered polling but not destruction, including nested span fields. | One private, allocation-free dispatcher wrapper owns the complete inner future through poll and Drop, across operation, HTTP, cleanup, and task boundaries. |
| First budget-skipped hook has no warning | Local control-flow duplication: the already-popped hook bypassed the shared skip path and retained captures while dependencies dropped. | Check budget before popping; one path reports, logs, and drops skipped hooks in LIFO order. |

Before their respective source corrections, targeted regression runs reproduced
the false aborts, unpolled readiness hang, missing cleanup warning, wrong skipped
capture-drop order, lost scoped operation completion, and HTTP cross-registry
panic. Tests that protect borrowed/non-Send compatibility already passed on the
baseline. Additional tests cover completed errors/panics, mixed finished/running
work, native !Unpin capture destruction, critical/finite task abort, cleanup-driver
abort, and cleanup-hook timeout. These later cases are coverage, not a claim that
every added test was individually run against the old source.

Platform and toolchain versions are unchanged from the preceding evidence.
Cargo generated the updated lockfile after promoting existing transitive
`pin-project-lite` 0.2.17 to a direct dependency. No resolved version changed.
Removing only that dependency edge from the lockfile in a read-only comparison
reproduces its previous SHA-256. Current Cargo.lock SHA-256:
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.

Executed on the integrated final Rust source, all exit 0:

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run --locked --example process_owned
cargo run --locked --example operation_budget
python3 scripts/check_package.py
```

Each toolchain passed **107 core tests**, **122 all-feature tests**, **one doctest**,
all five example compilation targets, formatting, Clippy, and rustdoc with warnings
denied. Zero failures, ignored tests, or filtered tests. This adds 17 tests to the
previous 105-test suite. The three HTTP process checks passed; both finite-work
and finalization-budget examples completed successfully. Static inspection found
122 authored tests and no lexical/TOML or internal-link failures. The historical
archive checksum manifest and package-check JSON were not rewritten.

The original isolated global-subscriber/scoped-subscriber reproduction also
passed after the fix: operation and HTTP abort return cancelled, not panic; the
scoped subscriber receives dropped events and the global fallback stays empty.
Repository tests use independent per-thread ambient/scoped registries to exercise
the same mismatch without global test interference. Independent code review
checked whole-future destruction, pin projection, task ownership, and unchanged
borrowed/non-Send support.

No consumer wrapper or new public runtime type is required. Genuine aborts,
panics, and unjoined work still conservatively skip dependent finalizers. Runtime
death, non-yielding work, hidden descendants, live PostgreSQL/upstream integration,
full transport/load coverage, hosted CI, and other platforms remain outside this
evidence. No commit, push, publication, deployment, or external provisioning occurred.

## Clippy complexity and length limits: 2026-09-08

Root `clippy.toml` sets cognitive complexity to 20 and function length to 250.
Both packages inherit the explicitly enabled warning-level workspace lints;
verification's existing `-D warnings` makes violations fail the check.

The initial Clippy run exited 101, reporting complexity 34 in cleanup's
`close_inner`, 29 in the supervisor's `drive_until`, and 27 in task-result
`record`. No function-length violation was reported. Private helpers now handle
cleanup result classification/abort observation, task startup/result reporting,
and waiting for shutdown. No lint suppression or threshold increase was used.

Existing failure assertions remain intact. A new component-failure regression
asserts that the shutdown future's captured guard survives through dependency
cleanup and drops when the driver finishes. Existing tests also cover cleanup
errors/panics/timeouts, conservative finalizer skipping, readiness, cancellation,
and tracing during destruction.

Executed locally, all exit 0:

```sh
cargo fmt --all
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/check_package.py
git diff --check
```

On Rust 1.98.1 and 1.94.0, each complete verification run passed 108 core tests,
123 all-feature tests, and one doctest, with zero failures or ignored tests.
Formatting, Clippy, rustdoc, and all five example targets passed. All three HTTP
smoke scenarios passed. Exact compiler/Cargo versions and platform remain those
recorded above; Cargo.lock is unchanged, SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
Live PostgreSQL, hosted CI, other platforms, and the remaining operational
hardening work were not exercised. No commit or publication was performed.

## Jig adoption verification: 2026-09-08

Baseline: initial commit `3e64cb2`, followed by the owner's uncommitted Jig
adoption. Local platform and compiler/Cargo versions remain those recorded above:
Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu; Rust 1.98.1 and 1.94.0.
Jig runtime 0.3.0 accepts contract version 7. Cargo.lock is unchanged, SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.

The first `scripts/jig check` exited 1. Clippy rejected the shared example
`mod.rs` layout and the 107-line HTTP telemetry test under the current 100-line
limit. The default test gate also reproduced an empty-log failure in
`budget_exhaustion_reports_and_logs_each_skipped_hook_once_in_dependency_order`;
a separate repetition of `cargo test --locked --test cleanup` reproduced it on
run 9. Initial doctor failed its SQLx CLI probe, although the configured SQLx
check returned success. Neither result established any database behavior.

At the owner's request, Jig's inferred SQLx adapter, migration/metadata actions,
profile target, required command, generated CI jobs/environment, and associated
agent instructions were removed. The optional SQLx dependency and lifecycle
example remain. Both Jig test aliases now execute locked core, all-feature
all-target, and doctest commands. Shared example support moved to `support.rs`;
all five examples are explicitly declared to exclude that support module from
Cargo's executable discovery. HTTP completion assertions moved into a private
helper. No lint threshold, failure assertion, or library runtime logic changed.

The two cleanup observation tests and their fixture moved intact into
`tests/cleanup_observation.rs`, isolating their scoped subscribers from other
cleanup tests that register the same tracing callsites without subscribers.
The callsite-interference diagnosis and its limits are in [references](references.md).
After isolation, 100 fresh invocations of the following command passed, each
running 12 cleanup tests and two observation tests:

```sh
cargo test --locked --test cleanup --test cleanup_observation
```

Executed successfully on the final Rust source, all exit 0:

```sh
scripts/jig --json doctor
scripts/jig check
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig check agent-map
scripts/jig check agent-guides
scripts/jig check test
python3 scripts/check_package.py
git diff --check
```

Jig's five default targets passed: Clippy, formatting, tests, contract, and
file-budget. Each Rust verification matrix entry passed 108 core tests, 123
all-feature tests, one doctest, five example targets, and warning-denied Clippy
and rustdoc. No failures, ignored tests, or filtered tests occurred in these
final runs. All three loopback HTTP smoke scenarios passed.

Static package inspection initially rejected relative links in Jig's transient
adoption backup. It now excludes `.agent/.cache`, `.agent/runtime`, and
`.agent/tmp`, while retaining durable agent documents. A temporary fixture with
broken links in all three transient directories and `.agent/plans` verified
that only the durable-plan link is reported. Final repository inspection finds
123 authored tests and no lexical/TOML/internal-link failures. Historical archive
checksums and package-check JSON were not rewritten.

These results are local only. Hosted CI, live PostgreSQL, upstream application
integrations, other platforms, and the remaining operational hardening work
remain unverified. Repetition supports test isolation; it does not establish a
general tracing race or production concurrency guarantee. No further commit,
push, publication, deployment, or external provisioning occurred in this work.

## Jig footprint and installation policy audit: 2026-09-08

The repository selects the official Jig v0.3.0 release. The command below ran
in a disposable clone to review the generated changes before transferring the
release metadata and launcher/installer to this checkout:

```sh
scripts/jig update /tmp/batter-jig-fresh-e2rj43mj \
  --template https://github.com/bpcakes/jig-sh.git \
  --vcs-ref v0.3.0 --no-input --force
```

Jig resolved the tag to `8629700b92cd9ab8b09f8ff86de4fc1573469c83`, matching the
upstream tag's peeled commit. The generated launcher and installer were already
byte-identical to the release templates. Repository customizations were retained.
This is the accepted interim policy: `update --recopy` retains the revision;
ordinary `update` advances upstream. Persistent version-only enforcement remains
a future upstream feature, not a prerequisite for this change.

Removed the generated duplicate Rust and agent-map workflows, the optional
checkout helper, Swift/TypeScript plugin requests, frontend/SQLite ignore rules,
and unused proxy/package-manager settings. One Jig workflow checks installation,
contract, guides, and file budgets alongside the original Rust matrix. The
deprecated machine-local adoption receipt was removed from the change set;
append-only work records remain. Workspace bootstrap now uses `cargo fetch --locked`.
The launcher and installer retain their generated implementations.

Source packaging now includes `scripts/jig` with executable permissions, Git
attributes, and durable agent records. A temporary fixture executed the actual
packager and asserted those members were present while sentinel files in all
three transient Jig directories, the deprecated receipt, environment files,
build output, and local validation output were absent from ZIP and inventory.
Historical repository archive checksums were not regenerated.

The following passed locally on the unchanged Rust source and dependency graph:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build --example http_service --features axum --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
scripts/jig doctor
scripts/jig check contract
scripts/jig check agent-map
scripts/jig check agent-guides
scripts/jig work check --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig work evidence --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig work gates --plan-id plan_01M1ZXWMP2P73CEFVJENNG55TN
scripts/jig check test
python3 scripts/check_package.py
git diff --check HEAD
```

Each Rust matrix entry passed 108 core tests, 123 all-feature tests, and one
doctest; Clippy, formatting, documentation, and all three HTTP scenarios passed.
Cargo.lock still has SHA-256
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
The moved cleanup fixture and observation tests were compared byte-for-byte
against their original bodies. Both workflow YAML files and embedded Bash
syntax parsed successfully. A local JSON-RPC client launched the exact command
from `.mcp.json`, initialized server 0.3.0, discovered its four repository tools
plus 13 agent/work tools, and inspected exactly the seven configured targets.

Fresh installation of the selected release was checked with no repository Jig
cache or runtime override. Installation and doctor exited 0 without an initialized
vault; the runtime build disabled default features. The main checkout then reused
that binary with its original source stamp and passed doctor and MCP checks.
Local and hosted evidence remain separate: no hosted CI execution, live
PostgreSQL test, publication, deployment, or commit occurred.

## Jig review fixes: 2026-09-08

Addressed the three accepted review findings. The CI file-budget helper now
compares manual runs with `origin/master`; pull-request, push, and merge-group
events keep their exact bases and provenance. A missing exact base remains a
blocking error. The repository policy workflow caches only
`.git/jig-tools/*-runtime` with actions/cache v4.3.0, pinned to its verified SHA.
Its key includes OS, architecture, runtime profile, configuration/source pin,
contract, toolchain, launcher/installer, and workflow contents. No broad fallback
key restores an unrelated runtime.

Markdown plans now use the text merge driver. The override follows the Jig
managed block, so a template refresh that restores its union rule still leaves
conflicting plan edits visible. Append-only JSONL streams retain union merging.

Added seven standard-library Python regression tests and wired them into the
Jig policy workflow. They invoke the actual event helper and selected Jig runtime
in isolated Git repositories containing `origin/master` and no local `master`.
Each event accepts a small source file and rejects growth past the configured
budget. Missing event bases block, and an all-zero push-before identity succeeds.
Assertions inspect structured receipt findings rather than the abbreviated CLI
display. Merge tests exercise actual Git merges, including a regenerated union
rule before the plan override. The archive test executes the real packager and
checks ZIP members, inventory exclusions, and launcher executable permissions.

The cache test blocks Cargo with an executable sentinel: a cold cache fails and
records the attempted install; restoring the selected runtime and its source
stamp then passes without invoking Cargo. This establishes local restored-cache
behavior, not a hosted actions/cache hit. Workflow YAML, embedded shell syntax,
cache ordering, artifact paths, and key inputs were also inspected locally.

Executed successfully:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
bash -n scripts/check_file_budget.sh
scripts/jig doctor
scripts/jig check test
scripts/jig work check --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
scripts/jig work evidence --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
scripts/jig work gates --plan-id plan_01M1ZZF1CVV1XECCP6FKH71QF9
python3 scripts/check_package.py
git diff --check
```

The Python suite passes seven tests. Jig's locked test command passes 108 core
tests, 123 all-feature tests, and one doctest. Rust source and Cargo.lock are
unchanged by this follow-up, so the earlier two-toolchain and HTTP evidence
continues to apply. No hosted CI/cache operation, PostgreSQL provisioning,
commit, push, or publication occurred.

## Effect v4 analysis reconciliation: 2026-09-08

Baseline: `5c77593c6700de9b2e8d3cbc1d2acf4bbbb0b71d`. This change updates
documentation and append-only Jig work records. Runtime source, tests, manifests,
and Cargo.lock are unchanged. Lock SHA-256 remains
`7bcb7ba0d656d0f996ed7c91cc49409c2b75c92ba63749dcc81324ca2eddbca5`.
Local platform: Linux 7.0.11-76070011-generic, x86_64 GNU/Linux;
rustc 1.98.1 (48a229cea 2026-09-01), cargo 1.98.1 (797e8a9bc 2026-08-05).

Independent source reviews checked the runtime and HTTP/context/test boundaries.
Primary upstream release, migration, and boundary references were rechecked in
[references](references.md). Proposed capabilities remain labelled unimplemented;
HTTP handler panic propagation is source-inspected with no dedicated regression.

Executed successfully:

```sh
scripts/jig doctor
scripts/jig work check --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3
scripts/jig work evidence --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3 --json
scripts/jig work gates --plan-id plan_01M200DQFXPVDNMTBK1E40YPD3 --json
python3 scripts/check_package.py
git diff --check
```

The configured profile passed Clippy, formatting, locked tests, repository
contract, and file budget. Tests passed 108 core cases, 123 all-feature cases,
and one doctest; all five example targets compiled. No tests failed or were
ignored. The work gate reported passed with fresh target receipts. Static
package inspection found no lexical/TOML/internal-file-link failures.

The two-toolchain verification script and HTTP process smoke were not repeated
for these documentation changes; their earlier evidence remains historical.
No new PostgreSQL, durable integration, hosted CI, production, or performance
evidence was produced. No commit or publication was requested or performed.

## Virtual workspace and Axum extraction: 2026-09-08

Baseline: `5c77593c6700de9b2e8d3cbc1d2acf4bbbb0b71d`, with the preceding
documentation changes already present in the working tree. The workspace now
contains the `batter`, `batter-axum`, and `batter-test-support` libraries plus
the unpublished `batter-example-postgres-lifecycle` executable package. The
external PostgreSQL harness and downstream repositories were not modified.

Platform: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu. Toolchains:
rustc 1.98.1 (48a229cea 2026-09-01), Cargo 1.98.1 (797e8a9bc 2026-08-05);
rustc 1.94.0 (4a4ef493e 2026-03-02), Cargo 1.94.0 (85eff7c80 2026-01-15).
Every member retains Rust 1.94 and `publish = false`; this relocation makes
no claim of compatibility with an older compiler.

Cargo regenerated local workspace package entries during an offline check.
All external package versions, sources, and checksums match the preceding lock.
Current Cargo.lock SHA-256:
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

Executed successfully on the final Rust source (exit 0):

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
cargo run -p batter --example process_owned --locked
cargo run -p batter --example operation_budget --locked
cargo build -p batter-example-postgres-lifecycle --locked
cargo tree -p batter --edges normal --locked
python3 -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/check_package.py
scripts/jig check agent-guides
scripts/jig check agent-map
scripts/jig work check --plan-id plan_01M201112SDGVHTPKG68SW2165
scripts/jig work evidence --plan-id plan_01M201112SDGVHTPKG68SW2165 --json
scripts/jig work gates --plan-id plan_01M201112SDGVHTPKG68SW2165 --json
scripts/jig check test
```

Each full Rust verification run passed core-only compilation and **106 core
tests**, then **127 workspace tests** (106 core, 16 Axum, five generic support),
**two doctests**, every example/binary compilation target, Clippy and rustdoc
with warnings denied. No failures, ignored tests, or filtered tests occurred.
All 123 pre-refactor named tests remain present. Four additional tests cover
the public dispatch wrapper's capture/poll/abort/unpolled destruction behavior,
borrowed non-Send work, and HTTP budget validation after extraction. The core
and adapter still use the same private pin/drop implementation. HTTP readiness,
request budgets, failure rendering, and scoped destruction assertions were
preserved; the adapter's combined RequestPolicy was not redesigned.

The three HTTP process checks passed probes, work/deadline behavior, sanitized
error envelopes, request IDs, ordinary INFO observations and clean SIGTERM or
SIGINT termination. The process-owned and operation-budget examples exited
successfully. Cargo's normal dependency tree for `batter` contains neither
Axum nor SQLx. The SQLx executable builds as an independent package. The seven
Python tooling tests passed, including source-archive coverage for nested
members and exclusion of nested build/environment artifacts. All four package
guides and the agent map passed their checks.

The unchanged 848-line lifecycle source exceeds the existing 800-line budget
by 48 lines. Its exact rename was recorded in the Git index so Jig can compare
it to the original path; the policy thresholds were not changed. An initial
intent-to-add attempt was rejected as lacking stable index authority. A
subsequent check recognized unchanged legacy debt but rejected concurrent
documentation edits as worktree drift. With edits stopped, the complete Jig
profile passed Clippy, formatting, tests, contract and file-budget checks; its
required evidence gate reported passed with fresh receipts. The final
`scripts/jig check test` also passed. No semantic tests or policy limits were
relaxed.

Local logs are retained under ignored `.agent/tmp/workspace-refactor/`.
Live PostgreSQL, external library integrations, hosted CI, production, and
performance tests were not run. No commit, publication, or deployment occurred.

## Seeded cancellation and admission exploration: 2026-09-08

Owning task: `batter-953`. This extends Git baseline
`5db16f6f18fd918d3d3558a68843120bf9a13b79` with test-local scheduling exploration,
process containment and oracle challenges. No production source, public API,
dependency or lockfile change was needed. The library's current capacity behavior
was correct; the audit exposed a missing rejecting test.

Executed on Linux x86_64 with Python **3.12.3**, default Rust **1.98.1** and MSRV
**1.94.0**. Lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
The final source includes the readiness observation refinement: an acknowledged
component remains alive until Draining is checked, so a later Stopped state cannot
hide an invalid readiness revival. The initial matrix also passed; the matrix
and corpus repeats were rerun after this test refinement.

| Executed check | Outcome |
| --- | --- |
| `bash scripts/verify.sh` | PASS, exit 0 on 1.98.1; final run 37.349 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, exit 0; final run 39.043 s. |
| Full scheduling corpus, two workers, three fresh processes | PASS, 32 seeds / 64 workload lifecycles / 4,096 finite completions per invocation, all eight families 32 times each; commands took 6.276–6.331 s. |
| Full scheduling corpus, four workers, three fresh processes | PASS, identical required counts; commands took 6.277–6.278 s. |
| Capacity mutation challenge, each toolchain | PASS: six original controlled replays passed and six mutant replays failed the intended shared-capacity assertion with Rust exit 101. Neither compile failure nor watchdog timeout counted as rejection evidence. |
| Blocked-runtime negative control, direct recorded run | Expected FAIL, runner exit 1 / child SIGKILL (`-9`), reaped at 3.001 s; timer-armed and runtime-blocked records retained, no profile success. |
| Unjoined finite-work negative control, direct recorded run | Expected FAIL, runner exit 1 / child SIGKILL (`-9`), reaped at 3.012 s; unjoined report, pending receipt and skipped-cleanup assertions completed before runtime destruction blocked. |
| Early-exit, capture-overflow and controlled-schedule Cargo tests | PASS in both verification matrices. Exit zero without an oracle and overflow with an early success marker were rejected. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1 after the MSRV matrix. |
| `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` | PASS, exit 0; SIGTERM mode. |
| Same smoke command with `--signal SIGINT` | PASS, exit 0. |
| Same smoke command with `--deadline` | PASS, exit 0. |

Each complete Rust verification ran 162 foundation entries in the isolated core
pass, 183 workspace entries in the all-feature pass, and two doctests, plus core
compilation, Clippy and rustdoc with warnings denied. The eight scheduling entries
include two full subprocess profiles, one test of two capacity orderings, the
inert child entry and four process controls. There were no ignored tests or silent prerequisite
skips. The six explicit fresh-process repetitions account for **24,576** successful
finite workload completions; those counts exclude additional matrix runs and do
not stand in for the behavioral assertions.

The new test-owned ledger checks accepted, started and returned numeric task IDs,
including lost receipts, then reconciles the complete successful result set with
the report. Separate held gates check queued work and eight simultaneous roots
and descendants. Failure scenarios retain four concurrent task causes and two
cleanup causes by identity and name, including the exact shared typed cause;
scoped capture rejects synthetic error contents. Completion, cancellation,
readiness, scope expiry, task failure, forced closure, ownership drop and real
panic/abort cases use explicit orderings plus seeded yields. Existing deterministic
paused-time delayed-result regressions remain unchanged and ran in both matrices.

Reproduction commands, from the repository root:

```sh
cargo test -p batter --test scheduling --locked
# Resolve the executable using the JSON command in docs/testing.md, then:
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 4
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule capacity-after-drain
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule stuck --watchdog 3
python3 scripts/stress_scheduling.py --binary "$scheduling_binary" --workers 2 --seed 17 --schedule unjoined --watchdog 3
python3 scripts/check_scheduling_mutation.py --output-dir validation/local/scheduling-mutation-1.98.1
RUSTUP_TOOLCHAIN=1.94.0 python3 scripts/check_scheduling_mutation.py --output-dir validation/local/scheduling-mutation-1.94.0
```

The two negative-control commands intentionally return 1. Choose a new mutation
output directory on repetition; existing evidence is not overwritten. The final
full-matrix and repetition command ledger/logs are under
`validation/local/batter-953-final/`, with `controls.json` retaining negative-control
command/status/timing records. Mutation output directories retain the exact patch,
commands, build logs and outcome records. These ignored local artifacts are not
hosted-CI evidence. The structured Jig `verify` profile passed all five targets (Clippy, formatting,
tests, contract and file budget), and `work evidence` / `work gates` reported fresh
required evidence. Receipts are associated with `plan_01M20R9776KRBS3TBW7QGPDM38`.
The final backend command `scripts/jig check test` passed (exit 0, 35.2 s); its
receipt is retained alongside the work-profile evidence. Jig receipts are refreshed
after final documentation/tracker updates so the closing work snapshot has fresh
required gate evidence.

During implementation, initial lifetime/move compilation errors and two Clippy
complexity failures were repaired by correcting captures and separating report
assertions; no contract assertion was weakened. Inspection also distinguished
factory return from wrapper permit release: transient Full is handled as an inert
submission rejection within the family timeout. It is not a failed test retried
until green. No production race defect was found, so no production fix is claimed.

Limits: seeds reproduce scenario choices, not Tokio/OS scheduling. Fixed action
replay demonstrates the rejecting capacity oracle, not arbitrary scheduler replay.
Five-second family and 120-second profile timers rely on Tokio polling; the external
watchdog provides a separate termination deadline and bounded output, subject to
OS scheduling/reaping. A killed fixture does not demonstrate application cleanup.
macOS and hosted execution of the scheduling suite remain unverified; the macOS CI
command now includes it. These results do not establish exhaustive concurrency
correctness, detached-task termination, real HTTP load/streaming behavior or any
PostgreSQL/upstream integration.


## Scheduling subprocess ownership corrections: 2026-09-08

Owning task: `batter-953`, reopened after review. The scheduling workload and
library behavior above are unchanged; this follow-up corrects the test subprocess
protocol. The root cause was fragmented ownership of launch, process exit, pipe
completion and evidence reporting. Exceptions bypassed the evidence object, and
the mutation checker parsed console text to recover control data. The newline
omission was a local manifestation of that reporting boundary.

Before the fix, an inherited fixture flag changed inert discovery from exit 0 to
exit 101 on stdin EOF; a pipe held open could block before the emergency timer
was armed. A descendant retaining stdout caused the runner to raise after
5.106 seconds without its captured checkpoint or result summary. The overflow
fixture produced one summary marker but zero standalone summary lines. A timeout
in the build helper skipped its log write. Follow-up inspection and a live probe
also found that ignored SIGCHLD could make CPython report a child exiting 7 as
status 0 and accepted success. These were harness defects, not evidence of a
production admission/cancellation defect.

The corrected tools use explicit child arguments and a bounded PID-bound record,
a native startup deadline armed before input, one shared bounded process owner,
and structured outcomes consumed directly by the mutation checker. Direct-child
exit and pipe EOF remain separate facts. Partial output and failure metadata are
saved before classification. A non-default SIGCHLD disposition is rejected without
changing the caller's signal handler. The maximum external watchdog is now
140 seconds, followed by at most five seconds of cleanup observation, before the
149-second emergency backstop. This resolves the previous overlapping limits
without extending the task's external-deadline ceiling.

Executed on Linux x86_64 (kernel `7.0.11-76070011-generic`, glibc 2.39), Python
**3.12.3**, Rust **1.98.1** and **1.94.0**. Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
No production source, public API or dependency change accompanies the correction.
The complete command ledger, logs, environment and source hashes are in
`validation/local/batter-953-containment/` (ignored local evidence).

| Executed check | Outcome |
| --- | --- |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 18 controls, 7.528 s. |
| Same control command with `PYTHONOPTIMIZE=1` | PASS, all 18 controls, 8.279 s; optimization did not bypass rejection checks. |
| `python3 -m unittest discover -s scripts -p 'test_*.py' -v` | PASS, 20 tests, 73.207 s: 13 scheduling process controls plus seven Jig integration regressions. The five real Rust launch controls are supplied by Cargo/the explicit binary command. |
| Two-worker corpus, three fresh processes | PASS, each 32 seeds, 64 workload lifecycles, 4,096 completed tasks and all eight families 32 times; 6.277–6.327 s. |
| Four-worker corpus, three fresh processes | PASS, same counts; 6.226–6.276 s. |
| `bash scripts/verify.sh` | PASS on 1.98.1, 42.948 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 41.694 s. |
| `python3 scripts/check_scheduling_mutation.py --output-dir validation/local/batter-953-containment/mutation-1.98.1` | PASS, 10.637 s: six originals passed and six mutants failed the expected capacity assertion with status 101, complete output and no watchdog/overflow/I/O failure. |
| Same mutation command with `RUSTUP_TOOLCHAIN=1.94.0` and output directory `mutation-1.94.0` | PASS, identical rejecting evidence, 10.485 s. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1 after the MSRV matrix. |
| `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` | PASS, SIGTERM mode. |
| Same HTTP smoke command with `--signal SIGINT` and with `--deadline` | PASS in both modes. |

Each Rust verification executed 163 foundation entries in the isolated pass,
184 workspace entries in the all-feature pass, and two doctests: 349 successful
entries total, with zero failures or ignored tests. Both matrices also passed
formatting, isolated core compilation, Clippy and rustdoc with warnings denied.
The six explicit corpus repetitions account for 24,576 finite task completions;
these counts supplement the contract assertions rather than replacing them.

The process controls cover real timeout/output retention, EOF before process
exit, inherited and escaped pipe writers, overflow framing, spawn failure,
partial build/replay evidence, ignored child status, inert ambient flags,
missing/malformed launch records, parent EOF and the watchdog ceiling. Read/close
I/O failures and inability to observe reaping use narrow fault injection around
real children; no claim is made that an OS-unreapable process was created. The
escaped-pipe control explicitly reports incomplete EOF and separately terminates
its test-owned escaped fixture. Killing a group never proves arbitrary detached
work stopped or application cleanup ran.

An initial Jig contract edit accidentally included a profile entry when selecting
the two test actions, broadening the file-budget action's inputs. Contract checks
rejected it. The edit was corrected without weakening policy; native regeneration
in a disposable Git copy produced a contract exactly equal to the working one,
and `scripts/jig check contract` passed. Both Rust test targets now include their
four Python dependencies so Python-only changes invalidate their test evidence.
The required Jig verify profile passed all five targets (Clippy, formatting, tests,
contract and file budget). The final explicit `scripts/jig check test` passed
(exit 0, 40.234 s). Work receipts belong to
`plan_01M20XAG6XNY8GDQXMGJ4NJG7M`; command logs are retained with the other local
evidence.

Limits: macOS and hosted execution of these corrections remain unverified. Local
Python execution was 3.12.3; the documented 3.9 minimum follows the tool's language
features, not an executed lower-version matrix. A Python deadline cannot preempt
OS process creation or guarantee scheduling/reaping latency. Escaped pipe writers
produce explicit incomplete-output evidence; abrupt parent death has no general
application-finalization guarantee. Seeds still reproduce scenario generation,
not Tokio scheduling, and these tests do not establish exhaustive concurrency
correctness or live database/HTTP-load behavior.

## Scheduling group identity and timing margins: 2026-09-08

Owning task: `batter-953`, reopened for the next independent review's three
findings. The observation loop now defers reaping while output pipes remain open,
retaining the leader's numeric identity through the last group-signal decision.
Cached reaped status independently forbids signalling. Python control budgets
now allow three seconds for startup/work; the outer Rust watchdog assertion uses
three seconds of observation, five of cleanup and four of startup overhead.
Exit-status, checkpoint, overflow, failure classification and cleanup assertions
are preserved. No production source, public API or dependency changed.

Before the fix, `python3 -m unittest discover -s scripts -p
test_scheduling_process.py -k reaped_child -v` failed because `killpg` was called
after an explicit wait. The same command with `-k descendant_pipe` failed because
the leader already had cached exit status 0 at the group signal. After the fix,
both controls pass. A real one-second pre-checkpoint delay was killed at 0.303 s
under the former 0.3-second budget, retaining no checkpoint; the new delayed-start
control completes successfully. These probes do not claim an actual unrelated
process was killed or that macOS startup latency was measured.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
The exact command/environment ledger, logs and final source hashes are retained
under ignored `validation/local/batter-953-reap-order/`.

| Executed check | Outcome |
| --- | --- |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 20 controls, 27.635 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 20 controls, 28.517 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.223–6.274 s per run. |
| `bash scripts/verify.sh` | PASS, 1.98.1, 79.776 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 79.846 s. |
| Capacity mutation challenge on each toolchain | PASS, 10.494 / 10.485 s; each six originals passed and six mutants failed the expected capacity assertion with status 101. No timeout or build failure counted as oracle evidence. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1. |
| HTTP smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each full Rust verification executed 163 isolated foundation entries, 184
workspace entries and two doctests, with zero failures or ignored tests; both
also passed formatting, Clippy and rustdoc. The standalone Python suite now has
15 process controls; Cargo supplies the five additional Rust launch controls.
The six corpus repetitions account for 24,576 finite completions. Work receipts
belong to `plan_01M210XR2A7PWP4E540FHQK4GW`.
The required Jig profile passed Clippy, formatting, tests, contract and file-budget
checks; closing checks and their receipts are retained with the same work record.

Limits: macOS/hosted execution and Python 3.9 execution remain unverified. Finite
startup margins do not guarantee progress during OS suspension or starvation.
Pipe EOF, child reaping and application finalization remain separate facts; an
escaped writer still produces explicit incomplete-output evidence. The earlier
dated sections describe their respective pre-correction worktrees.

## Scheduling escalation oracle correction: 2026-09-08

Owning task: `batter-953`. Independent review identified a test assumption that
cooperative work always finishes inside a live 25 ms shutdown phase. The actual
contract allows a delayed worker to miss that deadline. The live oracle now
reconciles completion, observed abort and a recorded abort request racing with
completion or panic. It retains exact task counts, names, error categories and
conservative cleanup decisions. Production source, public APIs, dependencies,
phase allowances and seeded scenario choices are unchanged.

Two current-thread paused-clock regressions retain precise assertions. A real
thread sleep of 100 ms does not consume the paused 25 ms phase allowance; drain
and forced-cancellation modes must complete successfully with their exact phase
flags. A Tokio sleep of 100 ms crosses the shutdown deadlines and must yield the
named abort, retained JoinError, incomplete task count and skipped cleanup. Before
the oracle correction, the delayed-completion regression failed the former
`result.is_ok()` assertion. After correction both regressions pass. This is
negative evidence against the old test oracle, not a production defect.

The implementation also checked pinned Tokio 1.53.1 cancellation semantics:
normal completion or panic in the final poll can race with an abort request.
Any recorded request still requires skipped cleanup. The exact abort/completion
race is not claimed as a deterministically replayed interleaving. Existing
delayed-coordinator tests continue to prove already-finished work is not aborted.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Final command logs, environment and source hashes are retained under ignored
`validation/local/batter-953-escalation-oracle/`. The preliminary run is retained
separately under `batter-953-escalation-oracle-preliminary/`; it preceded the
additional abort/completion reconciliation and is not final-source evidence.

| Executed check | Outcome |
| --- | --- |
| `cargo test -p batter --test scheduling cooperative_escalation --locked -- --nocapture` | PASS, two paused-clock regressions, 0.314 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 20 controls, 27.712 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 20 controls, 28.487 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles and 4,096 finite completions; 6.276–6.583 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 11 Cargo entries, including the 20 Python controls, 31.637 s. The exact local Linux driver is retained as `contended-driver.py` with the command ledger. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 79.475 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 80.926 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.639 / 10.536 s; each six originals passed and six mutants failed the intended capacity assertion with status 101. |
| `cargo build -p batter-axum --example http_service --locked` | PASS, rebuilt with 1.98.1. |
| HTTP smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 165 isolated foundation entries, 186 workspace entries
and two doctests: 353 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. The six explicit corpora account
for 24,576 finite completions. The final source-hash audit matched the recorded
files. No failed build, timeout or preliminary run counted as mutation evidence.
Work receipts belong to `plan_01M2134C2JK2J3YBB8X4JFK4KR`.

Limits: macOS/hosted execution and Python 3.9 execution remain unverified. The
paused-clock tests prove controlled current-thread outcomes; the live corpora
and CPU-contention run do not establish exhaustive concurrency correctness or
an OS latency guarantee. Seeds reproduce generated choices, not Tokio scheduling.

The first Jig work check passed tests, Clippy, formatting and contract validation,
but file-budget scope capture rejected Git intent-to-add entries. Staging the
new Rust test files provided stable index contents; the standalone file-budget
check then passed. This changed index metadata, not source or budget policy.
Closing work-profile checks and the explicit backend check are recorded with the
same work receipts. The test files are staged; no commit or publication was made.

## Scheduling fixture lifetime correction: 2026-09-08

Owning task: `batter-953`. The remaining review findings exposed two harness
lifetime errors. The unjoined fixture reused the stuck fixture's three-second
watchdog despite needing to complete shutdown and pending-receipt checks before
its intentional hang. The escaped pipe writer became an orphan whose later
cleanup used a saved numeric process-group ID without retained child ownership.
No unrelated process was observed being signalled; that finding concerned lost
identity protection. Production source, public APIs and dependencies are unchanged.

The unjoined deadline now derives from four seconds of startup slack, the complete
five-second case allowance and three seconds of hang observation. A new replay
delays runtime creation by two seconds. With the former deadline, its Cargo parent
failed for missing report evidence after a watchdog kill at 3.001 seconds. The
fixed parent requires the startup checkpoint, reconciled unjoined report, pending
receipt, skipped cleanup, SIGKILL and absent success marker. The separate pipe
writer is now a direct child in its own session sharing a test-created pipe with
the observed child; its owner retains the handle through bounded kill/reap.
Controls cover incomplete EOF while that writer runs, cleanup after observation
raises and refusal to signal an already-reaped child. No PID-file cleanup remains.

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact command ledgers, logs, the old-deadline failure, environment, matrix and
contention drivers, and source hashes are retained under ignored
`validation/local/batter-953-fixture-lifetimes/`.

| Executed check | Outcome |
| --- | --- |
| `cargo test -p batter --test scheduling delayed_unjoined_start_preserves_report_evidence --locked -- --exact --nocapture` | PASS, 12.147 s; the same regression rejected the old deadline. |
| `cargo test -p batter --test scheduling cooperative_escalation --locked -- --nocapture` | PASS, both paused-clock regressions, 0.321 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 21 controls, 27.634 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 21 controls, 28.470 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.226–6.280 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 12 Cargo entries, including the 21 Python controls, 39.358 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 80.829 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 81.926 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.736 / 10.439 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 166 isolated foundation entries, 187 workspace entries
and two doctests: 355 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 20 recorded test/tool source
hashes matched during the matrix, at completion and in the current worktree.
Work-gate and completion receipts use `plan_01M216ACYSQ3P9R1GX02JPVAX0`;
the final backend command is `scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Fixed startup
allowances and the CPU-contention run do not establish OS latency guarantees.
The outside-group writer control proves retained test ownership and incomplete
EOF; it does not prove arbitrary detached descendants terminate. Stress remains
non-exhaustive, and watchdog kills do not establish application finalization.

## Descendant closure and partial capture correction: 2026-09-08

Owning task: `batter-953`. Two further confirmed review findings concerned the
scheduling harness. The closure scenario held an admitted ancestor until after
observing closure but unconditionally required successful completion, even when
its one-second cancellation allowance expired inside the five-second case budget.
A partial `Capture` constructor failure left its selector without an owner able
to close it; selector reference cycles deferred descriptor release until garbage
collection. Both corrections preserve production APIs and the dependency graph.

The closure scenario now reconciles each admitted receipt with named report
outcomes and completed counts, preserves the original closing-task error, and
checks actual dependent cleanup against recorded abort requests. Post-closure
admission still must return `Closed`. Two paused-clock regressions exercise both
force- and task-failure closure with prompt observation and with a 1.25-second
delay. The latter rejected the previous implementation when cancellation dropped
the held ancestor's release receiver; the fixed test requires its named abort,
no unjoined work and skipped dependent cleanup. The prompt case still requires
no abort request. The ordinary corpus keeps its generated choices unchanged.

Capture construction now explicitly closes the selector on any setup exception,
including interruption, before propagating it to the process owner. The regression
uses real selectors and children, failing the second nonblocking setup or second
registration with either `OSError(EIO)` or `KeyboardInterrupt`. All four subcases
rejected the previous code because the retained selector's descriptor stayed open.
All now require descriptor closure before garbage collection, child SIGKILL,
reaping and preservation of the error category. Focused commands were:

```sh
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k partial_capture -v
cargo test -p batter --test scheduling descendant_closure_ --locked -- --nocapture
```

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact commands, before/after failures and successes, full logs, environment,
contention/matrix drivers and source hashes are retained under ignored
`validation/local/batter-953-closure-capture/`.

| Executed check | Outcome |
| --- | --- |
| Focused Python partial-construction control | PASS, all four subcases, 0.086 s; all four failed before the fix. |
| `cargo test -p batter --test scheduling descendant_closure_ --locked -- --nocapture` | PASS, both regressions and both closure causes, 0.115 s. |
| Existing paused-clock escalation regressions | PASS, both, 0.315 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 22 controls, 27.726 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 22 controls, 28.526 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.225–6.325 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 14 Cargo entries including the 22 Python controls, 39.288 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 81.575 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 83.090 s. |
| Capacity mutation challenge on both toolchains | PASS, 10.689 / 11.489 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 168 isolated foundation entries, 189 workspace entries
and two doctests: 359 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 24 mutation replay records
retained expected exit status, complete EOF and reaping, with no watchdog,
overflow or observation errors. All 20 recorded test/tool source hashes matched
before and after the matrix and in the current worktree. Work-gate and completion
receipts use `plan_01M218ETNJTVXHS4TS8S885QX5`; the final backend command is
`scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Paused time checks
logical ordering, not live scheduler latency. The live oracle still rejects
unexpected errors, panics, unjoined work and inconsistent cleanup. Corpus success
is non-exhaustive, and watchdog kills do not establish application finalization.

Final repository checks also passed: all five Jig work targets (Clippy, formatting,
tests, contract and file budget), followed by `scripts/jig check test` with all
359 entries passing. The source hashes still matched after that final command.

## Scoped SIGINT ownership correction: 2026-09-08

Owning task: `batter-953`. The cleanup-interruption defect was a structural
boundary error in the private Python process owner. Catching `KeyboardInterrupt`
around observation left settlement and descriptor release exposed, and another
catch around one wait would still leave arbitrary Python instructions exposed to
signal exceptions. Python's signal guidance was researched before changing the
implementation; the source references are recorded in [references](references.md).
The stale standalone counts were a separate documentation omission caused by
repeating the same number in multiple instructions.

The private helper now owns a scoped, non-raising SIGINT handler from before child
acquisition through resource release. The handler records a stop request; normal
observation checks it, while settlement keeps its original absolute deadline.
After release, the default handler is restored and interrupted outcomes remain
failed evidence. A non-main thread or non-default SIGINT owner is rejected before
launch, alongside the existing SIGCHLD ownership check. All existing call sites
are standalone main-thread tools. Descriptor release is protected by nested
finally blocks, and simultaneous selector/stream close errors both survive.
Production Rust source, APIs and dependencies are unchanged.

The new cleanup regression uses real, directly owned children and a real retained
selector. An outside-group writer keeps a pipe open while one or three real
SIGINTs are delivered after cleanup's selector polls. Both subcases rejected the
old code with `SIGINT escaped cleanup without an outcome`. They now require the
original cleanup deadline, retained checkpoint, interrupted error category,
SIGKILL/reaping of the observed child, incomplete EOF and closed descriptors.
Other controls deliver SIGINT during setup, observation and close, verify handler
restoration after callback failure, and reject incompatible handlers/threads
without starting a child. The two stale numeric instructions now refer to process
controls without duplicating their count; executable discovery and the coverage
summary retain the actual current totals.

Focused commands:

```sh
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k sigint -v
python3 -m unittest discover -s scripts -p test_scheduling_process.py -k close_failure -v
```

Executed on Linux x86_64, Python **3.12.3**, Rust **1.98.1** and **1.94.0**.
Cargo.lock SHA-256 remains
`3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.
Exact command ledgers, before/after failure evidence, full logs, environment,
contention/matrix drivers and source hashes are retained under ignored
`validation/local/batter-953-sigint-ownership/`.

| Executed check | Outcome |
| --- | --- |
| Focused SIGINT controls | PASS, four controls covering real signals and ownership/restoration, 1.017 s. Both cleanup subcases failed before the fix. |
| Simultaneous selector/stream close-error control | PASS, both error categories and descriptor release retained, 0.165 s. |
| Existing paused-clock escalation controls | PASS, both, 0.316 s. |
| `python3 scripts/test_scheduling_process.py --binary "$scheduling_binary"` | PASS, 26 controls, 28.629 s. |
| Same controls with `PYTHONOPTIMIZE=1` | PASS, 26 controls, 31.291 s. |
| Full corpus, three fresh processes per worker count | PASS, each 32 seeds, 64 workload lifecycles, 4,096 finite completions and all eight families 32 times; 6.276–7.229 s per run. |
| Scheduling target with two-CPU affinity and two competing CPU workers | PASS, all 14 Cargo entries including the 26 Python controls, 40.247 s. |
| `bash scripts/verify.sh` | PASS, Rust 1.98.1, 81.528 s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS, 80.477 s. |
| Capacity mutation challenge on both toolchains | PASS, 11.638 / 11.034 s; each six originals passed and six mutants failed the intended capacity assertion. |
| Rebuilt HTTP example and smoke in default, `--signal SIGINT` and `--deadline` modes | PASS, all three. |

Each Rust matrix executed 168 isolated foundation entries, 189 workspace entries
and two doctests: 359 successful entries, zero failures and zero ignored tests.
Both also passed formatting, Clippy and rustdoc. All 24 mutation replay records
retained their expected statuses, complete EOF and reaping, with no watchdog,
overflow or observation errors. All 20 recorded test/tool source hashes matched
before and after the matrix and in the current worktree. Work-gate and completion
receipts use `plan_01M21AB7YKJ8HDRC8SVB1S1ETQ`; the final backend command is
`scripts/jig check test`.

Limits: macOS/hosted and Python 3.9 execution remain unverified. Signal handling
belongs to these synchronous private tools; it is not a new library handler or
support for arbitrary custom signal owners, interpreter threads, asynchronous
exceptions or process death. Process creation, OS scheduling and non-yielding
work can still exceed cooperative deadlines. Stress remains non-exhaustive, and
watchdog kills do not establish application finalization.


## Inherited SIGINT policy and unjoined elapsed checks: 2026-09-08

The review found a local signal-policy design error: the private process owner
classified every non-Python-default SIGINT disposition as competing ownership.
A non-interactive background Cargo run inherited SIG_IGN and failed
`two_worker_scheduling_corpus` before child creation with
`unsupported-sigint-owner`, despite the foreground scheduling target passing.
The missing elapsed assertion on the unjoined fixtures was a separate test
omission: eventual SIGKILL could satisfy their oracle even after fallback to the
140-second full-profile watchdog.

Primary Python 3.12 documentation, CPython 3.12.3 initialization and GNU Bash
signal documentation were checked before implementation; see
[references](references.md#inherited-sigint-policy-in-scheduling-tools-2026-09-08).
The correction distinguishes inherited policy from a competing callback. Ignored
SIGINT stays ignored in both owner and child. Python-default and SIG_DFL retain
the scoped non-raising recorder, and every path restores the exact prior
disposition after resource release. The main-thread/default-SIGCHLD prerequisites,
custom/unknown-handler rejection, output evidence, child identity and cleanup
deadlines remain enforced.

Three new standalone controls exercise background signal inheritance/delivery,
all compatible dispositions over success/spawn failure/callback failure/timeout,
and real SIGINT under SIG_DFL. Before the helper changed, these produced ten
failed subcases; afterward all seven focused SIGINT controls passed. A fourth
new control runs the real Rust replay through a background shell. Existing tests
that intentionally send active SIGINT explicitly establish their signal policy
and restore the launcher's disposition afterward. Both unjoined parent tests now
require elapsed time from twelve to less than twenty-one seconds; a synthetic
140-second result must fail the same elapsed oracle without sleeping for it.

The final matrix ran on Linux x86_64 (kernel `7.0.11-76070011-generic`, glibc
2.39), Python 3.12.3 and Rust 1.98.1/1.94.0. All 23 recorded commands succeeded.
Evidence is retained under ignored
`validation/local/batter-953-inherited-sigint/`: exact argument vectors,
environment, wall-clock durations, before/after logs, the matrix driver, source
hashes, corpus output and mutation records. Cargo.lock remained unchanged at
SHA-256 `3f7596122e7c093dc8af791c6c33bd05b04422ef53206055042103c1e4036d0b`.

| Executed check | Observed result |
| --- | --- |
| Focused SIGINT controls (`python3 -m unittest discover -s scripts -p test_scheduling_process.py -k sigint -v`) | Seven passed; 1.367 seconds. |
| Synthetic unjoined elapsed oracle (`cargo test -p batter --test scheduling unjoined_elapsed_oracle --locked`) | Expected late-duration rejection passed; 0.114 seconds. |
| Complete scheduling target (`cargo test -p batter --test scheduling --locked`) in foreground and through a `/bin/sh` background command with explicit child-status propagation | Fifteen tests passed in each context. Background command took 29.134 seconds; the foreground test target reported 29.01 seconds. |
| Existing controlled escalation regressions | Both passed; 0.315 seconds. |
| Python controls with the built Rust fixture, normal/background/`PYTHONOPTIMIZE=1` | Thirty passed in each mode; command durations 29.029 / 29.029 / 30.292 seconds. |
| Three fresh full corpora for each of two/four workers | All six passed in 6.276–6.582 seconds each; every run recorded 64 workload cycles, 4,096 completions and all eight families at 32 cases each. |
| Scheduling target restricted to two CPUs with two competing CPU workers | Fifteen passed; 41.773 seconds, including both unjoined elapsed assertions. |
| `bash scripts/verify.sh` on Rust 1.98.1 | 361 test executions passed, zero failed/ignored; formatting, Clippy and rustdoc passed; 88.960 seconds. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | 361 test executions passed, zero failed/ignored; formatting, Clippy and rustdoc passed; 86.801 seconds. |
| Capacity mutation challenge on each toolchain | Each passed six originals and rejected six mutants with status 101 at the intended shared-capacity assertion; 12.994 / 12.091 seconds. |
| Rebuilt HTTP example and default, SIGINT, deadline smoke modes | All three passed with the expected telemetry/response checks and exit zero. |

All 20 implementation/test/script hashes matched before and after the matrix and
when audited afterward. All 24 mutation replay records showed reaped children,
complete EOF and no watchdog, overflow or I/O errors. Mutant logs contained the
intended capacity assertion and no completed-profile marker. Corpus outcomes
were also audited for counts, markers, successful status and complete observation.

The closing Jig gate/backend logs and evidence are retained beside the matrix
logs; plan `plan_01M21CBBH20FMCC3Q6FP0PQDBP` connects `work check`, the final
`scripts/jig check test`, `work evidence`, `work gates` and `work finish` receipts.
These are local results. macOS/hosted scheduling and Python 3.9 execution remain
unverified, and no PostgreSQL service was provisioned. The corpus is not exhaustive;
process creation and OS scheduling remain outside hard preemption guarantees.
No production API, dependency graph or publication change accompanies this fix.

### Test inventory correction (2026-09-08)

The review follow-up for `batter-953` corrected stale discovery counts in
`docs/testing.md`. On Linux with Rust 1.98.1,
`cargo test --workspace --all-features --locked -- --list` succeeded and listed
190 test entries: 169 foundation, 16 Axum adapter and five test-support entries,
plus two foundation doctests. The scheduling executable listed fifteen entries.
Python 3.12.3 `unittest.TestLoader().loadTestsFromTestCase(...).countTestCases()`
reported 24 `ProcessTests` and six `FixtureLaunchTests` in
`scripts/test_scheduling_process.py`, matching the documented 30 controls.
The other-Unix total of 188 is derived by excluding the two Linux-only probes;
this correction adds no macOS execution evidence. These were discovery checks,
not another runtime test pass; the implementation and preceding matrix evidence
are unchanged. The implemented-status row now links to the test inventory
instead of duplicating its Python count.

## Optional PostgreSQL connection disposition: 2026-09-09

Bead `batter-7r3.2`; plan `plan_01M23KF4J87DZTKKK59Z1TYHK7`, baseline
`7601916addb42dff51d59c544f3f97e0b2993961`. Added unpublished `batter-sqlx`
0.1.0 with Rust 1.94 minimum and adopted its probe/close registration in the
native lifecycle example. The lease API preserves native PgConnection and
Transaction use; rustdoc compiles a transaction composition. Setup database
creation/reconciliation was optional and is not included.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01) and
1.94.0 (`4a4ef493e`, 2026-03-02). The existing local server reported PostgreSQL
18.6 (`Ubuntu 18.6-1.pgdg24.04+2`). Tests used the current Unix role through
`/var/run/postgresql`, database `postgres`, with native SQLx 0.9.0. No database,
persistent schema, or external harness was provisioned or modified. Test-owned
session advisory locks and a transaction-local temporary table were released.

| Command / evidence | Executed outcome |
| --- | --- |
| `bash scripts/verify.sh` | Passed: locked minimal core, full workspace runtime tests, doctests, Clippy, formatting and rustdoc. Four new offline adapter tests passed; ten live cases remained ignored in the ordinary all-feature/all-target gate. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Same final matrix passed on the retained minimum toolchain. |
| `bash scripts/test_sqlx_live.sh` with external `DATABASE_URL`, on each toolchain | Ten named cases passed, zero ignored/filtered. Inventory is checked before execution. The explicit invocation without DATABASE_URL failed, as required. |
| `cargo build -p batter-axum --example http_service --locked`, on each toolchain; `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` | All ten profile invocations passed. |
| `cargo test -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked tests::live:: -- --ignored`, on each toolchain | Three live pool/startup/shutdown cases passed on each toolchain. |
| `cargo build -p batter-example-postgres-lifecycle --bin postgres_lifecycle --locked`; `python3 scripts/smoke_postgres.py --binary target/debug/postgres_lifecycle`, default and `--signal SIGINT`, on each toolchain | Four process smokes passed with readiness, pool cleanup and exit 0. |
| `cargo metadata --locked --format-version 1`; `cargo tree -p batter-sqlx --duplicates`; normal/build graph traversal for core and generic test support | One SQLx 0.9.0 graph; neither core nor test support reaches SQLx. Parsed old/new Cargo.lock external package entries are identical. Only local package/dependency entries changed, generated by Cargo. |
| `scripts/jig work check --plan-id plan_01M23KF4J87DZTKKK59Z1TYHK7 --json`, followed by `work evidence` and `work gates` | All five targets passed. Recording this evidence changed Jig input digests, so the final profile is refreshed after documentation and Bead closure; final receipt identities are retained in the plan's Jig evidence. |

Logs are retained locally under `.agent/tmp/batter-7r3.2/`: final two-toolchain
`verify-*` and `examples-*` logs, live discovery/results and Cargo metadata. The
live cases used two-second local replacement/close budgets with an independent
elapsed-time assertion, and separate five-second server-state observations.
They first witnessed each backend waiting on its advisory lock. Cancellation,
deadline, returned concrete application error, panic and outer task abortion
released client accounting. Independent work and pool close completed while the
retired backends remained blocked. After unlock, every recorded backend
disappeared before closing the observer. Three repeated interruptions retained
three distinct blocked sessions despite a one-slot pool. This is evidence of
residual remote sessions, not a server-side concurrency bound.

Successful query, probe and acknowledged commit/rollback reused one backend.
Division-by-zero and deferred-constraint commit failures retained SQLSTATE
`22012` and `23505`, respectively, and retired their connections. Fixed adapter
diagnostics omit native messages; the original cause remains inspectable.
The native ordinary-return control reproduced pool acquisition timeout and a
pending pool close before unlock. Its first cleanup attempted server-disappearance
observation before re-driving the cancelled pool-close future; that failed and
was corrected to explicitly close its still-accounted pool after unlock. No
semantic assertion was removed. The initial connection attempt without an
explicit SQLx username failed peer authentication; the configured Unix role
resolved it. Final live runs passed on both toolchains.

The panic case intentionally invokes the default panic hook before the test
observes the task panic; no hook suppression is claimed. No macOS execution,
hosted CI, TLS execution, non-yielding SQL client preemption, remote cancellation
acknowledgement or ambiguous-commit rollback guarantee is established by this
change. PostgreSQL fixture provisioning and the broader compatibility bundle
remain separate. No commit, push, publication or deployment occurred.

## Owned startup and service composition: 2026-09-09

Bead `batter-7r3.3`; plan `plan_01M23N75G41J82N9CGAYB7TZQQ`, baseline
`7601916addb42dff51d59c544f3f97e0b2993961`. The existing uncommitted SQLx adapter
work was preserved. Added inert owned startup, validated cleanup reservations,
retained application/panic/cleanup reports, immediate Unix signal installation,
and shared shutdown-result checking. HTTP and PostgreSQL compositions now use
those helpers. PostgreSQL acquisition runs inside the owner; its reserved slot
registers native pool closure synchronously after acquisition succeeds.

Executed on Linux x86_64, rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02), using the existing PostgreSQL 18.6 fixture and SQLx
0.9.0. Tokio remains 1.53.1; the core now enables its Unix signal feature.

| Command / evidence | Outcome |
| --- | --- |
| `bash scripts/verify.sh` | Passed minimal core, workspace tests, doctests, formatting, Clippy and rustdoc. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Same complete matrix passed on the retained minimum. |
| `cargo test -p batter --test startup --locked` and final matrices | Fifteen tests pass: LIFO concrete failures, rejected reservation with prior cleanup, cancelled borrowed waiters, dropped owner before first poll/during initialization/during cleanup, unclaimed running handoff, inert construction, deadline, factory/poll/destructor panic, and delayed readiness acknowledgement. |
| Injected Unix signal installation failure, run in core unit tests | Original IO cause and registered cleanup survive; diagnostics omit the cause. |
| `cargo build -p batter-axum --example http_service --locked` followed by `scripts/smoke_http.py` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, `--warn-filter --deadline` | All five profiles passed on each toolchain. |
| Live PostgreSQL example `tests::live:: -- --ignored`, executable SIGTERM/SIGINT smokes, and `bash scripts/test_sqlx_live.sh` | Three example cases, two process smokes and ten adapter cases passed on each toolchain. |

Local logs are under `.agent/tmp/batter-7r3.3/` (`verify-*`, `examples-*`, focused
startup/example logs). The example script lists every build/smoke/live invocation;
all database checks use the existing Unix socket fixture, without provisioning.
Initial example tests hit sandbox loopback denial, then passed with socket
permission. A preliminary command used the directory name as a Cargo package
name; using `batter-example-postgres-lifecycle` resolved that selection error.
No semantic assertion was relaxed. The native panic controls intentionally leave
the default hook active. New macOS/hosted execution remains unverified.

Barrier tests deliberately pause between acquisition and registration to prove
borrowed waiter cancellation leaves the initializer alive. Production code must
register without that suspension; dropped ownership or deadline cancellation
before registration retains only native resource destruction semantics. Signals
are installed during initialization and consumed when the critical driver starts.
Shutdown projection reuses the existing report's success predicate, preserving
critical failures, forced abort, skipped cleanup and unjoined-work outcomes.
No runtime-death, non-yielding preemption, async Drop or detached-child guarantee
is added. Final Jig `work check`, `work evidence`, `work gates` and `work finish`
results are retained in the plan's append-only evidence after metadata updates.
The standalone file-budget check passed with size warnings for `lifecycle.rs`
(766 lines) and the new startup regression file (513 lines); no budget was waived.
No commit, push, publication or deployment was performed.

## Owned dependency health: 2026-09-09

Bead `batter-7r3.1`; plan `plan_01M23QHHMZHDQB2RGHP6A6SR7D`, baseline
`7601916addb42dff51d59c544f3f97e0b2993961`. Added the foundation health monitor,
validated timing policy and read-only observation API. The HTTP example combines
cached dependency health with lifecycle readiness. Its dependency probe is an
explicit simulation; this change establishes no real database-health result.
The completed uncommitted SQLx and startup work was preserved.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01), Cargo 1.98.1,
and rustc 1.94.0 (`4a4ef493e`, 2026-03-02), Cargo 1.94.0. Tokio remains 1.53.1;
this change adds no dependency or feature. No new macOS/hosted execution is claimed.

| Command / evidence | Executed outcome |
| --- | --- |
| `cargo test -p batter --test health --locked` | Fifteen focused tests passed. |
| `cargo test -p batter-axum --example http_service --locked` | Six tests passed, including controlled health transitions and existing real loopback lifecycle/telemetry checks. Initial sandbox loopback denial was resolved by running with socket permission. |
| `bash scripts/verify.sh` on 1.98.1 | Core/minimal and workspace runtime tests, doctests, formatting and Clippy passed. The final rustdoc step rejected an unescaped `Arc<E>` comment. |
| `RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc --workspace --all-features --no-deps --locked` on 1.98.1 | Passed after enclosing that comment's type in backticks. This was the only source change after the preceding runtime checks; those checks were not repeated solely for the comment correction. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Entire script passed, including runtime tests, doctests, formatting, Clippy and rustdoc. |
| `cargo build -p batter-axum --example http_service --locked`; `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, `--warn-filter --deadline`, on each toolchain | All ten process smoke invocations passed, preserving readiness, request envelopes/correlation, telemetry filtering and signal exit 0. |

Local evidence is under `.agent/tmp/batter-7r3.1/`: `health-final.log`,
`http-final.log`, `verify-*`, `rustdoc-1.98.1.log`, `http-*` and the exact smoke
command list in `http.sh`. Final Jig `work check`, `work evidence`, `work gates`
and `work finish` records belong to the plan after metadata updates; its fresh
api:test receipt supplies the final backend gate on the default 1.98.1 toolchain.

The four-worker read test releases 2,000 reader tasks together and independently
counts one probe and an unchanged completion timestamp. Paused-clock tests park
the owner across exact expiration and a long scheduling delay, then observe only
one next attempt. Independent active/drop counters show whole acquisition-plus-
query timeout, no overlap, and destruction before another probe. Error, timeout
and success transitions preserve a concrete non-Clone cause and keep the process
Ready until explicit drain; Debug is tested against a cause that panics if formatted.

Separate guards record active-probe destruction, monitor return and dependency
cleanup in that order. Drain during delay and a probe destructor requesting drain
before publication prevent late success. Unpolled owner loss, already-draining
start, forced-cancellation state and outer task abort invalidate readers. A panic
remains an unsuccessful critical-task report. Bounded 20 ms factory/poll delays
exceed a 1 ms probe budget and cannot publish Healthy; this demonstrates late
classification, not preemption. A replaced error's destructor allows a concurrent
read to complete before the destructor returns, with its test thread explicitly
joined. No test assertion was relaxed to obtain these outcomes.

Snapshots are historical; only fresh reads recompute expiration. Publication uses
a short private mutex, without a lock-free/wait-free claim. No detached-task,
remote cancellation, async Drop, non-yielding preemption, runtime-death or actual
database-health guarantee follows. No commit, push, publication or deployment
was performed.
