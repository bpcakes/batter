# Direct VictoriaMetrics export, 2026-11-09

Owning Bead: `batter-cw5f`. Baseline: `dd1ed7c1e40454b3a6d8a3e122dfe77eb825e566`.
This record covers the uncommitted delivery diff relative to that baseline.

## Executed live evidence

On Linux x86_64 with Rust 1.98.1 and Docker Engine 29.7.2,
`python3 scripts/check_victoriametrics.py` passed against
`victoriametrics/victoria-metrics:v1.156.0@sha256:da8a90ac0912f16589be2b96a6441a2fca4bca415c83783d972d92e3b5b66970`.
The harness uses an isolated container, temporary container-local storage and a
random loopback port, then removes its container. No existing backend is used.

Each of two isolated producer processes selected
`prepare_victoriametrics` and the protected service lifecycle, observed periodic
acknowledgement and successful final export after cleanup, and retained clean
exporter/provider closure. Queries against `/api/v1/query` then confirmed:

- separate `service.instance.id` series under one `service.name`;
- final cumulative operation counts of 2 and 5, including observations added
  after the periodic export;
- matching histogram counts and `+Inf` buckets, with positive duration sums;
- one successful cleanup and shutdown per instance and a shutdown-duration sample.

This demonstrates actual ingestion and read-back, not only HTTP acceptance. It
does not establish durability across server crashes. The single-node backend's
empty HTTP 200 without Content-Type was also observed directly. The explicit
profile accepts exactly that exception; the dual-profile scripted transport
matrix retains generic OTLP strictness and checks other failures without retry.
The live query uses default VictoriaMetrics naming/resource promotion.

