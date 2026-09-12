# Validation evidence

Latest evidence: 2026-09-12. Earlier sections retain their historical scope.

## Retained completion publication core (DU-001), 2026-09-12

Command, startup and process completion observers now delegate their identical
snapshot-before-wait loop to one private helper. Each public observer retains its
existing type, outcome, cancellation ownership and runtime-loss diagnostic.
Managed native settlement observation remains separate because publisher loss
returns its last snapshot instead of panicking. Direct unit controls prove that
an already-published value is ready on the helper's first poll, a pending waiter
observes later publication, and closure before publication returns an error.

Local macOS arm64 acceptance from Git baseline
`6f25e6476efd614b68cf884d0271707cbb36c6e8` and Cargo.lock SHA-256
`13d5a89554eb34020d89b63855d00a64307b6e1711394e2603455f9850bc248e`
passed on rustc/Cargo 1.98.1 (`48a229cea` / `797e8a9b`) and 1.94.0
(`4a4ef493e` / `85eff7c8`):

| Check | Executed result |
| --- | --- |
| `cargo test -p batter completion --locked` | PASS: the three new helper controls plus existing filtered completion cases. |
| `bash scripts/verify.sh` on each toolchain | PASS: complete offline/workspace matrix, observer runtime-loss and retained-identity tests, doctests, formatting, strict Clippy and warning-denied rustdoc. |
| HTTP example build plus five `scripts/smoke_http.py` profiles on each toolchain | PASS: ten executions covering SIGTERM, SIGINT, deadline, WARN filtering and WARN-filtered deadline. |
| `scripts/jig check --profile verify` | PASS: all five targets, including `api:test`, formatting, strict Clippy, contract v9 and file-budget checks; run plan `run-plan_sha256:eefa68fdeff182b30c5b4df7c670de4f377db7630dac24ac09d5dce87acd6899`. |

No dependency or lockfile change was needed. Live PostgreSQL was not required
and was not executed. No new Linux, hosted, publication or deployment evidence
is claimed.

## Constrained owned startup registration (batter-lp2.1), 2026-09-12

`Startup::scoped` now projects a private-field `ProtectedStartupScope` over the
existing owned coordinator. Its sealed `Registration` authority can register
ordinary and managed components and reserve cleanup, but has no supervisor,
context, process-start, replacement or cleanup-extraction operation. Returned
application failures enter `InitializationError<E>` at the poll boundary before
initializer destruction. Exact legacy startup and Axum/Runledger adapter
signatures remain available; the new `_in` entry points share their native bodies.

Linux x86_64 acceptance from baseline
`39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4` passed on rustc/Cargo 1.98.1
(`48a229cea` / `797e8a9b`) and 1.94.0 (`4a4ef493e` / `85eff7c8`):

| Check | Executed result |
| --- | --- |
| `bash scripts/verify.sh` on each toolchain | PASS: complete offline/workspace matrix, doctests including authority compile-fail controls, formatting, strict Clippy and warning-denied rustdoc. |
| Protected foundation, Axum and Runledger cases | PASS: channel request `21 -> 42`, component join before resource release, retained application error plus destruction panic, duplicate/invalid ordinary and managed rejection without factory invocation, actual loopback HTTP request, and prepared native two-loop initialization/settlement without PostgreSQL. |
| HTTP example build plus five `scripts/smoke_http.py` profiles on each toolchain | PASS: ten executions covering SIGTERM, SIGINT, deadline, WARN filtering and WARN-filtered deadline. |
| `scripts/jig work check --plan-id plan_01M2AZ3VXCTVCDKGHHAFKZR1X7` | PASS: fresh `api:test` receipt `receipt_01M2B08EHZ6KBX47RCEJR54KCY`, plus fmt, Clippy, contract-v9 and file-budget targets. |

The authority controls compile positive public imports before rejecting supervisor
access, process start, forged target implementation and a registration view moved
to a `'static` task. Native adapter tests retain typed function-pointer and
`DerefMut` calls to the old concrete signatures. No dependency or lockfile change
was needed. Existing managed pending-settlement and uncertain-cleanup tests passed
unchanged in both full matrices. Live PostgreSQL was not required for this task
and was not executed. No new macOS or hosted evidence is claimed.

## Non-blocking retained panic inspection (batter-ai6), 2026-09-12

The retained panic diagnostic boundary now exposes
`PanicPayload::try_inspect`. It returns the typed `PanicPayloadBusy` result
instead of waiting when inspection is already in progress. The startup
failure-path regression performs recursive inspection while the outer callback
holds access and observes the contention result; it also proves that a callback
panic does not prevent a later typed downcast. Existing startup, finite-command
and managed-component payload checks use the non-blocking contract. The public
rustdoc example compiles.

Linux x86_64 acceptance passed from Git baseline
`034ce0085220044dcf5f3561b00a0bfce96a801f`:

| Check | Executed result |
| --- | --- |
| `bash scripts/verify.sh` on pinned Rust 1.98.1 | PASS: formatting, complete offline matrix including failure contracts and doctests, workspace strict Clippy and warning-denied rustdoc. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete verification on the exact minimum toolchain. |
| HTTP example build and all five `scripts/smoke_http.py` profiles | PASS on both toolchains: SIGTERM, SIGINT, deadline, WARN-filtered and WARN-filtered deadline. |
| `scripts/jig check --profile verify` | PASS on pinned Rust 1.98.1: all five targets, including exhaustive `api:test` receipt `receipt_01M2AQ2VB76ZFHXK3K7M8D03BC`, contract v9 and file-budget checks. |

The file-budget target reported the existing startup integration test above its
warning threshold but no policy error; the profile passed without a waiver.
Live PostgreSQL endpoints were not required for this foundation-only change and
remain unset. No new macOS or hosted execution is claimed. No staging, commit,
push, publication or deployment was performed.

## Pushed native source pin (batter-vly), 2026-09-12

The user reported that the native changes were committed and pushed. The clean
local Runledger checkout and `git ls-remote origin HEAD` both identified
`d57ec6be61e9f00ccce373b19ca356cafe98f206`. Root and archived-consumer manifests
now select that revision for core/postgres/runtime 0.12.0 without path overrides.
Cargo regenerated both lockfiles; only the three native `source` entries changed,
with no registry version or dependency-edge changes. Initial targeted
`cargo update -p runledger-runtime` could not identify the removed path package;
ordinary Cargo metadata resolution then generated the new Git entries successfully.

A temporary copy of the current files had no sibling native checkout. A negative
control restoring a sibling override failed with the expected missing manifest;
`cargo metadata --offline --locked --format-version 1` then passed for both final
workspaces, and all six native entries identified the exact Git revision. Offline
resolution used the Git source already fetched from the remote, not an empty Cargo
cache. Logs: `/tmp/batter-vly-isolated-negative.log`,
`/tmp/batter-vly-isolated-root.log`, `/tmp/batter-vly-isolated-consumer.log`.

Linux x86_64 acceptance passed on exact Rust 1.98.1 and 1.94.0:

| Check | Executed result |
| --- | --- |
| `bash scripts/verify.sh` with each `RUSTUP_TOOLCHAIN` | PASS: formatting, complete offline matrix including failure contracts and doctests, workspace strict Clippy and warning-denied rustdoc. |
| HTTP example build and all five `scripts/smoke_http.py` profiles | PASS on both: SIGTERM, SIGINT, deadline, WARN-filtered and WARN-filtered deadline. |
| `CARGO_TARGET_DIR=target cargo test --locked --manifest-path docs/evidence/batter-gi4/Cargo.toml` | PASS on both: all five original and two modification oracles unchanged. |

Final `scripts/jig work check --plan-id plan_01M2ADTDEGCN0TP433NWHXFZEQ`
passed all five targets on Rust 1.98.1. The first invocation returned nonzero
because this evidence document changed during validation, although every target
passed; a stable refresh reused the passing receipts and succeeded. Evidence and
gates report fresh passes, including `api:test` receipt
`receipt_01M2AE92EX64Y1WNKVYHQFG99R`. Commands and outcomes are retained in
`/tmp/batter-vly-acceptance.py`, `/tmp/batter-vly-acceptance.log` and
`/tmp/batter-vly-results.json`; Jig outputs are `/tmp/batter-vly-jig-check.log`
and `/tmp/batter-vly-jig-refresh.log`.
Live PostgreSQL endpoints are unset, so no new live, macOS or hosted execution is
claimed. Earlier paired-source evidence remains historical; the current pin does
not retroactively change those records. No application source/tests or migrations
were changed, and no staging, commit or push was performed.

## Completion classification follow-up (batter-vef), 2026-09-12

The user authorized three localized review repairs after the earlier review
closure: snapshot command interruption at the final poll before destruction,
distinguish native stop/settlement before initialization from process drain, and
replace the stale reference README worker description. Managed readiness still
requires a live context after initializer destruction; that separate contract
has not changed. The added `ManagedInitialization::Stopped` variant requires an
additional arm in exhaustive consumer matches.

Before the behavior fixes, two command destruction controls failed (unexpected
Cancelled/DeadlineExceeded instead of no late interruption), and two managed
native-exit controls failed (Draining instead of Stopped). Final-poll interruption
and process-drain controls passed. After repair, all 20 command and 22 managed
tests pass on Linux x86_64 with Rust 1.98.1. Each command scenario covers both
a success value and an original application error, plus completed cleanup.
Logs: `/tmp/batter-vef-red.log`, `/tmp/batter-vef-managed-red.log`, and
`/tmp/batter-vef-focused.log`.

The first focused strict Clippy check rejected added complexity in the managed
driver. Extracting the pending-initialization classification into a private
helper restored the configured limit; no lint or semantic test was relaxed.

Final Linux x86_64 verification passed:

| Check | Executed result |
| --- | --- |
| `bash scripts/verify.sh` on pinned Rust 1.98.1 | PASS: formatting, complete offline matrix including doctests, workspace Clippy, warning-denied rustdoc. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete verification on the exact minimum toolchain. |
| HTTP example build and all five `scripts/smoke_http.py` profiles | PASS on both toolchains: SIGTERM, SIGINT, deadline, WARN-filtered, WARN-filtered deadline. |
| `scripts/jig work check --plan-id plan_01M2A27S15RXHQDATXZ3B5QGWR` | PASS on pinned 1.98.1: all five required targets. Evidence and gates report fresh passes, including `api:test` receipt `receipt_01M2A2QYY9F7CTHW7A3F31V0JP`. |

Acceptance commands/logs: `/tmp/batter-vef-acceptance.py`,
`/tmp/batter-vef-acceptance.log`, and the fourteen per-command logs named there.
Jig output is `/tmp/batter-vef-jig-check.log`. Before/after source guards for both
runs are `/tmp/batter-vef-acceptance-guard.json` and
`/tmp/batter-vef-jig-guard.json`; both report unchanged executable inputs:
Batter `273cd639177a066c85887285914cdefb68c0aa61b6884ee24d214f018257121d`,
native `e5ed58464d235f4c436caabfe5706f2dce8f7fce8e1e96d746ec35d59ca48d3c`.
The guard includes native source independently of root-only Jig receipts and
excludes documentation/tracker metadata; its implementation remains
`/tmp/batter-gi4-source-guard.py`. The sibling Runledger development dependency
remains unchanged and outside this fix scope. No new macOS, live PostgreSQL,
hosted execution or independent reviewer pass is claimed. No staging or commits.

## Final review and Jig closure (batter-gi4), 2026-09-12

The complete working-tree review/fix loop converged at its configured medium
severity threshold: Batter used three repair rounds and four full review passes;
the native sibling used five repair rounds and six full passes. Claude, Codex and
Cursor completed both final scopes, with Cursor at xhigh effort. Parent captures
were complete and matched: Batter
`44cbd1eaf8173dc497a6b7a73ea4a93061e9d4a6d125a9d7490c2a58a808faa5`,
native `696e4278939fa626943809e3433afa07ad446328fb804a3fa70293e65d075559`.
Batter's trusted HEAD excludes `.agent`; native has no exclusions. A temporary
review index exposed new files without modifying the original real index.

After review, `scripts/jig check --profile verify --plan-id
plan_01M294ERDA6M993R8MK890RQQ4` passed all five targets on pinned Rust 1.98.1,
including the required `api:test` receipt. Both executable-input identities stayed
unchanged at the values in the following section. Logs:
`/tmp/batter-gi4-final-jig-verify.log` and
`/tmp/batter-gi4-final-jig-verify-guard.json`. This supplements the already executed
exact two-toolchain, rustdoc, HTTP and live acceptance; it does not replace it.

All verified medium-or-higher findings are repaired. Low findings remain explicit:
conservative cleanup refusal for aborted never-polled wrappers, top-level Requested
labeling for managed failures, one stale reference README sentence, native report
extensibility and planned-abort diagnostics. The full finding dispositions and
coverage qualifications are in `/tmp/batter-gi4-review-ledger.json` and
`/tmp/batter-gi4-review-handoff.md`. No claim of zero findings or population-wide
agent convergence is made. Native 1.98.1 baseline Clippy diagnostics and the
unpublished sibling dependency remain the documented acceptance boundaries.

Final ExecPlan, validation and tracker closeout follows the frozen review and
changes metadata only. The superseded witness repair tasks do not claim their
original implementation was delivered. Provider task `batter-8q8.2` remains open;
application readiness remains unapproved. No staging, commits or publication.

## Reference settings construction (batter-gi4), 2026-09-12

Static review found that `WorkerSettings::builder()` and its documented consumer
still started a native supervisor directly and dropped it without observing
settlement. The obsolete convenience API is removed. The existing configuration
consumer now checks its native settings, transfers prepared work to the adapter,
observes initialization and awaits successful managed shutdown. Its focused
cleared-environment execution passes on Rust 1.98.1. The original independent
`InitializationFailure` redaction/source assertions were restored verbatim from
the preserved index and pass. Focused logs:
`/tmp/batter-gi4-root-round3-worker.log` and
`/tmp/batter-gi4-root-round3-redaction.log`.

The refreshed paired matrix passes all sixteen commands on exact Rust 1.98.1
and 1.94.0: full `verify.sh`, HTTP build and five smokes, all 58 live-runner entries
and the separate maintenance-session probe. This matrix has no failed command;
the earlier runtime-loss fixture incident remains recorded below. The restored
redaction regression runs on both toolchains. The seven unchanged archived
consumer oracles also pass on 1.98.1.

All source guards are unchanged. Batter input identity:
`831e334ba7791fe6dcb1070099d18d19494f35b180406b9e6b6798539e282b6e`;
unchanged native identity:
`e5ed58464d235f4c436caabfe5706f2dce8f7fce8e1e96d746ec35d59ca48d3c`.
Logs/manifests: `/tmp/batter-gi4-round6-root-acceptance-{toolchain}-{0..7}.{log,json}`
and `/tmp/batter-gi4-round6-consumer-archive.{log,json}`. Native acceptance remains
481 library tests, 22 doctests, five PostgreSQL and ten supervisor integrations on
both toolchains, with strict Clippy on 1.94.0; native source has not changed since
that matrix. Final review and Jig closure are separate from these executions.

## Final descendant harvest (batter-gi4), 2026-09-12

An external scheduling probe reproduced a finished join reported as unjoined when
another collector held its ready notification. The same probe passes after the
bounded shutdown-boundary harvest; both logs are
`/tmp/batter-gi4-harvest-probe-{red,green}.log`. Two repository regressions cover
all boundary consumers, delayed notification delivery, record uniqueness and
retention of the original shared panic. All eight registry controls pass.

Exact Rust 1.94.0 and 1.98.1 pass 481 native library tests, 22 doctests, five
PostgreSQL integration cases and ten supervisor integration cases. Strict native
all-target Clippy passes on 1.94.0. The documented 1.98.1 baseline Clippy limitation
remains. No semantic assertion or lint was relaxed; the first Clippy run required
idiomatic iterator filtering and explicit test failure messages.

All nine paired source guards are unchanged. Native identity:
`e5ed58464d235f4c436caabfe5706f2dce8f7fce8e1e96d746ec35d59ca48d3c`;
Batter identity: `148e5dbf90a05e72ce6eb735cd380870c0d013d7b59b61600bbb551aed1fa217`.
Logs/manifests: `/tmp/batter-gi4-native-round5-acceptance-{toolchain}-{index}.{log,json}`.

The paired Batter matrix passes on both exact toolchains: full `verify.sh`, HTTP
build and all five smokes, all 58 reference entries plus the separate session
replacement probe. All seven archived consumer oracles pass again on 1.98.1.
Logs: `/tmp/batter-gi4-round5-root-acceptance-{toolchain}-{0..7}.{log,json}` and
`/tmp/batter-gi4-round5-consumer-archive.{log,json}`.

The first 1.94.0 live sweep passed 57/58: the unchanged
`fixture_runtime_loss_exposes_native_drop` failed. PostgreSQL's server log showed
its `DROP DATABASE ... WITH (FORCE)` cancelled by a statement timeout. The same
fixture has an earlier intermittent failure recorded below. It passed alone in
0.44 seconds, then the complete 58-case suite and separate session probe passed on
unchanged inputs. No assertion or timeout was altered. The successful full rerun
is `/tmp/batter-gi4-round5-root-live-msrv-recheck.{log,json}`; the isolated result is
`/tmp/batter-gi4-round5-runtime-loss-recheck.{log,json}`. The initial failed attempt
remains in the matrix log. These passes do not establish that the existing fixture
is free from intermittent environmental failures. Final review and delivery
closure remain tracked separately.

## Failure publication and transaction classification (batter-gi4), 2026-09-12

A repeated managed stop callback panic was caught but absent from the frozen
process report while native settlement remained pending. The new regression first
failed with zero retained failures; it passes after publication moved into the
shared catch boundary. Sixteen foundation library tests, nineteen managed contracts
and twenty-nine process ownership cases pass in the focused Rust 1.98.1 run.
The late native result is released and joined before asserting the frozen report.

The refreshed paired matrix passes on exact Rust 1.98.1 and 1.94.0: full
`bash scripts/verify.sh`, HTTP build plus all five smokes, all 58 reference entries
and the separate maintenance-session replacement probe. All sixteen guards retain
the identities below. The example target executes all four CLI diagnostic tests
on both versions, including an actual native closed-pool BEGIN failure and combined
work/cleanup failures. Logs/manifests:
`/tmp/batter-gi4-round4-root-acceptance-{toolchain}-{0..7}.{log,json}`.

Two native regressions also failed before repair: BEGIN/COMMIT exposed ordinary
query/business codes. Both now require fixed internal transaction-control code,
kind, category and client message while preserving SQLx source and durable readback.
Exact Rust 1.94.0 and 1.98.1 pass 479 native library tests, 22 doctests, five PostgreSQL
integration cases and ten supervisor cases. Strict all-target Clippy passes on
1.94.0; the separately reproduced 1.98.1 baseline limitation still applies.
All nine command source guards are unchanged. Native input identity:
`efcc361a8607d6497c659c3b846cf5051132517c09a6681ec3e1e4e705822108`.
Logs/manifests: `/tmp/batter-gi4-native-round4-acceptance-{toolchain}-{index}.{log,json}`.
RED/GREEN: `/tmp/batter-gi4-native-round4-classification-{red,green}.log`.

The strengthened live owner-drop and production-root cases pass on Rust 1.98.1:
drop alone initiates stop, and a pending delivery job retains its full row and zero
attempts across production initialization, health sampling and shutdown. The actual
CLI emits structured wrong-target facts, repeated absent-definition success and
three usage refusals on both exact toolchains. The archived consumer again passes all seven unchanged
oracles. Their paired source guards retain the native identity above and Batter
`148e5dbf90a05e72ce6eb735cd380870c0d013d7b59b61600bbb551aed1fa217`.
Logs/manifests: `/tmp/batter-gi4-root-round2-live-targeted.{log,json}`,
`/tmp/batter-gi4-cli-smoke-round2.{log,json}`,
`/tmp/batter-gi4-cli-smoke-round2-msrv.{log,json}` (after its guarded 1.94.0 build),
and `/tmp/batter-gi4-round4-consumer-archive.{log,json}`. Review and delivery closure
remain recorded by the owning Beads; these checks alone do not establish convergence.

## Shared callback-owner repair (batter-gi4), 2026-09-12

Four native worker-observer regressions first reproduced fatal destructor panics
on timeout/abort, before/during shutdown. Observers and dead-letter hooks now share
a private polling/destruction owner. Eight running/terminal cases pass, together
with dual-panic and timeout-plus-destruction hook controls. The existing reaped
case now requires both interruption facts. Existing public supervised-worker tests
continue to require fatal main job-task destruction failures.

Exact Rust 1.94.0 and 1.98.1 pass 116 core, 111 PostgreSQL and 252 runtime library
tests, 21 doctests, five PostgreSQL integration cases and ten supervisor cases.
Strict native all-target Clippy passes on 1.94.0. No semantic test or lint was
relaxed: one setup helper was extracted after Clippy rejected complexity 21/20.
The native 1.98.1 baseline lint limitation documented below remains separate.

All nine paired source guards are unchanged, with native input identity
`a40eabb8aea6d203724b2561a84a34a10dc2f4bfdefa5b3cae9cf00cbaa27046`.
Logs/manifests: `/tmp/batter-gi4-native-round3-acceptance-{toolchain}-{index}.{log,json}`.
The failure reproduction is `/tmp/batter-gi4-native-round3-observer-red.log`.

Paired Batter acceptance also passes again on both exact toolchains: full
`bash scripts/verify.sh`, HTTP build and all five process smokes, all 58 reference
runner entries, and the separate maintenance-session replacement probe against
PostgreSQL 18.6. All sixteen command guards retain the native identity above and
Batter identity `2cba17441121afb34c122f871ecbcc67606b3a8fbc633f55f20cec19518448f5`.
Logs/manifests: `/tmp/batter-gi4-round3-root-acceptance-{toolchain}-{0..7}.{log,json}`.
The archived consumer passes all seven unchanged checks on Rust 1.98.1 against
these same inputs (`/tmp/batter-gi4-round3-consumer-archive.{log,json}`). This is
revalidation of the earlier exercise, not another independent agent evaluation.

