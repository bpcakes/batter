# API assessments: Typed flags control plane

[ADR-010 policy](../../adr/010-agent-only-consumption.md) · [Assessment index](README.md)

These records apply ADR-010 to the flags packages of
[ADR-016](../../adr/016-typed-flags-control-plane.md). They retain the evidence
and limitations recorded at delivery; current contracts remain in rustdoc
and [guarantees](../../guarantees.md#typed-flags-experimental-adr-016).
The wakeable periodic seam (T-18) and the LISTEN wake (T-19) were withdrawn
before merge, and their assessments with them. The flags packages are
unreleased, so the `Breaking:` changelog entries the records below cite for
changes inside those packages were dropped before merge; entries for packages
that exist on master remain. Surface counts below are those of the commits
named and were not regenerated.

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
takes the others down. A view is built from one immutable `Record` that only a
`Publisher` advances, so freshness, standing and snapshot cannot be paired
across publications, and the publisher's crate-private judgement is the only standing transition
(`batter-jkw6.39` moved the record's lifecycle into the publisher). `gate_switch` returns an
exhaustive `GateReading`, so awaiting publication cannot be mistaken for open.
`pin` refuses an error decision with `Unpinnable` (`batter-jkw6.45`; it used
to pin the code default, and a replay honoured it as cached), and
`PinnedResolution` is exhaustive; `PinRefusal` must be answered. Residual (Q5): the disclosed `low_level` path builds a view with any
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
`Drop`; awaiting publication a `gated_direct` job continues after that recheck
without spending an attempt, while a workflow step, which cannot continue, and
a `flagged` job fail the attempt, so a long outage can exhaust theirs.
`JobFlags` has no
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
typed `RegisterError`s; the loader is the I/O shell around core's `Publisher`,
so an applied version and digest are always those of a judged snapshot. One
published record carries the snapshot, its authority and its age evidence;
`SnapshotStatus` reports the writer's liveness, what that record serves and the
latest judged read as separate fields, read under the lock the refresh
publishes under (`batter-jkw6.40`). `readiness_condition`
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
(`batter-owtx`; a `gated_direct` job has since continued without spending
one); and the operator command's JSON and retry guidance are
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
first ran before migrating when the schema existed and again after, and a
refused first migration had already committed. The second implicit-state audit
(`batter-jkw6.38`) counted that as a hidden third state, "schema absent, cannot
tell", folded into `Elsewhere`, and an outcome a caller could not tell from a
refusal that applied nothing. Since `batter-jkw6.41` the probe creates the
schema, if absent, inside a transaction it always rolls back, so PostgreSQL
resolves the path against it and the decision precedes the migrator on every
run; the check after migrating is gone. The probe needs no privilege beyond
the database `CREATE` that SQLx's own `CREATE SCHEMA IF NOT EXISTS` needs.

What types cannot express here is remote session state. The check samples one
session, the probe's, before the migrator runs on its own connection. Those of
one pool share their defaults unless the login's role or database settings or the
pool's connect options (`PgPool::set_connect_options`) changed while it held
connections, so such a pool can pass while other sessions are hazardous.
Inspecting the migrator's own connection would not close that gap: the flags
migrator's statements are schema-qualified, and the hazard lies in the host's
sessions, which the store cannot enumerate. A separate host migration login, a
host migrator that keeps a `SET search_path` made after connecting, and the
strict low-level `MIGRATOR` are outside the check, and stay application policy,
documented in the store README. A failure while migrating can follow
committed migrations, as the `StoreError` contract states; a refusal cannot. The new class is a hard cut of the
exhaustive `ErrorClass`, recorded with `Breaking:` and `Migration:` text and no
shim. Evidence: the live `migrate_refuses_a_session_whose_search_path_resolves_into_flags`
test. With an explicit `flags` entry, a first migration is refused before it
runs, even while another session holds SQLx's migration lock, and creates
nothing; a path without `flags` then migrates once; and with the schema in
place the path is refused again with nothing applied twice. A local run under
a temporary role named `flags`, refused on both migrations, predates the
rolled-back probe of `batter-jkw6.41` and is kept as history.

## Publication lifecycle (`batter-jkw6.39`)

The second implicit-state audit (`batter-jkw6.38`) found the publication record
was a state machine its loader drove by hand. `Published` could not represent
"nothing published", so the writer kept `Option<Published>` and chose the
public `Published::first` or `next` itself; `first` could be called at any time
and skipped the authority, accepting a regression; `Candidate::carrying` was a
paired call `next` never checked, so a loader that omitted it released a held,
engaged switch; a refusal before the first publication was dropped; and the
regression check, carrying, validation, deadline check and first-versus-later
choice were split between the refresh and the writer.

`low_level::Publisher` now owns the record through its lifecycle: `new` or
`restore` (the last-good file, aged from registration) creates it, `judge`
refuses a foreign scope, a file and a regression before looking at rows,
carries what it serves into the candidate (replacing whatever it carried; a
first publication carries nothing) and validates, and `Judged::publish` records
the outcome, or dropping the judgement records nothing. The publisher is not
`Clone`, so a superseded copy of a record cannot be advanced; `Record` is the
cloneable read side with an explicit `Unpublished` state that keeps a refusal.
The refresh calls one `Writer::read(candidate, deadline)`.

The publisher cannot prove the candidate's rows are the store's: a loader that
reads the wrong rows still publishes them, which stays the I/O shell's
responsibility. Standalone snapshots and the `*_view` constructors remain for
evaluation over a snapshot no publisher judged; they assert their own
freshness, as the module's caller obligations state. The removed calls get no
shim, because each accepted a snapshot that skipped carrying or the
first-versus-later choice. Evidence: `batter-flags-core/tests/published.rs`
(the carrying test fails with carrying removed from `judge`, checked by
mutation) and the writer tests in `batter-flags/src/reader/tests/writer.rs`.

## Snapshot status (`batter-jkw6.40`)

The same audit found `SnapshotStatus` was a precedence enum over three state
dimensions, loader liveness, what is served and the latest read's verdict, so
`Regressed`, `Rejected` and `SchemaBehind` dropped what was served. The crate's
own `Gate::Published` then needed a second read of the publication at another
instant, a published record whose standing read `Unpublished` was reported as
`NotLoaded` with a zero duration, and `Rejected.observed` was an `Option` that
was always `Some`.

`SnapshotStatus` is now a struct of `loader: LoaderState`, `served: Served`
and `latest_read: LatestRead`, refused reads carrying core's `Refusal` and the
store's version, and `satisfies(gate)` is a pure function of one status. The
refresh publishes and records its verdict in one critical section of the
shared state, and the status is read under it, so the fields belong to the
same read. Live-only questions moved from the reader, which answered them with
stand-ins in declared and test modes, to a `LiveRecord` only a control-plane
reader has; served state and the heartbeat's report are derived from the
record's own state and its snapshot's provenance, with no impossible arm.
Residual: the fields are public, so a caller can build a status no loader
reports, such as a control-plane publication with no judged read; a status
grants nothing, and the nested alternative was judged too costly for a report.
`RejectionCause` is removed. No shim: an enum alias cannot reproduce the
dropped `served` fact without the precedence that hid it.

Public-surface check for `batter-jkw6.39`/`.40`/`.41` against branch commit
`04627167`, with the repository's rustdoc JSON walker on cargo 1.100.0-nightly
(b2e9d5f9d 2026-09-02): `batter-flags-core` 1,253 -> 1,289 entries, removing
`Published::first`, `Published::next` and `Published::rejected` and adding the
`Publisher`, `Judged`, `Record`, `Unpublished` and `Restored` items and
`Base::record_view`; `batter-flags` 517 -> 536, removing `RejectionCause` and
the `SnapshotStatus` enum's variants and adding the struct's three fields, its
`satisfies`, and `LoaderState`, `Served` and `LatestRead`; `batter-flags-postgres`
517 -> 517 with nothing removed or added. Each removal has a `Breaking:`
changelog entry with before/after code, a `Migration:` sentence naming it on
the replacement and a recorded no-shim decision. The name-only walker cannot
see that `PooledStore::migrate` kept its signature while its refusal moved
before the migrator; the changelog records that behaviour change.

## Remaining audit items (`batter-jkw6.42` to `.57`)

The second audit's other findings were repaired as follows; each bead records
its tests and live evidence.

- `low_level::gate_switch` fails closed: a switch the view cannot answer for
  reads as `Tripped` (`.42`). A witness type was not added because every
  protected gate checks its switches at construction, and a new `GateReading`
  variant would force impossible arms on every surface.
- A `Candidate` takes its version when it is made and keeps its scope, so
  `MissingVersion` and a second scope argument are gone (`.43`).
- `CatalogBuilder::new` names the first process kind and environment class, so
  `build` cannot fail (`.44`); `BuildError::RepeatedCall` is gone.
- `pin` refuses an error decision with `Unpinnable` (`.45`).
- Every operator read runs the store's primary and schema guard in its own
  snapshot (`.46`), and each read operation's grant includes the guard's
  `flag_control` columns; the review of this round found the inventory had not
  followed the guard, and that `CatalogRead` already lacked the
  `flag_processes (manifest_digest)` its listing reads, which only a login
  holding exactly one read operation shows. The live grant case now runs each
  read under such a login. `history` returns a page with an explicit continuation
  (`.47`); write outcomes record an accepted coordinated flip (`.48`); a
  process presence is bound to the store that issued it (`.49`).
- Core counts gated periodic skips itself (`register_skippable_in`,
  `PeriodicSummary::skipped`), never as successes (`.50`).
- `register_in` checks every name before registering anything (`.51`).
- `RegisteredFlags::last_good()` reports the persister's state, a stale
  selection cannot release a newer one, an oversized snapshot is not retried,
  and a `LoadPolicy` is not `Clone` (`.52`). The persister's queue names the
  version it holds or has settled (`Empty`, `Settled`, `Waiting`,
  `TooLarge`), and only a newer offer replaces it: every confirming read
  offers its version again, which used to re-queue an oversized or
  newer-on-disk version and clear a waiting failure on each poll.
- `declared::resolve` refuses a narrowed settings source (`.53`). A type split
  of core's `SettingsSource` was weighed and judged too costly for every root;
  the source records its narrowing and the refusal is explicit at startup.
- `TestFlags` is behind `test-support` (`.54`); `FlagsRole::ServingProcess`
  names the serving grant and a `FlagsGrant` cannot be empty (`.55`); an
  ungated job defers before publication under `unready_until_loaded` (`.56`).
- `GateView`/`AdmittedFlags` outside a matching gated group (`.57`) is not made
  a compile-time error: the extractor would have to require a gate-owned router
  state, and the orphan rule forbids the `FromRef` that would let a handler
  extract `State<AppState>` beside it. The misuse stays a sanitized 500, now
  named `gate_outcome="unmatched_view"` in the completion event.

Public-surface check of the whole working tree, `.39` to `.57` together,
against branch commit `04627167`, with the same walker and nightly toolchain and
every feature enabled: `batter-core` 1,343 -> 1,368 entries (0 removed, 25
added); `batter-axum` 606 -> 606 (none); `batter-flags-core` 1,253 -> 1,295 (8
removed, 50 added); `batter-flags` 517 -> 580 (25 removed, 88 added);
`batter-flags-postgres` 517 -> 536 (2 removed, 21 added); the `batter` facade's
23 re-export entries are unchanged. This round's removals are `Candidate::at`,
`BuildError::NoProcessKind`, `NoEnvironmentClass` and `RepeatedCall`,
`SnapshotRejection::MissingVersion`, `LoadPolicy: Clone`,
`RegisteredFlags::last_persist_failure`, `GateError::DeferOnWorkflowStep`,
`FlagsGrantError::EmptySelection`,
`FlagsOperation::OperatorCli` and `PooledStore::heartbeat` and `stop`. Each one
has a `Breaking:` changelog entry and a `Migration:` sentence naming it. The
walker compares names only and runs with every feature enabled, so the
signature changes (`CatalogBuilder::new` and `build`, `Candidate::new`,
`SnapshotView::pin`, `grant_fragment`,
`Flags::flagged`, `PooledStore::history`, `KeyListing::kind` and `value_type`,
`Deferral::flip`, `PeriodicSummary::skipped`) and the `test-support` gate on
`batter_flags::test` were checked against the changelog by hand.