Cluster URLs are validated and sent unchanged to a scripted fixture, but no
multi-node cluster was provisioned. Authentication, custom trust roots, macOS
Docker execution, application-specific metrics and logs/traces are outside the
executed evidence. Upstream semantics and immutable source links are in
[references](../references/runtime.md#victoriametrics-v11560-reviewed-2026-11-09).

## Public API comparison

The [surface diff](victoriametrics-api-2026-11-09.txt) uses the repository's
`public-api-diff-2026-10-09/surface.py`. Rustdoc JSON was generated for the
baseline in a Git-free temporary source copy and for the delivery tree, using
Rust 1.98.1 with `RUSTC_BOOTSTRAP=1 cargo rustdoc -p batter-otlp --lib --locked --
-Zunstable-options --output-format json`. The environment override applied only
to documentation extraction, not the verification toolchain.

Result: no removed items. One public function is added, plus three re-export
records caused by the private preparation-module extraction. Manual signature
comparison confirms unchanged `prepare`, `reject_ambient_environment`,
`Schedule`, `Prepared` and diagnostic types. Existing callers keep their paths;
no deprecated shim or Breaking migration is needed. The facade already
re-exports this adapter, so `batter::otlp::prepare_victoriametrics` is the same API.
The [ADR-010 assessment](api-assessments/lifecycle.md#direct-victoriametrics-metrics-export-batter-cw5f)
records locally enforced invariants and deployment-policy boundaries.

## Verification

Focused OTLP tests, doctests, Clippy and live Docker read-back passed. Harness
negative controls reject missing/merged instance evidence and retain container
cleanup on producer failure or ambiguous Docker launch.

`bash scripts/verify.sh` completed successfully on the pinned Rust 1.98.1:
formatting, file budgets, all four Clippy configurations, all six matrix parts,
rustdoc with warnings denied and all five HTTP smoke profiles. The workspace
batch took 943.287 seconds; the integrated VictoriaMetrics ingestion/read-back
check took 19.882 seconds and passed again inside this run. The single-facade
consumer and the new negative-control tests also passed. No successful matrix
was repeated after this run. A separate strict-inventory file-budget check found
zero errors; existing repository notices/warnings remain.

No Cargo dependency or lockfile change was needed. The test container was
removed. These are local Linux results, not hosted CI or macOS execution.
The owning Bead retains the completion summary.

## Harness cleanup follow-up

Owning Bead: `batter-pdfk`. The completed panel review identified two harness
defects: default SIGTERM termination skipped `finally` between subprocess
batches, and a failed Docker listing prevented any removal attempt. The
follow-up gives the fixture lifetime explicit scoped stop ownership compatible
with the subprocess runner and directly invokes bounded forced removal by the
owned unique name. Stop requests are recorded until controlled work checkpoints;
original signal dispositions are restored afterward. Native exception chaining retains
both the primary failure (including its cause) and any removal failure.

Real-signal controls synchronize at health and read-back polling checkpoints,
then send SIGINT/SIGTERM and additional stop signals during removal. The read-back
case executes real nested runner calls before interruption. Other controls check
failed removal after successful work and preservation of primary/cleanup errors.
A Docker 29.7.2 probe returned status 0 for forced removal of a random absent
name; the checked native implementation ignores only typed NotFound under force.
No error-string parser or listing prerequisite is needed. An attempted removal
does not establish daemon settlement after an ambiguous launch or an unavailable
daemon, and no SIGKILL/runtime-death cleanup claim is made.

The full Rust matrix above predates this follow-up. Backend code and dependencies
are unchanged; the repair controller pins the affected scripts matrix, actual
Docker ingestion/read-back, formatting and file-budget checks. Its final outcomes
and fresh independent review results are recorded in `batter-pdfk`.

The second fresh review found a coupled caller-budget defect: the matrix's
default ten-second escalation could preempt the harness's diagnostics/removal
or nested producer settlement. The consumer/API assessment classified this as
test-composition ownership, not a Batter API gap: the shared runner already owns
bounded escalation and exposes its grace. The harness now owns the component
allowances, and the matrix imports their sum plus scheduling headroom (32s).
Existing generic runner semantics and the first repair remain intact.
The added outer-runner control uses real signals and controlled delays at
health/read-back and active-producer checkpoints. A negative control reinstates
the old grace and observes preempted removal; the derived grace permits removal
and retained failure/termination outcomes. This closes a gap the direct-signal
controls and successful normal Docker execution could not establish.

A subsequent review reproduced a cleanup-entry gap in the initial exception-
raising approach: a second SIGTERM between disposition changes skipped removal.
The assessment therefore changed the mechanism, rather than adding another
guard inside `finally`. The existing private process runner now offers an
explicit record-only stop scope. Its default behavior remains unchanged; nested
batches in that scope propagate requests to the same owner. The fixture keeps
this owner throughout acquisition, controlled polling and finalization, with no
signal-disposition transition on cleanup entry. Initial Docker work uses that
bounded runner, preventing the cooperative owner from waiting through a long
synchronous launch timeout. Bounded diagnostics/removal use separate Unix
sessions so process-group stop requests cannot terminate those commands directly.

Controls additionally synchronize before removal begins, deliver repeated stop
signals there, and exercise a real child removal surrogate under process-group
signals. A late stop observed during successful removal is checked only after
removal completes, so it cannot bypass cleanup or silently pass the run. Existing
default-runner signal tests remain part of required validation. This is a private
tooling ownership extension, with no Rust public API or dependency change.

The subsequent review confirmed cleanup entry and identified one remaining
opaque blocking boundary: urllib's socket timeout does not bound a progressively
arriving complete response or a sequence of queries. The parent now runs each
actual health/read-back attempt in a private child under the existing bounded
runner, with a five-second work timeout and the same stop/reap allowances already
included in the 32-second caller grace. The worker retains native urllib/query
logic and caps response bodies at 2 MiB; the parent caps its captured output at
64 KiB. A completed not-ready result is distinct from timeout/interruption/crash,
which fail the run rather than being automatically replayed.

Actual loopback HTTP regressions send progressively arriving headers or chunked
bodies, cancel both health and metrics polling, and observe parent cleanup and
signal restoration. A separate test proves that continued body progress cannot
extend the entire polling attempt. These tests exercise the real parser and
worker instead of replacing HTTP work with a cooperative checkpoint. Required
verification discovers the full `test_victoriametrics*.py` control family.

## Explicit tooling execution contracts

Owning Bead: `batter-kwfa`; follow-up to committed branch head `04766294`.
`run_fixture` now owns work, failure diagnostics and removal, without exposing
a caller-managed fixture context. Work always uses the bounded interruptible
runner; fixed diagnostics/removal operations retain their separate sessions
and deadlines. Matrix command specifications carry their stop grace independently
of labels, and batches preserve the largest declared allowance. Only
`PollingNotReady` authorizes another polling attempt.

Executed focused checks passed on Linux: all 36 private runner controls, the
complete scripts matrix with 19 VictoriaMetrics controls, and actual pinned
Docker ingestion/read-back for both producers. New controls exercise renamed
labels and regrouped commands, reject unexpected parent errors without retry,
require clean settlement for retryable child results, and prevent work launch
after a recorded stop while still removing the fixture. Existing real-signal,
progressing-HTTP and primary/cleanup failure controls remain in place.
The owning Bead records final verification for this follow-up. No Rust API,
dependency or lockfile change is involved.

The complete `bash scripts/verify.sh` also passed on Rust 1.98.1 after these
tooling changes: formatting, file budgets, all four Clippy configurations, all
six matrix parts, rustdoc and all five HTTP smoke profiles. The workspace batch
took 747.493 seconds; its actual Docker ingestion/read-back took 11.916 seconds.
The final scripts matrix ran all 19 VictoriaMetrics controls successfully.

The [export-attempt assessment](api-assessments/lifecycle.md#export-attempt-state-representation-batter-kwfa)
now records the implemented Rust ownership/transition change (`batter-fzw1`),
including its private compiler controls and runtime cancellation/dispatch tests.
That follow-up changes the internal attempt representation; its final verification
is recorded separately in its owning Bead. The prior timing measurements above
remain evidence for their stated tooling snapshot.