Full independent review and delivery closure are tracked by the owning Beads;
these tests alone do not establish convergence. The registry microbenchmark and
its limitations are recorded in [references](references.md#native-registry-overhead-probe-2026-09-12).

## Refreshed paired acceptance (batter-gi4), 2026-09-12

After both native repair rounds, `bash scripts/verify.sh` passes again on exact
Rust 1.98.1 and 1.94.0, including workspace/core tests, hostile-environment checks,
doctests, strict Clippy, formatting and warning-denied rustdoc. Both toolchains
pass the HTTP build plus all five process smokes, all 58 reference runner entries
and the separate library maintenance-session replacement probe. PostgreSQL is
18.6. All sixteen paired command guards report unchanged inputs.

Code-input identities for these executed commands:

- Batter: `2cba17441121afb34c122f871ecbcc67606b3a8fbc633f55f20cec19518448f5`.
- Native sibling: `c4a027e01515a495ff1c9b6bd954299f7f947ae2c148189252325ddad98b6a58`.

Logs/manifests: `/tmp/batter-gi4-round2-root-acceptance-{toolchain}-{0..7}.{log,json}`.
The archived fresh-agent consumer also passes all seven unchanged oracles again
on Rust 1.98.1 against these inputs (`/tmp/batter-gi4-round2-consumer-archive.log`
and its paired `.json` source guard). This is revalidation of the original bounded
exercise, not a new independent agent evaluation.

These direct checks refresh the earlier acceptance evidence; final Jig receipts
and review closure belong to the owning Bead and plan. They do not imply a hosted
CI/macOS execution or a standalone published native dependency graph.

## Native observation follow-up (batter-gi4), 2026-09-12

The second native review identified gaps in active descendant observation and in
caught observer-destruction failures. Targeted regressions failed before repair:
a driver needed an external descendant waiter to notice a panic; caught reaped-
observer destruction approved cleanup; one completion repolled 128 unrelated joins
256 times; and an exhausted cooperative budget hid 256 already-finished tasks.
All four now pass. A first fixture attempt did not compile because it constructed
an upstream non-exhaustive record; it was corrected to exercise the native observer
task directly before collecting the reported failing assertions.

Both real supervised-worker destructor-panic cases pass through the public legacy
entrypoints. Additional controls establish bounded handle-requested shutdown and
that a joined configuration failure permits cleanup while failing overall success.
The Shared-notifier lifetime/deadlock regression continues to pass with the per-
entry ready queue. The [research decisions](references.md#remaining-native-design-questions-2026-09-12)
record why historical interruption still prevents cleanup and why no extra
transaction/pooler abstraction was introduced.

Exact Rust 1.94.0 and 1.98.1 pass 116 core, 111 PostgreSQL and 242 runtime library
tests, 21 doctests, four cancellation/isolation cases plus one scope case, and ten
supervisor integration cases. Native strict all-target Clippy passes on 1.94.0;
the separately reproduced 1.98.1 baseline lint limitation remains unchanged.
Logs/manifests: `/tmp/batter-gi4-native-round2-acceptance-{toolchain}-{index}.{log,json}`.
Every command's paired source guard is unchanged. Native code-input identity:
`c4a027e01515a495ff1c9b6bd954299f7f947ae2c148189252325ddad98b6a58`.
Independent full review and delivery closure are tracked by `batter-gi4` and the
native owning Bead; the test evidence above does not itself claim convergence.

## Native review repair (batter-gi4), 2026-09-12

The native all-reviewer pass confirmed that intentional observer abortion at the
terminal cap incorrectly stopped the supervisor. Its regression failed before
repair. Expected pre-stop abortion is now counted without stopping processing or
retaining an unbounded history. Uncaught descendant failures remain fatal; older
Result methods retain the original shared join error and begin bounded drain on
internal stop. A public-API regression reproduced their earlier false success.

Six PostgreSQL-backed regressions independently kept an application child alive
after handler timeout, panic or lease loss, both before and during shutdown. Every
case originally approved cooperative cleanup despite that live child. The repaired
native boundary records these interruptions independently of successful job-task
joins. All six pass; a seventh control returns an ordinary business failure after
joining its child and continues to permit cleanup.

Owned native cancellation now explicitly establishes READ COMMITTED. Its trigger
oracle failed with a SERIALIZABLE session default before the fix and passes after,
while the session default remains SERIALIZABLE. A separately executed missing-job
classification followed by actual backend termination preserves both its original
`job.not_found` classification and the subsequent rollback failure. Current error
guidance and upgrade notes describe the enum/exhaustive-match and SQLx-source changes.

Initial repaired Linux runs passed 235 runtime library tests, eight supervisor
integration tests, four cancellation failure/isolation probes and one scope test.
The added bounded internal-stop test brings runtime library coverage to 236; the
missing-job rollback test brings PostgreSQL library coverage to 111. Subsequent
source identities and matrix results are recorded in the newer sections above.
Logs: `/tmp/batter-gi4-native-round1-*-red.log`,
`/tmp/batter-gi4-native-round1-runtime-green.log`, and
`/tmp/batter-gi4-native-round1-acceptance-{toolchain}-{index}.{log,json}`.

## Managed initialization review repair (batter-gi4), 2026-09-12

The first comprehensive review reproduced a managed readiness error: successful
initializer polling or destruction could cancel/expire the startup context after
the driver's pre-poll check. Three public regressions failed at the readiness
assertion before the fix; the simultaneous original-error control already passed.
The driver now rechecks the startup context after destruction and before publishing
successful initialization. Returned errors remain intact, native settlement is
still driven and acquired dependencies are finalized after cooperative settlement.

On Linux Rust 1.98.1, all nineteen managed contracts, sixteen command contracts and
seven scoped-ownership cases pass, as does focused strict Clippy. Logs are
`/tmp/batter-gi4-round1-{red,green,clippy}.log`. The matrix/receipt identities below
precede this repair and require final refresh. Current documentation also removes
obsolete production witness/reconciliation instructions and distinguishes the
historical inventory from the current 58-entry runner. Review closure is tracked
by the owning Bead.

## Complete matrices and fresh consumer acceptance (batter-gi4), 2026-09-12

On Linux, `bash scripts/verify.sh` passes with Rust 1.98.1 and exact Rust 1.94.0:
locked core/workspace tests, hostile-environment configuration, doctests, Clippy,
formatting and warning-denied workspace rustdoc. Both toolchains also pass all
five HTTP smokes (SIGTERM, SIGINT, deadline, WARN filtering and filtered deadline).
Both pass the expanded 58-entry reference runner (56 database probes and two
offline entries) plus the separate library maintenance-session replacement case.
The new production-root case requires actual `/live` 200, persistent `/ready` 503,
zero control-job rows and clean SIGTERM with owned cleanup completed.

The forced `scripts/jig check --profile verify --plan-id
plan_01M294ERDA6M993R8MK890RQQ4` passes all five required targets, including
`api:test`, contract and file-budget. This is receipt evidence in addition to the
direct two-toolchain commands. Documentation updates afterward may require cheap
whole-repository policy refresh; they do not change Rust inputs.

A separate source guard hashes both repositories' tracked/untracked non-document
inputs before and after each HTTP/live command, the minimum-toolchain matrix and
the forced Jig profile. All identities were unchanged. It excludes Markdown,
`docs/`, `.agent/` and `.beads/`; those exclusions are not a general assertion that
arbitrary files under those paths can never affect verification. Current runtime
inputs remain in the declared crate/example/script roots. The archived consumer
is separately compiled below. Root-only Jig freshness cannot observe native
sibling changes and is not used as evidence of that sibling's identity.

Recorded code-input SHA-256 identities:

- Batter: `f5cc5251c8226045d12a6ad3cc9cf8cb67ed98bdb5db0804a1629d5ed3fa93bc`.
- Native sibling: `a0fed51ea9313517a954e7c72a6a51365fc8e02ac413a50e3b3f6b0a4fff8eaf`.

Logs and full before/after manifests: `/tmp/batter-gi4-acceptance-1.98.1-0` through
`-6` and `/tmp/batter-gi4-acceptance-1.94.0-0` through `-7`, with `.log` and `.json`
suffixes. The default workspace log is `/tmp/batter-gi4-workspace-1.98.1.log`;
Jig has `/tmp/batter-gi4-jig-profile.log` and `.json`.

Native verification additionally passes 116 core, 110 PostgreSQL and 233 runtime
library tests on both exact toolchains, plus twelve PostgreSQL and nine runtime
doctests, three cancellation-failure regressions and the authorization-scope case.
Native all-target strict Clippy passes on 1.94.0, its pinned maintenance line.
The supplementary 1.98.1 native Clippy run fails 18 `result_large_err` diagnostics.
An untouched archive of native HEAD `50620137e36aab2333213fa8d8e51a095484e6eb`
reproduces the same 18 diagnostics and 160-byte maximum variant. This is an
existing error-representation/newer-lint compatibility gap, not introduced by the
lifecycle or cancellation changes. No lint was suppressed and no public error
representation was changed to hide it. See the [primary-source explanation](references.md#native-newer-clippy-baseline-2026-09-12).
Logs: `/tmp/batter-gi4-native-acceptance-{1.94.0,1.98.1}-{0,1,2,3}.log`;
baseline `/tmp/batter-gi4-native-baseline-clippy.log`. Each changed-source native
command also has an unchanged before/after source manifest.

The actual production executable and retirement CLI also pass a separate smoke:
initialize one fresh disposable database, observe liveness, SIGTERM and await a
successful process exit, independently observe database-session absence, reject
wrong identity, then execute retirement twice successfully with zero controls.
The disposable database is dropped afterward. The CLI reports only its database
scope and explicitly does not claim deployment completion.
Log/manifest: `/tmp/batter-gi4-cli-smoke.log` and `.json`.

The [fresh consumer exercise](evidence/batter-gi4/README.md) passed first-attempt
integration and modification: five initial behavioral checks, then two new total
budget checks with all five originals preserved. The agent received public
guidance and fixed interfaces, without private rationale or oracles. The archive
also compiled and passed separately. This limited exercise is not a general
convergence claim. All-reviewer repair and final scope verification were still
pending at this stage.

## Offline retirement and native teardown repair (batter-gi4), 2026-09-12

The reference now has an owned offline retirement command and separate read-only
readback. It verifies expected identity, rejects hidden/other sessions and prepared
transactions, refuses physical connection replacement, disables the native legacy
definition and preserves historical state. Native cancellation retains SQLx causes
and secondary rollback failure. A returned primary failure is published before
optional readback; actual lost COMMIT acknowledgement does not authorize replay.

Executed on Linux Rust 1.98.1 against PostgreSQL 18.6:

- Seven retirement cases pass: preservation/old catalog disable, wrong identity,
  hidden sessions, late enqueue, prepared enqueue, commit rejection followed by
  cancelled readback, and a transport fault discarding an actual COMMIT response.
  A subsequent full-row assertion also confirms an unrelated job type remains
  unchanged; its focused preservation case passed after the full-suite run.
- The separate required library probe terminates its exact maintenance backend and
  confirms replacement fails with retained acquisition error and completed cleanup.
- The complete `scripts/test_reference_live.sh` runner passes 57 reference entries
  (55 live and two offline) plus that library probe. An isolated repeat passed
  after an earlier passing run overlapped native tests on the primary server.
  Log: `/tmp/batter-gi4-reference-full-live-isolated.log`.
- Adapter/reference all-target Clippy, six reference doctests, warning-denied
  reference rustdoc and six Python inventory controls pass. These are direct
  executions, not final Jig receipts or the complete two-toolchain matrix.

The first complete run failed the callback cleanup probe. Its Notify-based fixture
could resume only after the non-yielding callback's safety timeout, missing the
intended scenario. Independent timer/atomic observation now proves the callback
is still held at stop and report boundaries. The stronger probe then exposed a
real native Shared-notifier/registry destruction deadlock. GDB confirmed the lock
cycle; the stuck process was terminated and counted as failed. The native waker
now owns only a separate notification signal. The new ownership regression failed
before the fix and passes after it; cleanup assertions were preserved and strengthened.
The repaired callback probe completes in under one second with cleanup skipped.

Native Rust 1.94.1 verification: all 233 runtime library tests pass; three new
cancellation regressions and the existing authorization-scope test pass; the
rollback error/source unit test, twelve PostgreSQL doctests and PostgreSQL/runtime
all-target Clippy pass. Native Error::RollbackFailure is an added public enum
variant; coordinated exhaustive-match changes are required before upstream release.
No persisted schema or cancellation scope changed. Native runtime log:
`/tmp/batter-gi4-native-runtime-after-fix.log`.

The task-owned `batter-retirement-primary` cluster uses loopback port 55433,
`max_prepared_transactions=10` and `autovacuum_naptime=1s`; the distinct observer
is `batter-review-observer` on 55432. No credentials were checked in or printed.
An initial retirement invocation omitted the required `sslmode=disable` and was
rejected before fixture work; corrected endpoints passed the native preflight.
Existing fault fixtures still emit their deliberate deferred-cleanup failure
diagnostic; the exact runner verifies all named test outcomes and both targets.

Rust 1.94.0 also passed the 57-entry reference runner and separate replacement
probe, ordinary reference targets, six doctests and adapter/reference all-target
Clippy. Log: `/tmp/batter-gi4-reference-msrv-retirement-final.log`. An earlier
Clippy run rejected the expanded preservation test's size; extracting its repeated
row-read helper fixed that without changing assertions. The new production-root case passed separately on Rust 1.98.1: actual HTTP
liveness 200, readiness 503 across multiple health sampling intervals, no startup
control rows and successful SIGTERM with awaited owned cleanup. The runner now
requires 58 reference entries. Log: `/tmp/batter-gi4-production-root.log`.
Those earlier 57-entry runs do not cover this added case.

The libraries still resolve through uncommitted sibling path patches. Final
external-source identity/Jig evidence, exact Rust 1.94.0 matrix, full HTTP smokes,
production-root readiness proof, fresh-agent exercises and comprehensive all-model
review convergence remain outstanding. No commit or push was made.

## Native preparation and stop-clock exchange (batter-gi4), 2026-09-12

The optional adapter consumes owned `PreparedSupervisor` values; the protected
path accepts neither live supervisors nor factories. Preparation errors remain
owned startup failures with retained cleanup. Native first causes are independent
of clock tightening. Idempotent stop callbacks exchange earliest timestamps, and
active native/process waits observe changes.

Executed on Linux Rust 1.98.1:

- `cargo test -p batter --lib --tests --examples --locked --quiet`: 301 tests
  passed, including fifteen public managed contracts, process phase wakeups and
  an update arriving during a stop callback.
- Foundation doctests: 22 passed. Adapter: three runtime tests and three doctests
  passed, including compile-fail controls for live-supervisor/closure inputs.
- Adapter/reference all-target Clippy and foundation/adapter warning-denied
  rustdoc passed. Reference library/configuration tests passed (six/twenty);
  all five Python live-inventory controls passed.

On exact Rust 1.94.0, fourteen selected lifecycle library tests, fifteen managed
contracts, twenty-nine process ownership tests, adapter's three runtime tests and
three doctests, and adapter/reference all-target Clippy passed. The first combined
invocation applied a `lifecycle` name filter to the integration targets and selected
none; both integration targets were subsequently run without that filter and
passed. This is focused minimum-toolchain evidence, not the full workspace matrix.

In the sibling native checkout on Rust 1.94.1, twenty-two selected supervisor
tests (excluding the PostgreSQL settlement case), three preparation tests,
fifty-two shutdown-selected tests, seven complete-report tests, nine doctests and
all-target Clippy passed. These selections overlap; they are not a new full-suite
count. A real non-yielding callback remains unjoined when an earlier parent
deadline interrupts active native abort observation. That test releases and
observes the callback before returning. No new PostgreSQL live case was executed.

The native packages are uncommitted sibling path patches. These direct Cargo
executions produced no Jig receipts and do not establish external-source reuse.
Full workspace/two-toolchain/live/Jig/fresh-agent/review acceptance was still
pending at this stage.

## Owned finite commands (batter-gi4), 2026-09-12

The finite-command API, optional absolute total reserve and UDP consumer cutover
passed on Linux Rust 1.98.1:

- `cargo test -p batter --tests --example finite_command --locked --quiet`:
  298 library, integration and example tests passed.
- `cargo test -p batter --doc --locked --quiet`: 22 doctests passed, including
  implicit report-disposal rejection.
- `cargo clippy -p batter --all-targets --locked -- -D warnings` and
  `RUSTDOCFLAGS='-D warnings' cargo doc -p batter --no-deps --locked --quiet` passed.
- `RUSTUP_TOOLCHAIN=1.94.0 cargo test -p batter --test command --test
  scoped_owned_tasks --example finite_command --locked --quiet`: 16 command,
  seven scoped ownership and six example tests passed on the exact minimum toolchain.

After `cargo build -p batter --example finite_command --locked --quiet`, the actual
`target/debug/examples/finite_command` default mode returned 0. `--fail-work`,
`--fail-cleanup`, `--fail-both`, `--cancel` and `--deadline` each returned 1.
Cancellation and deadline modes reported successful cleanup after resource
registration. A Python subprocess driver imposed an eight-second bound per mode
and checked the actual status and outcome facts. No database was required.

The ownership tests gate a real semaphore permit behind dependent cleanup,
drop command owners and borrowed waiters independently, and inspect retained
original errors and both polling/destruction panics. Other cases cover parent
cancellation isolation, total reserve rejection, expired factory inertness,
cleanup timeout, final-poll cancellation, subscriber context through destruction,
and published versus missing reports across runtime destruction. These do not
establish remote transaction disposition or arbitrary detached-task termination.

Logs: `/tmp/batter-gi4-command-tests.log` and `/tmp/batter-gi4-command-msrv.log`.
The full workspace matrices, HTTP/live reference tests, fresh-agent exercises and
all-reviewer convergence were still pending; the reference used its old native
hosting path at this stage.

## Managed component foundation (batter-gi4), 2026-09-12

The new managed registration path and first-stop process clock passed on Linux
Rust 1.98.1 with the current working tree:

- `cargo test -p batter --tests --locked --quiet`: 281 tests passed across 24
  targets/groups, including 14 new managed-component contracts.
- `cargo test -p batter --doc --locked --quiet`: 18 doctests passed.
- `cargo clippy -p batter --all-targets --locked -- -D warnings`: passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc -p batter --no-deps --locked --quiet`: passed.
- `git diff --check`: passed.

The managed tests own actual Tokio descendants, control initialization and
settlement independently, cancel borrowed waiters, drop the service owner and
force direct-wrapper abortion. They verify frozen incomplete process evidence,
late native report retention without later cleanup, original error and destructor
panic retention, classifier panic handling, inert invalid/expired construction,
native early-stop propagation and the original parent timestamp. Paused clocks
test ordering and budgets; they do not establish preemption of blocked threads.

Four existing completed-task timing controls initially failed because their
schedules relied on cancellation starting a fresh interval after delayed drain
observation. Their schedules now complete work within the absolute cancellation
interval and delay collection past that boundary. Their original assertions about
retaining failures, avoiding false aborts and observing actual aborts remain intact;
all 29 process-ownership tests passed afterward.

The sibling native checkout `/home/aa/Documents/runledger`, still uncommitted,
passed `cargo test -p runledger-runtime --lib --quiet` (224 tests), `cargo test
-p runledger-runtime --doc --quiet` (eight doctests), and all-target Clippy with
warnings denied on its Rust 1.94.1 toolchain. New stop observation/timestamp
contracts passed along with the native descendant/real-PostgreSQL suite documented
in the task plan. Batter still consumes the old pinned dependency: these are
separate source-tree results, not native adapter integration evidence.

The exact Rust 1.94.0 and 1.98.1 workspace matrices, HTTP/live reference acceptance,
fresh-agent consumer exercises and comprehensive reviewer convergence remain
pending for the full redesign. Existing Jig receipts do not authenticate this new
source merely because earlier foundation tests passed.

## Startup witness and shutdown lease follow-up (batter-8q8.4), 2026-09-11

The newly authorized review/fix cycle addressed two medium findings from the
preceding terminal review. Pinned Runledger inspection confirmed that an empty
claim is followed by the complete configured poll interval. The existing
14-second predecessor retry could therefore miss the fixed 20-second witness for
otherwise valid long polling settings. This was a composition-contract omission,
not a defect in Runledger: Serve now requires retry + poll + one-second margin to
fit strictly inside the witness, making 4,999 ms the largest whole-millisecond
poll. Standalone `WorkerSettings` parsing retains its prior native one-year range.
`prepare_probe_worker` applies the same relationship before acquisition and again
to the actual interval remaining after database preparation.

The other finding exposed an incomplete lifecycle branch. Reconciliation failure
dropped the combined monitor before awaiting up to eleven seconds of native stop
and six seconds of reconciliation-pool close, even though the lease session had a
ten-second idle timeout. The repair continues lease queries throughout both
operations. A later lease failure is retained alongside the initiating
reconciliation failure; it does not cancel bounded settlement or replace the
primary cause. Clean native stop plus pool-close timeout now has its own typed
`ReconciliationClose` classification. The composed paused-clock test includes
native shutdown, abort drain, reconciliation close and release. Late-commit test
owners now retain their parent and termination gate, request cancellation, and
await preparation settlement on every failure path.

The first complete Rust 1.98.1 live sweep correctly rejected the old 250 ms
witness-failure fixture before acquisition. That oracle now uses 16 seconds, long
enough to enter native execution while still forcing the intended durable-success
timeout, and passed alone. A subsequent full sweep encountered the unrelated
expected-failure fixture's deferred-cleanup race; the exact case passed alone and
the required complete rerun then passed. These failed attempts are not counted as
acceptance evidence.

| Command / evidence | Outcome |
| --- | --- |
| `cargo test -p batter-example-reference-service --locked` | PASS: 43 library tests, one binary test, 20 configuration tests, three fixture-diagnostic tests, ordinary discovery with all live cases ignored, and 12 doctests. |
| `cargo clippy -p batter-example-reference-service --all-targets --locked -- -D warnings` | PASS after extracting root witness validation and owned-native observation helpers. |
| Targeted PostgreSQL regressions for late commit, blocked reconciliation, lease-loss takeover, normal drain and witness failure | PASS on Rust 1.98.1 against PostgreSQL 18.6. |
| `bash scripts/verify.sh` | PASS on Rust 1.98.1: all required test, configuration, lint and rustdoc phases. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS on the declared minimum toolchain with the same required phases. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS on the final source: exact 56-entry inventory, 56 passed, zero failed/ignored/filtered. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS: exact 56-entry inventory, 56 passed, zero failed/ignored/filtered; 101.68 seconds. |
| Five `scripts/smoke_http.py` modes after an explicit `batter-axum` example build | PASS on both Rust 1.98.1 and 1.94.0: SIGTERM, SIGINT, deadline, WARN-filter and WARN-filter/deadline modes. |
| `cargo fmt --all -- --check`; `git diff --check` | PASS on the frozen pre-review tree. |

The same two disposable `postgres:18.6` containers and image digest recorded
below were recreated on loopback ports 55431/55432 with `track_counts=on` and
one-second autovacuum naptime. Cargo.lock remained byte-identical at the SHA-256
recorded below.

## Startup-control ownership fencing review repair (batter-8q8.4), 2026-09-11

The all-reviewer pass found that continuous stale-control reconciliation had been
serialized with the ownership-session ping. Application-pool acquisition or a
row-lock wait could therefore outlive the lease session's ten-second idle timeout,
release the advisory lock, and let the old worker continue destructive work while
a successor started. This is a design-boundary defect, not only a missing timeout.

The repair gives each owner a PostgreSQL-sequence epoch on the lock-owning session
immediately after lock acquisition and stores it in the control payload; older payloads decode as epoch zero. Runtime
reconciliation selects only lower epochs. Lease checks and reconciliation are
independently selected, and reconciliation uses a lazy one-connection pool with a
four-second pass bound, 3.5-second statement timeout and three-second lock timeout.
Returned reconciliation connections undergo SQLx's viability check before reuse,
and the pool receives a separate six-second close allowance before lease release.
The complete worker stop reserve is now 20 seconds. The application explicitly
retries the replay-safe read/cancel pass after one or two failures; a third
consecutive failure stops the worker and retains its concrete cause.

The first fresh review of that repair found a timing coupling introduced by the
larger reserve: the predecessor-generation retry delay had also become 18 seconds,
while the production parent left only 17 seconds ahead of shutdown. The retry
lower bound is again a separate 14 seconds because reconciliation-pool closure
and lease release cannot keep native claiming loops alive. An exact unit assertion
guards that relationship and the equal-witness/equal-epoch success path.

The second fresh all-reviewer pass found that epoch allocation and the advisory
lock were still split across sessions, so a delayed predecessor could acquire a
later epoch after a successor had taken the lock. It also found that pool-close
timeout composition replaced native, join or unexpected-success identity, and
that two live failure branches detached started owners. Epoch allocation now
occurs as part of lock acquisition on the exact lease connection. Close failure
is supplemental on every prior completion kind and primary only after a requested
successful native stop. The live branches retain, abort where needed, and await
their owners before returning. The reference startup parent is now 45 seconds,
leaving eleven seconds beyond the 14-second retry lower bound and ahead of the
20-second stop reserve.

Research before the follow-up edit confirmed that PostgreSQL can reorder `WHERE`
predicates, so a job-type condition cannot protect a fallible JSON-text-to-bigint
cast. The candidate predicate now uses `CASE`, casts only JSON numbers to native
`numeric`, and treats missing or other JSON types as legacy epoch zero. The hosted
normal-drain case includes a malformed string epoch and proves it is canceled
without breaking preparation. The initialized-schema upgrade case directly
advances the new sequence twice.

SQLx 0.9 documents that an incomplete pool close may leave connection disposal to
internal work and that client-side drop need not promptly notify PostgreSQL. A
close timeout therefore remains unconfirmed and keeps dependent cleanup
conservative. Its error is now supplemental to an earlier lease/reconciliation
failure instead of replacing the initiating cause. A deterministic unit control
races an immediately failed lease monitor against permanently pending
reconciliation; serializing them makes the control time out. The row-lock live
case remains the integrated cause-retention and successor-fencing proof rather
than the sole evidence of scheduling independence.

Pinned Runledger research also corrected an overclaim in the first regression.
A late predecessor may be claimed once before reconciliation wins; a witness
mismatch is not a supervisor-loop failure and cannot acknowledge the current
witness. The live oracle now requires epoch ordering, zero or one stale attempt,
terminal cancellation, and no subsequent attempt growth. A separate live case
holds a legacy row lock, terminates the lease, and keeps the lock held until driver
settlement proves that reconciliation cannot mask lease loss. The successor then
cancels that never-claimed row at attempt zero.

The last permitted repair round accepted three bounded follow-ups rather than a
new architectural defect. It restored normal checked reuse on the long-lived
reconciliation pool, while retaining forced non-reuse only for the temporary
preparation pool; made unexpected native success depend on an actual recorded
shutdown request instead of assuming every observation followed drain; and made
the row-lock fixture retry until it locks a still-pending control while retaining
and settling every started owner on all failure paths. A claim that reconciliation
also had to keep running during shutdown was rejected: monitor failure immediately
requests native stop, the owner epoch fences older controls, and dependent cleanup
already treats close or lease-release uncertainty conservatively. PostgreSQL 18
documentation confirms that ordinary `pg_dump` archives include sequence values;
restoring `job_queue` without the epoch sequence and then resetting that sequence
is explicitly outside this example's supported operational contract.

Linux x86_64; Git baseline `f1cafe9abeb9c08960523288c2c2c25da5e18202`.
Cargo.lock remained unchanged at SHA-256
`848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.
Two task-owned `postgres:18.6` containers used image digest
`sha256:1957b2ff3137e4ef7f3bc813e74fff50b1e1ffddc85c8b9d6f14ade972be8687`,
SCRAM host authentication, `track_counts=on`, `autovacuum_naptime=1s`, and
loopback ports 55431/55432. Provisioning remained external to Batter.

| Command / evidence | Outcome |
| --- | --- |
| Focused unit, warning-denied Clippy and package tests | PASS: exact retry timing, matching/mismatching epochs, legacy decoding, mutation-sensitive monitor selection, three-consecutive-failure policy, typed primary/supplemental failure composition and all ordinary reference targets. |
| `hosted_preparation_late_control_commit_reconciliation` | PASS: lower-epoch cancellation, bounded zero-or-one attempt and stable post-cancellation count. |
| `initialized_schema_upgrade`, `hosted_worker_probe_registry_and_normal_drain`, `hosted_worker_lease_loss_stops_host`, `hosted_worker_blocked_reconciliation_preserves_lease_monitor`, and `hosted_preparation_late_control_commit_reconciliation` after the first fresh review | PASS on Rust 1.98.1: upgraded sequence advancement, malformed-epoch cancellation, takeover timing, primary lease-loss classification under a blocked cancellation, and lower-epoch late-commit disposal. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: complete Rust/Python matrix, formatting, warning-denied Clippy, doctests and rustdoc. |
| Rebuild `http_service` on each toolchain, then run the five documented smoke modes | PASS: all ten process smokes. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS on the final repaired source: exact 56-entry inventory, 56 passed, zero failed/ignored/filtered; 86.18 seconds. The immediately preceding sweep passed all changed ownership cases but the unrelated `fixture_runtime_loss_exposes_native_drop` case failed with its sanitized probe error; that case passed alone and the required complete rerun then passed. An earlier round also found a now-corrected 15-second witness-failure test context. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS on the final repaired source: the same exact 56-entry inventory and zero failed/ignored/filtered; 87.26 seconds. |

The recurring sanitized deferred-cleanup message is the deliberate
`fixture_body_and_cleanup_failures_retained` scenario. No macOS, hosted CI, TLS,
deployment, publication, commit or push is claimed.

## Late startup-control commit reconciliation (batter-8q8.4), 2026-09-11

The investigation confirmed a supported late-publication ordering, but rejected
the review's original blocked-`INSERT` trigger. Pinned Runledger awaits its
enqueue before dispatching `COMMIT`; cancellation while that insert is blocked
cannot publish the row later. The executable regression instead blocks an actual
deferred constraint trigger after SQLx has sent `COMMIT`, cancels the predecessor,
lets a distinct successor backend pass its initial stale-control scan, and proves
that successor blocks on the still-uncommitted predecessor transaction. It then
releases the old commit and requires the running owner to cancel the late row.
The initial assertion only proved the attempt count stopped after cancellation;
it did not prove that the row had never been claimed. The review-loop repair
below corrects that contract and oracle. Exact
`(pid, backend_start)` observation also requires the predecessor backend to exit.

This initial implementation reconciled non-current startup controls continuously
while monitoring its dedicated ownership lease, but serialized those operations.
The subsequent review demonstrated that a blocked reconciliation could suppress
lease checks; its replacement is recorded above this historical evidence. A reconciliation failure retained
the concrete startup failure separately from any native shutdown result, and
still permits the independently owned lease to attempt explicit unlock. This is
periodic eventual reconciliation, not transaction commit acknowledgement or a
general guarantee that remote work stopped when a local client was dropped.

Linux x86_64; baseline `f1cafe9abeb9c08960523288c2c2c25da5e18202`, tree
`b952d044396f40fb958803b65296334a925c242d`; plan
`plan_01M28JG9J1QRHJ67QQZJJGQMTA`. Cargo.lock remained unchanged at SHA-256
`848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.
The primary was a task-owned native PostgreSQL 18.6 Ubuntu cluster on loopback
port 55431; the independent observer was task-owned `postgres:18.6` image
`sha256:a6638641707cdf047e5d5c2781f437e2e809323cab22c70b280be8389fbb7878`
on loopback port 55432, PostgreSQL 18.6 Debian. Both used SCRAM host
authentication, `autovacuum_naptime=1s`, and `track_counts=on`.

| Command / evidence | Outcome |
| --- | --- |
| `cargo test -p batter-example-reference-service --lib worker::tests::runtime_reconciliation_failure_retains_cause_and_native_shutdown --locked -- --exact` | PASS: concrete reconciliation cause identity and the separate successful native shutdown result were retained. |
| `cargo test -p batter-example-reference-service --test reference_live hosted_preparation_late_control_commit_reconciliation --locked -- --ignored --exact --nocapture` with the primary endpoint | PASS in 2.76s after the final test-helper refactor. An initial invocation omitted the required `sslmode=disable` URL setting and was rejected by endpoint policy before fixture work. |
| Mutation control removing the production runtime reconciliation call, then running the same focused live test | FAIL as required: the late row was not canceled before the bounded oracle expired. The production call was restored and the focused test passed again. |
| `PYTHONPATH=scripts python3 -m unittest scripts/test_reference_live.py -v` | PASS: five inventory/preflight/exact-execution controls, including the new required preparation case. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh` | PASS: complete Rust/Python matrix, formatting, warning-denied Clippy, doctests and rustdoc. The first run found test-helper cognitive complexity 24/20; splitting the orchestration into named phases resolved it without a lint allowance. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete matrix on the minimum toolchain. |
| Rebuild `http_service` on each toolchain, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes | PASS: all ten process smokes on their respective rebuilt binaries. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` with both endpoints | PASS: PostgreSQL 18 preflight, exact 55-entry inventory, 55 passed, 0 failed, 0 ignored, 0 measured, 0 filtered; 81.74s. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` with both endpoints | PASS: identical preflight and inventory, 55 passed, 0 failed, 0 ignored, 0 measured, 0 filtered; 80.66s. |
| `scripts/jig work check --plan-id plan_01M28JG9J1QRHJ67QQZJJGQMTA`, then `work evidence` and `work gates` with a 30000ms freshness timeout | PASS: all five applicable targets; fresh required `verify` evidence, no unresolved gates. Initial final receipt `receipt_01M28M9J9PZFERRCP81FJ5DYT5`; a tracker/validation-only refresh follows this evidence entry. |

The trailing sanitized deferred-cleanup message in each full live run belongs to
the deliberate cleanup-failure scenario; both runners exited zero. The new
behavior was not executed on macOS or hosted CI. No TLS, deployment, publication,
commit or push is claimed.

## Complete preparation ownership (batter-2zw), 2026-09-11

The cancellation repair retains preparation independently of its waiter, settles
its dedicated control session before dependency cleanup, and publishes typed
unlock and client-close results alongside preparation/native failures. It validates
deadline arithmetic before acquisition or native build. The shared worker shutdown
allowance is fourteen seconds: ten native stop, one abort drain, two lease release,
and one scheduling margin. Early initialization now installs and polls Unix signals
before pool acquisition; completed signal reception survives registration.

A live cancellation regression exposed an additional SQLx 0.9 behavior: returning
a connection with an interrupted query could block its reuse ping and delay shared
pool closure. Native preparation helpers now use a temporary one-slot pool that
closes returned connections before that ping. Preparation explicitly closes this
pool before starting the native supervisor. Application pool capacity excludes one
control session during normal operation and up to two sessions during preparation.
This is documented capacity, not a total-session limit.

Regression evidence covers cancelled/aborted preparation waiters, immediate
successor acquisition after confirmed unlock, `Duration::MAX` before side effects,
owned-task panic publication, late nested cleanup failures, completed Unix signal
handoff, SIGTERM during actual pool/schema/control initialization, native LEASED
cancellation and an orchestrated concurrent terminal transition. Release fault
tests independently retain unlock/close errors and timeouts and exercise the
composed native-stop/abort-drain/release deadline with Tokio's paused clock.
Those fault tests inject the release operations; they do not claim live network
fault coverage. Client closure is explicitly not PostgreSQL backend-exit proof.

Linux x86_64; Rust/Cargo 1.98.1 (`48a229cea` / `797e8a9bc`) and 1.94.0
(`4a4ef493e` / `85eff7c80`). Git baseline remains
`d82f5bfac71d47fc429b381bc61f42350b301764`, with existing uncommitted work
preserved. Cargo.lock remains unchanged at SHA-256
`848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.
Two task-owned `postgres:18` containers, `batter-2zw-primary` and
`batter-2zw-observer`, run PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2), SCRAM host
authentication, `autovacuum_naptime=1s`, and `max_connections=100`, on loopback
33395/33396. Both task-owned containers were removed after verification.
Provisioning is external to Batter. `DATABASE_URL` is unset.

| Command | Result |
| --- | --- |
| `bash scripts/verify.sh` | PASS on Rust 1.98.1: complete Rust/Python matrix, formatting, warning-denied Clippy and rustdoc. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete matrix on the minimum toolchain. |
| Build `batter-axum` example `http_service` with `--locked` on each toolchain, then run `scripts/smoke_http.py --binary target/debug/examples/http_service` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes | PASS: all ten process smokes, including readiness, deadline/correlation behavior and exit 0. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` with both task-owned endpoints | PASS: native preflight, exact 54-entry inventory, 54 passed, 0 failed, 0 ignored, 0 filtered; 78.58s. Includes 52 database cases and two offline signal entries. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` with the same endpoints | PASS: identical preflight and inventory; 54 passed, 0 failed, 0 ignored, 0 filtered; 78.59s. |

The first full database execution passed all 52 live cases but the strict runner
correctly rejected two filtered offline entries. Its repaired command now executes
all 54 entries with `--include-ignored`, preserving the exact-name and zero-filtered
oracle. The initial Python control retained the former `--ignored` command and
failed; its expected command was updated and both full verification runs passed.
The lease-loss test was updated to assert both the retained native failure and
the new release uncertainty. The trailing deferred-cleanup diagnostic in the live
log belongs to the deliberate `fixture_cleanup_failure` scenario.

The first Jig check rejected every receipt because documentation was edited while
its read-only verification layer ran. This was an agent sequencing error: direct
checks passed, but the mixed-snapshot receipts are invalid and cannot be reused.
The stationary-worktree rerun passed all five targets. Its Rust receipts are
Clippy `receipt_01M28GP4CG2FYNR292TYCH5YZK`, formatting
`receipt_01M28GP4RA8DKE1VHBKCCA0YKJ`, and tests
`receipt_01M28GP546TXHQ6Z3E5BXD90C9`, for plan
`plan_01M28EQQEM2E9XNGVMS3ED78F6`. Final documentation/tracker edits refresh
only the repository policy checks and reuse these unchanged Rust inputs. A
default two-second freshness inspection reached `collection_limit`; the required
follow-up uses `--freshness-timeout-ms 30000` to inspect existing evidence.

The prior 49-case run below was executed only on Rust 1.98.1; its Rust 1.94.0
follow-up was Clippy/compile evidence. Its claim of confirmed release on every
preparation exit was disproved by cancellation and is superseded by this section's
owned preparation and typed outcomes. No general async-drop/runtime-death guarantee
or new macOS, TLS, hosted CI, publication or deployment evidence is claimed.

## Worker-host lease and witness follow-up, 2026-09-11

A fourth review pass over the staged worker found five follow-up gaps: the
control witness carried a two-attempt budget that overlapping predecessor
generation mismatches could exhaust and dead-letter; the advisory lease was
dropped rather than released before driver completion was published; the
lease-loss live case terminated any advisory-lock backend and did not wait for
its exit before successor acquisition; early post-start failures in that case
could drop the running Batter supervisor without awaiting shutdown; and a native
driver failure during startup witnessing was classified twice. The repaired
source gives the control an unbounded attempt budget (the witness remains bounded
by its deadline and stale controls are canceled by the next owner), awaits a
bounded server-confirmed `pg_advisory_unlock` and session closure before every
completion publication and before every preparation or build failure returns,
exports the two-key lock identity so live cases target only that lock, waits for
the terminated backend to leave `pg_stat_activity` and for the lock to be free
before preparing the successor, keeps the started supervisor awaited on every
failure path, and returns the single driver classification directly.

Linux x86_64, kernel `7.0.11-76070011-generic`; Rust/Cargo 1.98.1
(`48a229cea` / `797e8a9bc`) and minimum 1.94.0 for the package Clippy check.
Audited Git baseline `d82f5bfac71d47fc429b381bc61f42350b301764` with the
uncommitted worker-host tree; Cargo.lock remained unchanged at SHA-256
`848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`. Two
task-owned disposable containers `batter-goal-db` and `batter-goal-other` ran
`postgres:18` (**PostgreSQL 18.6 Debian x86_64**, `-c autovacuum_naptime=1s -c
max_connections=100`, SCRAM host authentication) on loopback ports 33393 and
33394 as `POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL`; both were
removed after the run. `DATABASE_URL` was unset.

| Command | Result |
| --- | --- |
| `cargo test -p batter-example-reference-service --locked` | PASS: 18 library tests, 20 configuration tests, three plus one further offline integration tests, nine doctests, 49 ordinary live ignores. |
| `cargo clippy --workspace --all-targets --locked`, `cargo fmt --all -- --check`, `RUSTDOCFLAGS="-D warnings" cargo doc -p batter-example-reference-service --no-deps --locked` | PASS with no warnings on 1.98.1. |
| `RUSTUP_TOOLCHAIN=1.94.0 cargo clippy -p batter-example-reference-service --all-targets --locked -- -D warnings` | PASS on minimum Rust 1.94.0. |
| `cargo test -p batter-example-reference-service --test reference_live --locked -- --ignored --test-threads=1 hosted_worker worker_startup` | PASS: all eight hosted-worker and startup-witness live cases, including the rewritten lease-loss takeover; 33.03s. |
| `bash scripts/test_reference_live.sh` with both endpoints | PASS: PostgreSQL 18 preflight; exact 49-case inventory; 49 passed, 0 failed, 0 ignored, 0 filtered; 71.49s. The trailing deferred-cleanup message is the intentional `fixture_cleanup_failure` scenario. |
| Three further sequential executions of `hosted_worker_lease_loss_stops_host` and `hosted_worker_probe_registry_and_normal_drain` | PASS each time (3.34s, 3.31s, 3.23s); no flake observed in the targeted backend termination, awaited exit or successor acquisition. |

This is the first complete execution of the 49-case live inventory: the
database-exclusive probe ownership, forced lease loss, successor takeover,
unstarted-supervisor cleanup and dropped-wrapper driver observation cases now
have live evidence on this machine. The complete `scripts/verify.sh` matrix and
HTTP smokes were not rerun for this follow-up; the changed files are confined to
the reference package and documentation. No macOS, arm64, TLS, hosted CI,
publication or deployment claim is added.

## Worker-host review repairs, 2026-09-11

A comprehensive three-reviewer pass over the staged Runledger worker found
unsafe shared-pool cleanup after another direct component exit, process-global
startup-control claim collisions, cancellation-sensitive native driver ownership,
an unreserved parent startup deadline, delayed signal consumption and fixture
failure paths that could abandon the driver. The repaired source retains the
native join independently, waits for observed completion before dependency
cleanup, applies the final Batter unsafe-exit decision, fences the probe host with
a PostgreSQL session advisory lock, reconciles stale control rows, clamps the
witness before a 12-second shutdown reserve, polls installed Unix signals during
initialization and explicitly stops unregistered test hosts.

A fresh three-reviewer pass over those repairs found five follow-up gaps. The
second repair round now checks the dedicated lock session every second with a
two-second query bound and session-local ten-second idle timeout, requests native
shutdown on lease loss, and releases the connection before publishing driver
observation. Pending/leased startup controls are canceled under that ownership
before enqueue; registration rejection retains an awaitable host. Process-level
cleanup veto inspects direct panic/abort, abort-request and unjoined evidence even
when the outer cleanup stack is empty. The real-child signal readiness reader now
accepts libtest-prefixed output and has a three-second watchdog.

A third three-reviewer pass found an unstarted-registration owner gap, a lease
liveness gap during database preparation, successor-witness exposure to a stopping
predecessor, two incomplete cleanup/error classifications and one over-broad
readiness sentence. The final repair round retains a drop guard in the registered
factory, monitors the lease throughout preparation, binds each handler to its
unique witness generation with one delayed takeover retry, treats unjoined process
cleanup as unsafe for retained dependencies, preserves registration and native
shutdown failures together, and documents explicit readiness-approval opt-out.
The lease-loss live case now begins successor preparation before awaiting the
predecessor; the normal live case drops a successfully registered but unstarted
Batter supervisor and waits for native stop and lease release.

Linux x86_64, kernel `7.0.11-76070011-generic`; audited Git baseline
`d82f5bfac71d47fc429b381bc61f42350b301764`. Cargo.lock remained unchanged at
SHA-256 `848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.

| Command | Result |
| --- | --- |
| Focused reference package check, 18 library tests, nine doctests, warning-denied Clippy and runner-control tests | PASS on Rust 1.98.1. Added unit controls reject a predecessor/successor witness mismatch and classify unjoined process cleanup as unsafe. The earlier cleanup and Unix-signal regressions remain passing. |
| `cargo test -p batter-example-reference-service --test reference_live --locked -- --ignored --list` after the final repair | PASS: exact 49-case inventory, 49 unique names. No live case executed. The lease-loss case now compiles concurrent successor preparation plus retained predecessor failure; the normal case compiles unstarted-supervisor stop observation and lease release. |
| `bash scripts/verify.sh` | PASS on Rust/Cargo 1.98.1 (`48a229cea` / `797e8a9bc`): complete Rust/Python matrix, formatting, warning-denied Clippy and rustdoc, 12 foundation doctests and nine reference-service doctests. Ordinary discovery found all 49 reference live cases and intentionally ignored them. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS on minimum Rust/Cargo 1.94.0 (`4a4ef493e` / `85eff7c80`) with the same verification scope and 49 ordinary live ignores. |
| Build `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes, once per toolchain | PASS: all ten process smokes retained readiness/probes/fallback, request identity, completion telemetry, deadline behavior, both signal choices and exit 0. |

`POSTGRES_TEST_ADMIN_URL`, `POSTGRES_TEST_OBSERVER_URL` and `DATABASE_URL` were
unset. PostgreSQL provisioning remains external, so the expanded 49-case live
suite was not executed and this section does not extend the prior 47-case live
claim. In particular, database-exclusive probe ownership, forced lease loss,
successor takeover, unstarted-supervisor cleanup and dropped-wrapper driver
observation have current compile/offline coverage but await live execution.
No macOS, TLS, hosted CI, publication or deployment claim is added.

## Staged Runledger worker acceptance (batter-0cp), 2026-09-11

The expanded 47-case reference inventory passed against two dedicated temporary
PostgreSQL 18.6 clusters on both supported Rust toolchains. Both clusters used
SCRAM-SHA-256 host authentication; the primary used superuser authority,
`track_counts=on`, and `autovacuum_naptime=1s`. Native preflight verified the
versions, required privileges and distinct signed cluster identifiers before
fixture creation. These local clusters were verification infrastructure, not
application provisioning or deployment evidence.

Linux x86_64, kernel `7.0.11-76070011-generic`; PostgreSQL
`18.6 (Ubuntu 18.6-1.pgdg24.04+2)`; Rust/Cargo 1.98.1
(`48a229cea` / `797e8a9bc`) and 1.94.0
(`4a4ef493e` / `85eff7c80`). The audited Git baseline is
`d82f5bfac71d47fc429b381bc61f42350b301764`; the implementation remains an
uncommitted working-tree change as required. Cargo.lock was unchanged, with
SHA-256 `848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.

| Command | Result |
| --- | --- |
| `POSTGRES_TEST_ADMIN_URL=<temporary-primary> POSTGRES_TEST_OBSERVER_URL=<temporary-observer> RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS: PostgreSQL 18 preflight; 47 passed, 0 failed, 0 ignored, 0 filtered; 68.36s. |
| `POSTGRES_TEST_ADMIN_URL=<temporary-primary> POSTGRES_TEST_OBSERVER_URL=<temporary-observer> RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS: the same preflight and exact inventory; 47 passed, 0 failed, 0 ignored, 0 filtered; 66.48s. |
| `cargo test -p batter-example-reference-service --lib --locked` | PASS: 14 library tests, including exact probe-registry contents and cancellation-independent nested cleanup. |
| `cargo clippy -p batter-example-reference-service --all-targets --all-features --locked -- -D warnings` | PASS. |
| `bash scripts/verify.sh` | PASS on Rust 1.98.1: complete Rust/Python matrix, formatting, warning-denied Clippy, five reference doctests and warning-denied rustdoc. Ordinary discovery found all 47 live cases and intentionally ignored them. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS with the same complete verification scope on the minimum toolchain. |
| Rebuild `batter-axum` example `http_service` on each toolchain, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes | PASS: all ten process smokes, including readiness/probes/fallback, custom envelopes, work/deadline, filtered correlation, both signals and exit 0. |
| `scripts/jig work check --plan-id plan_01M27YA6NV07SYD5NGKDWS1WXH` | PASS: Clippy, formatting, tests, contract and file-budget targets. Source-validation receipts are Clippy `receipt_01M2812PDYF745JX00TM3P0ZVK`, formatting `receipt_01M2812PSYWDC3P6VRF9761PA7`, tests `receipt_01M2812Q6MYKP25G6WW4ZH35D6`, contract `receipt_01M2812QJZT2J9NPTCYGC2783C`, and file budget `receipt_01M2812QYK6CPJJR5XR2VKR5W8`. `work evidence` and `work gates` report the required verify gate fresh and passing. |

The staged root now continuously owns the pinned Runledger supervisor and
acknowledges its Batter component only after `jobs.startup.control` reaches its
actual typed handler and persisted `SUCCEEDED` state. The registry contains no
delivery handler: a delivery submitted through the production command path stays
PENDING with zero attempts, and application readiness deliberately remains
unapproved until `batter-8q8.2` supplies the real provider. Each control attempt
creates a fresh operation context from its native attempt deadline rather than
inheriting request cancellation.

Live controls also prove that a claimed handler can finish after the drain
request, configured concurrency 1/2 changes actual hosted execution, controlled
retry produces two handler invocations and exactly two durable attempts, and
local replay adds neither. A PostgreSQL trigger rejects only the startup control
job's success persistence after its real handler invocation; no handoff becomes
ready, and owned partial-startup cleanup runs. A held handler crosses the actual
ten-second native shutdown bound: the component retains
`RuntimeError::ShutdownTimeout`, no dependent finalizer runs, and the nested
report records `UnsafeTaskExit` while the termination gate stays Unproven.

The async/concurrency audit exposed two defects before final validation. First,
requesting Runledger's cloneable shutdown handle did not resolve the external
future that starts `run_until_shutdown`'s timeout; the host now owns both signals,
requests stop-claiming first, then resolves the bounded-driver trigger. Second,
the first cleanup waiter initially owned the nested cleanup future; the final
owner drives cleanup independently and publishes one retained report, so waiter
cancellation cannot abandon finalization. Focused regressions failed or hung on
the earlier designs and pass after both corrections.

An initial full verification attempt found one broken shorthand rustdoc link
after all executable tests and doctests had passed. The link was fully qualified;
a focused warning-denied rustdoc run and both complete final matrices pass. The
runner's intentional deferred-cleanup diagnostic appeared during both successful
live inventories, as designed. No macOS, TLS, hosted CI, delivery-provider,
publication or deployment claim is added.

## Atomic reference command live acceptance (batter-kpd), 2026-09-11

The current 42-case reference inventory passed against two dedicated temporary
PostgreSQL 18.6 clusters on both supported Rust toolchains. Both clusters used
SCRAM-SHA-256 host authentication; the primary used superuser authority,
`track_counts=on`, and `autovacuum_naptime=1s`. Native preflight verified the
versions, required privileges, and distinct signed cluster identifiers before
creating a fixture. The clusters were local verification infrastructure, not
application provisioning or deployment evidence.

Linux x86_64, kernel `7.0.11-76070011-generic`; PostgreSQL
`18.6 (Ubuntu 18.6-1.pgdg24.04+2)`; Rust/Cargo 1.98.1
(`48a229cea` / `797e8a9bc`) and 1.94.0 (`4a4ef493e` / `85eff7c80`). The
audited Git baseline is `acb5df43d7ac2fcea38683f4b0b90fe748dc11e3`; the
implementation remains an uncommitted working-tree change as required.
Cargo.lock SHA-256 is
`848f4a89b6f50b35e1a14d0776f18601a5bdc05ee10a4a217e74dc51f6ebc70b`.

| Command | Result |
| --- | --- |
| `POSTGRES_TEST_ADMIN_URL=<temporary-primary> POSTGRES_TEST_OBSERVER_URL=<temporary-observer> RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS on the final split source tree: PostgreSQL 18 preflight; 42 passed, 0 failed, 0 ignored, 0 filtered; 54.22s. |
| `POSTGRES_TEST_ADMIN_URL=<temporary-primary> POSTGRES_TEST_OBSERVER_URL=<temporary-observer> RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS on the final split source tree: the same preflight and 42-case inventory; 0 failed, ignored, or filtered; 54.13s. |
| `cargo test -p batter-example-reference-service --all-targets --all-features --locked` | PASS: 32 ordinary library/binary/integration tests plus three preflight-example tests; all 42 live cases discovered and intentionally ignored. |
| `cargo test -p batter-example-reference-service --doc --locked` | PASS: four doctests, including delivery submission/reconciliation and authentication examples. |
| `cargo clippy -p batter-example-reference-service --all-targets --all-features --locked -- -D warnings` | PASS after separating service orchestration and live scenarios below repository complexity/length limits. |
| `PYTHONPATH=scripts python3 -m unittest scripts/test_reference_live.py` | PASS: five strict inventory/execution controls. |
| `bash scripts/verify.sh` | PASS on the final split source tree with Rust 1.98.1: complete Rust/Python matrix, formatting, warning-denied Clippy, doctests, and warning-denied rustdoc. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS on the final split source tree with the same complete verification scope on the minimum toolchain. |
| Rebuild `batter-axum` example `http_service` on each toolchain, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes | PASS: all ten process smokes, including readiness/probes/fallback, custom envelopes, work/deadline, INFO/WARN-filtered correlation, both signals, and exit 0. |
| `scripts/jig work check --plan-id plan_01M27S0SZB2H9KHXBYT0MDCFM7` | PASS: Clippy, formatting, tests, contract and file-budget targets. Source-validation receipts are Clippy `receipt_01M27WG2R5D1W3N0Z8XD19ECKY`, formatting `receipt_01M27WG34BVMADZTN4RRFH6HTA`, and tests `receipt_01M27WG3GBH7RJHYQEZFCNY7SZ`; the final tracker/documentation-only refresh reused them and passed both whole-repository policy targets. |

The new command case applies Runledger and application startup twice, uses the
production authentication/router path, and proves exact replay, canonical JSONB
comparison, foreign-owner lookup and target isolation, replacement-generation
fencing, response-discard reconciliation, pending state, and exact equality of
the command/delivery/job/enqueue-event counts. It also verifies Runledger's
organization, key, type, payload, and status against the retained immutable
request. The configured-root case distinguishes pool acquisition timeout from
request deadline and exhausts the production Bulkhead and finite-process
capacities with held work.

The first full development run passed 41 cases but exposed a weak timing setup:
the intended 30ms held-pool timeout could expire during initial SCRAM connection
establishment, before the held-checkout scenario began. The final test uses a
500ms pool acquisition budget inside a 1.5s request budget; a focused strict-SCRAM
rerun and both complete inventories pass. No semantic assertion was removed.
The runner's intentional deferred-cleanup diagnostic appeared while its
error-retention case passed, as designed. True concurrent same-key/fault injection
and controlled lost-commit-acknowledgement proof remain assigned to
`batter-8q8.1`/`batter-fms`; the current case discards an acknowledged response
to prove the public reconciliation path without overstating transport fault
injection. No macOS, TLS, hosted CI, worker, provider, publication, or deployment
claim is added.

The first connected Jig profile rejected the new 989-line delivery module at the
repository's 800-line hard limit. Transaction orchestration was moved into its
own 226-line child module, leaving the domain and SQL mapping at 768 lines. The
targeted file-budget check and the final connected profile pass without a waiver;
`work evidence` and `work gates` report all required evidence fresh and passing.
The acceptance record was added to `batter-kpd` before its verified closure;
worker/provider execution and the stronger concurrency/fault suite remain open
under their separately owned Beads.

## Combined configuration live acceptance (batter-5pm), 2026-09-11

The reconciled forty-case reference inventory passed against two operator-started,
disposable PostgreSQL 18.6 Docker clusters on both supported Rust toolchains. The
primary used SCRAM host authentication, superuser authority, `track_counts=on`
and `autovacuum_naptime=1s`; the observer was a distinct cluster and allowed
`pg_control_system()`. Native preflight verified both server versions, required
primary capabilities and distinct cluster identifiers before any fixture ran.

Linux x86_64, kernel `7.0.11-76070011-generic`; Rust 1.98.1
(`48a229cea 2026-09-01`) and Rust 1.94.0 (`4a4ef493e 2026-03-02`). The Docker
image was `postgres@sha256:1957b2ff3137e4ef7f3bc813e74fff50b1e1ffddc85c8b9d6f14ade972be8687`
and reported PostgreSQL `18.6 (Debian 18.6-1.pgdg13+2)`. The tested source
baseline was commit `29856ba8e0998db9cc0adae489077ebe1d60c02c`, tree
`59d7c7995d333b42a3f237483665e6b6031346f0`; only documentation was modified
locally. Cargo.lock SHA-256 remained
`86b82ac103a8447dcfeb10ab834b620ac2492d4bb405681a0b6e71413b706d49`.

| Command | Result |
| --- | --- |
| `POSTGRES_TEST_ADMIN_URL=<disposable-primary> POSTGRES_TEST_OBSERVER_URL=<disposable-observer> RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS: PostgreSQL 18 preflight; 40 passed, 0 failed, 0 ignored, 0 filtered; 74.51s. |
| `POSTGRES_TEST_ADMIN_URL=<disposable-primary> POSTGRES_TEST_OBSERVER_URL=<disposable-observer> RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS: PostgreSQL 18 preflight; 40 passed, 0 failed, 0 ignored, 0 filtered; 74.83s. |

The runner emitted its intentional deferred-cleanup failure diagnostic while the
corresponding retention regression passed; each complete command exited zero.
Together with the unchanged two-toolchain matrices, ten HTTP smokes and current
Jig `api:test` evidence recorded immediately below, these executions satisfy the
remaining AC5 and AC8 live evidence for `batter-5pm`. The containers were local
test infrastructure, not application provisioning or deployment evidence. No
macOS, TLS, hosted CI or publication claim is added.

## Rebase reconciliation onto 4e3e45b, 2026-09-11

The local HTTP lifetime and typed-settings changes are reconciled with the
upstream session-observation delivery. The live runner retains all 37 upstream
cases plus the three configuration cases. Both endpoint roots share the native
validator; the SQLx preflight retains the upstream PostgreSQL 18, primary
superuser/catalog-lock/autovacuum/track_counts and distinct-cluster checks.
Invalid secondary endpoints fail before either connection is opened. Cluster
identifiers retain the signed bigint domain, including both extrema.

Linux x86_64, kernel `7.0.11-76070011-generic`; Rust 1.98.1
(`48a229cea 2026-09-01`), Cargo 1.98.1 (`797e8a9bc 2026-08-05`), and
Rust 1.94.0 (`4a4ef493e 2026-03-02`), Cargo 1.94.0
(`85eff7c80 2026-01-15`). Cargo regenerated the merged lockfile; all locked
package versions and sources match the upstream lockfile. Only dependency
edges changed. Its SHA-256 is
`86b82ac103a8447dcfeb10ab834b620ac2492d4bb405681a0b6e71413b706d49`.

| Command | Result |
| --- | --- |
| `cargo test -p batter-example-reference-service --test configuration --locked` | PASS: 20 tests. |
| `cargo test -p batter-example-reference-service --example reference_preflight --locked` | PASS: three tests, including the native child rejection controls and signed cluster policy. |
| `cargo test -p batter-example-reference-service --test reference_live --locked -- --ignored --list` | PASS: all 40 required cases discovered; no live case executed. |
| `bash scripts/verify.sh` | PASS: 764 Rust executions, 81 summaries, 53 intentional live ignores; Python controls, formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete matrix, counts, lint and rustdoc checks. |
| Rebuild `http_service` on each toolchain, then run all five `scripts/smoke_http.py` profiles from testing.md | PASS: all ten smokes. |

The first preflight test extraction placed its subprocess entry in a nested
module, so the exact root selector executed zero tests and the focused check
failed. Keeping the entry at the crate root repaired that failure; the final
focused check and full matrix pass. The first Jig run's commands completed, but
its receipts were rejected because documentation and tracker edits overlapped
the read-only run. A stable worktree rerun under
`plan_01M27NF33BB8C5388C8G5F5F9D` passes all five required targets on Rust
1.98.1. Its successful `api:test` receipt is
`receipt_01M27P27T2ZZ67BEJHPYZNWQYG`; both `work evidence` and `work gates`
confirm fresh passing evidence. The final documentation-only update refreshes
whole-repository policy checks while reusing unchanged Rust receipts.

The Beads database was reconciled in place with `br sync --reconcile` after a
read-only dry run and backups. All 88 issues and upstream comment contents are
preserved; only `batter-5pm` receives the combined acceptance note. Upstream
`batter-kjl` remains closed. At this checkpoint `batter-5pm` remained in progress
for AC5/AC8 because neither required PostgreSQL endpoint was configured and the
combined 40-case suite had not run. The live acceptance record above supersedes
that pending state. Earlier 19- and 37-case results retain their historical scope.

## Configuration prerequisites and acceptance status (batter-5pm), 2026-09-11

The ordinary reference configuration target requires enabled IPv6 loopback
(`::1`) for its native protocol fixture, alongside IPv4 loopback and Unix
subprocess permissions. The testing guide and reference package guides now state
that prerequisite explicitly. No test was ignored, skipped or weakened.

`batter-5pm` is reopened: the current handoff has not passed the nineteen-case
live suite after its URL and fixture-caller corrections. AC5 and AC8 remain
unchanged and pending for live revalidation on both supported toolchains.
The implementation is delivered with offline evidence; the earlier live runs
remain historical. Beads confirms `batter-kpd`, `batter-0cp` and `batter-7r3.6`
are not ready while this delivery prerequisite remains open.

Only documentation and tracker state changed. The Rust source, dependencies,
test commands and configuration match the preceding validated working files.
Rust/Cargo 1.98.1 and 1.94.0 version identities were rechecked and match the
records below. The preceding two full verification matrices and ten HTTP smokes
remain the latest such executions; they were not repeated for these text changes.

`cargo test -p batter-example-reference-service --test configuration --locked
native_ipv6_connection_sends_exact_decoded_credentials` passes on Linux x86_64
with Rust 1.98.1: one executed test, nineteen filtered out. The fresh native
review also ran all five `test_reference_live.py` controls successfully with
Python bytecode writes disabled. No PostgreSQL endpoint is configured, no live
case was rerun and no database was provisioned. No macOS, TLS or hosted execution
claim is added.

Jig follow-up plan `plan_01M27GHQXGY9DN4PET2RF2EWN6` in the temporary validation
checkout executes and passes all five required targets, including a fresh Rust
test matrix (`api:test` receipt `receipt_01M27GNVNS9M3YTJ5GG7319EVJ`), formatting,
Clippy, contract and file-budget checks. It uses Rust 1.98.1,
`PYTHONDONTWRITEBYTECODE=1` and
`CARGO_TARGET_DIR=/home/aa/Documents/batter/target`. The checkout has the same
included working files; `.agent` in this repository remains review-excluded and
untouched. This local gate does not satisfy the pending live acceptance.

## Native live handoff boundary correction (batter-5pm), 2026-09-10

This record supersedes the hostname-only handoff evidence below. The private
live boundary rejects IPv6 literals before acquisition: SQLx 0.9.0 retains the
URL's brackets in its TCP host, which fails native tuple-address resolution.
The root's direct native-options IPv6 wire test still passes. Supported live
URLs preserve the selected hostname, database, TLS mode and password across
SQLx and the harness's tokio-postgres parser. Explicit empty passwords prevent
fixture pools from consulting an ambient passfile. All special fixture roots
now use the same handoff, including acquisition and cleanup-failure probes.

The native parser regressions cover encoded loopback names, leading-slash and
percent-encoded database names, case-insensitive input TLS spelling, credentials
and application names. A real preflight child checks the IPv6 rejection reason.
The existing hostile-passfile child now also checks the fixture handoff; that
assertion failed against the previous handoff and passes after the correction.
The status row now identifies the nineteen-case live runs as historical.

Linux x86_64; Rust/Cargo 1.98.1 and 1.94.0 as recorded below. Cargo.lock and its
recorded SHA-256 are unchanged.

| Command | Result |
| --- | --- |
| `cargo test -p batter-example-reference-service --test configuration --locked` | PASS: all 20 tests, including native parser, passfile and direct IPv6 wire checks. |
| `cargo test -p batter-example-reference-service --example reference_preflight --locked` | PASS: both tests, including the actual preflight child. |
| `bash scripts/verify.sh` | PASS on the final caller changes: 756 Rust executions, 81 summaries, 32 intentional live ignores; Python controls, formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete matrix and counts. |
| Rebuild `http_service` on each toolchain, then run all five `scripts/smoke_http.py` profiles from testing.md | PASS: all ten smokes. |

Jig's temporary validation checkout contains the same included working files;
the review-excluded `.agent` state in this repository remains untouched. Plan
`plan_01M26MXSY8Y9HWPB81K76CQYXN` passes all five required targets and the
`verify` gate with Rust 1.98.1, `PYTHONDONTWRITEBYTECODE=1` and
`CARGO_TARGET_DIR=/home/aa/Documents/batter/target`. The final-source `api:test`
receipt is `receipt_01M26N7B8RSP6PWNJX5W54WYSD`; receipts reside in that temporary
checkout, not this repository's excluded current-plan state.

No PostgreSQL endpoint is configured, so live authentication and the nineteen
live cases were not rerun. No macOS, TLS negotiation, hosted CI or deployed
adoption evidence is added.

## Live endpoint hostname correction (batter-5pm), 2026-09-10

The live handoff now serializes the validated loopback hostname before passing
its URL to the harness. Previously `local%68ost` validated as `localhost` for
preflight but remained encoded in SQLx fixture pool options. The regression
first failed on that exact mismatch, then passed for encoded DNS and IPv4 names.
It models the pinned harness's `Url` normalization and fixture database-path
replacement and independently checks host, port, username and database. Existing
credential cases and IPv6 authority preservation still pass. The native
credential test now also includes the harness's initial `Url` normalization.

Linux x86_64; the same Rust/Cargo 1.98.1 and 1.94.0 versions recorded below.
Cargo.lock remains unchanged, SHA-256
`ebfcb6780d104dd56bd7bb6c4b2143dbaac1225d34e96b39e72c9dde38d6b9cc`.

| Command | Result |
| --- | --- |
| `cargo test -p batter-example-reference-service --test configuration --locked live_preflight_and_fixture_share_the_native_endpoint_policy` before the repair | Expected failure: fixture host `local%68ost` differs from expected `localhost`. |
| `cargo test -p batter-example-reference-service --test configuration --locked` after the repair | PASS: all 20 tests. |
| `bash scripts/verify.sh` | PASS: 756 Rust executions, 81 summaries and 32 intentional live ignores; Python controls, formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: the same complete matrix and counts. |
| Rebuild `http_service` on each toolchain, then run all five `scripts/smoke_http.py` profiles from testing.md | PASS: all ten smokes. |

Jig verification uses a temporary checkout of the same working files to preserve
the review's exclusion of `.agent` in this repository. The temporary plan is
`plan_01M26KEKZC2DHYE4NN7NJ5WZFP`. The first attempt ran all five targets
successfully but returned incomplete `source_raced` freshness evidence despite
unchanged global source digests. The temporary index was refreshed and Python
bytecode writes disabled for a bounded recheck, which passed all five targets
and the required `verify` gate. Both attempts used Rust 1.98.1 and
`CARGO_TARGET_DIR=/home/aa/Documents/batter/target`; the successful attempt also
set `PYTHONDONTWRITEBYTECODE=1`. Receipts remain in the temporary checkout;
no current-plan receipt was written into this repository's excluded state.

No live endpoint is configured, so the nineteen-case live suite was not rerun.
The earlier live results remain historical; this correction adds no live
authentication, TLS, macOS, hosted CI or deployed-adoption evidence.

## Live endpoint query credential correction (batter-5pm), 2026-09-10

The private preflight/fixture URL handoff now encodes query spaces as `%20` for
the harness's native parser, retaining literal userinfo plus signs and existing
percent escapes. The regression uses the actual locked SQLx 0.9.0 and
tokio-postgres 0.7.18 parsers in a cleared child. Its first run against the old
handoff failed with `+` (byte 43) where the expected password had a space
(byte 32); the correction passes all seven password cases and application-name
checks. The only new dependency edge is a reference-package dev dependency on
the already resolved tokio-postgres version. Cargo regenerated the lockfile;
no package version changed.

Linux x86_64; rustc 1.98.1 (`48a229cea`, 2026-09-01), Cargo 1.98.1
(`797e8a9bc`, 2026-08-05); rustc 1.94.0 (`4a4ef493e`, 2026-03-02), Cargo
1.94.0 (`85eff7c80`, 2026-01-15). Cargo.lock SHA-256:
`ebfcb6780d104dd56bd7bb6c4b2143dbaac1225d34e96b39e72c9dde38d6b9cc`.

| Command | Result |
| --- | --- |
| `cargo test -p batter-example-reference-service --test configuration --locked` | PASS: 20 tests, including native credential handoff. |
| `bash scripts/verify.sh` | PASS: 756 Rust executions, 81 summaries, 32 intentional live ignores; Python controls, formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS: same counts and checks, including hostile-parent PG* configuration tests. |
| Rebuild `http_service` with `cargo build -p batter-axum --example http_service --locked` on each toolchain, then run the five `scripts/smoke_http.py` profiles from testing.md | PASS: default, SIGINT, deadline, warn-filter and warn-filter+deadline on both rebuilt binaries (ten smokes). |

Jig evidence for this correction belongs to
`plan_01M26GQD8JZJX1N14XVF925G62`; managed work evidence and gates record the
current receipt conclusions. No live PostgreSQL endpoint is configured for
this correction, so live authentication/the nineteen-case suite were not rerun.
The live results below remain historical evidence. This parser regression adds
no macOS, TLS, hosted CI or deployed-adoption claim.

## Documentation consistency follow-up: 2026-09-11

The complete review matched fingerprint
`a7a10600a99f7987b7d2d3c0186753f664534a1f54598887d5682c060c741740`.
Codex and Cursor found no actionable defects. Opus found two low documentation
drift issues: an obsolete prerequisite list in the compatibility guide and a
fixture contract paragraph present only in the package agent guide. The
compatibility guide now links to the canonical testing prerequisites. The package
README owns the full fixture contract; the agent guide links there and retains
its source ownership map. This removes duplicated contract text that had drifted.

This follow-up changes documentation only. The immediately preceding final7
macOS/Linux Rust, live, HTTP, Python and repeated timing evidence remains applicable:
no Rust/Python sources, test commands, toolchains, dependency graph or server
prerequisites changed. Fresh Jig receipts and the final independent review are
recorded in the owning plan before closure. SQLx parsing research and the narrower
unisolated lazy-parse failure path are recorded in [references](references.md).
Linux MSRV/full-matrix, hosted execution and process-death cleanup remain unverified.

## Signed cluster identity and repeated timing controls: 2026-09-10

The preceding complete review matched fingerprint
`45bb648c3ecf03c3f2d24658c416016b8e359d9dedf9e1d084668c488556e6c0`.
Codex and Cursor found no actionable defects. Opus identified one low parser
mismatch: pg_control_system exposes a signed bigint, but preflight accepted only
unsigned digits. Primary-source research is recorded in [references](references.md).
The runner now parses bounded signed64-bit decimal values. Python controls cover
negative identifiers, both extrema, malformed/oversized output and failed
prerequisite rows with valid identifiers; the primary command control also retains
the track_counts predicate. There are still **ten Python controls**.

Both macOS arm64 matrices passed on Rust **1.94.0** and **1.98.1**: **655 Rust
test/doctest executions, zero failures, 50 intentional live ignores and 73 Rust
summaries each**, plus formatting, Clippy, rustdoc and all Python controls. Both
strict live runs executed **37 cases**, zero ignored/filtered, in **30.90 s** and
**29.37 s** respectively. Both rebuilt HTTP examples passed all five smoke modes,
**ten total**. Native Linux arm64 Rust1.98.1 passed all **37 live cases in 38.64 s**.
Client and both PostgreSQL18.4 servers remained limited to **0.5 CPU each**.

On that constrained Linux setup, ten serial repetitions each of the restricted-role
and autovacuum controls passed: **20 additional live executions, zero failures**.
Each two-case invocation completed in2.39–2.70 seconds. This is bounded repeated-run
evidence, not an arbitrary scheduling guarantee. No observation deadline changed.
The native retry loop remains tested against PostgreSQL in the explicit live suite;
ordinary verification intentionally does not substitute a simulated implementation.

Exact command vectors and outcomes are in `.agent/tmp/fixture-gaps/final7/matrix.py`,
`matrix.log`, per-command logs, `linux-live-half-cpu.log`, `stress.py`, `stress.log`
and ten per-iteration logs. Rust sources, dependency graph and toolchain/container
identities are unchanged from the preceding matrix. Linux MSRV/full-matrix and
hosted CI remain unverified. Prior api:test receipt
`receipt_01M26M3QHZZGHANT1TEKCECZ10` is historical for the changed runner; fresh Jig
receipts and final review evidence are recorded in the owning plan before closure.
Task-owned containers are removed after final review. No staging or commit is performed.

## Startup barrier and preflight controls: 2026-09-10

The preceding complete review matched fingerprint
`2d76fd09208bc65f697da975c1c74f27dbb15ec861f4a67a004c5d5518e47716`.
Codex and Cursor found no actionable defects. Opus identified two low test issues:
pending elapsed time alone did not prove the startup connection blocked DROP's
process barrier, and preflight omitted the track_counts autovacuum prerequisite.
The startup control now witnesses the actual target DROP backend waiting on
ProcSignalBarrier and rechecks that wait before releasing its raw SCRAM connection.
The strict runner requires track_counts before inventory. Primary-source research
and the dedicated serial server prerequisite are recorded in [references](references.md).

A real negative invocation with `PGOPTIONS='-c track_counts=off'` was rejected with
exit1 before inventory or fixture execution. A separate raw connection measurement
confirmed that Docker forwarding changed frontend port50977 to server-visible
port65048; the connection was closed afterward. Thus socket.local_addr is not used
as a PostgreSQL backend identity. These are bounded executed controls, not claims
about arbitrary proxies or concurrent server activity.

Both unchanged-source macOS arm64 matrices passed on Rust **1.94.0** and **1.98.1**:
**655 Rust test/doctest executions, zero failures, 50 intentional live ignores and
73 Rust summaries each**, plus formatting, Clippy, rustdoc and **ten Python
controls**. Each strict live run executed all **37 cases**, zero ignored/filtered,
in **30.45 s** and **29.13 s** respectively. Both rebuilt HTTP binaries passed five
smoke modes, **ten total**. The rebuilt native Linux arm64 Rust1.98.1 client passed
all **37 live cases in 38.95 s**, with client and both PostgreSQL18.4 servers at
**0.5 CPU each**, under the unchanged 180-second watchdog.

Exact command vectors and terminal outcomes are in
`.agent/tmp/fixture-gaps/final6/matrix.py`, `matrix.log`, per-command logs,
`linux-build.log`, `linux-live-half-cpu.log`, `track-counts-rejected.log` and
`startup-port-mapping.log`. Dependency graph, lock hash and container versions
remain unchanged from below. Linux MSRV/full-matrix and hosted CI remain
unverified here. Prior api:test receipt `receipt_01M26JS8QYAB565W4TNQM4QBC3` is
historical after this correction; fresh Jig receipts and final review records are
stored in the owning plan before closure. Task-owned containers are removed after
final review. The pre-existing index is preserved.

## Autovacuum witness correction: 2026-09-10

The complete unchanged-scope review fingerprint was
`c00b392a0432e36e7559cd6ceb243ed61be8005691a3ba681e3855bffee9972d`.
Codex and Cursor found no actionable defects. Opus identified one low test race:
database-wide autovacuum worker selection could capture an unrelated, short-lived
worker. The test now records its table's relation OID, selects that actual vacuum
through pg_stat_progress_vacuum with most heap scanning remaining, and rechecks
the same pid/database/relation after the observation timeout. Primary-source
research and the bounded scheduling limitation are recorded in [references](references.md).

Both final macOS arm64 `scripts/verify.sh` matrices passed on Rust **1.94.0** and
**1.98.1**: **655 Rust test/doctest executions, zero failures, 50 intentional live
ignores and 73 Rust summaries each**, plus formatting, Clippy, rustdoc and **ten
Python controls**. All **37 live cases** ran with zero ignored/filtered in
**31.08 s** and **29.26 s**, respectively. Both rebuilt HTTP binaries passed the
five smoke modes, **ten total**. The rebuilt native Linux arm64 Rust 1.98.1 client
passed all **37** live cases in **38.16 s** with client and both PostgreSQL 18.4
servers at **0.5 CPU each**, under the unchanged 180-second watchdog.

Exact commands and terminal outcomes are in `.agent/tmp/fixture-gaps/final5/matrix.py`,
`matrix.log`, per-command logs, `linux-build.log` and `linux-live-half-cpu.log`.
The dependency graph, lock hash and container versions are unchanged from below.
Linux MSRV/full-matrix and hosted CI remain unverified here. Earlier api:test receipt
`receipt_01M26HQFKQM780SVYQB5AMC92H` is historical after this correction; fresh Jig
and final review records are stored in the owning plan before closure. Task-owned
containers are removed after final review. The pre-existing index is preserved.

## Fixture recovery access and diagnostic counts: 2026-09-10

The next independent pass completed on unchanged fingerprint
`e650c0fecdf7da8aa017d37964d34742d0da80c09d97364995d950a94cd2c1dd`.
Codex and Cursor had no actionable findings. Claude Opus identified two low issues:
the completion helper hid pools needed to repair an inside-target observer, and
database_failures conflated failed consuming cleanup with retained recovered errors.

The pending owner now exposes its retained native observer pools as a read-only
slice for explicit repair. The wrong-target regression moves its sole misplaced
pool handle into the helper, recovers pending, closes that retained pool, witnesses
backend exit and retries the same owner. Automatic closure of arbitrary replaced
pools would disrupt active attempts or shared runs, so native closure stays explicit.
Report summaries now separately count failed consuming cleanups, pool failures and
observation failures. Offline, live and diagnostic controls verify that recovered
errors keep the report unsuccessful without implying a failed DROP. A positive
runner-entry control follows distinct cluster identities through inventory and full
execution. There are **ten Python controls** and still **37 live cases**.

Open-question research is recorded in [references](references.md): same-server
identity and native catalog visibility remain caller preconditions; name presence
alone does not identify a cluster. Prepared transactions, active logical slots and
subscriptions can prevent DROP after a session-absence witness. Authentication of
an admin connection cannot certify database-specific pg_hba rules; the actual
disposable-database SCRAM exchange remains the live oracle. No new runtime-death
or general async-drop guarantee is added.

Final unchanged-source macOS arm64 runs passed on Rust **1.94.0** and **1.98.1**:
**655 Rust test/doctest executions, zero failures, 50 intentional live ignores and
73 Rust summaries each**, plus formatting, Clippy, rustdoc and ten Python runner
controls. Both strict live runs executed all **37** cases with zero ignored/filtered
in **30.32 s** and **29.07 s** respectively. Both rebuilt HTTP examples passed all
five smoke modes (**ten total**). Commands and terminal outcomes are in
`.agent/tmp/fixture-gaps/final4/matrix.py`, `matrix.log` and the per-command logs.
The intermediate final3 matrix was superseded because the accessor was finalized
while it ran; it is not final unchanged-source evidence.

The rebuilt native Linux arm64 Rust 1.98.1 client passed all **37** live cases in
**38.93 s** with the client and both PostgreSQL 18.4 servers limited to **0.5 CPU
each**, under the same 180-second watchdog. Logs are in `final4/linux-build.log`
and `final4/linux-live-half-cpu.log`. Versions, image and lock hash remain as below.
Linux MSRV/full-matrix and hosted CI execution remain unverified here.

Earlier api:test receipt `receipt_01M26G7P8WPEN6RA6E1NC9ZFVB` is historical after
these corrections. Fresh final Jig/review records are stored in the owning plan
before closure. Task-owned containers are removed after the final review; the
pre-existing index remains unchanged and no code commit is authorized.

## Fixture coverage final follow-up: 2026-09-10

The next complete all-reviewer snapshot was
`0732a9c91e30ae2e5f55dddf866e18051869c14d4dca4aa66dff2dad2bafe40b`.
Codex and Cursor found no actionable defects; Claude Opus found three low test/doc
issues. The replacement-close regression now awaits the empty driver before its
short close bound. Temporary LOGIN credentials are random with a five-minute
server-clock password expiry; successful login and finite expiry are checked,
and the existing assertion-panic control still requires explicit role removal.
Ownership descriptions now identify the advisory-lock gate and admission liveness
limit. The active-retry tests also recover the shared bounded completion owner on
both Tokio runtimes. Python now has **nine** controls, including full-entry rejection
of remote, TLS-required and host-override secondary URLs. The live inventory remains
**37 cases: four compatibility and thirty-three fixture cases**.

Primary-source research and a rolled-back ordinary-role query on PostgreSQL 18.4
confirmed public pg_control_system access by default. Docs explain an explicit
grant only when that access has been revoked. See [references](references.md).
The dependency graph and lock hash remain unchanged from the preceding entry.

Refreshed macOS arm64 verification passed on both Rust **1.94.0** and **1.98.1**:
**655 Rust test/doctest executions, zero failures, 50 intentional live ignores and
73 Rust summaries each**, plus formatting, Clippy, rustdoc and nine Python runner
controls. All **37** live cases ran with zero ignored/filtered, in **30.53 s** and
**29.42 s** respectively. Each compiler rebuilt the HTTP example and passed the
five smoke modes, **ten total**. Exact commands and exit statuses are retained in
`.agent/tmp/fixture-gaps/final2/matrix.py` and `matrix.log`, with per-command logs.

The same Linux arm64 Rust 1.98.1 client and both PostgreSQL 18.4 containers from
the preceding entry remained limited to **0.5 CPU each**. After rebuilding the test
binary, the strict live runner passed all **37** cases in **38.22 s** under the
unchanged 180-second watchdog. Logs: `final2/linux-build.log` and
`final2/linux-live-half-cpu.log`. Linux MSRV, the full Linux matrix and hosted CI
remain unverified here. Task-owned containers are removed after the final review.

The earlier api:test receipt `receipt_01M26F63NK29KXY2T3AWEGBCVS` is historical
after these corrections. Fresh final Jig evidence and independent review records
are stored in plan `plan_01M26CV7EZ9G4YSG5R0M9ZTVQ9` before closure. No application
code is staged or committed by this work; the pre-existing index is preserved.

## Fixture coverage review corrections: 2026-09-10

Follow-up for `batter-kjl`, plan `plan_01M26CV7EZ9G4YSG5R0M9ZTVQ9`. The
independent Claude Opus/Codex/Cursor pass matched complete fingerprint
`bf8cb67aea9e7b068a3fd04bbe0aa4256fcd9c186aa14b9dbe7fd8edc7f3a033`.
Codex and Cursor had no actionable findings. Claude identified four low issues:
sequential replacement-pool close, assertion unwinding past temporary role cleanup,
a stale fixture count, and missing distinct-cluster preflight. All four are fixed.

Completion now polls all retained native pool closes concurrently. A held original
checkout cannot postpone starting replacement closure; the regression resumes
the same cached outcome after releasing it. A native Tokio task contains assertions
in the restricted-login probe, while its outer owner joins that task, closes the
observer pool and drops the role before interpreting failure. A deliberate panic
must remain a native JoinError after the role is confirmed absent. The visibility
probe now uses an actual unprivileged LOGIN role, not SET ROLE. Preflight compares
pg_control_system system identifiers and rejects endpoint aliases for one cluster
before fixture execution. Both endpoints need permission to call that function.
The manifest and strict inventory now agree on **37 cases: four compatibility
and thirty-three fixture cases**. Python prerequisite/inventory controls total eight.

Cargo added only the reference package's direct development dependency edge to
already-resolved futures-util 0.3.34 for join_all; no native version changed.
Final Cargo.lock SHA-256:
`733b324bc403b92b39f792663dc751dc6ac952b5f0f8c02a52b0aa53c11ff303`.

Both full macOS arm64 `scripts/verify.sh` runs passed on Rust **1.94.0** and
**1.98.1**: **655 test/doctest executions, 73 Rust summaries, zero failures and
50 intentional live ignores each**, plus formatting, Clippy and rustdoc.
Both strict live runs passed all **37 cases** with zero ignored/filtered, in
**30.07 s** and **29.18 s**, respectively. Both rebuilt HTTP examples passed all
five smoke modes, **ten total**, with the same commands as the preceding entry.
The exact executable command driver is
`.agent/tmp/fixture-gaps/followup/final_matrix.py`; its final-matrix.log records
successful exit status for every verification, live, build and smoke invocation.

Replacement task-owned PostgreSQL 18.4 containers are `batter-gap-final-db` and
`batter-gap-final-other`, on the same dedicated loopback ports and SCRAM settings
as below. The Linux arm64 client is `batter-gap-final-linux`, using the same
Rust 1.98.1 image and read-only workspace mount. After compilation, all three
containers were limited with `docker update --cpus=0.5`; the complete strict
**37-case** Linux live run passed in **38.42 s**. This measures CPU-constrained
clients and servers under the unchanged 180-second watchdog, not arbitrary
network failures, server saturation, Linux MSRV or hosted CI. Quotas are recorded
in `followup/container-quotas.txt`; logs include `linux-build.log` and
`linux-live-half-cpu.log`. The earlier task-owned containers were removed; these
replacement containers are removed after final review/evidence collection.

The earlier Jig check passed all five gates with api:test receipt
`receipt_01M26DZB0TMEKVJEHTN191C0NX`; it is historical after these corrections.
Fresh final work-check, evidence/gates and independent review records are stored
in this plan before closure. No staging, commit or publication is authorized.

## Fixture observation coverage: 2026-09-10

Bead `batter-kjl`, plan `plan_01M26CV7EZ9G4YSG5R0M9ZTVQ9`, baseline
`486e0b0f9f4c4439077418715843b30042205f7e` plus existing uncommitted changes.
Added nine named live cases and two Python prerequisite controls. The inventory
is now **36 live cases**; template success already used SessionObserver, so the
new template case proves retained failure and recovery. The existing active-retry
oracle now uses acknowledged blocked PostgreSQL queries and runs on both Tokio
runtime flavors. Every retained native failure still survives recovery by identity.
No production API or dependency version changed. Tokio net/io-util features are
now explicitly declared by the reference package that uses the startup socket.
Cargo.lock SHA-256: `023f51eed6b46caed9e331e7d5b45374ddfb55d2ed14b1e32beeda52821e8a4f`.

Primary-source research and the startup experiment are recorded in
[references](references.md#fixture-observation-coverage-follow-up-2026-09-10).
Initial verification found ordinary SQLx 0.9 dynamic-SQL typing and a Clippy
complexity violation; audited numeric-only identifiers, typed error wrapping and
a separate assertion helper resolved them without suppressions. The first
36-case run passed 35 and failed the startup oracle because native DROP waited
on ProcSignalBarrier. The corrected test requires CleaningLease/pending while
the unassigned SCRAM backend remains alive, then closes it and verifies completion.
It never equates an absence witness or a native cleanup phase with completion.

Executed macOS arm64 results:

| Command | Result |
| --- | --- |
| `bash scripts/verify.sh` on Rust 1.98.1 (`48a229cea`) | PASS; 655 test/doctest executions, 73 Rust summaries, zero failures, 49 intentional live ignores; formatting, Clippy and rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS; same counts and checks. |
| `bash scripts/test_reference_live.sh` on each pinned toolchain | All 36 cases pass, zero ignored/filtered; 30.02 s on 1.94.0 and 28.88 s on 1.98.1. |
| `cargo build -p batter-axum --example http_service --locked` on each toolchain, followed by `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` in default, SIGINT, deadline, warn-filter and warn-filter+deadline modes | Both builds and all ten smoke executions pass. |
| `python3 -m unittest discover -s scripts -p test_reference_live.py -v` | Six controls pass, including missing-secondary and failed-secondary rejection before fixture execution; also run by both full matrices. |

Task-owned PostgreSQL containers `batter-gap-db` and `batter-gap-other` run
**PostgreSQL 18.4 Debian Linux arm64**. Primary and secondary endpoints are
`postgres://postgres:fixture@127.0.0.1:49618/postgres?sslmode=disable` and
`postgres://postgres:fixture@127.0.0.1:49619/postgres?sslmode=disable`, assigned to
`POSTGRES_TEST_ADMIN_URL` and `POSTGRES_TEST_OBSERVER_URL`. These are disposable
fixture credentials. Primary startup uses `postgres -c autovacuum_naptime=1s
-c max_connections=100`; both use SCRAM host authentication. Existing unrelated
containers were untouched. Live preflight now requires these explicit endpoints,
primary superuser authority and fast autovacuum; startup explicitly rejects a
non-SCRAM host handshake. No server provisioning was added to the libraries.

Native **Linux arm64** live execution used task-owned `batter-gap-linux`, image
`rust:1.98.1-bookworm` digest
`sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa`,
Rust 1.98.1, Cargo 1.98.1, kernel `6.12.76-linuxkit`, and psql 15.19.
The workspace was mounted read-only, with `CARGO_TARGET_DIR=/tmp/batter-target`.
Task-owned socat listeners forwarded container loopback ports 5432/5433 to the
two dedicated host endpoints; the same strict live runner executed all 36 cases.
The two-CPU run passed in **36.28 s**. After `docker update --cpus=0.5
batter-gap-linux`, the repeat passed in **38.10 s**, still within the unchanged
180-second watchdog. This measures client CPU restriction against the same
server, not arbitrary slow networks, overloaded servers, Linux MSRV or hosted CI.
The ordinary 30-second consumer policy remains unchanged; tests use explicit
bounds and controlled releases. Hosted execution remains unverified.

Logs and the exact final command driver are under `.agent/tmp/fixture-gaps/`:
`final_matrix.py`, `final-matrix.log`, `verify-{1.94.0,final-1.98.1}.log`,
`live-final-<toolchain>.log`, `http-<toolchain>-<profile>.log`, `linux-build.log`,
`linux-live.log` and `linux-live-half-cpu.log`. Task-owned containers are removed
after evidence collection; completion, final independent review and Jig
work-check/evidence/gates receipts are retained in this plan's append-only records.
No staging, commit, publication or hosted CI execution is part of this task.

## Fixture completion diagnostics and isolated report errors: 2026-09-10

Bead `batter-kjl`, plan `plan_01M2681XJ8YA22QEVZ69V7F499`, unchanged baseline
`486e0b0f9f4c4439077418715843b30042205f7e`. The next frozen review matched
fingerprint `116695266f69630e089c64638b5ae6a7661bdb438c77adbc3c786a9c9310ce79`.
Codex found no defects; Claude Opus found three low-severity follow-ups: pending
summary ambiguity after successful join, incomplete ObservationTimeout rustdoc,
and missing isolated offline coverage for retained error branches.

All three are corrected. The helper records an unobserved driver separately
from a joined driver and formats `driver_joined` plus `driver_failed`. A live
successful-join/held-session-checkout control distinguishes administrative close
from an unfinished driver and verifies that diagnostics stay usable after
recovery. Three offline report tests isolate handled pool errors, recovered
observation errors and source precedence while the body/acquisitions/drain
succeed. They verify unsuccessful outcomes, database counts and native source
identity. Timeout rustdoc now covers lock relation and session-absence attempts.
Pinned-source research confirms that parked leases retain shared admission
capacity; the adapter README, rustdoc and integration contract state the resulting
liveness limit. Untracked module files remain build/review inputs; no Git commit
or staging operation is part of this task.

Both full verification scripts passed on **macOS arm64**, Rust **1.98.1** and
**1.94.0**: **655 test/doctest executions, 73 summaries, zero failures and 40
intentional live ignores each**, plus formatting, warning-denied Clippy and
rustdoc. Both explicit live runs passed all **27 cases**, including the extended
terminal-completion regression. Both rebuilt HTTP binaries passed all five
profiles, **ten smokes total**. No dependency changes occurred in this iteration.

The replacement task-owned PostgreSQL **18.4 Debian Linux arm64** container was
`batter-kjl-terminal-round3`, exposed only at `127.0.0.1:63014`, and was stopped
and removed after execution. Exact commands are the same verification/build/smoke
commands in the following section, with this live endpoint:
`POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:63014/postgres?sslmode=disable' bash scripts/test_reference_live.sh`.
Logs are `.agent/tmp/batter-kjl-terminal-round3/{verify,live,http-build,http}-<toolchain>.log`.
The previous iteration's Jig gate passed with api:test receipt
`receipt_01M268JAYVYWS0N2FBF368HN32`; it is historical after these source changes.
The final `scripts/jig work check --plan-id plan_01M2681XJ8YA22QEVZ69V7F499 --json`
runs after this evidence update and writes `jig-check.json` in the new log
directory. Its exact final receipts, `work evidence`/`work gates` outcomes and
subsequent frozen Opus/Codex review results are retained in the same plan's
append-only records and checked before closure. No new Linux client or hosted
execution is claimed.

Final Jig verify passed all five targets with api:test receipt
`receipt_01M269J23XD0XA084HHXQHQBJP`; evidence/gates were fresh and passed.
Both final reviewers completed against unchanged, complete fingerprint
`5b302afc91c82d9f894b7a6aaf0b8bf8b5e4a07c927b604fa3fffc1224cec65a`.
Codex reported no findings. Claude Opus found no working-tree code defect; its
sole low-severity finding concerned hypothetical partial commits from the mixed
index/untracked state. That finding is not actionable in this full-working-tree,
uncommitted task, and the new module was included in review and verification.
No index changes were made to satisfy it. The remaining background-worker and
runner-timing questions are researched in references. No actionable findings
remain after adjudication. Subsequent edits only record research/review outcomes
and tracker completion; the verified Rust/test sources and dependencies are
unchanged. The documentation/metadata edits require fresh file-budget evidence. Targeted
refresh was unavailable: `jig check --plan-id ... repo:file-budget` rejected a
prepared/executed plan identity mismatch, and `work check --tool jig.file_budget`
reported an unsupported native tool kind. The supported full work-check profile
is therefore rerun for final closure; no test assertion failed in these CLI
errors. Its final output is `final-jig-check.json` in the round3 log directory.

## Terminal fixture-driver admin closure: 2026-09-10

Owning Bead: `batter-kjl`; follow-up plan `plan_01M2681XJ8YA22QEVZ69V7F499`;
Git baseline `486e0b0f9f4c4439077418715843b30042205f7e` plus the existing fixture
working changes. The prior final review retained one low-severity defect:
`ObservedRun::finish` propagated driver JoinError before either caller closed
its diagnostic pool. This was a private ownership split, not a defect in native
lease retention. Closure now belongs to the completion owner on that error path.
The same bound covers driver observation and administrative closure; pending
retains the cached error and both capacities for resumable close. Success still
leaves diagnostics available for catalog checks. No native-error or public
adapter API changes were made.

Research in [references](references.md#fixture-terminal-completion-and-review-questions-2026-09-10)
answers the runtime-loss, missing-target and shared-retry questions against
SQLx 0.9.0, Tokio 1.53.1, pinned harness `3d525e6fc5745ce2e2437c7997de5cccdecff4ac`
and PostgreSQL 18. These are explicit limited contracts. The example now marks
its consuming final wait as unbounded; dedicated cross-run/pre-first-attempt
coverage remains absent rather than implied by the existing two-database test.

The new live test uses actual driver-runtime destruction before first poll and
administrative pools on a surviving runtime. With held checkouts, both closes
start, bounded finish returns the retained owner, and release/resume returns the
same cancelled task ID only after both pool sizes reach zero. Without held
checkouts the terminal error returns after both closes. The focused test and
all **27 explicit live cases** passed on Rust 1.98.1.

Validation ran on macOS arm64, Rust 1.98.1 (`48a229cea`) and 1.94.0
(`4a4ef493e`). The unchanged dependency graph uses a task-owned PostgreSQL
**18.4 Debian Linux arm64** container, `batter-kjl-terminal-review`, exposed only
at `127.0.0.1:61307`. This server does not establish Linux Rust-client evidence.
Logs are in `.agent/tmp/batter-kjl-terminal/`:

- `bash scripts/verify.sh` -> `verify-1.98.1.log`
- `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` -> `verify-1.94.0.log`
- `POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:61307/postgres?sslmode=disable' bash scripts/test_reference_live.sh`
  on each toolchain -> `live-<toolchain>.log`
- `cargo build -p batter-axum --example http_service --locked`, then
  `python3 scripts/smoke_http.py --binary target/debug/examples/http_service`
  with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and
  `--warn-filter --deadline`, on each toolchain -> `http-<toolchain>.log`
- `scripts/jig work check --plan-id plan_01M2681XJ8YA22QEVZ69V7F499 --json`,
  followed by `work evidence` and `work gates` -> `jig-check.json` and plan records.

Both complete verification scripts passed: **652 test/doctest executions,
73 summaries, zero failures and 40 intentional live ignores per toolchain**, plus
formatting, warning-denied Clippy and rustdoc. Both explicit live runs passed all
**27 cases**, and both rebuilt HTTP binaries passed all five profiles (**ten
smokes total**). The task-owned container was stopped and removed afterward.
The first full verification passed its test matrix but failed Clippy on the new
regression's complexity (26/20). Extracting terminal assertions alone still gave
21/20; separating the per-case setup resolved it without suppressions or weaker
assertions. The complete runs above followed that correction.

The final Jig command and independent Opus/Codex review run after this evidence
update. Their exact receipt and frozen reports belong to the named plan's
append-only records and `.agent/tmp/batter-kjl-terminal/`. This section claims no
Jig or review success in advance; those outcomes are checked before Bead closure.
No commit, push, publication or hosted execution is claimed.

## Fixture progress privacy and final pool transfer: 2026-09-10

A further independent review found a redundant session-pool close in the detached
probe's outer wrapper, which could close a pool retained by a pending error. That
wrapper now transfers the pool directly to the completion owner and never closes
it afterward. `DatabaseProgress` Debug now prints only phase and failure count;
an offline regression rejects database-name and native-error disclosure. Its
public fields retain deliberate inspection. The detached-session attempt was
increased from 200 ms to two seconds to give independent backend exit more margin.

After these changes, both complete verification scripts passed on macOS arm64,
Rust 1.98.1 and 1.94.0: **652 test/doctest executions, 73 summaries, zero failures,
39 intentional live ignores per toolchain**, plus formatting, warning-denied
Clippy and rustdoc. Both explicit live runs passed all **26 cases**, and both
rebuilt HTTP examples passed all five smoke profiles (**ten total**). The final
task-owned PostgreSQL 18.4 container exposed only 127.0.0.1:58500 and was stopped
and removed after execution. Exact commands and final logs remain under
`.agent/tmp/batter-kjl-review/`. Earlier sections retain their prior-round counts;
no new Linux client or hosted execution is claimed.

The working-tree review includes the newly added completion/retry source files.
All commands above and the reviews included those new build inputs. Git staging
status is independent of that execution evidence. No commit or publication was
performed by that remediation. Tracker closure follows the completed review loop; the
implemented-status row describes the capability rather than claiming a closed
review task. The documented 30-second consumer wait is intentional, including
for compatibility probes; a slow overrun fails with retained pending ownership
and retains the stated runtime-teardown limitation.

## Fixture completion boundary follow-up: 2026-09-10

The next independent review of the full diff identified the diagnostic pool
still closing outside the bounded pending path. The common helpers now return
the typed pending error immediately; it retains both administrative capacities,
and only actual body/run completion permits diagnostic close. The private recovery
method retains every replacement session pool until driver completion so callers
cannot update the retry control while forgetting pool ownership. The live outer
helper regression holds a real diagnostic checkout, requires pending return before
an explicit body release, then resumes that same body and verifies lease cleanup.

Also corrected the public retry example to inspect completed cleanup separately
from retained observation failures; combined detached-test release/close/absence
errors after bounded completion; and increased deliberate active-attempt starvation
to a three-second attempt window. The reference assertion boundary's runtime-death
behavior is explicit: an unrecovered pending error followed by test panic can
trigger native FORCE deletion. Returning pending does not promise survival across
runtime destruction. Missing/wrong-target observation remains pending by design.

On the same macOS arm64/toolchains and unchanged dependency graph described below,
final `bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
both passed: **651 test/doctest executions, 73 summaries, zero failures, 39 intentional
live ignores each**, plus formatting, warning-denied Clippy and rustdoc. All
**26 required live cases passed on each toolchain**, and freshly rebuilt HTTP
examples passed all **ten smoke invocations** across the five profiles per compiler.

The replacement task-owned PostgreSQL 18.4 container used only 127.0.0.1:57010 and
was stopped/removed after the two runs. Logs and exact commands under
`.agent/tmp/batter-kjl-review/` now describe this final snapshot. Focused Clippy
initially rejected a Result fold; using an explicit loop preserves all errors
without the suggested short-circuiting try_fold or a lint suppression. Final file
budget has only a nonblocking 475-line notice for fixture_failures.rs. The prior
25-case evidence below is historical. Jig evidence and final independent review
are tracked under `plan_01M26254CNX0RB5MJ0QYYD2H0Z`; no commit or publication is
included in this work.

## Fixture completion review remediation: 2026-09-10

Bead `batter-kjl`; plan `plan_01M26254CNX0RB5MJ0QYYD2H0Z`; baseline
`486e0b0f9f4c4439077418715843b30042205f7e`. Independent Claude Opus and Codex
reviews found unbounded waits in shared observed-fixture helpers; Claude also
found competition between detached-session diagnostics and the observer pool.
The cause was consumer protocol composition: the retained cleanup driver needs
explicit recovery, while those helpers exposed only an unbounded finish.

The private shared `ObservedRun::finish` now returns a bounded pending error
retaining the actual run, retry control and dedicated session pool. Its automatic
summary contains phases/counts, and the caller can downcast, repair and resume.
Completed runs explicitly close that session pool. Both common helpers use this
path with a 30-second whole-run wait. No automatic retry/deletion was introduced.
Detached-session tests separate diagnostic capacity, acknowledge blocking before
body exit and witness server-session absence before retry. Public retry docs now
state the existing broadcast/coalescing semantics. Primary-source research and
native FORCE deletion are recorded in [references](references.md).

Executed on macOS arm64 with Rust/Cargo 1.98.1 and 1.94.0, using the unchanged
SQLx 0.9.0 / harness `3d525e6fc5745ce2e2437c7997de5cccdecff4ac` graph. The task-owned
`batter-kjl-review-validation` container ran PostgreSQL 18.4 Debian on Linux
arm64, exposed only at 127.0.0.1:55014 using local trust. It was stopped and
removed after both live runs. Existing database servers were not used. The Linux
database container does not establish Linux Rust-client validation.

| Executed command | Outcome |
| --- | --- |
| `bash scripts/verify.sh` with Rust 1.98.1 | Passed: 651 Rust test/doctest executions in 73 summaries, zero failures, 38 intentional live ignores; formatting, warning-denied Clippy and rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed with the same counts and all checks. |
| `POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:55014/postgres?sslmode=disable' bash scripts/test_reference_live.sh`, on each toolchain | All 25 required cases passed, none ignored/filtered; inventory and privilege preflight passed. |
| Rebuild `http_service` and run default, SIGINT, deadline, WARN filter and WARN-filter/deadline smoke profiles on each toolchain | All ten HTTP process smokes passed. |
| `scripts/jig file-budget check --base HEAD`; `git diff --check` | Passed; only the existing nonblocking line-budget notice for fixture_failures.rs (now 477 lines), no warnings/errors or waivers. |

New live controls recover the original owner from a bounded pending error and
retain body SQLSTATE 22012 plus the same observation-error Arc. Another control
holds the observer pool connection intentionally, sends coalescing retries while
two observations are active, and checks both ordered timeout/PoolClosed histories
survive recovery. A separate offline diagnostic test retains its concrete cause
while printing only the supplied pending summary. No exhaustive scheduling,
Linux client or hosted execution claim is added.

Two development verification attempts passed their test matrices but failed
Clippy on the new recovery probe's complexity (21/20). Separating body setup alone
did not resolve it; extracting the retained-report assertions did. The subsequent
full verification passed without lint suppression or weaker assertions. Logs and
exact matrix/smoke commands are under `.agent/tmp/batter-kjl-review/`; Jig gate
receipts and review-loop progress belong to the named plan's append-only records.

## Fixture failure retention: 2026-09-10

Bead `batter-kjl`; plan `plan_01M25Y0VN2H93NVFSZNRQ8D14P`, baseline
`486e0b0f9f4c4439077418715843b30042205f7e`. Reused the existing owned runner,
added bounded/resumable observation, explicit session-observer retry and retained
pool/observation failure histories. The audit also fixed handled pool acquisition
errors disappearing from successful-body reports. Native causes share identity;
report formatting includes those failures without formatting their contents.

Executed on macOS arm64 (`aarch64-apple-darwin`), Rust/Cargo 1.98.1 and 1.94.0.
Rust commits: `48a229ceaefd4985c50990b14116b6d856af0985` and
`4a4ef493e3a1488c6e321570238084b38948f6db`. SQLx remains 0.9.0 and the harness
remains revision `3d525e6fc5745ce2e2437c7997de5cccdecff4ac`. Cargo generated the
sole lockfile change for the reference test's direct batter dependency; lockfile
SHA-256: `023f51eed6b46caed9e331e7d5b45374ddfb55d2ed14b1e32beeda52821e8a4f`.

A task-owned Docker container, `batter-kjl-validation`, used the already available
`postgres:18` image: PostgreSQL 18.4 Debian on Linux aarch64. It exposed only
127.0.0.1:62202, with an isolated disposable cluster and explicit local trust
configuration. The task-owned container was stopped and removed after both live
runs. No existing database server was mutated. Native client/runtime
claims here concern macOS; the Linux database container is not a Linux Rust
workspace test. No new hosted CI execution is claimed.

| Executed command | Outcome |
| --- | --- |
| `bash scripts/verify.sh` | Passed: 650 Rust test/doctest executions in 73 summaries, zero failures, 36 intentionally ignored live cases; formatting, warning-denied Clippy and rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed with the same 650/73 counts, zero failures, 36 intentional ignores and all formatting/lint/doc checks. |
| `POSTGRES_TEST_ADMIN_URL='postgres://postgres@127.0.0.1:62202/postgres?sslmode=disable' bash scripts/test_reference_live.sh`, on each toolchain | All 23 required cases executed and passed, none ignored or filtered. Inventory and PostgreSQL privilege preflight passed. |
| `env -u POSTGRES_TEST_ADMIN_URL bash scripts/test_reference_live.sh` | Failed with exit 1 before inventory/fixtures and a configuration diagnostic, as required. |
| `cargo build -p batter-axum --example http_service --locked`, then `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, `--warn-filter --deadline`, on each toolchain | All ten HTTP process smokes passed. |
| `scripts/jig file-budget check --base HEAD`; `git diff --check` | Passed; one nonblocking 466-line notice for fixture_failures.rs, no warnings/errors or waivers. |

Logs and the exact smoke command script are under `.agent/tmp/batter-kjl/`:
`verify-1.98.1-final.log`, `verify-1.94.0.log`, `live-default-final.log`,
`live-1.94.0.log`, `http.log`, and `http.sh`. An initial full default run passed
its tests but failed Clippy on new probe complexity. Splitting setup, observation
and assertions resolved the lint without suppressions or relaxed assertions;
the later complete verification passed. Early development compiles also caught
borrowed JoinError propagation and a missing upstream non-exhaustive pattern.

The live controls hold acknowledged native-detached and PgLease-retired backends
through successful pool close, then witness their exact IDs and retained database
from an independent one-slot observer. A parked database does not block independent
database cleanup. Explicit release/retry permits completion while retaining the
original observation timeout. PoolClosed and wrong-database observation failures
likewise retain both attempts; the original Arc survives into the final report.
Partial acquisitions and body panics also run with session observation enabled.
Handled native PoolTimedOut still fails the report and appears in its redacted
counts. A returned assertion plus Script exhaustion retains both generic failure
branches after native cleanup and catalog absence checks.

An acknowledged pg_shdescription lock causes real body SQLSTATE 22012,
consuming-lease cleanup failure and a distinct deferred lease's queue failure.
The tests inspect each native branch and database identity; external shutdown
does not consume the drain failure. Both tagged residuals are reclaimed through
upstream owner-aware stale cleanup after unlock. The upstream harness emits its
own deferred failure diagnostic; no adapter redaction of upstream logs is claimed.

Waiter loss on a live runtime keeps the driver and held lease alive until checkout
release. A different test destroys the actual driver runtime: repeated waits
retain its cancelled JoinError, and native lease Drop deletes the database while
a checkout is still held. A surviving harness explicitly drains that fallback.
This is evidence against runtime-death cleanup guarantees, not clean pool reuse.
Session absence is a point-in-time observation; callers must stop new connection
producers and select the same server. Manual DatabaseFixture finish and template
initializer-owned pools retain their documented ownership limits.

The explicit `scripts/jig check test` ran its tests successfully (process exit 0)
but Jig rejected that receipt because the container-removal documentation edit
changed the worktree while the target ran. No test assertion failed. The subsequent
`scripts/jig work check --plan-id plan_01M25Y0VN2H93NVFSZNRQ8D14P` verify profile
passed all five targets; `work evidence` and `work gates` reported fresh/passed
with no unresolved failures. Its api:test receipt is
`receipt_01M25Z7JQEMS291NY5D39KW613`. Final metadata and plan-finish evidence are
retained in the append-only plan records; receipt freshness is rechecked after
this note. No commit, push, publication or
deployment was performed.




## Configuration live completion (batter-5pm), 2026-09-10

After the user requested Docker provisioning, a dedicated disposable container
using the official `postgres:18` image supplied the missing endpoint. Docker
Engine 29.8.0; image digest
`sha256:4ef4dbc939d61acea57712655ddb4b4ab27419c913f94cca0cd57cb3ea3c2280`;
server PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2), Linux x86_64. The container
published only a dynamically assigned loopback port, used an ephemeral generated
password and had no repository or existing database mounts. Provisioning remains
external to application and test code.

With `POSTGRES_TEST_ADMIN_URL` pointing to this container's explicit
`postgres://postgres:<password>@127.0.0.1:<port>/postgres?sslmode=disable` endpoint:

| Command | Result |
| --- | --- |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/test_reference_live.sh` | PASS: native authentication/version/privilege preflight, exact inventory, 19 passed / 0 failed / 0 ignored; test body 21.65 seconds. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | PASS: same preflight and inventory, 19 passed / 0 failed / 0 ignored; test body 20.19 seconds. |

These runs execute the configured pool timeout/reuse, worker capacities 1 and 2
with committed lease-count observations, and startup pool-close-before-lease
probes. They supersede the missing-endpoint limitation in the historical records
below. No code, test command, dependency or toolchain changed after the preceding
754-execution verification matrices and ten HTTP smokes; those results remain
applicable. Final Jig evidence is refreshed after delivery documentation and
tracker updates. No macOS, TLS negotiation, hosted CI or deployed-adoption claim
is added. The disposable container and its anonymous volume are removed after
verification.

## Configuration boundary corrections (batter-5pm), 2026-09-10

This follow-up addresses the independent comprehensive review's root causes:
process-global native constructors in otherwise injected tests, divergent live
preflight policy, and a duplicated documentation inventory breakdown. Research
and resolved open questions are recorded in [references](references.md#configuration-boundary-follow-up-research-2026-09-10).
The implementation also replaces the worker probe's quiet-window inference with
a committed native lease-count observation; that live oracle is not yet executed.

Same Linux x86_64 and exact Rust/Cargo versions as the typed-root record below.
No dependency versions or Cargo.lock contents changed. Final current-source evidence:

| Command / evidence | Result |
| --- | --- |
| `PGDATA=/tmp/batter-hostile-parent PGPASSWORD=parent-secret-marker cargo test -p batter-example-reference-service --test configuration --locked` | PASS: nineteen tests; native consumers run in cleared child processes. |
| `cargo test -p batter-example-reference-service --example reference_preflight --locked` | PASS: two tests, including the actual entrypoint's bounded child rejection cases. |
| Built native preflight, invoked with missing URL, PGDATA, unsupported query, remote host and TLS-required URL | All five return exit 1 with expected static diagnostics, no fake-secret marker and no success marker. No fixture is acquired. |
| `bash scripts/verify.sh` | PASS: 754 Rust executions, 81 summaries, 32 intentional live ignores; Python controls, formatting, Clippy and warning-denied rustdoc pass. Includes the hostile-parent configuration target. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS with the same counts and checks. |
| Rebuild HTTP example on each toolchain, then default, SIGINT, deadline, warn-filter and warn-filter+deadline smoke profiles | PASS: all ten on their respective rebuilt binaries. |
| `bash scripts/test_reference_live.sh` on both toolchains | Both exit 1 through native preflight: POSTGRES_TEST_ADMIN_URL is absent. No fixture starts and no live case executes. |

Initial follow-up checks caught a module-inception lint and a scheduling control
that still expected two runtime passes. The module name and scheduling controls
were corrected, including explicit failure propagation from the new third pass;
no existing semantic test was weakened. A later regression addition invokes the
actual native preflight main rather than relying only on shared-validator tests;
both complete matrices were rerun after it. Final Jig evidence for
plan_01M262XKYHQ8SPX2XT63H2TN2M is retained in managed work evidence and gates.

Python now owns scheduling/inventory only; Rust preflight and fixture acquisition
share the same application validator and live endpoint policy. The selected
native credentials authenticate the read-only preflight connection itself.
Do not claim successful authentication/version/privilege checking, the stronger
worker oracle, or the three new live cases without an externally selected server.
The endpoint/configuration has been requested; no provisioning was performed.
The Bead remains in progress, with no new macOS/TLS/hosted/deployed evidence.

## Configuration review fixes (batter-5pm), 2026-09-10

Both collated review findings are addressed in the uncommitted implementation.
The live pool-close probe retains its StartupError report together with evidence
or channel failures. Its private checker is also exercised offline with actual
Startup primary/cleanup failures, preserving concrete causes and redacted output.
Four settings fixture consumers now use exclusive Unix directory creation with
mode 0700 and checked explicit cleanup. Existing directory/symlink controls pass.
An isolated child-process reproduction precreated the allocator's first candidate
with a symlink to a review-owned canary: the fixture skipped that candidate,
passed, and preserved all 19 canary bytes. Previously the same fixture truncated
the canary while passing. No unrelated files were used; reproduction files were
removed.

Environment/toolchain identities are the same as the typed-root record below.
No dependency versions or lockfile contents changed during these fixes.

| Command / evidence | Result |
| --- | --- |
| `cargo test -p batter --test settings --locked` | PASS: seven tests, including exclusive directory/symlink regression. |
| `cargo test -p batter-example-reference-service --test configuration --locked` | PASS: eighteen offline tests, including retained primary/cleanup/channel failure evidence. |
| `bash scripts/verify.sh` | PASS: 732 Rust executions / 79 summaries / 32 intentional live ignores; formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS with the same counts and checks. |
| Rebuild HTTP example on each toolchain, then `scripts/smoke_http.py` default, SIGINT, deadline, warn-filter, warn-filter+deadline | PASS: all ten profiles after their respective builds. |

The first verification attempt passed its tests but Clippy rejected a redundant
borrow. Follow-up Clippy checks also caught a duplicate shared-module inclusion
and the formatted live probe's function length. The final sources remove the
borrow, include the fixture module once per test binary, and separate startup
observation from fixture teardown; no semantic test was relaxed. Final Jig
receipts for plan_01M262XKYHQ8SPX2XT63H2TN2M are recorded in managed work evidence.
The original nineteen-case PostgreSQL live acceptance remains pending an
externally selected endpoint. These review fixes do not close the owning Bead
or claim live, macOS, hosted CI, TLS, publication, or deployment evidence.

## Typed root configuration (batter-5pm), 2026-09-10

Implementation is present; the owning Bead remains **in progress** because the
required PostgreSQL live executions lack an externally selected endpoint. This
record does not relabel compiled/ignored cases as live results.

Environment: Linux 7.0.11-76070011-generic, x86_64-unknown-linux-gnu, glibc 2.39,
Python 3.12.3. Baseline HEAD `0e47f7dbd5d0c04d878d290c5d8c9181d1162b18`, with the
uncommitted batter-5pm implementation. Rust 1.98.1 is
`48a229ceaefd4985c50990b14116b6d856af0985` (LLVM 22.1.8); Rust 1.94.0 is
`4a4ef493e3a1488c6e321570238084b38948f6db` (LLVM 21.1.8).
Cargo versions: 1.98.1 `797e8a9bc` and 1.94.0 `85eff7c80`.
Cargo generated the lockfile's new direct dependency edges; no upstream version
changed. Lock SHA-256:
`afed3580de9f3993d9e08630f6c6824c8ea22d618e30ff40269e19f3a74a7379`.

| Command / evidence | Result |
| --- | --- |
| `bash scripts/verify.sh` | PASS on Rust 1.98.1 after the final redaction test: 729 passing Rust executions across 79 successful summaries, 32 intentional live ignores; formatting, Clippy and warning-denied rustdoc pass. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | PASS with the same 729 executions / 79 summaries / 32 ignores and all remaining checks. |
| `cargo build -p batter-axum --example http_service --locked` | PASS on 1.98.1 and with `RUSTUP_TOOLCHAIN=1.94.0`; each binary was rebuilt before its smoke set. |
| `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` | PASS on both rebuilt binaries. |
| Same smoke with `--signal SIGINT` | PASS on both. |
| Same smoke with `--deadline` | PASS on both. |
| Same smoke with `--warn-filter` | PASS on both. |
| Same smoke with `--warn-filter --deadline` | PASS on both. |
| Selected-file executable exercise on 1.98.1 | A temporary file set bind to an ephemeral loopback port, timeout 5 ms, capacity 1 and RUST_LOG=off. `/work` returned 503/deadline_exceeded with no environment timeout; overriding timeout to 100 ms returned 200/`ok`. Both processes passed readiness and exited 0 after SIGTERM; temporary files were removed. |
| `python3 -m unittest discover -s scripts -p test_reference_live.py` | PASS: five runner controls, including omission of each added configuration case. Also passed in both verify matrices. |
| `scripts/jig doctor` | PASS; no required or optional setup step. |
| `scripts/jig work check --plan-id plan_01M262XKYHQ8SPX2XT63H2TN2M` | PASS with fresh final receipts after the added secret-bearing startup report test; Clippy, formatting, api:test, contract and file-budget targets pass. The current api:test receipt and target freshness are retained in this plan's managed evidence (`scripts/jig work evidence --plan-id plan_01M262XKYHQ8SPX2XT63H2TN2M`). |
| `bash scripts/test_reference_live.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/test_reference_live.sh` | Both exit 1 before fixtures: POSTGRES_TEST_ADMIN_URL is unset. No live case ran. |

The new foundation target executes six source/bounds/redaction tests. The HTTP
example now has `test = true` and executes twelve tests in normal discovery,
including actual configured router capacity/deadline changes and bounded child
entrypoint diagnostics. The reference configuration target executes seventeen
offline tests. Its native IPv6 protocol fixture observes decoded username,
database and password bytes. It is not a PostgreSQL server or TLS negotiation test.

Early focused runs caught the SQLx IPv6 URL formatter limitation and a test
assuming failed startup would report Stopped. The tests now exercise native IPv6
wire behavior and assert the native Draining state before running transfer.
A final audit added genuine secret-bearing parser and cleanup causes to the
startup formatting/tracing oracle; both complete toolchain matrices were rerun
successfully afterward. No existing semantic assertion was relaxed. The last change was
confined to that reference test; HTTP source, configuration and smoke prerequisites
were unchanged after both rebuilt smoke sets.

The explicit reference inventory is nineteen (previously sixteen), including
`configured_pool_capacity_and_acquire_timeout`, `configured_worker_concurrency`
and `configured_startup_pool_close_before_lease`. All compile and are ignored
only by ordinary discovery. Completion still requires both explicit live runs
against an operator-selected disposable local PostgreSQL 18 endpoint with CREATE
DATABASE plus the documented catalog-lock fault-injection privileges. The user
has been asked which existing configuration to use. This task did not provision
a server. Historical sixteen-case live results do not prove the new cases.

The selected feature graph has no SQLx TLS backend; native mode retention is
tested but TLS negotiation remains unverified. Native options/source inspection,
upstream logs and panic hooks remain outside redacted projections. No new macOS,
hosted CI, production command/worker, external adoption, publication or deployment
evidence is claimed. No commit was made.

## Cancelled wait diagnostics and terminal assertions, 2026-09-10

Owning Bead `batter-538`, following `batter-rv8`; baseline `486e0b0` plus the
existing working changes. Research preceded implementation. The deadline finding
was structural: private three-second wait timers competed with their containing
phase and could lose missing-event/wire details. Private pending-wait guards now
retain names, partial bytes and event snapshots on destruction. The driver retains
capture and any actual report after joining cancelled exercise/reconciliation.

Final handler counts were omitted when report checks moved out of exercise; both
suites now recheck after terminal shutdown, with late-entry injection controls.
Delayed-report drain now has 400 ms rather than 100 ms; the complete 3.4 s shutdown
allowance fits 3.5 s teardown. This reduces an avoidable scheduling risk, without
claiming the former timing had caused an observed failure. Remaining raw
`with_subscriber` arguments used implicit `Into<Dispatch>` construction; all seven
sites now use the shared test helper. [Primary research](references.md#implicit-dispatch-construction-and-cancelled-waits-2026-09-10)
records pinned tracing/Tokio semantics. No production API or dependency changed.

Linux x86_64, Rust 1.98.1/1.94.0, Python 3.12.3:

| Executed verification | Result |
| --- | --- |
| `bash scripts/verify.sh` | Passed: 693 Rust test/doctest executions, 76 successful summaries, zero failures, 29 intentional ignores; Python controls, formatting, Clippy and warning-denied rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed the same complete matrix and verification phases. |
| HTTP lifetime targets | 23 ownership and 27 observation tests passed. Fast/delayed missing reconciliation requires the same governing timeout and retained report/Drop evidence; exercise event/partial-wire, shorter disconnect and terminal-count negative controls pass. |
| Four concurrent workers, both compilers | 470 invocations passed: 70 selected HTTP deadline/delayed-report/terminal-count controls, and 400 complete foundation telemetry, adapter telemetry, cleanup-observation and isolated tracing-dispatch binaries (50 per target per compiler). Builds were copied before switching compilers; evidence records binary hashes and commands. |
| Rebuilt `http_service` per compiler, five profiles each | All ten smokes passed: default, SIGINT, deadline, WARN and WARN plus deadline. |
| Portable HTTP mutation runner | Four baseline cases passed; premature-close and producer-only variants each failed all four at their required assertions. All builds succeeded; independently replaying recorded replacements reproduced every source hash. |

The initial complete run found an unused capture clone in the reconciliation task;
removing it restored warning-denied Clippy. The final complete runs above include
that correction. A temporary repetition script initially keyed both telemetry
binaries by version; its six-binary inventory assertion rejected setup before any
repetitions. Package-specific keys and immutable copies produced the successful
470-run evidence. No semantic test was relaxed.

Full logs: `/tmp/batter-diagnostics-verify-{1.98.1,1.94.0}.log`.
Repetition commands, hashes and logs: `/tmp/batter-diagnostics-stress.json`.
Smoke commands/logs: `/tmp/batter-diagnostics-smokes.json`.
Mutation evidence: `/tmp/batter-http-graceful-mutation-0errvs_p/evidence.json`.
The checked-in tests/mutation runner remain reproducible; temporary logs are local
evidence. Final Jig receipts attach to `plan_01M25Z56CQT7SJZG20CMX0HP65` after the
source/tracker update. Existing staging is preserved byte-for-byte, with follow-up
changes unstaged. Current macOS/hosted execution remains unverified; live PostgreSQL
checks remain opt-in. Non-yielding work still requires the Unix watchdog.


## Terminal report ownership and dispatcher bootstrap, 2026-09-10

Owning Bead `batter-rv8`, following `batter-88d`. Research preceded implementation.

| Review issue | Root cause and implemented correction |
| --- | --- |
| Terminal report waits used the exercise deadline | Incomplete fixture ownership split: the driver owned teardown but scenarios still awaited completion. Both suites now validate terminal reports after exercise. Scenario observers expose only a restricted intermediate abort checkpoint, preserving the blocked-body ownership proof. |
| Mutation evidence described only one variant | Executable edits and descriptive metadata had separate definitions. One variant mapping now drives edits and records exact replacements and patched-source SHA-256 for baseline and both mutations. |
| Capture documentation retained the old lifetime rule | A documentation omission after removing the retention vector. The current testing guide describes per-case capture release and the shared dispatcher constructor. |
| Concurrent first-sentinel registration | Reproduced upstream bootstrap window: NoSubscriber has no maximum-level hint and raises global filtering to TRACE before the real subscriber rebuild. An OFF-filtered inert registry keeps macros disabled until registration/rebuild finishes. |

The tracing experiment used a private tracing-core 0.1.36 copy with a scheduling
hook immediately before the first unscoped DefaultCallsite Never store. Pausing
there, registering the real dispatcher and then resuming made the old sentinel
lose the later scoped span (exit 101); the OFF-filtered sentinel passed (exit 0).
The registry cache and workspace dependency graph were not modified. A separate
isolated copy of the checked-in `tracing_dispatch` test passes with the fixed helper
and fails at the maximum-level assertion when restored to the previous sentinel.
[Primary sources and scope](references.md#dispatcher-bootstrap-and-terminal-report-phases-2026-09-10)
also resolve the timeout question: pinned Tokio polls the inner future before
checking elapsed time. Pending reconciliation can still expire, while the report
remains separately owned. No claim covers every possible upstream tracing race.

Linux x86_64, Rust 1.98.1/1.94.0, Python 3.12.3:

| Executed verification | Result |
| --- | --- |
| `bash scripts/verify.sh` | Passed: 689 Rust test/doctest executions, 76 successful summaries, zero failures, 29 intentional ignores; Python controls, formatting, Clippy and warning-denied rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed the same complete matrix and phases. |
| HTTP lifetime targets | 22 ownership and 24 observation tests passed. Each suite proves a real 2.2 s finalizer can complete after exercise under teardown's allowance. The observation suite also retains a report across a failed reconciliation event wait and subsequent resource Drop. |
| Four concurrent workers, both compilers | 870 runs passed: 450 forced-handler/blocked-body/full-disconnect/write-half-close cases (25 of each selected case per compiler), 20 delayed-report successes, 200 complete core-unit binaries and 200 complete tracing-dispatch binaries. |
| Observed repeat durations | Largest end-to-end forced/disconnect run 0.4765 s; delayed-success run 2.2189 s; core/dispatch run 0.0210 s. These are local process durations, not individual cancellation timings or hosted bounds. |
| Rebuilt `http_service` per compiler, five smoke profiles each | All ten passed: default, SIGINT, deadline, WARN, WARN plus deadline. |
| Portable HTTP mutation runner | Four baseline cases passed; premature-close and producer-only variants each failed all four at the required assertions, with every build successful. Applying each evidence record's replacements independently reproduced its source hash. |

Full logs: `/tmp/batter-report-verify-1.98.1.log` and
`/tmp/batter-report-verify-1.94.0.log`. Repeat builds, compiler identities, immutable
binary hashes, selected commands and individual logs are indexed by
`/tmp/batter-report-stress.json`; smoke commands/logs by
`/tmp/batter-report-smokes.json`. Mutation evidence is
`/tmp/batter-http-graceful-mutation-zt0ckmin/evidence.json`.
Research sources/logs: `/tmp/batter-dispatch-bootstrap-pa3bvlgo`,
`/tmp/batter-dispatch-bootstrap-original.log`, `/tmp/batter-dispatch-bootstrap-fixed.log`,
`/tmp/batter-bootstrap-regression-original.log`, and
`/tmp/batter-bootstrap-regression-fixed.log`. The checked-in regression and mutation
runner remain reproducible entry points; temporary logs are local execution evidence.

Final Jig receipts attach to `plan_01M25WZG9VX1YSFW1SGXDTG7SK` after these
source/tracker updates. Existing staging is preserved byte-for-byte; follow-up
changes remain unstaged. Cargo manifests/lock and production behavior are unchanged.
The one-second disconnect checkpoint applies after close/EOF and before release or
drain, within exercise. The dedicated delayed-report policy has 3.1 s total allowance
below 3.5 s teardown; ordinary fixture budgets and the eight-second watchdog remain.
Current macOS/hosted runs remain unverified and live PostgreSQL tests remain opt-in.


## HTTP phase-budget repair, 2026-09-10

Owning Bead `batter-88d`, follow-up to `batter-27t`. Research confirmed a fixture
composition omission: individually bounded waits could exceed the shared watchdog.
Both HTTP drivers now share startup/exercise/teardown limits of 1/2/3.5 seconds,
with a one-second reserve checked below the unchanged eight-second parent limit.
Observation shutdown allowances fit the teardown limit. The running owner is
retained before readiness waiting; the exercise timeout is inside its joined task.
Report waiting and reconciliation use one absolute deadline, with any observed
report retained outside reconciliation. Production policy and Cargo.lock are unchanged.

The remaining filtered-test dispatch-retention vector was obsolete and removed.
A Weak-storage regression proves release after temporary concurrent cache borrowers
finish. Actual tracing-core source was checked before resolving the review question;
[primary references](references.md#fixture-phase-cancellation-and-subscriber-retention-2026-09-10)
record the verified Tokio 1.53.1 ownership semantics and callsite registration order.

Linux x86_64, Rust 1.98.1/1.94.0, Python 3.12.3:

| Executed verification | Result |
| --- | --- |
| `bash scripts/verify.sh` | Passed: 684 Rust test/doctest executions across 76 successful summaries, zero failures, 29 intentional ignores; Python controls, formatting, Clippy and warning-denied rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed the same complete matrix and verification phases. |
| HTTP lifetime targets | 21 ownership and 22 observation tests passed. New controls retain both exercise/reconciliation timeouts plus the actual shutdown report, ordered resource Drop/finalizer evidence, and startup-timeout teardown, without watchdog termination. |
| Complete telemetry target repeated with four processes | 100 runs per compiler, 200 total; all seven tests passed each run, including capture-storage release and filtered observations. |
| Rebuild `http_service` on each compiler; default/SIGINT/deadline/WARN/WARN-plus-deadline profiles | All ten smokes passed. |
| Portable HTTP mutation runner | All four original cases passed; premature-close and missing-native-event variants each failed all four cases at their intended assertions. All builds succeeded. |

Logs: `/tmp/batter-budget-verify-1.98.1.log`,
`/tmp/batter-budget-verify-1.94.0.log`, `/tmp/batter-budget-smokes.json`,
`/tmp/batter-budget-repeat.json`. Mutation evidence:
`/tmp/batter-http-graceful-mutation-zd52jea4/evidence.json`.
An initial focused run caught the old dual-failure output matcher after the driver
introduced explicit optional exercise/reconciliation results. The matcher was
updated to require the new error structure and both original failure payloads;
all subsequent focused/full checks passed. No existing semantic assertion was relaxed.

Final Jig receipts are attached to `plan_01M25V0FY8F7JAH8P0JAY6GFFK` after this
source/tracker update. Existing staging is preserved; follow-up edits are unstaged.
New macOS/hosted execution remains unverified; live PostgreSQL tests remain opt-in.
These bounds apply to yielding diagnostic work. Non-yielding work and runtime
shutdown still require the independent watchdog and imply no async-drop cleanup.



## Test infrastructure hardening, 2026-09-10

Owning Beads: `batter-27t` and `batter-gg4`; exact baseline `486e0b0`, retaining
prior uncommitted HTTP work. Research preceded implementation. The review's two
runner omissions had a structural cause: the observation suite forked launch and
success policy from an existing native harness. Both HTTP targets now share the
PID-bound launcher, eight-second parent watchdog and scenario-specific completion.
The old Python wrapper and its Jig inputs are removed. Controls reject zero-test
success, absent/wrong completion and an ambient `stall` setting during discovery.

The event-log failure was a guard-lifetime mistake made possible by exposing
storage locking to assertion code. Both fixtures now use private EventLog storage;
owned snapshots keep panics outside its lock. Regressions exercise resource Drop
during unwinding from missing/wrong ordering and a timed-out missing event.
Notifications and async fixture ownership remain suite-local.

The intermittent core failure has a separate, deterministic upstream cause:
tracing-core 0.1.36's single-dispatch interest cache can disable a callsite first
visited by an unsubscribed thread ([primary sources](references.md#test-dispatcher-interest-cache-2026-09-10)).
A private test constructor registers one inert dispatch before real subscribers.
It installs no global default, keeps enabled/filtered assertions intact and
replaces indefinite retention of actual capture subscribers. Removing this
workaround in an isolated copy makes the new `tracing_dispatch` regression fail
at “registered subscriber lost callsite interest”; restoring it passes. No
production runtime behavior, public API, dependency version or root lock changed.

The raw-wire and instrumented-IO scenarios remain separate because they observe
different boundaries; launch/evidence machinery, event storage and current contract
and status summaries are shared. This preserves startup/teardown diagnostics,
upload polls, post-response body ownership and write-half-close coverage without
maintaining two process protocols. Current native acknowledgement still depends
on the verified Axum 0.8.9 event ordering and enforced current-thread runtimes.

Linux x86_64, Rust 1.98.1 and 1.94.0, Python 3.12.3. Cargo.lock SHA-256:
`ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.

| Executed evidence | Result |
| --- | --- |
| `bash scripts/verify.sh` | Passed: 680 Rust test/doctest executions, 76 successful summaries, zero failures and 29 intentional live/ignored cases. Python controls, formatting, Clippy and warning-denied rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed the same complete matrix and all verification phases. The earlier missing-span failure is now repaired, rather than waived. |
| Both HTTP targets through focused discovery and workspace matrix | 21 ownership tests and 20 instrumented observation tests passed, including launch/completion, unwind/Drop, stall, dual-failure and native acknowledgement controls. |
| Repeated actual core test binaries, four concurrent processes | 250 complete unit-binary runs per compiler per workspace-all-feature/core-no-default profile: 1,000 runs, all passed. Every run retained all 12 unit tests and their original enabled/filtered callback assertions. |
| Cooperative HTTP cases under four concurrent processes | Four cases repeated 25 times on each compiler: 200 runs, all passed; each selected exactly one outer test. Maximum observed process duration 0.277 s. Native acknowledgement's one-second timeout and held-work assertions were unchanged. This local result does not predict loaded hosted CI. |
| Rebuilt `http_service`, default/SIGINT/deadline/WARN/WARN-plus-deadline on each compiler | All ten process smokes passed. |
| `python3 scripts/check_http_graceful_mutation.py` | Original Axum passed all four cooperative cases; immediate native connection exit failed all four at pending-work assertions after acknowledgement; removing only the connection event failed all four at missing acknowledgement with the producer event present. All three builds succeeded and each negative case exited 101 at the expected assertion. |
| Mutation launch controls | `--output .` and optimized Python (`-O`) rejected with exit 2 before creating artifacts. |

The reviewable [mutation runner](../scripts/check_http_graceful_mutation.py) resolves
Axum with locked offline Cargo metadata and makes private workspace/dependency/build
copies. Python 3.11+ is needed only for this optional reproduction. Its final logs
and `evidence.json` are in `/tmp/batter-http-graceful-mutation-3d8isc38/`; the copied
lock differs only in Axum's path-source identity. Earlier valid run
`/tmp/batter-http-graceful-mutation-g6gn71jb/` also hosted the bare-dispatch mutation;
that expected regression failure is in `/tmp/batter-hardening-tracing-mutation.log`.
The portable runner and assertions survive independently of temporary output files.

Full verification logs are `/tmp/batter-hardening-verify-1.98.1.log` and
`/tmp/batter-hardening-verify-1.94.0.log`; smoke commands/results are in
`/tmp/batter-hardening-smokes.json`, repetition results in
`/tmp/batter-hardening-repeat.json`. Repetition builds used workspace all-feature
selection and core no-default selection, then copied executable artifacts before
later builds. An initial setup assumed feature selections always produced distinct
artifact paths; that assertion stopped before any repetition. It was corrected
rather than counted as profile evidence. The final repeat run passed all 1,200
process invocations. No test failure was retried into a claimed repair.

Jig completion receipts belong to `plan_01M25PRSMPNS0V3ZW6VWD44M6F` in append-only
`.agent/state/`; final gate evidence is recorded after this source/tracker update.
These local runs do not establish new macOS or hosted execution. External live
PostgreSQL cases remain intentionally opt-in; no database was provisioned here.
Earlier qualified failures and synchronization-gap notes below remain historical,
superseded for the current tree by this repair and the preceding `batter-mhp` fix.



## Native graceful acknowledgement repair, 2026-09-10

Owning Bead: `batter-mhp`, follow-up to `batter-u0m`; baseline master `486e0b0`
with the reconciled observation target present. Both HTTP suites now wait for the
resolved Axum 0.8.9 connection-task event before releasing cooperative work, then
check pending/live resources and absent server completion/cleanup. The shared
helper enforces a current-thread runtime, rejects producer/accept-loop messages,
and fails after one second if the native event is unavailable. The producer
marker is now `graceful-signal-ready`. The 50 ms held-work observation is after
the native milestone; it is not used to infer that shutdown delivery occurred.
No production Rust API, serving behavior, dependency version or lockfile changed.

Linux x86_64; default Rust 1.98.1 and minimum Rust 1.94.0, Python 3.12.3.
Cargo.lock SHA-256 remains
`ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.

| Executed evidence | Result |
| --- | --- |
| Focused `cargo test -p batter-axum --test http_lifetime --test http_lifetime_observations --locked` and both final workspace passes | 18 native-suite and 15 observation-suite tests passed; includes the shared producer/accept-loop rejection control in each target. |
| Final `bash scripts/verify.sh` / Rust 1.98.1 | Passed 668 Rust test/doctest executions across 74 successful summaries, 29 intentional ignores; Python controls, formatting, Clippy and warning-denied rustdoc passed. |
| Final `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Workspace all-feature/all-target tests passed, including both repaired suites. The independent no-default core pass failed the unchanged subscriber callback test at `subscriber.rs:136` (“enabled task span must be created”), the known intermittent `batter-gg4` failure. Full verification did not pass. |
| Rust 1.94.0 separate workspace/all-feature doctests, Clippy and warning-denied rustdoc | All passed. These complete the phases prevented by the core failure; they do not repair or override it. |
| Rebuild `http_service` with each compiler and run default, SIGINT, deadline, WARN, WARN-plus-deadline smoke profiles | All ten invocations passed. |
| Isolated native-source controls | All four cooperative cases passed with original Axum. Replacing native `graceful_shutdown()` with immediate connection exit made all four fail at the held-resource/pending-wire assertions. Keeping graceful behavior but removing only the connection event made all four fail for missing native acknowledgement despite the producer marker. All builds succeeded; negative cases executed exactly one test and exited 101 at the expected assertion. |

The twelve-run mutation evidence is
`/tmp/batter-http-graceful-mutation-xzq44i7b/evidence.json`, with per-case logs beside
it. The exact executed [mutation runner](../.agent/plans/plan_01M25MJHFHKRKTZXF2PNHCG6DG-mutation.py)
is retained with its plan. It copies the workspace and selected Axum source,
changes only the disposable manifest/lock/source, and uses a fresh private build
directory. Its lock comparison permits only the Axum path-source identity change.
A repeated setup initially reused a stale patched artifact in a shared build
cache; that baseline failed and is excluded. The private-directory run above is
the final evidence. Initial complexity/dead-code lint failures were repaired by
extracting named assertions and matching the existing shared-capture module policy;
no semantic assertion was relaxed.

Source checks and smoke commands/results are retained in
`/tmp/batter-http-graceful-verification.json`, with individual logs named there.
Separate minimum-toolchain outcomes are in
`/tmp/batter-http-graceful-minimum-supplement.json`. Earlier lint-failing runs are
archived under `/tmp/batter-http-graceful-before-refactor`.
Plan `plan_01M25MJHFHKRKTZXF2PNHCG6DG` owns final archival Jig gate receipts.
The prior HTTP synchronization finding is repaired; the independent tracing issue
remains open, now also reproduced on the minimum compiler. No new macOS or hosted
execution is claimed.

## Master reconciliation and lifetime finding recheck, 2026-09-10

Fast-forwarded master from `9a49422` to `486e0b0`. Upstream independently added
`http_lifetime` and private Unix process machinery. Its source remains intact.
The independent local suite is retained as `http_lifetime_observations`, with
its own socket/upload resource acknowledgements, post-deadline stream checks,
write-half-close case and Python watchdog. Its ADR is now 009; upstream ADR-008
is unchanged. Older evidence below retains the commands used before this rename.
Both append-only Jig histories were retained and the owning Bead records both
implementations. The recovery stash is `5b9d85b2a6bccf155b2fd8701771206a9d828c29`.

Rechecked the review finding against both current implementations and the resolved
Axum 0.8.9 source. Each `graceful-delivered` marker is emitted from the graceful
signal future; native connection tasks handle the signal on their own polls.
Cooperative cases can release the pending handler/body before that handling.
The synchronization coverage gap therefore remains; this integration does not
fix it. Upstream's success-marker validation already prevents zero-test child
runs for its own target; the separate Python runner retains its prior limitation.

`cargo test -p batter-axum --test http_lifetime --test http_lifetime_observations
--locked` passed 17 upstream and 14 observation tests on Linux x86_64 / Rust
1.98.1. `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed 666 Rust
executions across 74 successful summaries, with zero failures and 29 intentional
ignores, plus formatting, Python controls, Clippy and warning-denied rustdoc.
Rebuilt `http_service` under each compiler and passed all five documented smoke
profiles per compiler (ten invocations).
The initial Rust 1.98.1 `bash scripts/verify.sh` **failed** in unchanged core test
`lifecycle::state::tests::subscriber::subscriber_callbacks_run_before_finite_admission_lock`
at `subscriber.rs:136`: “enabled task span must be created”. A direct rerun passed,
but repeated executions of the actual all-feature library binary reproduced the
same failure on iteration 30. This unresolved intermittent failure is tracked in
`batter-gg4`; subsequent passing checks do not establish its remediation. No core
source or semantic assertion was changed during reconciliation. The failed script
stopped before Clippy/rustdoc; default-toolchain rustdoc was executed separately,
and the final Jig checks record their own Clippy and test outcomes.

Logs: `/tmp/batter-u0m-reconcile-focused.log`,
`/tmp/batter-u0m-reconcile-verify-1.98.1.log`,
`/tmp/batter-u0m-reconcile-verify-1.94.0.log`,
`/tmp/batter-reconcile-core-repro.log`, `/tmp/batter-u0m-reconcile-smokes.log`,
and `/tmp/batter-u0m-reconcile-rustdoc-1.98.1.log`.
Reconciliation plan: `plan_01M25KX657HQZ1632GVNTKB247`; final gate results are recorded
in its append-only receipts. Full verification is qualified by `batter-gg4`, even
if that gate invocation passes. New macOS/hosted execution remains unverified.

## Ownership and operational adapter reconciliation: 2026-09-10

User-authorized rebase of ownership commit `f5449cd` onto upstream `9a49422`
produced `aa00f77`. The three textual conflicts retained both Axum guide entries,
workspace ownership of the same `http-body` 1.0.1 dependency, and both independent
validation histories below. The new ownership tests and shared process sources
are byte-for-byte unchanged from the reviewed commit. All records from both
branches remain in the four append-only Jig state files. Upstream operational
helpers, context-filtering fixes and retained test dispatches remain intact.

On macOS arm64, both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed: 648 test/doctest executions,
zero failures, 29 ignored live cases and 73 summaries per toolchain. Both runs
also passed formatting, warning-denied Clippy and rustdoc. Rebuilt `http_service`
with each compiler and passed all five documented smoke profiles per binary
(default, SIGINT, deadline, WARN filter, WARN filter plus deadline): ten smokes.
Agent-map validation and `git diff --check` passed. Cargo.lock matches upstream
unchanged at SHA-256 `ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.

Logs are in `/tmp/batter-reconcile-REgraB`. Final Jig gates and completion are
recorded under `plan_01M25JQW66KYD3QYWX2TK47207`; this pre-gate note does not claim
those receipts yet. No new Linux or hosted execution is claimed for this combined
snapshot; previous platform evidence remains tied to its original commits.

## HTTP/1.1 connection lifetimes, 2026-09-10

Owning Bead: `batter-u0m`; baseline `9a49422308e662461524d62132cc81cbbc937c53`.
The new `crates/batter-axum/tests/http_lifetime_observations.rs` target contains eleven loopback
cases, a private child entry point and two negative runner controls. It uses the
native Axum serving/graceful composition with test-owned socket acknowledgements.
No production Rust API, example, dependency or Cargo.lock change was made.

Local platform: Linux x86_64, Python 3.12.3. Rust 1.98.1 (`48a229cea`) / Cargo
1.98.1 (`797e8a9bc`); minimum Rust 1.94.0 (`4a4ef493e`) / Cargo 1.94.0 (`85eff7c80`).
Cargo.lock SHA-256:
`ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.

| Command / evidence | Result |
| --- | --- |
| `cargo test -p batter-axum --test http_lifetime --locked` | Passed 14 tests on Rust 1.98.1. The final upload-during-drain case is also exercised by the full matrices below. |
| `cargo clippy -p batter-axum --test http_lifetime --locked -- -D warnings` | Passed; initial fixture complexity warnings were repaired by extracting named assertions, with no lint waiver. |
| `bash scripts/verify.sh` | Final Rust 1.98.1 run passed: 631 Rust test executions across 71 successful summaries, zero failures, 29 intentional ignores; formatting, Python runner controls, core check, all-target/all-feature Clippy and warning-denied rustdoc passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Passed with the same 631 executions, 71 successful summaries, zero failures and 29 intentional ignores; all other verification phases passed. |
| Rebuild `cargo build -p batter-axum --example http_service --locked`, then `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, `--warn-filter --deadline`, for each toolchain | All ten process invocations passed, including signal exit zero, envelopes, correlation and filtered deadline observations. |
| `scripts/jig work check --plan-id plan_01M25HA1BB22RN328GHN7681PY` | Passed all five targets: Clippy, formatting, tests, contract and file budget. The source-validation `api:test` receipt is `receipt_01M25JDQ3RVBZHXG2KXV016AE0` on default Rust 1.98.1. No source, test-command, configuration, toolchain or prerequisite changes followed this test run. Documentation/tracker finalization changed Jig input digests, so archival gate checks are refreshed on the final worktree and retained under the same plan. |

The suite positively witnesses rejection on an established connection with graceful
delivery temporarily withheld, while preserving ordinary drain-driven graceful
shutdown in the companion case. Pending upload and admitted handler resources are
observed before interruption/release. Complete response framing, body destruction,
client EOF/reset and server socket destruction have separate assertions. The
blocked stream survives request-context cancellation and the elapsed response
budget. Forced request cancellation returns 503 `operation_cancelled` and permits
clean direct completion/finalization; wrapper abortion instead retains an aborted
`http` task and cancelled JoinError, no unjoined direct task, and `UnsafeTaskExit`
cleanup skipping while the body/socket remain owned at report inspection. Later
body release and native teardown are recorded separately, with cleanup still absent.

Full client closes before response and during streaming require resource/socket
drop acknowledgements before any test release, within a three-second observation
bound. Write-half closure is a separate pending-handler case: the native default
closes the server connection and the retained client read side sees EOF. These
explicit regressions do not allow either outcome merely because it was logged.
The 50 ms pending read used for blocked streams is a finite checkpoint, not a
promise of indefinite survival. Body destruction after headers emits no second
HTTP completion. The server wrapper's successful result remains separate from
connection errors.

Every public case uses the independent 20 s Unix process watchdog even under
focused Cargo discovery. The deliberate runtime stall is rejected after three
seconds and must report direct-child reaping and output EOF. The dual-failure
control requires both exercise and cleanup diagnostics. Normal Cargo discovery
already includes the target in `scripts/test_matrix.py`; `.jig.toml` and its
resolved contract now include `scripts/http_lifetime.py` for both test aliases.
The generated contract was produced with `scripts/jig update --recopy --no-input
--force` in a disposable checkout, then normalized to the existing JSON format.
Only the two intended input additions were retained; customized managed guides
were preserved. `scripts/jig doctor` reports ready.

Logs: `/tmp/batter-u0m-focused.log`, `/tmp/batter-u0m-clippy.log`,
`/tmp/batter-u0m-verify-1.98.1.log`, `/tmp/batter-u0m-verify-1.94.0.log`,
`/tmp/batter-u0m-http-1.98.1.log`, `/tmp/batter-u0m-http-1.94.0.log`, and final
Jig evidence for plan `plan_01M25HA1BB22RN328GHN7681PY`.

One archival gate refresh (`receipt_01M25JK2ZP3ZT7A5DPKS42QEA1`) failed in
`core-tests` (exit 101), while `workspace-tests`, including all HTTP lifetime
cases, passed. Jig retained only the beginning of stdout, so the exact failing
assertion is unavailable; no cause or fix is inferred. A direct repeat of
`cargo test -p batter --no-default-features --lib --tests --locked` passed, as did
100 executions of its unchanged twelve-test library binary. A separately captured
full `python3 scripts/test_matrix.py` run also passed. The final archive
requires a new passing full matrix/Jig test receipt; the failed run is preserved
in repository evidence. This test-only Axum change does not alter the core library.
Investigation output is in `/tmp/batter-u0m-core-investigation.log` and
`/tmp/batter-u0m-matrix-investigation.log`.

New macOS and hosted execution remain **unverified**. The existing macOS
all-targets job discovers the new suite, but configuration is not execution.
No HTTP/2, WebSocket, TLS, load-capacity, universal disconnect propagation,
transitive task joining, live database, or runtime-death claim is made.
[ADR-009](adr/009-http-lifetime-observations.md) records the measured boundaries;
[references](references.md#http11-lifetimes-reviewed-2026-09-10) records actual
resolved upstream source inspection and failed versioned web retrieval.

## Review fixes: final verification, 2026-09-10

Reviewed implementation `3f2ba37`, then separate fixes `f0f6669`, `4ec1556`, and
`b161d7a`; Beads `batter-7r3.9`, `.10`, `.11` are closed/exported. The correlation
loss was a structural coupling between filterable diagnostic spans and retained
execution context, plus a shared request/completion target. The startup concern
was an omission in composition tests; actual owner/waiter cleanup behavior was
already correct. Native filtering, ConnectInfo and enum-evolution decisions are
recorded with primary sources in [references](references.md).

Executed after all implementation slices on Linux x86_64:

| Verification | Result |
| --- | --- |
| `bash scripts/verify.sh` | Rust 1.98.1 (`48a229cea`), Cargo 1.98.1 (`797e8a9bc`): passed 617 Rust test executions across 70 successful summaries, zero failures, 29 intentional ignores. Formatting, isolated no-default core check, all-feature workspace Clippy and warning-denied rustdoc passed. Included process-runner, PostgreSQL smoke-oracle and reference-runner Python controls passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | Rust 1.94.0 (`4a4ef493e`), Cargo 1.94.0 (`85eff7c80`): same counts and all checks passed. |
| Rebuild `cargo build -p batter-axum --example http_service --locked` with each toolchain, then `python3 scripts/smoke_http.py --binary target/debug/examples/http_service` with default, `--signal SIGINT`, `--deadline`, `--warn-filter`, `--warn-filter --deadline` | All ten process invocations passed, including response IDs/envelopes, event-local HTTP fields, filtered nested-operation correlation and signal exit zero. |
| `python3 -m unittest discover -s scripts -p test_smoke_http.py` | Ten positive/negative controls passed, including absent/wrong nested-operation identity and missing deadline completion. |

Cargo.lock SHA-256 remains
`ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.
Logs: `/tmp/batter-review-fixes-verify-1.98.1.log`,
`/tmp/batter-review-fixes-verify-1.94.0.log`,
`/tmp/batter-review-fixes-http-1.98.1.log`,
`/tmp/batter-review-fixes-http-1.94.0.log`, and
`/tmp/batter-review-fixes-http-controls.log`. Jig gate receipts and closure are
associated with plan `plan_01M25F5H2GP5V53N74ZC541MX0` in repository evidence.
The final `scripts/jig work check --plan-id plan_01M25F5H2GP5V53N74ZC541MX0`
verify profile passed all five targets on the unchanged source with default Rust
1.98.1. Source-verification `api:test` receipt: `receipt_01M25FY7BMNBWMEZXKE87AGDJY`.
`work evidence` and `work gates` both reported fresh/passed with no unresolved
gates before this evidence addition. Updating these Markdown records invalidated
all five Jig input digests, so the required profile is refreshed for closure;
the final receipt IDs are retained in the plan closure evidence. File-budget succeeded with a nonblocking 513-line warning for
`tests/operational/correlation.rs` (500 warning, 800 hard limit); no waiver or
policy change was used. The CLI's trailing legacy "no checks configured" text
does not describe the five completed profile targets; retained receipts do.

These executions establish Linux behavior only. Ignored live PostgreSQL probes,
macOS and hosted CI were not executed in this review-fix run. Streaming and
native connection-task limits remain as documented in [guarantees](guarantees.md).

## HTTP ownership review coverage: 2026-09-10

Bead `batter-7r3.11`, parent `4ec1556` plus this slice, Linux x86_64, Rust 1.98.1.
All 17 operational tests, six adapter doctests and adapter all-target clippy
passed. New channel-controlled tests exercise real StartingSupervisor waiter
cancellation/owner drop, listener release before awaited dependent cleanup,
duplicate-name listener disposition, and supervised health writer stop during
drain. Existing runtime ownership needed no change. The ConnectInfo rustdoc
composition compiles. No macOS, hosted CI or live PostgreSQL claim is added.

## Request context filtering review fix: 2026-09-10

Bead `batter-7r3.10`, parent `f0f6669` plus this slice, Linux x86_64, Rust 1.98.1.
The 14 operational tests and seven HTTP example tests passed, as did adapter
all-target clippy. Ten Python oracle controls passed. Rebuilt the example and
passed both quiet-filter process smokes (normal and deadline). These now use
`info,batter=warn,batter::request=info` and require correlated WARN operation
completions, closing a hole in the prior smoke oracle. Existing all-INFO-disabled
HTTP field tests remain. An initial new test failed on formatter-leading whitespace;
its level check now trims that whitespace without weakening correlation assertions.
Full two-toolchain evidence follows the remaining ownership slice.

## Operation context review fix: 2026-09-10

Bead `batter-7r3.9`, prerequisite `3f2ba37` plus this slice, Linux x86_64,
Rust 1.98.1. `cargo test -p batter --test telemetry --locked` passed all six tests,
including two new filtered-parent regressions. `cargo clippy -p batter
--all-targets --locked -- -D warnings` passed. The first compile identified a
missing integration-test module path attribute; corrected before passing checks.
Full two-toolchain and HTTP verification follows all review slices below.

## Axum operational defaults: 2026-09-10

Bead `batter-7r3.4`, baseline `495e46fdbfd2009edb56d2e00d838407465a0c72`
plus the task worktree, on Linux x86_64. Audited and updated the Bead before
implementation. Added opt-in UUID correlation/one-observer composition, standard
infrastructure JSON, read-only readiness reasons/severity and native supervised
HTTP registration; the runnable HTTP root deletes its duplicated implementations.
Legacy request_scope, custom renderer and default Problem JSON behavior remain
covered by the unchanged adapter suites.

Cargo generated the lockfile after adding tower-http 0.6.11/request-id and the
http-body 1.0.1 adapter test dependency. UUID remains 1.26.0; no existing package
version changed. Final Cargo.lock SHA-256:
`ff50d56c475cf3b043a9c55ad6873d7dafec82582019ef312613dc0ec0e6b23a`.
A normal-dependency `cargo tree -p batter --edges normal --locked` inspection
confirms the foundation still excludes Axum, Tower HTTP and SQLx.

Both required verification commands passed:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Rust 1.98.1 (`48a229cea`, LLVM 22.1.8) and Rust 1.94.0 (`4a4ef493e`, LLVM 21.1.8)
each ran **606 Rust test/doctest executions**, zero failures and 29 explicitly
ignored live cases across 70 summaries. Totals include repeated core/workspace
profiles. Formatting, warning-denied Clippy and rustdoc, process-runner controls,
PostgreSQL-smoke controls and reference-runner controls passed. The twelve new
operational tests cover concurrent/forged/replayed IDs and nested operations,
custom/missing-ID rendering, complete-router 405/fallback/admission coverage,
INFO-disabled event-local identity on deadline/cancellation/drop, all dependency
and lifecycle readiness reasons, severity independence, owned startup/registration
and native streaming through wrapper abort. Test registries remain strongly owned
across cases. Initial draft compile failures (a never-returning handler type and
an incorrect skip-reason name) were corrected before these runs; no semantic
assertions were weakened.

On **each** toolchain, rebuilt the HTTP example and passed all five process modes:

```sh
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The minimum-toolchain build used `RUSTUP_TOOLCHAIN=1.94.0`; each smoke immediately
executed that rebuilt binary. The smoke oracle now requires a generated UUID and
request_id on the completion event itself, independently of formatted span fields.
`python3 -m unittest discover -s scripts -p test_smoke_http.py` passed all eight
negative/positive controls, including missing and conflicting event correlation.

The streaming regression receives real socket bytes before advancing virtual
time beyond the request budget. After drain/escalation it observes every direct
task joined but an outstanding body still alive, unsuccessful shutdown and
`UnsafeTaskExit` cleanup skip; it then releases and awaits body completion. This
is evidence of the limit, not a new streaming/disconnect/WebSocket ownership
contract. No live PostgreSQL, new macOS, hosted CI, publication or deployment
execution is claimed. Prior evidence remains scoped to its recorded snapshots.
Final Jig gate receipts are recorded with the task's execution plan.

## Process-harness review follow-up: 2026-09-10

Comprehensive review of the preceding component/HTTP change found a Linux-only
test-target defect. The HTTP target imported foundation watchdog self-tests as
well as mechanics. Two Linux-only controls resolved a Python asset relative to
the consuming Axum package, where it did not exist. The earlier macOS results
remain valid for that snapshot; they did not prove Linux gate compatibility.
`batter-u0m` was reopened for this correction. `batter-zu8`'s component behavior
was unaffected; its target is now also selected by the hosted macOS definition.

The fix places private std-only process machinery under `test-support/process/`.
Foundation-local wrappers alone attach fixture-specific timing policy, self-tests
and the Linux Python asset. HTTP supplies its own dispatcher and explicit deadline
without importing any foundation test source. At that stage the HTTP target had 12 tests,
not 51: ten scenarios, its deliberate-stall rejection and an inert dispatcher.
No controls were deleted: the foundation still owns and executes them. This also
removes HTTP's dependency on the foundation test relocation in `batter-tmx.1`.
Isolated scheduling copies and archive tests include the new shared source root.
The admission companion checks its second observation's 503/server_error, and
ordinary-drain fixtures retain rejection versus transport-closure evidence.

Both macOS arm64 matrices passed after these changes, using Rust 1.98.1 and
1.94.0: 611 Rust test/doctest executions each, zero failures, 29 explicitly ignored
live cases, and 72 summaries. The reduction is removal of 39 duplicated controls
from the HTTP target, not weaker foundation coverage. Formatting, Clippy,
warning-denied rustdoc and all runner controls passed. The archive regression
also passed. Cargo.lock is unchanged from the preceding section.

Linux verification uses an isolated Docker Linux arm64 environment from
`rust:1.97.0-bookworm`, with explicitly installed 1.98.1 and 1.94.0 toolchains.
A bind-mount launch stalled before container creation and was terminated; source
was copied into a fresh disposable container instead. SHA-256 comparison of all
Rust, Python, shell and manifest/config source files proved the copy matched the
worktree before execution. This is local container evidence, not hosted CI.
The first Linux compilation rejected a macOS-generated `._*.sql` metadata sidecar
copied beside the reference migration. Removing that generated sidecar from the
disposable container (not changing the repository migration) corrected the copy.
Both Linux matrices then passed: 615 Rust test/doctest executions each, zero
failures, 29 ignored live cases and 72 summaries, including the Linux-only
parent-death controls. Both platforms passed formatting, warning-denied Clippy
and rustdoc. Commands were `bash scripts/verify.sh` and the same command with
`RUSTUP_TOOLCHAIN=1.94.0`.

After rebuilding `http_service` separately with each supported toolchain, all
five documented HTTP smoke profiles passed on each platform/toolchain combination:
20 executable smokes in total. Linux commands used the container's
`/tmp/batter-target/debug/examples/http_service`; macOS used
`target/debug/examples/http_service`. No live database cases were run. The
independent review and final Jig receipts belong to plan
`plan_01M25ACNPR6F0SFN17Z180CK4B`; they are pending at this verification snapshot.
Logs: `/tmp/batter-review-fix-d9d5cW`.

### Second review and diagnostic corrections

Claude Opus and native Codex completed a same-scope review with matching complete
fingerprint `ccb5838776ccad9c4863ccd2488f0e9c182686872ef191f38b1c2a271e9ce150`.
Codex found no actionable defects. Claude found two low-severity issues: the
forced-cancellation case assumed it would resume before a 100 ms timer, and the
closed component Bead's notes attributed superseded HTTP evidence to itself.
The cooperative case retains the drain-survival assertion; forced cancellation
now checks its actual final outcome. Component notes explicitly limit scope and
point to this correction rather than presenting old Jig receipts as fresh.

Diagnostic follow-ups add three-second exercise/one-second teardown bounds,
incremental fixed event/wait records, complete failure snapshots, and assertion
ordering outside the destructor's event lock. HTTP now has 14 tests, including
missing-event and lock-poisoning controls. Component comparisons now have a
ten-second virtual-time bound and a seventh pending-future rejection control;
this is not non-yielding preemption. Linux CI now has the same 30-minute job bound
as macOS. The test-only http-body requirement moved to workspace ownership with
no lockfile change. Full-disconnect scope still excludes write-half closure.

Focused controls and the archive regression passed. The isolated scheduling-copy
check passed six original replays and rejected all six deliberate capacity
mutants. Initial macOS revalidation exposed two pre-existing controls: an exited
child could produce the missing-event deadline diagnostic before the exit
diagnostic, and one enabled-span assertion failed in the library tracing probe.
The former now accepts only the two valid missing-evidence diagnostics while
still requiring a panic after observed child exit. The tracing assertion was not
changed and did not reproduce in 30 consecutive full library-test runs; its exact
cause remains unconfirmed. Subsequent complete `verify.sh` runs passed on both
1.98.1 and 1.94.0: macOS 615 executions each and Docker Linux 619 each, zero
failures, 29 ignored live cases and 72 summaries in each matrix. All twenty
rebuilt HTTP smoke profiles passed again. Formatting, warning-denied Clippy and
rustdoc passed; the lock hash is unchanged. These are the current code snapshot's
results, logged in `*-round3*.log`. Repeated independent review and final Jig
receipts are pending; the one-off tracing failure is a residual test-stability
observation, not claimed repaired.

### Third review and strict process-control correction

Both reviewers completed against matching complete fingerprint
`c5b4a6d40e8551e8423e8f26d59fb71a67070a6258e54c7401e1b8df8d77dca7`.
Codex found no actionable defects. Claude correctly identified that accepting
the deadline diagnostic weakened the exited-child control, and that the HTTP
diagnostic control consumed too much of the five-second parent allowance.

The watchdog now joins final capture and decides missing evidence from an
observed child exit before consulting the startup clock. The strict exit-message
assertion is restored; separate live controls cover both an unexpired and already
expired startup clock. Late *present* events still undergo timestamp validation.
HTTP now centrally checks its three-second exercise, one-second teardown,
eight-second parent and ten-second emergency bounds with startup/unwind margin.
The missing-event control uses 500 ms rather than intentionally consuming three
seconds. Component cooperative completion explicitly rejects forced cancellation.
The component Bead's close reason is now component-only; ADR-008 documents the
explicit compatibility-review workflow when dependency upgrades change disconnect
behavior. Focused tests and both supported-toolchain matrices passed again:
macOS 617 executions each, Docker Linux arm64 621 each, zero failures, 29 ignored
live cases and 72 summaries per matrix. Formatting, warning-denied Clippy/rustdoc
and all twenty rebuilt HTTP smoke profiles passed. Source hashes matched the
Linux copy. Logs are `*-round4.log`; another independent review and final Jig
receipts are pending. The earlier tracing-test observation remains disclosed.

### Fourth review and diagnostic completeness

The complete same-scope fingerprint was
`806387e8bc9e7c3a2ab904a2f27f20be8bfca3546e7a187565c87b097ffc9704`.
Codex found no actionable defects; Claude identified weak rejection/trace
matching, a teardown bound below the server budget, and loss of more precise
capture diagnoses on the new finalized-exit path.

The rejection oracle now requires a second 503/server_error observation when the
client actually receives the rejection. Transport closure still permits one or
two observations: construction may precede lost delivery. A direct control covers
both valid closure cases and rejects a missing received-response observation.
The shared evidence owner checks capture limits first, preserves panic-before-event
diagnosis and still lets present events undergo timestamp checks. Live final-exit
controls exercise panic and capture overflow; the strict clean-exit controls remain.

All HTTP fixture budgets now share one definition: two-second exercise, 3.5-second
teardown, eight-second parent and ten-second emergency exit. Server drain/cancel/
abort/cleanup/reap maxima are asserted below teardown; the fixture's cleanup
allowance is 500 ms. A yielding unresponsive-server control verifies that an
elapsed exercise retains the actual aborted-task and skipped-cleanup report,
not only a timeout label. Snapshot controls inspect the snapshot/report sections
specifically. Component ordering is asserted after cleanup, not under its event
lock. The component tracker audit now explicitly explains its metadata-only
reopen/reclose. Both supported-toolchain matrices passed: macOS arm64 621
executions each and Docker Linux arm64 625 each, zero failures, 29 ignored live
cases and 72 summaries per matrix. Formatting, warning-denied Clippy/rustdoc,
static repository gates and all twenty rebuilt HTTP smokes passed. The Linux
source copy matched hashes. Logs are `*-round5.log`; the next independent review
and final Jig work receipts are pending. Hosted CI/load headroom remains unverified.

### Fifth review and bounded startup ownership

Both reviewers completed against complete fingerprint
`5530d5397d2c0eec8551eec27ccba00bd373aa053051a681b4c3214b2978053b`.
Codex reran 16 HTTP, seven component and 48 subprocess tests successfully and
found no actionable defects. Claude identified an unbounded readiness diagnostic
phase and ambiguous baseline wording for the test-only lockfile edge.

Construction/readiness now share a one-second deadline; the server owner is
retained before awaiting readiness. A timed-out startup skips exercise but still
drives and reports teardown. A separate control withholds critical readiness and
requires that timeout and its actual aborted/skipped-cleanup report before the
parent kill bound. Construction failure before a running owner exists is reported
as that boundary, with state/trace diagnostics. Budget relationships now use the
real `ShutdownBudget::total_allowance()`. Component event assertions take owned
snapshots before panicking. The references preamble and baseline lockfile-edge
wording are corrected; resolved versions remain unchanged.

Five focused ordinary-drain samples on each of macOS arm64 and Docker Linux
arm64, Rust 1.98.1, all reported `transport-closed`. The synchronized admission
case separately proves 503 routing. This samples the race; it does not establish
that a platform always closes transport. Logs: `reject-samples-macos.log` and
`reject-samples-linux.log`, produced by the focused target with `--exact --nocapture`.
The 30-minute CI cap is a bounded-job policy, not a hosted performance claim;
each matrix job selects one toolchain, not three serial compiler runs. Hosted cold
cache and shared-runner timing headroom remain unverified. Both supported-toolchain
matrices passed: macOS arm64 622 executions each and Docker Linux arm64 626 each,
zero failures, 29 ignored live cases and 72 summaries per matrix. All twenty
rebuilt HTTP smokes, formatting, warning-denied Clippy/rustdoc and file-budget
checks passed. Source hashes matched the Linux copy. Logs: `*-round6.log`.
Another independent review and final Jig work receipts remain pending.

### Sixth review and CI policy correction

Both reviewers completed against complete fingerprint
`7eaca4678717e1dc6dd5b3e8a4eb90886db212d9e1c3b7b97b804b1db6e9970d`.
Codex found no actionable defects; Claude identified one low-severity issue:
the newly added 30-minute Linux job cap was shorter than the three existing
sequential matrix phase bounds. The added cap is removed, restoring the prior
Linux job policy; macOS retains its existing cap. Fixture and matrix bounds are
unchanged. No hosted timing result is inferred. The nonconforming component
comparison now also explicitly requires cooperative, non-forced completion.

`scripts/jig check --affected HEAD --explain --json` identifies all five shared
`test-support/process/*.rs` files as direct inputs for `api:clippy`, `api:fmt`
and `api:test` (log `affected-round7.json`). The root API component and existing
`**/*.rs` inputs cover this source; `rust_crate_roots` need not pretend the private
source directory is a Cargo package. Earlier ownership validation below is marked
historical to distinguish its superseded counts and platform limitations.
Revalidation passed on both supported compilers: macOS arm64 622 executions each,
Docker Linux arm64 626 each, zero failures, 29 ignored live cases and 72 summaries
per matrix. All twenty rebuilt HTTP smoke profiles passed, along with formatting,
warning-denied Clippy and rustdoc. Linux source hashes matched. Logs use the
`-round7` suffix. The next independent review and final Jig receipts are pending.

### Seventh review and root navigation correction

Both reviewers completed against complete fingerprint
`e9096c2a8a3eb1d88a0d7b9fefb28c1404e7fc04599e23dbe15a59e9d7f4adac`.
Codex found no actionable defects and reran seven component, 48 process-control
and 17 HTTP tests successfully. Claude found one low-severity documentation
omission: the root guide denied a root source tree and neither root guide nor
agent map identified the private process sources. Both now link their ownership
README and distinguish the directory from the public test-support crate.

No Rust source, test command, toolchain, dependency or environment changed in this
correction; the round7 matrices and smokes remain applicable. The parent margin
is intentional but not a hosted latency guarantee. Disconnect regressions require
the measured pre-release drop, as specified in ADR-008; a pending checkpoint is a
failure, not an alternative passing outcome. Shared-source formatting currently
depends on the HTTP module declarations, as documented in `test-support/README.md`.
The unseen ordinary-drain 503 branch, hosted timing and one-off tracing cause
remain disclosed limits, not newly established guarantees.

### Final independent review

Claude Opus (restricted file access, default configuration) and native Codex
both completed with no actionable findings against complete fingerprint
`dfb4e38753ad02af95e358887c8d924b942bc6f7b25d0f15fb190c50feeeb607`.
Parent and reviewer pre/post captures matched. The only exclusion was `.agent`,
from `495e46fdbfd2009edb56d2e00d838407465a0c72:.reviewignore`.
Codex independently reran 17 HTTP, seven component and 48 process controls;
Claude performed a read-only review without executing tests. The agent-map
check passed with no missing guides or broken links.

Residual limits: no hosted CI, Linux x86_64 or separate CI `stable` run is claimed.
Hosted timing headroom is unmeasured. Ordinary drain's live samples all closed
transport; synchronized admission and a synthetic trace control cover 503.
Write-half closure is excluded, not experimentally contrasted; there is no
dedicated negative control for the disconnect checkpoint. Direct component tests
bound yielding deadlocks with virtual time, not non-yielding execution; the matrix
has an independent phase owner. Context-lock poisoning can add diagnostics after
an already failing scenario, and shared-source formatting currently depends on
HTTP module declarations. None was identified as an actionable current defect.
The previously recorded one-off tracing failure remains unexplained.

Delivery closure and final gate receipts are recorded under
`plan_01M25ACNPR6F0SFN17Z180CK4B` in Jig's append-only state. Those records, not this
pre-gate review snapshot, establish final gate success. No commit, push or hosted
execution is implied.

## Component and HTTP/1.1 ownership: 2026-09-10

Historical pre-review snapshot, superseded by the process-harness follow-up
sections above. Counts, timeouts and platform limits below describe that earlier
implementation, not the current test targets or latest verification.

Beads `batter-zu8` and `batter-u0m`, baseline
`495e46fdbfd2009edb56d2e00d838407465a0c72` plus the working changes for Jig plan
`plan_01M258E53R0W1F1CF14VB5TBK8`. Host: macOS arm64, Darwin 25.6.0,
`aarch64-apple-darwin`. These new component/transport cases have no Linux or
hosted execution evidence. Prior Linux runs below remain historical snapshots.

Cargo regenerated the lockfile after adding a test-only direct http-body edge;
its SHA-256 is
`18218c96d01d996eb5bf82efa1f56f23b0eb7d1280dc8a83da830de70d65ce3d`.
No resolved versions changed: Axum 0.8.9, Hyper 1.11.1, hyper-util 0.1.20,
Tokio 1.53.1 and http-body 1.1.0. Tokio io-util is explicitly enabled for the
test client. Primary registry-source inspection is recorded in
[references](references.md#http11-transport-ownership-reviewed-2026-09-10).

Focused commands passed on Rust 1.98.1:

```sh
cargo test -p batter --test component_ownership --locked
cargo test -p batter-axum --test http_lifetime --locked
```

The component target passed six comparisons. The HTTP target passed 51 tests:
ten real loopback scenarios, one deliberate-stall rejection control, one inert
fixture entry and 39 reused Unix runner controls. The latter are re-executed
controls, not 39 additional HTTP behaviors. Each ordinary HTTP case must exit
successfully within its independent five-second parent bound; killed/partial
cases fail. The stall control verifies kill/reap and rejection as successful
lifetime evidence. Existing parent-death and emergency-exit controls also pass.

The conforming component acknowledges child initialization and joins it before
cleanup. The nonconforming child answers a fresh request after successful direct
reporting and cleanup. Other cases retain concrete task/cleanup failures together
and native panic/abort errors with conservative skipping.

HTTP tests independently witness handler, response, body, framing, socket and
direct-server events. A blocked body survives server-wrapper abortion through
report inspection, then finishes after test release while the runtime remains
alive. Full SHUT_RDWR disconnect drops pending handler/body before release within
the one-second observation checkpoint. Forced process cancellation yields 503
`operation_cancelled`, joins the direct server and permits cleanup. Incomplete
upload yields 503 `deadline_exceeded`. One HTTP completion is retained after
streaming/disconnect/abort; no replacement status is invented. See
[ADR-008](adr/008-http-transport-ownership.md) for exact scope and exclusions.

Initial focused Clippy found an existing macOS `StartupFailure` layout above its
128-byte large-error threshold in the private startup driver. A local documented
allowance preserves the complete report immediately before the monitor's existing
Arc allocation; no public type or runtime behavior changed. New test helpers also
exceeded cognitive-complexity limits and were split without relaxing assertions.
The subsequent full workspace Clippy command passed with warnings denied.

Both `bash scripts/verify.sh` and
`RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` passed. Toolchains were Rust
1.98.1 (48a229cea, Cargo 1.98.1) and Rust 1.94.0 (4a4ef493e, Cargo 1.94.0).
Each ran 650 Rust test/doctest executions across 72 summaries, zero failures and 29
explicitly ignored live cases. Counts include repeated core/workspace runs.
Formatting, Clippy, warning-denied rustdoc, 22 process-runner controls, 13
PostgreSQL-smoke controls and four reference-runner controls passed. No live
PostgreSQL services/cases were invoked for these component/HTTP tasks.

On each toolchain, rebuilt the executable with
`cargo build -p batter-axum --example http_service --locked` (setting
`RUSTUP_TOOLCHAIN=1.94.0` for the minimum) and passed all five profiles:

```sh
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

All ten executable smokes passed, including response envelopes, telemetry,
readiness and signal exit 0. `scripts/jig work check --plan-id
plan_01M258E53R0W1F1CF14VB5TBK8 --json` passed all five verify targets, including
the full api:test matrix on the default Rust 1.98.1 toolchain. Crate guide edits
after that pass invalidated the receipts; the final refresh and gate audit are
recorded in the owning ExecPlan. No source, dependency, test command, toolchain or
prerequisite changed after the two full compiler matrices. Logs for this working
session are in `/tmp/batter-ownership-Z5h9dT`.

## Service startup and finite-command guidance: 2026-09-10

Bead `batter-7r3.7`, baseline `188c389790cad4682d31b9d7a4606141c8ed08b4`
plus this task's working changes, on Linux x86_64. Library runtime behavior and
public APIs are unchanged. Cargo.lock remains `ed37786b14f40d59b12b0155889bc2f6d7c191d1a578bf35dc5dca144ec8b2f5`;
no dependency versions or features changed. The new `finite_command` example is
explicitly declared with `test = true` so normal Cargo/Jig discovery executes it.

The `Startup` rustdoc is now an executable, self-contained service: native
capacity acquisition, pre-reserved finalization, actual channel request/reply,
readiness acknowledgement and awaited shutdown. Two integration controls
independently prove empty-supervisor failure with successful cleanup and
successful finite-work-only supervision. Six command example tests cover native
loopback work, individual/simultaneous failures, and separately awaited cleanup
after post-acquisition cancellation/deadline. These are composition controls,
not a new standalone command owner or a changed service success policy.

Initial focused execution passed all eight runtime/example tests. The first
executable doctest compile caught two mistakes in the new draft: `Readiness`
does not implement Error for direct `?` conversion, and `check_shutdown` returns
`()`, not a report. Both were corrected; the doctest passed, and its request
phase was then explicitly bounded before the final matrix. A documentation patch
initially failed to match a status row; no partial edits were applied. No semantic
assertions were weakened.

After the final source changes, both commands passed:

```sh
bash scripts/verify.sh
RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh
```

Each toolchain (1.98.1 and 1.94.0) ran **591 Rust test/doctest executions**, zero
failures and 29 explicitly ignored live cases across 69 summaries. Totals include
repeated core/workspace profiles. Formatting, Clippy with warnings denied,
warning-denied rustdoc, 22 process-runner controls, 13 PostgreSQL-smoke controls
and four reference-runner controls passed. No live PostgreSQL cases were executed
for this non-database composition task; earlier database evidence remains scoped
to its recorded snapshots. No new macOS or hosted execution is claimed.

On **each** toolchain, built the example and HTTP executable and ran:

```sh
cargo build -p batter --example finite_command --locked
target/debug/examples/finite_command
target/debug/examples/finite_command --fail-work
target/debug/examples/finite_command --fail-cleanup
target/debug/examples/finite_command --fail-both
target/debug/examples/finite_command --cancel
target/debug/examples/finite_command --deadline
cargo build -p batter-axum --example http_service --locked
python3 scripts/smoke_http.py --binary target/debug/examples/http_service
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --signal SIGINT
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --deadline
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter
python3 scripts/smoke_http.py --binary target/debug/examples/http_service --warn-filter --deadline
```

The command smoke driver requires default exit 0, all five injected modes exit 1,
exact work/cleanup count summaries and empty stderr, under an independent ten-second
process bound. Cancellation/deadline retain successful cleanup; cleanup failures
remain unsuccessful even after a successful body. All six cases passed on both
toolchains. All five HTTP profiles also passed on both toolchains.

Commands, logs and machine-readable totals are retained in ignored
`.agent/tmp/batter-startup-command-20260910/`. Jig completion evidence belongs to
`plan_01M253XQB5YVPMXR2DRJZKE5Q7`, using `scripts/jig work check --plan-id
plan_01M253XQB5YVPMXR2DRJZKE5Q7`; its receipts/gates record the final result. The
examples retain explicit waiter/runtime limits and add no asynchronous Drop,
remote termination, rollback, detached-child or standalone cleanup-owner claim.

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
No existing semantic assertion was relaxed. The native panic controls intentionally leave
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

## Protected startup-owned Unix signals: 2026-09-12

Bead `batter-lp2.2`; plan `plan_01M2B0BJMHHFSF51D0NDGTYPWQ`; baseline
`39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`. Protected startup now optionally
reserves a component name and installs SIGTERM/SIGINT synchronously before its
owner is returned. The existing coordinator owns reception through initialization
and transfers the listeners into one real critical task without a fallible
application handoff. Legacy startup and explicit signal helpers remain unchanged.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02). Tokio remains locked at 1.53.1.

| Command / evidence | Executed outcome |
| --- | --- |
| `cargo test -p batter --test startup_signals --locked -- --nocapture` | Three top-level cases passed. Fresh children cover TERM and INT immediately after start return, after resource registration, after borrowed-waiter cancellation, in approved and unapproved running services, plus default termination for configured-but-unstarted and started-but-unconfigured controls. Ordinary and managed reserved-name theft and configured owner loss also pass in isolated children. |
| `cargo test -p batter startup::driver::tests --lib --locked -- --nocapture` | Two injected controls passed: pre-cancellation skips installation, while preflight IO survives immediate owner loss alongside an unused initializer-capture panic and failed cleanup. Default panic-hook output remains expected. |
| `cargo test -p batter lifecycle::unix::tests::partial_reserved_install_retains_the_second_native_error --lib --locked -- --nocapture` | A fresh child installed the first actual native source, injected failure for the second, and retained the original IO marker. No process-disposition rollback is claimed. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh` | Passed core/minimal checks, complete workspace runtime tests, hostile-environment controls, doctests, formatting, Clippy and rustdoc. An initial run reached Clippy after all runtime/doctest checks and rejected the driver's eighth argument; the private callback/listener state was grouped and the complete verifier then passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same complete matrix passed on the retained minimum toolchain. |
| Rebuild `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes on each toolchain | All ten process smokes passed with readiness/probe behavior, response and telemetry assertions, selected signals, and exit zero. |

The real signal controls are Linux evidence only. Tokio installs process-wide
handlers and does not restore the prior disposition when listeners drop; a
partial installation is not rollback. No new macOS or hosted-CI execution,
second-signal force-exit, non-yielding preemption,
runtime-death cleanup or atomic kernel-arrival fence is claimed. Final Jig gate
receipts are recorded in the living plan and append-only state.

## Startup arbitration post-review repair: 2026-09-12

Reopened Bead `batter-lp2.2`; plan
`plan_01M2B7RSCGTA0A2PJ4M0D5RRYH`, baseline
`b87708db06640fa9fd81f6d15cd2f2cfa538f7be`. The all-reviewer commit review
found that biased branch selection could discard a simultaneously ready concrete
initializer result, and that the protected handoff guard had accidentally
suppressed the legacy final lifecycle check when successful-future destruction
panicked. The repair replaces branch-only selection with private
observe-then-arbitrate polling, retains consumed signal reception across successful
handoff, and restores the independent final check. A subsequent all-reviewer
review-fix pass found that this boundary polled the initializer before rejecting
an interruption that was already visible. The corrected boundary now prechecks
drain, cancellation, deadline and signals, polls application work only while
clear, then observes facts that became ready during that poll. Public APIs are
unchanged.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02). Tokio remains locked at 1.53.1; no dependency changed.

| Command / evidence | Executed outcome |
| --- | --- |
| `cargo test -p batter startup::driver::tests --lib --locked -- --nocapture` | Five controls passed. A pending initializer receives exactly one poll and no extra poll after drain becomes visible. Injected reception becomes ready during the application poll for both an error and a success: the error stays concrete; the success transfers the consumed fact into reserved registration and ends Draining without a second signal or Ready publication. |
| `cargo test -p batter --test startup --locked -- --nocapture` | Seventeen tests passed. A new legacy control returns success, requests drain during future destruction, panics in that destructor, and retains both `StartupCause::Draining` and the independent panic payload. |
| `cargo test -p batter --test startup_signals --locked -- --nocapture` | Four top-level cases passed. For TERM then INT and INT then TERM, the cleanup hook installs listeners before publishing its marker, the parent waits 1.5 seconds before the repeat, and a listener must publish `repeat-signal-observed`. Each child invokes one hook, reports `TimedOut`, remains unsuccessful, and bounds elapsed cleanup observation to 3.75 seconds against the original 3-second total plus 250-millisecond abort allowance. Marker or delivery failure kills and reaps the child before reporting both streams. |
| `bash scripts/verify.sh` | Complete default-toolchain core/minimal, workspace runtime, hostile-environment, doctest, format, Clippy and rustdoc matrix passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same complete matrix passed on the retained minimum toolchain. |
| Rebuild `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes on each toolchain | All ten process smokes passed with selected-signal exit zero, readiness, response, correlation and telemetry assertions. |
| PostgreSQL lifecycle three-case live target plus SIGTERM/SIGINT executable smokes, on each toolchain | All six live tests and four process smokes passed against the existing external PostgreSQL 18.6 Unix-socket database. |

The deterministic controls distinguish an event already visible before an
application poll from one that becomes visible during it. The former prevents
new work; a concrete error or panic returned by the latter poll remains owned.
The subprocess cases also prove the delayed repeat reaches a cleanup-owned
listener and reject either deadline restart or reporting a timed-out hook as
success. This is cooperative observation, not an atomic fence at kernel arrival.
Tokio coalesces notifications and retains its process-wide handler. The dedicated two-cluster
reference live suite was not rerun because its privileged admin and observer
endpoints were unavailable; its offline runner controls and workspace tests did
pass in both complete matrices. No macOS or hosted execution is newly claimed.
No commit, push, publication or deployment was performed.

## Cleanup-slot-owned SQLx pools: 2026-09-12

Bead `batter-lp2.3`; plan `plan_01M2B21R4PY11K6GCMER1CW48K`; baseline
`39c1b6e3c1c75f808becb5a5e7c33f58001a2ee4`. The adapter now exposes the exact
synchronous `pool_in(CleanupSlot, PgPoolOptions, PgConnectOptions) -> PgPool`
boundary. The slot is validated before native construction, and successful
construction is followed by infallible close-hook registration before return.
The legacy `register_pool_close` signature remains unchanged.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02), SQLx 0.9.0, Tokio 1.53.1, and an externally provisioned
PostgreSQL 18.6 (`18.6-1.pgdg24.04+2`) database reached through its local Unix
socket. The suite did not provision or mutate roles, databases, or server policy.
Its missing-role case preserved the selected endpoint and observed an actual
PostgreSQL authentication rejection; no TLS or specific password method is claimed.

| Command / evidence | Executed outcome |
| --- | --- |
| `cargo test -p batter-sqlx --features test-support --locked`, adapter Clippy, and adapter rustdoc | Offline registration, legacy signature, ignored-target inventory, doctests, the adapter-owned example build, and warnings-as-errors checks passed. |
| `bash scripts/test_sqlx_live.sh` on each toolchain | The retained ten-case disposition target and new fourteen-case pool-ownership target each matched their exact inventory and passed serially: 24 live cases per toolchain. Expected application panic hooks remained visible. |
| `cargo run -p batter-sqlx --example owned_pool --locked` on each toolchain | The real bounded query, pool cleanup, and complete finite command all reported success. |
| `RUSTUP_TOOLCHAIN=1.98.1 bash scripts/verify.sh` | A first matrix run found one `BrokenPipe` in the signal-test parent release write while all focused repetitions passed. The initial workaround deferred that auxiliary write error to the final child status/transcript assertion. Comprehensive review then found that the parent could still block while waiting for its first stdout marker or final `Command::output`, and could stop draining stderr. The repaired harness drains both pipes concurrently, bounds both phases, kills and reaps on timeout, and retains semantic markers, status, cleanup and capture diagnostics. A fresh complete matrix passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same post-review complete minimal/workspace runtime, hostile-environment, doctest, formatting, Clippy and rustdoc matrix passed. |
| Rebuild `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes on each toolchain | All ten process smokes passed with the selected signal, readiness, response, correlation and telemetry assertions. |

The ownership live cases cover successful Command and legacy Startup queries,
invalid and duplicate reservations before construction, native callback/options
and positive-minimum maintenance, server authentication failure, cancelled
acquisition, later application error and panic, two-pool dependency order, held
checkout completion/timeout, retained work plus cleanup failures, and negative
oracles for omitted or premature cleanup. SQLx's default idle/lifetime timers
delay the minimum-maintenance loop; the explicit native test disables both and
observes its configured `after_connect` callback before consumer checkout.

The review repair also made the authentication case derive its connection from
the configured `DATABASE_URL`, overriding only the deliberately missing role and
password, so a selected Unix socket, TCP host, port, database and TLS policy are
not silently replaced. Its focused live case and the complete fourteen-case
ownership target passed on both toolchains. The signal harness has a separate
fresh-child regression that floods stderr before its startup marker and another
that stalls before the marker; the former completes and the latter is killed,
reaped and followed by a successful child run. The Axum adapter README now calls
the protected registration API a protected composition path; migrating the
remaining legacy HTTP example is separately owned by `batter-lp2.4`.

These results establish local pool ownership and awaited close reporting. They do
not establish construction inertness, schema readiness, remote cancellation,
rollback, detached-session termination, runtime-death cleanup, macOS behavior, or
hosted CI. Jig receipts and final review evidence are recorded separately in the
living plan and append-only state.

## SQLx ownership-oracle post-review repair: 2026-09-12

Reopened Bead `batter-lp2.3`; plan
`plan_01M2B8NWYSW61AM69FDSV6N1NP`, baseline
`b87708db06640fa9fd81f6d15cd2f2cfa538f7be`. The all-reviewer commit review
found that several live checks could pass for the wrong reason: generic
authorization failure stood in for password rejection, `is_closed()` stood in
for completed native closure, synthetic reservation failures bypassed their
production composition path, and matrix controls did not inject failure into
every command in the real four-batch topology. The repair changes tests and
runner contracts only; no public or production Rust API changed.

Executed on Linux x86_64 with rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02), SQLx 0.9.0 and Tokio 1.53.1. The initial repair used
an external PostgreSQL 18.6 server. Review-fix-loop round 2 found that PostgreSQL
SCRAM deliberately returns the same invalid-password class for nonexistent roles,
so exact `28P01` alone was insufficient. The repaired oracle was rerun against an
isolated no-volume `postgres:18` container reporting PostgreSQL 18.6
(`18.6-1.pgdg13+2`) over loopback. It first completed a real query with the
configured role, then changed only the parsed connection's password and required
`28P01`. The disposable container was stopped and auto-removed after validation.

| Command / evidence | Executed outcome |
| --- | --- |
| `python3 -m unittest discover -s scripts -p 'test_sqlx_live.py' -v` | Five runner-contract controls passed. Missing either the ordinary endpoint or known-good password-authenticated endpoint stops before test invocation. |
| `python3 -m unittest discover -s scripts -p 'test_parallel_process.py' -v` | Twenty-two controls passed. The matrix negative now injects each of nine failures across actual batch sizes `[4, 1, 3, 1]` and requires later batches not to start. |
| `cargo test -p batter-sqlx --features test-support --locked` | Offline adapter tests, ignored-live inventory and doctests passed. |
| `bash scripts/test_sqlx_live.sh` with `DATABASE_URL` and `BATTER_SQLX_AUTH_ACCEPT_URL`, on each toolchain | The ten disposition cases and fourteen ownership cases passed: 24 live cases per toolchain. The authentication case completed and closed both the successful control pool and derived rejection pool. |
| Focused authentication case with a deliberately missing role supplied as `BATTER_SQLX_AUTH_ACCEPT_URL` | Failed as intended with status 101 at the explicit successful-query control; the mock SCRAM `28P01` cannot satisfy the rejection oracle. |
| Focused `native_options_and_maintenance_are_preserved` case on each toolchain after review-fix-loop round 3 | Passed against an isolated PostgreSQL 18.6 container; the case now requires its named cleanup record, zero pool size and a later `PoolClosed` acquisition in addition to the native callback/options observations. |
| `bash scripts/verify.sh` | The complete default-toolchain core/minimal, workspace runtime, hostile-environment, doctest, formatting, Clippy and rustdoc matrix passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same complete matrix passed on the retained minimum toolchain. |
| Rebuild `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes on each toolchain | All ten process smokes passed with readiness, response, correlation, telemetry, selected-signal and exit-zero checks. |

Successful cleanup now requires the named successful cleanup record, zero native
pool size, and `PoolClosed` from a later acquisition. The held-checkout case also
proves the owner remains unfinished before release. Positive and negative cases
now call the same cleanup-plus-native-close oracle. The missing-record negative
first closes its native pool so only the cleanup record is absent. The premature
negative supplies the claimed successful record while its held checkout keeps
native size nonzero; after release and close completion, that unchanged oracle
passes. Invalid and duplicate reservations propagate the exact
registration error through the real fallible command path and prove the native
constructor counter remains zero; the legitimate first pool in the duplicate
case still closes completely. The native-options/maintenance case retains its own
pool clone and applies the same completion oracle, so an empty cleanup report
cannot satisfy that acceptance path.

A successful query followed by exact `28P01` after changing only the parsed
password proves rejection for that known-good role and endpoint, not SCRAM versus
MD5 negotiation. Zero size plus `PoolClosed` proves SQLx pool-close settlement,
not termination of arbitrary detached server work. No macOS, hosted-CI, TLS,
remote-cancellation, rollback, ambiguous-commit or runtime-death guarantee is
added. No commit, push, publication or deployment was performed.

This continuation reran both complete Rust verification matrices and the 24-case
live SQLx suite on each toolchain after the oracle changes. The trusted review
exclusions cover `.agent`; no fresh Jig command was run because Jig appends there
and this loop's validation was constrained not to write excluded paths. Existing
Jig state was left unchanged.

## Direct TCP peer HTTP registration: 2026-09-12

Delivery Bead `batter-rme`; Jig plan `plan_01M2BHP64PFFA99BPV57MT2WH2`,
baseline `6f25e6476efd614b68cf884d0271707cbb36c6e8`. Implementation is confined
to Batter and shaped by downstream direct-peer authentication admission.
`register_http_with_connect_info_in` opts into native `ConnectInfo<SocketAddr>`
while retaining the same private supervised serving implementation as the plain
helpers. No dependency or Cargo.lock change was made.

Environment: Linux x86_64; rustc 1.98.1 (`48a229cea`, 2026-09-01) and 1.94.0
(`4a4ef493e`, 2026-03-02); locked Axum 0.8.9 and Tokio 1.53.1. Cargo.lock
SHA-256: `13d5a89554eb34020d89b63855d00a64307b6e1711394e2603455f9850bc248e`.

| Command / evidence | Executed outcome |
| --- | --- |
| `cargo test -p batter-axum --test operational --locked` | All 21 cases passed, including four added peer/lifecycle cases. |
| `bash scripts/verify.sh` | Final default-toolchain full core/minimal, workspace runtime, hostile-environment, doctest, formatting, Clippy and rustdoc matrix passed. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same complete matrix passed on the retained minimum toolchain. |
| Rebuild `batter-axum` example `http_service` with `--locked`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes | All ten process smokes passed, five per toolchain, with readiness, response, correlation, telemetry, selected-signal and exit-zero assertions. |
| `scripts/jig work check --plan-id plan_01M2BHP64PFFA99BPV57MT2WH2`, with evidence/gates inspection | All five required profile targets passed and were fresh in evidence/gates: api:test receipt `receipt_01M2BJDGXRDFAMGR1ZCCHTTZSR`, plus Clippy, formatting, contract and file budget. Final tracker/document changes reuse unchanged Rust receipts and refresh whole-repository policy checks. |

The peer oracle opens two simultaneous real TCP connections and compares both
request middleware and handler output to each client's independently obtained
local address and port. Forwarded, X-Forwarded-For and X-Real-IP headers carry
different forged values. Protected startup must reach Ready, graceful shutdown
must produce a successful report and named cleanup, and cleanup independently
rebinds the released listener. Exact invalid/duplicate registration errors release
only rejected listeners; an unstarted owner retains its listener until drop.
Shared existing scenarios additionally exercise peer-enabled startup waiter/owner
abandonment and a body surviving forced server-wrapper abort, retaining the
unsuccessful report and skipped dependent cleanup before explicit body release.

An initial focused compile misspelled `RegistrationError::Duplicate`; it was
corrected to the actual variant and exact component name. The first two full
default-toolchain attempts passed runtime tests and doctests, then hit Clippy's
complexity limit in shared lifecycle test helpers. Selecting the registration
function directly and extracting the complete startup-report assertions removed
that complexity without dropping assertions or relaxing lints. The final matrix
above ran after those changes.

The tracker initially rejected automatic import of existing exported feedback.
Its native hash-bound source-path reconciliation retained the newer issue's two
comments, normalized 16 stale local source paths, and restored normal `br` access.
Native export assigned those imported comments new local IDs while preserving
text, author and timestamp. The dedicated delivery Bead was then created normally.

No live PostgreSQL checks, macOS or hosted-CI execution, downstream migration,
commit, publication or deployment was performed for this delivery. Authentication
and proxy trust remain application policy; the peer is the proxy when a proxy
opens the socket. No custom listener/connection-info support, stream deadline,
descendant-joining or runtime-death guarantee is added.

## Protected operational composition alignment: 2026-09-12

Owning Bead `batter-7r3.6`; Jig plan
`plan_01M2BQFY9P0WSNX0988TPED3Q8`; baseline
`33a8c0cf7d6062b3ede023a52fb07c22e84910c7`. The foundation adds
`HealthMonitor::register_in`, moves the shared opaque panic payload behind a
neutral crate-root path with compatible startup re-exports, and makes protected
startup the documented and runnable composition path. No dependency or lockfile
change was made. Cargo.lock SHA-256 remained
`13d5a89554eb34020d89b63855d00a64307b6e1711394e2603455f9850bc248e`.

Executed locally on macOS 26.6.2 arm64 (Darwin 25.6.0) with rustc 1.98.1
(`48a229cea`, 2026-09-01) and 1.94.0 (`4a4ef493e`, 2026-03-02), Tokio 1.53.1,
Axum 0.8.9 and SQLx 0.9.0.

| Command / evidence | Executed outcome |
| --- | --- |
| Focused foundation command, startup, protected-startup and health targets | 63 cases passed. The health inventory increased from 15 to 17 with duplicate-registration inertness and protected registration/readiness/shutdown coverage; command tests compile the neutral panic path while startup tests retain the compatible path. |
| `cargo test -p batter-example-postgres-lifecycle --locked`, reference all-target/all-feature check, and the HTTP example tests | The PostgreSQL package passed 10 non-live cases with three live cases explicitly ignored; the protected reference root compiled; all 12 HTTP example cases passed. Protected startup's `InitializationError` layer retained the concrete PostgreSQL `ProcessFailure` and cleanup source chain. |
| `bash scripts/verify.sh` | The final default-toolchain core/minimal, workspace runtime, hostile-environment, doctest, formatting, strict Clippy and rustdoc matrix passed. An earlier run reached doctests with two new documentation return-type coercion errors; those snippets were corrected and the complete matrix was rerun successfully. |
| `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh` | The same complete matrix passed on the retained minimum toolchain. |
| Rebuild `batter-axum` example `http_service`, then run `scripts/smoke_http.py` in default, `--signal SIGINT`, `--deadline`, `--warn-filter`, and `--warn-filter --deadline` modes on each toolchain | All ten rebuilt process smokes passed with readiness, response, correlation, telemetry, selected-signal and exit-zero assertions. |
| `scripts/jig work check --plan-id plan_01M2BQFY9P0WSNX0988TPED3Q8`, followed by evidence/gates inspection | All five required sibling targets passed with fresh coverage of the 24 source and contract paths: api:test receipt `receipt_01M2BRBMFH34WRDSM9RHC7YE9E`, plus Clippy, formatting, contract and file budget. |

The duplicate health-registration case proves name rejection occurs before any
probe invocation and destroys the unregistered writer. The protected case proves
the registered monitor acknowledges component startup, publishes a successful
sample, participates in process readiness, stops under owned shutdown and then
invalidates its reader. Direct `HealthMonitor::run`, `Startup::new`,
`register_http`, `batter_runledger::register`, and the startup panic-payload path
remain available as lower-level compatibility contracts.

No live PostgreSQL suite was executed for this change, so the migrated database
composition has compile, offline failure-contract and prior historical live
evidence only. No hosted CI, new Linux execution, publication, deployment,
commit, or remote-effect guarantee is claimed. Bead `batter-7r3.6` comment 66
records this partial delivery without closing the umbrella task. The comment was
added through `br --no-db` because the authoritative JSONL has 23 issues absent
from the stale SQLite cache and native reconciliation reports 21 unrelated
semantic conflicts; no unsafe database flush or conflict resolution was attempted.

### Review follow-up: process-root signal ownership

The PostgreSQL production root now selects `.with_unix_signals("signals")`
before passing its configured `ScopedStartup` to `serve`; the reusable unit-test
helper constructs the same protected startup without installing process-global
handlers. A fresh-process regression runs that helper, then separately requires
SIGTERM and SIGINT to retain their default terminating dispositions. The task
failure case again requires exactly its one application component, so an
unexpected signal task cannot disappear from the report assertion.

The focused signal-disposition test and strict package Clippy passed. The full
`bash scripts/verify.sh` and `RUSTUP_TOOLCHAIN=1.94.0 bash scripts/verify.sh`
matrices then passed on the same macOS arm64 host, including 10 passing non-live
PostgreSQL cases and three explicitly ignored live cases. Rebuilt HTTP examples
passed all five documented smoke modes on each toolchain. Consumer guidance now
names `register_http_in` and accurately assigns budgets and signal selection to
their owning PostgreSQL example files. No live PostgreSQL, new Linux, hosted CI,
publication or deployment evidence was added by this follow-up.
