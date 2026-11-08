# API assessments: Typed flags control plane

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to the flags packages of
[ADR-016](../../adr/016-typed-flags-control-plane.md). They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md#typed-flags-experimental-adr-016).

## Flags store assessment (T-12, batter-jkw6.12)

Review exposed an owning-adapter gap: an outer timeout could discard an already
observed application error while the native savepoint/snapshot owner awaited
recovery. Additive native deadline-aware methods now retain that error outside
the cancellable cleanup phase and report `RecoveryElapsed` explicitly. The store
does not maintain a parallel outcome slot or ask callers to coordinate cleanup.
Native migration likewise owns session timeout installation and unconditional
retirement because SQLx's advisory-lock phase precedes its migration transactions.
The public unbounded methods remain compatible; no shim or hard cut is needed.

The owner-directed explicit-state rule also applies internally. Request variants
retain their typed payload; stored rows decode to Absent/Active/Archived and
processes to Live/Silent/Stopped. Prepared decisions carry distinct commit and
rollback payloads. Their consuming completion methods require native proofs,
so public acknowledged outcomes are not used as provisional phase markers.
Wrong-kind JSON is refused before later policy checks, matching core decoding.
Real server/wire regressions target these causes; execution evidence belongs to
the registered review run, not this design assessment.

The canonical `PooledStore` accepts complete validated requests and owns all
transaction phases. Actor/reason limits, nonzero bounded timing and explicit
kind/state pairing are checked before acquisition. There is no raw-connection
store, transaction callback, caller finalization or retry API. `Loaded` exposes
read-only metadata: callers cannot relabel a database read's version before
converting it to a core candidate. Outcomes distinguish acknowledged mutation,
rollback-only no-op/refusal, CAS mismatch and uncertain commit.

Database state still determines declaration/encoding/archive/fleet compatibility;
those cannot be proved from local request construction. Triggers enforce the
counter/audit invariant for non-owner non-superuser writers. Authenticated actor
meaning, privilege provisioning, fleet completeness and remote effects remain
application/database policy. Explicit never-declared writes are a deliberate
capability and still pass the same database checks.

The current `batter-sqlx` owners always retire atomic/snapshot sessions, have no
separate snapshot-acquisition timeout seam, and do not expose the COMMIT send
point. T-12 retains retirement, caps complete inspections by the acquire budget,
and classifies commit-phase deadlines conservatively as uncertainty. It adds no
second native transaction owner and leaves no paired completion obligation to
consumers. These are deliberate limits; future narrower classifications or pool
reuse belong in the upstream owner. Live failure scenarios cover deadline/drop,
commit rejection/lost acknowledgement and concurrent first writes. Fresh-agent
usability evaluations remain unexecuted.

## Flags catalog and declarations assessment (T-04, `batter-jkw6.4`)

Relationships are stated on `CatalogBuilder` with both handles, so a pair can be
made reciprocal, with one call per direction, whichever flag was declared
first, and each side names its variant:
an implicit "enabled" would be ambiguous for enums and for toggles defaulting
on. `both_or_none` exists only between toggles, which its signature enforces.
Build refuses what pure construction can know: a self-relationship, an aliased
or undeclared variant, code defaults that break an invariant, a catalog without
a process kind or environment class, a duplicate key, a retired name in use.
Ownership restriction is a type state on the declaration (amendment A3); a
repeated `only_in` or `governed_by` call cannot compile: trailing policy
types preserve assignment through every other builder operation. Invalid drafts
retain their first rejection in an absorbing enum state (`batter-jkw6.36`).
`Catalog::owned_by` refuses a foreign kind instead of answering it. Residual
(ADR-016): a process kind can still name a handle it does not own; handles are
catalog-wide, so the protected outcomes are refusal at gate and wrapper
construction and `Reason::Error` at evaluation.

## Flags snapshots, evaluation and pins assessment (T-05, `batter-jkw6.5`)

Validation rejects a candidate whole only for structural corruption, and holds
any other bad row on its own with an exhaustive `HoldCause`, so one row never
takes the others down. A view is built from one immutable `Published` record,
so freshness, standing and snapshot cannot be paired across publications, and
`Authority::consider` is the only transition. `gate_switch` returns an
exhaustive `GateReading`, so awaiting publication cannot be mistaken for open.
`pin` is infallible and `PinnedResolution` exhaustive; `PinRefusal` must be
answered. Residual (Q5): the disclosed `low_level` path builds a view with any
caller-asserted `Source` and `Freshness`; only native code observes it, and no
protected surface accepts one.

