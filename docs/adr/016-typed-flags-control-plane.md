# ADR-016: Typed flags and kill-switch control plane

## Status

Accepted, 2026-11-08. Owning Bead: epic `batter-jkw6`; this record is
`batter-jkw6.26`. The repository owner directed the epic and answered its open
questions on 2026-11-05 (quoted on the epic). Status is split by provenance:

- The elements the provenance table below marks **evidenced** are a focused
  experimental extraction under
  [ADR-014](014-reuse-after-two-consumers.md#stable-adapter-promotion) track 1:
  every one is a requirement all three surveyed application roots share.
- Every element it marks **owner-selected** is an owner-directed experimental
  addition recorded beside track 1, not an extraction. No consumer-feedback
  Bead preceded it, and `batter-7r3` is closed, so the track is named by its
  description.

[ADR-015](015-outbound-transport-evidence.md) is a precedent for this Status
wording and its labels only, not for the overrides below: it extracted a rule
all three roots hand-write, added a standalone leaf that changed no existing
package API and was not SQLx-backed, and this record's control plane has none
of those properties. Everything here is experimental; the
[status](../status.md) rows written by `batter-jkw6.27` carry the label.

## Context

Every surveyed root reads its flags from the process environment once at
start, so every flip, every kill switch included, is a redeploy and a drain,
and serving and worker processes are replaced non-atomically. Each root
hand-writes the same startup rules, and each keeps its own out-of-band switch
whose staleness and unavailability semantics match no other's.

### Evidence

The survey covered the three application roots of ADR-014, anonymized here and
counted by each root's own method, so sums are indicative only.

- About 420 distinct environment names (419) at about 430 read sites (429) in
  237 files; one root's counts include about 38 false-positive lines and 122
  names from generated families.
- 54 names or name groups are self-reported as toggles (21, 12 and 21); 53 are
  enumerable (one root lists 20 against a stated 21): 23 bounded knobs, 16
  release toggles, 10 permission gates and 4 kill switches. Across all
  categories, 11 are presence-derived (a secret, URL or path enables a feature)
  and 7 are development-only.
- All three read every value once at process start. Out-of-band hot switches
  exist (Runledger `job_definitions.is_enabled` plus application rows re-read
  per attempt in one root, database-published readiness with a maximum age in
  another, a per-release activation file in the third), and their semantics do
  not match.
- Shared by all three: absence resolves to the closed state; an invalid value
  fails startup naming the setting and withholding the value; cross-flag
  dependencies are validated at startup; process-kind ownership; development
  flags unrepresentable in hosted classes; tests isolate configuration by
  injection or cleared subprocess environments, never `set_var`. Shared by
  none: bucketing, percentage rollouts, expiry, per-flag audit, runtime flips.
- No root carries a flag value in a job payload; each persists at least one
  admission decision in its own domain row and re-reads it on retry.

The requirements, all three roots unless marked:

| Id | Requirement |
| --- | --- |
| R-01 | Resolve once at start from an injected source into an immutable typed set; an invalid value fails startup naming the flag, never the value |
| R-02 | Absence resolves to a declared closed state, or the flag is required and absence fails startup |
| R-03 | Values are exact toggles, closed enums or bounded integers; no free strings or JSON; non-Unicode is an error |
| R-04 | Enablement is an explicit value, never inferred from a secret, URL or path; an enabling value declares the settings it requires |
| R-05 | Cross-flag invariants (requires, both-or-none, exactly-one, iff, conflicts) and composite enablement are validated |
| R-06 | A development-only flag presented in a hosted class fails startup |
| R-07 | A value presented to a process kind that does not own it fails startup |
| R-08 | Flags shared across process kinds agree, verified through the database at startup |
| R-09 | An admission-time decision is persisted in the application's own row and replayed on retry; Batter never writes flag values into Runledger payloads |
| R-10 | The resolved non-secret set has a stable, order-insensitive revision |
| R-11 | A redacted startup report carries name, non-secret value, source kind and revision |
| R-12 | Flag values are non-secret by type |
| R-13 | The reader consumes `SettingsSource`, never the process environment; no global resolver |
| R-14 | Retired and forbidden names fail startup with a migration reason |
| R-15 | Batter's own environment readers fail closed on invalid values |
| R-16 | The catalog exposes flip discipline: kind, closed state, owners, rolling or coordinated |
| R-17 (two roots) | A schema or authority precondition is verified before a flag resolves enabled |
| R-18 (two roots) | A one-way flag's first enablement is recorded durably |
| R-19 | A kill switch never recalls admitted or in-flight work; workers contain queued work by evaluating at claim |
| R-20 | The catalog has a machine-readable export |
| R-21 (policy) | Under ADR-010 the observed misuses are unrepresentable on the canonical path |

### Provenance

Every evidenced requirement is 3/3.

- **Evidenced:** literal resolution R-01 and R-03 (toggle and enum literals
  only; bounded integers are a follow-up), `Absence` R-02, `requires_settings`
  R-04, `requires`, `conflicts` and `both_or_none` R-05 (exactly-one, iff and
  composite enablement are follow-ups), `only_in` R-06, owners R-07,
  domain-row pins R-09, `revision()` R-10, the startup report R-11, non-secret
  values by type R-12, `SettingsSource` injection and no environment reads R-13
  (enforced by `crates/batter/tests/process_environment_ownership.rs`), global
  retired names R-14 (per-kind forbidden names are a follow-up),
  `FlipDiscipline` R-16, the invocation-time job gate R-19 (the
  attempt-neutral claim-time gate is a follow-up), the manifest R-20.
- **Policy, not a survey count:** the canonical-path shape R-21 is met except
  its first clause, the accepted residual below.
- **Not met:** R-08 in either mode (control-plane entries agree across kinds,
  but held entries, the declared base and the settings floor come from each
  process's own settings; the heartbeat records `declared_revision`, but no
  startup agreement check fails), R-15, R-17 and R-18.
- **Owner-selected:** toggle literals `true`, `false`, `on` and `off` in any
  letter case (owner answer 6, which departs from R-03's exact-literal clause),
  mandatory `Expiry` on release flags (0/3 roots), `StaleRule`, the
  route-group scope of the HTTP gate with the permanent `HttpFailure::Disabled`
  (owner answer 3), the periodic gate, the evaluation events, the control plane
  with its presence and fleet view, the operator command, the last-good file
  and control-plane invariant holds. Per-subject rollouts left
  this epic (owner answer 4).

## Decision

Four packages, all plain `crates/` leaves on Batter's version line (owner
answers 5 and 9), consolidating flags under the Batter name:

- `batter-flags-core` owns the catalog grammar, the three declaration kinds
  (`ReleaseFlag`, `ModeFlag`, `KillSwitch`), relationships stated on the
  `CatalogBuilder`, validated snapshots whose rows are published or held one
  by one, clock-free evaluation into a layered `Decision`, `Pinned` decisions,
  the `feature_flag.evaluation` event and, with `serde`, the manifest. It
  depends on no Batter crate.
- `batter-flags-postgres` owns the fixed `flags` schema (owner answer 2) and its
  own migration ledger, coherent loads, audited compare-and-set writes with
  their ordered refusals, history, archive and restore, catalog publication,
  presence, pruning and the fleet view, on `batter-sqlx` (owner answer 7).
- `batter-flags` owns declared resolution, the process reader, the protected
  HTTP, job and periodic gates, `TestFlags`, the exact-role grant inventory and
  `control_plane::register_in`, which registers the refresh, heartbeat and
  prune as process components. The refresh persists the
  optional last-good file at the end of a run, in a blocking write that can be
  abandoned at its deadline and outlive the run, so joining the component does
  not prove the write stopped; roots end with `Runtime::shutdown_timeout`.
- `batter-flags-cli` is the operator command `batter-flags`, a Batter-consuming
  executable root that installs the redacting panic hook first, and stays
  `publish = false` until the release-train record (owner answer 8).

The facade reaches them through `flags`, `flags-postgres`, `flags-axum`,
`flags-runledger` and, for test builds, `flags-test-support`, which alone
reaches `TestFlags` (`batter-jkw6.54`); `crates/batter/examples/rollout.rs`
shows both modes.

Declared mode resolves a catalog once from the application's
`SettingsSource` and changes by restart. In control-plane mode the same
resolution supplies the declared base: the values served before the first
publication, the kill-switch settings floor and the migration path for
existing environment toggles. The settings floor only engages: a `released`
literal is ignored, and a floor-engaged switch cannot be released through the
control plane.

The control plane publishes one validated snapshot per store version. Only a
structurally corrupt candidate (a repeated key, a foreign scope)
is rejected whole, and the previous publication stays served without renewing
its freshness. A row naming no declaration, or one this process kind does not
own, is ignored. Any other row that fails validation is held: a release or
mode flag serves its declared base, a kill switch keeps its last published
state, else its safe state. A published flag that would leave a `requires`,
`conflicts` or `both_or_none` invariant between flags its kind owns unmet is
held as well (`HoldCause::InvariantUnmet`), judged with a governed flag at its
code default while its switch is engaged and separately for a stale view.
`PooledStore::submit` with `WritePolicy::STRICT`, the operator command's
default, refuses a flag write that would newly hold a flag unless the hold is
accepted; `PooledStore::write` and `restore` accept it, and a kill switch is
never refused for what engaging it holds. Staleness counts
from registration until the first publication, and a file-served snapshot ages
from registration, because the file carries no clock.

### Conventions

Every public enum in `batter-flags-core` and `batter-flags-postgres` is
exhaustive, errors included, so a new variant is a recorded hard cut. In
`batter-flags` the outcome and status enums (`LoaderState`, `Served`,
`LatestRead`, `Gate`, `LastGoodStatus`, `StartupFile`, `ValueSource`,
`UnpinnedBecause` and the `WhenTripped` policies) are exhaustive, while the
grant vocabularies `FlagsOperation` and `FlagsRole` and the error and failure
enums (`DeclaredError`, `ValueProblem`, `GateError`, `GateRegistrationError`,
`RegisterError`, `PresenceError`, `LoadCause`, `PersistFailure`, the file
errors `DirectoryError` and `FileProblem`, `FlagsGrantError` and the test
fixtures' `FixtureError`) are `#[non_exhaustive]`, like `RunlimitOperation`
and `RouteGroupError`: a consumer matches an outcome completely and reports an
error it does not recognize.

### Decisions recorded from delivery

- The evaluation event target is `batter::flags`, so `batter=warn` filters it
  and `batter::flags` selects it.
- `Layer::Held { since }` is the snapshot version that last served the state,
  not the row's write version.
- Archived keys load as absent: a candidate row has no archived state and the
  snapshot digest has no archival input.
- A pin records the effective decision, a governing override included. `pin`
  refuses an error decision with `Unpinnable` (`batter-jkw6.45`): a handle the
  view cannot answer for, or a view before the first publication under
  `unready_until_loaded`, even where an engaged governing switch answers
  `evaluate` with the code default. It was infallible and pinned such a decision at its
  code default, which a replay then honoured as cached, losing the error.
  `Pinned::same_decision` compares key and variant only; the
  keyed-idempotency boundary is the retained pin itself, which a replay must
  enqueue unchanged, origin included, because Runledger compares the whole
  payload (`keyed_pin_identity` shows equal decisions conflicting).
- The held kill switch is the single history-dependent exception to "one
  (build, version, declared revision) denotes one rule set", and the reason
  core's `Publisher` carries what it serves into each read it judges; the D-06
  revisit trigger was re-examined for it and stands.
- `SnapshotStatus` is a struct of three separate facts, `loader`, `served` and
  `latest_read`, read in one critical section with the publication they
  describe, and `Gate` is judged from one status (`batter-jkw6.40`). The earlier
  precedence enum let a refusal hide what was served, so `Gate::Published`
  needed a second read of the publication, and reported an impossible standing
  as `NotLoaded` with a zero duration.
- One `low_level::Publisher` owns the published record through its whole
  lifecycle (`batter-jkw6.39`): it starts unpublished or restored from the
  last-good file, judges each read and publishes it as a second step, and is
  not `Clone`, so no stale copy of a record can be advanced. The earlier public
  `Published::first`/`next`/`rejected` and the loader's own `carrying` call
  left the first-versus-later choice, the file's age origin and carrying to
  the loader.
- `TestFlags::builder` takes the catalog and a `ProcessKind`, because
  ownership decides what a process evaluates; a fixture that injects a
  foreign, not-owned or repeated handle is refused at build.
- Invariants between flags owned by different process kinds are not judged,
  in either mode: the other kind's value is unknowable there.
- The expiry report is one WARN event per resolve, `flags.expired`.
- `requires_settings` "present and non-empty" means not exactly empty.
- `DeclaredError`'s `Debug` wraps its redacted `Display`; the only retained
  value is a non-Unicode setting, reachable through `source()`.
- The LISTEN/NOTIFY wake was withdrawn before merge: it bought at most one poll
  interval and made a full notification queue fail every audited write, kill
  switches included. A committed write notifies nobody, and the poll interval
  is the propagation bound.
- A write after a restore or a failover to an older head is numbered above the
  highest version a Live control-plane process reports, instead of refused:
  running processes apply it at their next poll without a restart, and versions
  skip from the restored head to above the fleet.

## Overrides

- [ADR-006](006-workspace-packages.md), quoted exactly: "No SQLx, Runlimit,
  Runledger, or fixture library is created speculatively. Integration
  composition belongs in an unpublished reference application first."
  `batter-flags-postgres` is a SQLx-backed library created without a reference
  application first, by owner direction; the user-approved Runlimit prototype
  in [integrations](../integrations.md) is the prior recorded instance. ADR-006's
  external-harness and generic-test-support rules are not overridden.
- The dynamic plane's failure semantics are not shared by the surveyed roots
  (above), so the control plane is owner-selected rather than extracted.
- `batter-flags-core` has a normal `tracing` dependency, because it emits the
  evaluation event; the load events are `batter-flags`'. Tokio and
  `serde_json` stay out of its closure without the `serde` feature.

## Distinctions

- **Settings.** Declared mode is not a configuration framework or a second
  deployment validator and adds no default source: it reads the
  `SettingsSource` the root supplies. `ProcessKind` and `EnvironmentClass` are
  vocabularies the application declares, not a foundation-owned mode
  parameter. `Catalog::manifest()` exports the same declaration for deployment
  tooling in place of a hand-copied list (R-20).
- **Not ADR-014's cache.** Only `batter-flags` publishes on the protected path,
  one validated snapshot per version; each key's layering is fixed by its
  declaration; evaluation never loads, evicts or misses per key and performs no
  I/O. An unknown key is ignored.
- **Not a circuit breaker or fallback.** A kill switch's automatic selections,
  before the first publication and through `StaleRule::EngageWhenStale` past
  the maximum age, follow the control plane's own availability, age or
  validity. No measured failure of an application dependency engages it, and
  nothing half-opens, probes or calls an alternative.
- **Not a receipt store or repository abstraction.** One fixed table family
  with native ownership.
- **No provider trait, source adapter or vendor mapping.** The disclosed
  `low_level` publication path accepts validated candidates with
  caller-asserted provenance and caller-computed `Freshness`; it is a weaker
  escape hatch, not a provider interface, and a trait or adapter would reopen
  ADR-014's rejection of a generic provider interface.

## Accepted residual and rejected alternatives

- R-21's first clause: a process kind can still name a handle it does not own.
  Handles are catalog-wide values; per-kind handles would need a kind marker
  type parameter on every handle, view and gate. The protected outcomes are
  refusal at gate and wrapper construction and the `Reason::Error` default at
  evaluation.
- Rejected: a `#[doc(hidden)]` adapter constructor for foreign sources (the
  `low_level` path is disclosed instead), and a fixture that forges
  control-plane provenance (`TestFlags` reports `Source::Test`).
- The write-side invariant judgement is advance notice, not enforcement: it
  sees stored rows and code defaults, not a process's settings, settings floor
  or staleness, and each process judges again when it loads.

## Consequences

- Reads of the process environment that remain by design are outside Batter's
  library sources: the operator command's `src/main.rs` captures its
  connection settings; `batter-otlp` checks that no `OTEL_*` variable is set,
  without reading a value; in dependencies, SQLx 0.9.0's
  `PgConnectOptions::new_without_pgpass`, which URL parsing starts from, reads
  `PGHOST`, `PGHOSTADDR`, `PGPORT`, `PGUSER` and the other libpq defaults
  (`sqlx-postgres/src/options/mod.rs:56-68`, `options/parse.rs:9-10`), and
  `rustls-native-certs` 0.8.4 reads `SSL_CERT_FILE` and `SSL_CERT_DIR`
  (`src/lib.rs:200-205`). Other dependencies are not audited here.
- Before the first publication without a last-good file, a restart during a
  control-plane outage loses every engagement that exists only in the control
  plane; `LoadPolicy::with_last_good` and the settings floor are the remedies.
  Meanwhile a gated direct job continues after its awaiting recheck without
  spending an attempt, whatever its tripped policy. A gated workflow step cannot
  continue and a `flagged` job does not know which it wraps, so each of those
  fails the attempt, and a long outage can exhaust its attempts without invoking
  its handler. Gated periodic components skip their runs.
- ADR-014's reopening condition is noted: one consumer now composes through
  the lifecycle layer. The re-survey is deferred to `batter-8pn8` (owner
  answer 10).
- Follow-ups outside this epic: per-subject rollouts, bounded integers, R-08
  agreement, R-15, R-17, R-18, exactly-one, iff and composite invariants,
  per-kind forbidden names and the claim-time job gate.

### Watch list

- A second consumer of the control plane, which would let track 2 assess
  promotion.
- Any root that writes flag values into a job payload (R-09 says none does).
