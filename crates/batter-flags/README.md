# batter-flags

Typed flags and kill switches for Batter applications. An application declares
every flag and kill switch once, in one catalog, and evaluates them only
through typed handles: one view per HTTP request, job invocation or periodic
run. Declared mode resolves values once at startup from the application's
existing settings source and changes them by restart. The optional control
plane, selected with `postgres`, loads and refreshes an audited PostgreSQL
snapshot that a registered component keeps fresh.

No default features. `postgres` re-exports the store as `postgres` and
exposes `control_plane`, the protected registration; `axum` exposes `http` for
route-group gates on the Batter Axum adapter; `runledger` exposes `jobs` for
job-handler gates and pinned decisions. `core` is always the catalog crate
(`batter-flags-core`) itself, so handles, decisions and views keep one type
identity through the direct and facade paths.

```sh
cargo test -p batter-flags --no-default-features --locked
cargo test -p batter-flags --all-features --locked
```

Experimental. Declared mode is implemented: `declared::resolve` consumes a
catalog and the root's captured `SettingsSource` once and returns a `Resolved`
with strict literals, ownership, retired and unknown names, required settings
and invariants, a redacted startup report and two revisions; `Resolved::into_flags`
enters declared mode. `test::TestFlags` is the fixture path, and
`low_level::capture_now` is the disclosed, non-canonical live read. The
protected periodic and `runledger` job gates are implemented. The protected
HTTP gates and control-plane registration arrive with later delivery tasks of
the flags epic. The catalog grammar and evaluation live in
[`batter-flags-core`](../batter-flags-core/README.md), the store in
[`batter-flags-postgres`](../batter-flags-postgres/README.md) and the operator
command in [`batter-flags-cli`](../batter-flags-cli/README.md).

## Periodic maintenance

`Flags::gate_periodic_in` registers gated maintenance through an existing
`RegistrationTarget`. Construct `periodic::GatedPeriodicPolicy` from the
interval and run budget; startup is always immediate and work stops at drain.
The callback receives an `OperationContext` and the one `SnapshotView` used to
gate that run. Awaiting publication and tripped switches skip without calling
the callback. `GatedPeriodicReader::skipped()` counts both; its `snapshot()`
returns core's periodic evidence, where `succeeded` includes skipped runs.
A publication during work reaches the next run and does not cancel active work.
Work needed for native settlement during drain must register ungated through
core. The method's rustdoc contains an executable registration example.

## Job handlers (`runledger`)

Implement `jobs::FlagAwareHandler`, then construct the native `JobHandler` with
`Flags::flagged`, `Flags::gated_direct`, or `Flags::gated_workflow_step`. Typed
handlers implement `FlagAwareTypedHandler` and enter through
`into_flag_aware()`. Register the wrapper in Runledger's native catalog/registry;
Batter owns its supervisor through `batter_runledger::register_in`.

Each invocation receives one `JobFlags` view, captured before the switch check.
Typed decoding happens only after admission. A later switch change reaches the
next invocation; it does not interrupt work already admitted. The native
`on_dead_letter` callback forwards the original raw payload, including malformed
payloads. Direct legacy `execute` is refused with
`job.execution_services_required`.

Both gate constructors require `GatePolicy { when_tripped, awaiting }`.
`Recheck::after` accepts 30 seconds through one hour; deterministic per-job spread
adds at most 25%. `DirectWhenTripped` allows `Defer`, `DeferAttempt`, and `Drop`;
`StepWhenTripped` allows only the latter two. Before the first publication the
wrapper always defers an attempt with `awaiting`. A settings floor is an actual
operator trip and can apply `Drop` even before publication. Prolonged initial
unavailability can exhaust attempts.

`Defer` returns a native continuation: after commit it resets attempts, advances
the run and releases resource claims. `DeferAttempt` consumes attempts and uses
stable code `flags.job_type_deferred` with a retry lower bound. `Drop` uses
`flags.job_type_dropped`. A direct gate used on a workflow step that has not
opted into continuations is rejected by Runledger with
`job.workflow_handler_continuation_not_enabled`; the direct-only assertion is
application policy. Scheduled jobs still materialize while a gate is tripped.
The complete durable-effects matrix and its execution-evidence label live in
[`jobs` rustdoc](src/jobs.rs).

For pinned flags, match all `PinnedResolution` arms. `Honoured` replays the pin,
`Overridden` applies its governing switch, and `Incompatible` carries a
`PinRefusal` bound to that view and flag. Consume the refusal with
`fall_back_to_live()` only if application policy permits live fallback;
otherwise return the application's deferral. Jobs without admission-time pins
use `evaluate_unpinned` with `ScheduledJob` or `PayloadPredatesPinning`.
Batter never persists pins: store one in the domain row atomically with enqueue
and replay it on retries. Keyed enqueue compares the complete payload, including
pin origin/version; `Pinned::same_decision` is not an idempotency equivalence.

Worker-free failure contracts run with:

```sh
cargo test -p batter-flags --features runledger --locked
```

The ignored `jobs_live` target requires `runledger,postgres` and an externally
provided `DATABASE_URL` naming a dedicated disposable PostgreSQL 18 database.
It runs the protected supervisor, inspects durable continuations, retry exhaustion,
drop hooks, resource release, workflow refusal and keyed pin conflicts. It waits
for the persisted rechecks, so allow several minutes. Provision using the existing
external harness or Docker test support; never use a production database.

```sh
cargo test -p batter-flags --all-features --test jobs_live --locked -- --ignored --test-threads=1
```