## Declared mode, reader and test flags assessment (T-06, `batter-jkw6.6`)

`declared::resolve` reads only the `SettingsSource` it is given and returns
`Resolved`, the witness `into_flags` and `register_in` consume, so no flags exist
before resolution. `Flags` has no view-returning method; the one live read is
`low_level::capture_now`. `TestFlags::builder` takes a `ProcessKind`, and refuses
a foreign, not-owned or repeated handle at build, so a fixture cannot evaluate
what the process would not. The reader publishes one record through a `watch`.
Residual: invariants across process kinds are unknowable at resolve and are not
checked, in either mode.

## Route-group gate seam assessment (T-07, `batter-jkw6.7`)

`GroupGate` is accepted only by `GroupPolicy::gated` and `browser_gated`, so a
gate cannot be installed outside the library-ordered boundary, and a disabled
group renders `HttpFailure::Disabled` through its own request policy. The new
variant is a recorded hard cut with no shim: a shim would need a wildcard that
hides a disabled group. Probes and the metadata fallback stay outside the gate
by construction.

## HTTP flags assessment (T-08, `batter-jkw6.8`)

`Flags::http_view`, `http_gate` and `http_gate_any` validate every handle at
construction and capture one view per request; handlers read it only through
`AdmittedFlags`, which never accepts a native or caller-built view, so a
request cannot mix views or evaluate after a later publication. The reading is
matched exhaustively in the gate builder; before publication the reader's
`LoadPolicy` decides. Q5 residual: the explicitly native `low_level` path can
still build a `SnapshotView` with any `Source`; only native code or an
application-built `GroupGate` observes it, and `AdmittedFlags` never accepts it.

## Job flags assessment (T-09, `batter-jkw6.9`)

`gated_direct` and `gated_workflow_step` are separate constructors taking
`DirectWhenTripped` or `StepWhenTripped`, so the workflow-step constructor
cannot select a deferral a step cannot honour. Both return `GatedHandler`, so a
direct handler enqueued as a workflow step is still refused only at run time,
an application assertion `jobs_live`'s `drop_and_workflow_refusal` exercises.
`GatePolicy` takes the awaiting-publication recheck explicitly, and `Drop`
applies only to a tripped reading, so missing initial data never selects
`Drop`; awaiting publication fails the attempt with that recheck, and a long
outage can exhaust the job's attempts. `JobFlags` has no
unrestricted live evaluation: unpinned facts report `UnpinnedBecause`. Batter
never writes a pin into a Runledger payload; the application stores it.

## Periodic flags assessment (T-10, `batter-jkw6.10`)

`Flags::gate_periodic_in` takes a `GatedPeriodicPolicy` whose immediate startup
and stop-at-drain are fixed, and runs work only on an `Open` reading of one
captured view; awaiting and tripped readings skip. `StopAtDrain` is fixed: a
switch that skipped work native settlement needs during drain would hold drain
until forced cancellation; such work registers ungated through core
`register_periodic_in`.

## Flags presence, prune and fleet assessment (T-13, `batter-jkw6.13`)

A heartbeat runs only through `PooledStore::heartbeat` with a `ProcessPresence`
built from the process's own base, so a process cannot report another
catalog's digest. `stop` consumes the presence, so a stopped process cannot
heartbeat again. Prune deletes in bounded batches under an explicit
`PruneLimit`. Fleet states are an exhaustive `ProcessState`. Residual: fleet
completeness, clock skew between hosts and remote process liveness are
database facts the store reports, not proves.

## Control-plane loader assessment (T-14, `batter-jkw6.14`)

`register_in` validates every input before registering anything and returns
typed `RegisterError`s; the loader is the I/O shell around core's `Authority`,
so an applied version and digest are always those of a judged snapshot. One
published record carries the snapshot, its authority and its age evidence;
`SnapshotStatus` combines that record with the loader's separately held schema
and refusal state and the writer's liveness. `readiness_condition`
returns the condition and its closure together under one name. The stop marker
is a reserved cleanup that runs before the pool closes. Residual: a restart
during an outage without a last-good file serves the declared base.

## Last-good file assessment (T-15, `batter-jkw6.15`)

