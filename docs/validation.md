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
