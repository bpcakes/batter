# Validation evidence

Snapshot date: 2026-09-07.

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

See [BTR-001](roadmap.md#btr-001-validate-the-source-snapshot) for the initial gate;
its original Rust 1.88 baseline precedes the SQLx-driven minimum-version increase.

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
