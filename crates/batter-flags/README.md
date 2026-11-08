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
protected periodic, `axum` HTTP and `runledger` job gates are implemented.
With `postgres`, `control_plane::register_in` turns a `Resolved` and a
`ControlPlaneStore` (whose pool `batter_sqlx::pool_in` registered on the same
target) into
`RegisteredFlags`: the `flags.refresh` loader reads, validates and publishes
snapshots on a `RefreshTiming` cadence under a `LoadPolicy`
(`unready_until_loaded` or `start_on_defaults`); `flags.presence` and
`flags.presence_prune` keep the fleet view current; and the
`flags.presence.stopped` cleanup records the stop before the application's
pool closes. Registration refuses caller-owned, foreign-owner and
extracted-cleanup pools before adding any component. `RegisteredFlags::status()` reports a `SnapshotStatus` in
precedence order, and `readiness_condition(Gate::Published | Gate::Fresh)`
returns the `flags.snapshot` condition. `grants::grant_fragment` lists the
exact privileges of each `FlagsOperation`. `LoadPolicy::with_last_good(SnapshotDirectory)`
keeps the last control-plane publication in a `BFSN` v1 file
([format](docs/snapshot-file-v1.md)): registration publishes a usable file
before the driver starts, as `Source::PersistedFile`, so a restart during an
outage serves its engaged kill switches; a missing, damaged or foreign file
publishes nothing, and `startup_file()` says which (`Disabled`, `Missing`,
`Published`, `Unusable`); each control-plane
publication is persisted under a directory lock, and a persist failure goes to
`last_persist_failure()` and never changes a run's outcome. The file is a
cache: deleting it is always safe. `LoadPolicy::with_listen(ListenEndpoint)`
also wakes the refresh when an audited write commits: a listener, on a
two-connection pool of its own for each session, LISTENs on the store's
channel, wakes the refresh once after every LISTEN to catch what it missed, and
again on every notification, never sooner than the wake spacing after the
previous run. The spacing may not exceed the poll interval, so the poll stays
the backstop and a lost listener delays a change by at most one poll. Each
session's pool is closed within a bound when the session ends, so a connection
a dead peer keeps busy never blocks the next one; at drain the listener
unlistens first. Live tests cover delivery to every listening process within
the spacing, coalesced bursts, a notification held by an uncommitted
transaction, a terminated listener rebuilt on a new backend, poll-only
convergence and drain. The standby refusal, a pool closed during a receive and
the rebuild after a black-holed peer are unverified. The catalog grammar and evaluation live in
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

## HTTP route groups (`axum`)

Use `Flags::http_view()` to capture without gating, `Flags::http_gate(&halt)?`
to check one switch, or `Flags::http_gate_any(&halt, &[&other_halt])?` to disable
while any listed switch is engaged. The first handle is mandatory; constructors
reject foreign catalogs, switches not owned by the process kind and duplicates.
Install the returned `GroupGate` with `GroupPolicy::gated` or `browser_gated`.
The [HTTP rustdoc example](src/http.rs) assembles a named group and consumes it
through `AssembledHttp::in_process()`.

Handlers extract `http::AdmittedFlags` and evaluate or pin through `view()`.
Every extraction shares the one immutable view captured after admission;
publishing a new snapshot does not change an in-flight request. Neither
`AdmittedFlags` nor its `RequestFlags` payload has a public constructor or
implements `Clone`. An absent flags gate, or an application-built gate returning
a native `SnapshotView` (including `low_level::capture_now`), yields the fixed
sanitized 500 `internal_error` response. Native evaluation remains owned by
`batter-flags-core`; the adapter does not reinterpret flag values or store pins.

An engaged gate skips handlers and renders `HttpFailure::Disabled` through the
group's renderer, 503 `disabled` by default. It covers all group methods and
fallbacks; use a named group to keep other routes available. Probes and the
metadata-rendered fallback bypass it. A custom renderer may choose a different
status if a load balancer would eject a host on 503. The separate disabled HTTP
completion defaults to INFO at target `batter`; select
`gate.with_disabled_level(tracing::Level::WARN)` if `batter=warn` must show it.

Capture-only gates emit no flag evaluation. Switch gates read every listed
switch once, even after finding an engaged one. Each emits one
`feature_flag.evaluation` at DEBUG while released or INFO while engaged, target
`batter::flags`; handler evaluations add their own events. Error decisions use
core's first-occurrence WARN/repeat DEBUG policy. At an INFO filter a released
single-switch request adds no evaluation event; an engaged request adds one.

Before publication, HTTP follows the reader's existing `PrePublication` policy.
`ServeDefaults` (the planned `start_on_defaults` loader policy) serves when every
switch's effective decision is released, and disables for an engaged safe state,
settings floor or stale rule. `Unready` also disables provider-not-ready decisions,
even with a released safe state. Capture-only groups impose no switch decision.
The source remains `NotLoaded`; serving is not proof of publication. This choice
follows the plan's startup-defaults contract and the existing reader/native
switch resolution. Jobs defer an awaiting-publication reading, and periodic
maintenance skips it. The loader and its first-attempt readiness sequencing
belong to `control_plane::register_in`, not to this HTTP adapter: under
`unready_until_loaded` admission stays closed until a control-plane read is
published, and under `start_on_defaults` it opens after the first bounded
attempt.

```sh
cargo test -p batter-flags --features axum --locked
```

HTTP tests cover refusals, private construction, wrong-view rejection, event
counts and severity, defaults/unready/stale/floor admission, and publication
while a request is suspended. They use the sealed in-process transport; they
do not establish real socket or TLS serving behavior.