The file is opened in the root's protected initializer as a
`SnapshotDirectory`, so an unusable directory fails startup with a typed error,
and `with_last_good` carries only an opened directory. Only control-plane
values are persisted, a held entry first read from the file without a value;
persistence runs after publish and never changes a run's outcome or readiness.
A persist failure is retained as a `PersistFailure` for
inspection. Residual: power-loss durability of the directory sync, which returns
`Ok` on macOS APFS, and `flock` on tmpfs or overlayfs are unverified.

## Operator command assessment (T-16, `batter-jkw6.16`)

Every argument is validated into a `Plan` before any connection, so a usage
error never depends on database state. Writes require an actor, a reason and
exactly one of `--if-revision` or `--yes`; the coordinated-flip and invariant
policies are typed `WritePolicy` fields the guard builds, never booleans on the
store API. Exit codes and the JSON document are a closed contract, and an
unknown commit outcome is its own exit with re-run guidance. A signal before
COMMIT rolls back; after it, the real outcome is reported.

## Flags facade and consumer assessment (T-17, `batter-jkw6.17`)

The facade exposes the flags packages only behind `flags`, `flags-postgres`,
`flags-axum` and `flags-runledger`, so one `batter` dependency reaches them
without a direct declaration, and moves no ownership. `grant_fragment` refuses a
schema other than `flags`. The rollout example separates the finite migration
command from serving, which uses restricted credentials, and awaits checked
shutdown and cleanup once its supervisor has started. Residual: a failure
between registering the pool's cleanup and starting the supervisor drops the
cleanup without running it.

## Wakeable periodic seam assessment (T-18, `batter-jkw6.18`)

`periodic::register_wakeable_in` returns a `PeriodicWake` beside the reader; a
wake only requests work, never bypasses admission, the stopping point or the
allowance, and is inert after the component ends, so a holder needs no paired
cancellation or join. Wakes coalesce and every start keeps the spacing, so a
notification burst cannot overrun the budget. Wake counts live in the adapter;
`PeriodicSummary` gains no field.

## Flags LISTEN wake assessment (T-19, `batter-jkw6.19`)

`ListenEndpoint::new` validates the spacing, its `Debug` is redacted and
`register_in` refuses a spacing longer than the poll. The listener is a
component on its own pool, owns rebuild, resync and drain-time unlisten, and
reports an exhaustive health state; the application only supplies connection
options. Residual: a standby refusal, a pool closed during a receive and a
black-holed peer are unverified, and sqlx's detached `UNLISTEN *` is disclosed.

## Control-plane invariants assessment (T-21, `batter-jkw6.21`)

The invariant judgement is one function, shared by snapshot validation and the
store's write-side replay through `Manifest::invariant_holds`, so the two
cannot disagree about which flag a hold moves; it visits relations in
canonical key order and reports what no hold can meet as `unresolved`.
`register_in` refuses a base an engaged governing switch would break, so every
fixed fallback is consistent. A strict write that would newly hold a flag is
refused inside its transaction; accepting the hold is an explicit
`InvariantPolicy::AcceptHeld`, and the outcome's `Deferral` lists what it
accepted. `RegisteredFlags::held` and the heartbeat choose the freshness of a
view captured at that moment, so a stale view's holds are reported while it is
stale; the low-level `Snapshot::held` takes the freshness as an argument, which
its caller chooses. Residual: the write side sees stored rows
and code defaults, not a process's settings, floor or staleness, and a catalog
with more than four unreadable governing switches is reported unjudged.


## Flags control-plane fresh-agent evaluation (T-24, `batter-jkw6.24`)

Executed 2026-11-08 against an oracle frozen before launch
([evidence](../flags-evaluation-2-2026-11-08/README.md)). A fresh agent
given product requirements, and a starting project that already declared the
one `batter` dependency and installed the panic hook, built a service on the
facade alone that meets nine of the ten oracle items: protected control-plane
registration over declared resolution, a kill switch with its safe state and
stale rule, the route-group gate read through `AdmittedFlags`, a `gated_direct`
job with explicit policies, and a pin stored in the application's row in the
enqueuing `run_atomic` and resolved with every `PinnedResolution` arm. The
operator script partly meets the retry item: every write keeps the original
`--if-revision` and none uses `--yes`, but an exit-4 report without a
`proposed_version` is re-issued without reading history first. Nothing ran
against a database. The friction the agent reported in this run, under its
offline setup, signals remaining design debt on the canonical path rather than
established API limits: the flags example does not use protected startup
(`batter-qrqy`); the agent did not drive a gated handler in an offline test
(`batter-alk2`); it found no protected invocation-phase composition for a
flag-aware handler (`batter-31p9`); awaiting publication consumes attempts
(`batter-owtx`); and the operator command's JSON and retry guidance are
undocumented (`batter-xuv6`). One run of one agent is a signal, not a measure.

## Branch state-protocol repairs (`batter-jkw6.36`)

The write's interrupt owns Pending/Disabled/Requested states across both waits;
observing sender closure cannot leave a completed receiver eligible for polling.
The last-good writer acquires its explicit Writing state and selects the queued
snapshot and prior-write evidence under the same mutex. Its selected owner stays
inside the blocking task until completion or destruction; dropping a waiter cannot
reopen admission, and failed/abandoned selections retain the latest pending value.
A write whose rename succeeded but whose directory sync failed (`NotDurable`)
spends the first-write authority and stays queued, so its retry keeps a newer
file another process wrote meanwhile (review repair, `batter-jkw6.37`).

`CleanupSlot::register_tracked` mints opaque stack-identity evidence only after
installing the hook. SQLx keeps that evidence privately with a protected pool;
caller-owned constructors keep an explicit unregistered state. Flags checks it
against one borrowed registration target before any mutation. Extracting cleanup
transfers its identity away from the supervisor. Moving the supervisor preserves
it. This proves local registration ownership, not pool liveness, callback meaning,
or remote termination. Native pool access can still close a pool explicitly.

Presence reservation consumes its preparation and holds that same target until
components are installed, and lends helpers only fresh reborrows of it, so
internal helpers cannot omit or change the stop marker's owner. Encoding-1's two required fields transition through an exhaustive
Empty/Encoding/Payload/Complete enum; duplicate and incomplete input is refused.
No new public hard cut except the builder policy return types is needed; their
old-type shim would reintroduce repeated assignment, so none is supplied.

Public-surface check against branch commit `ae6e4107`: the repository's rustdoc
JSON walker reports 1,249 -> 1,253 entries for `batter-flags-core`, no removed
paths, and four additions (the two policy markers and their re-exports). Manual
signature comparison also identifies the new trailing declaration parameters
and the assigned-state returns of both `only_in` methods and `governed_by`;
the name-only walker cannot detect these cuts. The changelog covers reassignment,
conditional finalization and helper return types; no compatibility shim can
retain those old types without making repeat assignment possible again.

## Search-path refusal (`batter-jkw6.34`)

The first hosted run of the flags suites failed because the CI admin role was
named `flags`: PostgreSQL's default `search_path`, `"$user", public`, then
resolved the host migrator's unqualified `_sqlx_migrations` into the flags
ledger. A local reproduction showed the silent form of the same state: a host
migrator that ignores missing versions creates its tables in `flags` and
records its versions in the flags ledger, and `PooledStore::migrate`, which
also ignores unknown versions, never refused it.

The store can see only its own sessions, so the protected path refuses there:
`PooledStore::migrate` returns `ErrorClass::SearchPath` when a session of its
pool has a default path that resolves into `flags`. PostgreSQL itself resolves
the path from `reset_val`, the default that `DISCARD ALL` restores, including
quoting and `"$user"`, so no client-side parser can disagree with the server. `current_schemas` lists only existing schemas, so the check
runs before migrating when the schema exists and again after; a first
migration has therefore run when the refusal arrives, and a rerun after the fix
applies nothing twice. That order is the residual: the refusal is reported
after, not instead of, a first migration.

What types cannot express here is remote session state. The check samples one
session before and one after migrating, on separate connections. Those of one
pool share their defaults unless the login's role or database settings or the
pool's connect options (`PgPool::set_connect_options`) changed while it held
connections, so such a pool can pass while other sessions are hazardous.
Inspecting the migrator's own connection would not close that gap: the flags
migrator's statements are schema-qualified, and the hazard lies in the host's
sessions, which the store cannot enumerate. A separate host migration login, a
host migrator that keeps a `SET search_path` made after connecting, and the
strict low-level `MIGRATOR` are outside the check, and stay application policy,
documented in the store README. Any `migrate` failure can follow committed
migrations, as the `StoreError` contract now states. The new class is a hard cut of the
exhaustive `ErrorClass`, recorded with `Breaking:` and `Migration:` text and no
shim. Evidence: the live `migrate_refuses_a_session_whose_search_path_resolves_into_flags`
test (an explicit `flags` entry, refused after the first migration and before
the second, and a reset path migrating with nothing applied twice), and a local
run under a temporary role named `flags`, refused on both migrations.
