# Changelog

## Backlog migration — 2026-09-08

Audited Markdown delivery requirements into dependency-linked Beads with completed
and deferred outcomes preserved. Removed duplicate roadmap/task lists; docs retain
contracts, capability facts and validation history.

## Unreleased

- Drive one job handler in a test without a worker, queue or database
  (`batter-3upk`, closing gap 7 of the
  [whole-application baseline](docs/evidence/whole-app-baseline-2026-10-09/README.md);
  [ADR-010 assessment](docs/adr/010-agent-only-consumption.md#handler-test-facility-assessment-batter-3upk)).
  `batter_runledger::test_support::HandlerInvocation`, reachable as
  `batter::runledger::test_support` with the facade's `runledger-test-support`
  feature, declares a budget and an optional resume checkpoint, and `execute`
  runs the value a catalog registers through the worker's own dispatch with a
  real `JobExecution`: services that own the invocation's exit signal, so
  `job_phases` derives and the end cancels derived contexts; one monotonic
  deadline the budget after the future is first polled, read by `deadline()`,
  progress writes and the timeout alike; and in-memory progress persistence
  with the queue row's merge rule. The handler future is destroyed before the
  invocation ends, on every exit and when the future itself is destroyed. The
  run's exhaustive `HandlerExit` is the handler's own result when observed
  strictly before the deadline, `TimedOut` otherwise, or `Panicked` with the
  message, beside `PersistedProgress` and the contained exit-hook panics. Panics
  are caught around dispatch as well as polling, and an opaque `panic_any`
  payload is retained rather than dropped; as in the worker, a destructor that
  panics while the drive destroys the handler future is not contained. No
  retry, continuation, dead-letter or lease behaviour is simulated. The
  adapter's `test-support` feature now also selects `serde_json` and `uuid`.

- Add `PgLease::migrate_and_apply_with` (`batter-aouh`) for scoped application SQL
  after managed definitions and before the same transaction commits. Ownership
  and grants can publish with changed callers; managed definitions, ledger
  evidence, transaction continuity and revoked PUBLIC execution are rechecked.
  Refusals roll back the uncommitted apply and grants; already committed
  migrations and unconfirmed disposition remain distinct. The existing
  `migrate_and_apply` API and its no-application path are unchanged. See
  [ADR-013](docs/adr/013-managed-database-objects.md).

- Breaking: browser cookies are presented only where the route group's
  browser policy accepts ambient credentials (`batter-52s3`,
  [ADR-010 assessment](docs/adr/010-agent-only-consumption.md#browser-credential-gate-assessment-batter-52s3)).
  `batter_axum::browser::read_cookie` (also `batter::axum::browser::read_cookie`)
  is removed, and `BrowserPolicy::without_mutation_checks` takes a required
  `MutationCheckWaiver`. Every browser group now installs a gate, in the
  position its mutation check held, that records privately from the arrival
  method whether the request may present cookies; the new
  `browser::BrowserCredentials` extractor reads only that record. Safe methods
  are always granted; other methods are granted after `with_mutation_checks`
  passes or under `MutationCheckWaiver::ApplicationChecksMutations`, and refused
  under `MutationCheckWaiver::NoAmbientCredentials`. Requests outside a browser
  group, including a group or default group without a browser policy, a
  lower-level composition and a bare router, are refused on every method. A
  refusal is the typed `BrowserCredentialsRejection` (`UncheckedMutation` or
  `NoBrowserPolicy`), which renders as the sanitized 500 Problem JSON of
  `HttpFailure::Internal`. Responses, checks and headers of routes that read no
  cookie are unchanged.

  Reading a cookie in a handler:

  ```rust,ignore
  // Before
  async fn save(parts: Parts) -> Response {
      let session = read_cookie(&parts.headers, &name, CookieHeaderPolicy::TargetOnly)?;
      // ...
  }

  // After: the route sits in a group whose BrowserPolicy accepts credentials
  async fn save(credentials: BrowserCredentials) -> Response {
      let session = credentials.cookie(&name, CookieHeaderPolicy::TargetOnly)?;
      // ...
  }
  ```

  Choosing an unchecked browser group:

  ```rust,ignore
  // Before
  let reports = BrowserPolicy::without_mutation_checks(PrivateResponsePolicy::NoReferrer);

  // After: mutating routes that read no cookie
  let reports = BrowserPolicy::without_mutation_checks(
      PrivateResponsePolicy::NoReferrer,
      MutationCheckWaiver::NoAmbientCredentials,
  );
  ```

  A group whose mutations authenticate with a cookie should move to
  `BrowserPolicy::with_mutation_checks(.., MutationPolicy::exact_origin(..), ..)`;
  `MutationCheckWaiver::ApplicationChecksMutations` is for an application that
  verifies those requests itself, such as a cross-site form post checked against
  its own state value. A composition that owns the forgery check itself, or
  installs no route group, can read any header map with the deliberately
  unchecked `low_level::read_unchecked_cookie`, which has the old function's
  parser, results and errors.

  No one-release deprecated shim is kept for either change: a `read_cookie`
  alias or a one-argument `without_mutation_checks` would restore exactly the
  unchecked cookie session this change removes. The escape hatch has a new name
  so that the compiler's import suggestion cannot move a canonical consumer onto
  it silently. `BrowserCredentials::cookie`, `without_mutation_checks`,
  `read_unchecked_cookie` and the `browser` module carry `Migration:` notes, and
  compile-fail rustdocs reject the removed import, the one-argument form, and
  constructing or inserting `BrowserCredentials`. The public-API surface diff is
  [recorded](docs/evidence/browser-credentials-api-diff-2026-10-17/README.md).

- Update the locked `rustls` to 0.23.46 (`batter-ddc`). RUSTSEC-2026-0285,
  -0346, -0347, -0348 and -0349 affect 0.23.44, which the workspace reached
  through `sqlx-core`, `reqwest` and `bollard`; the update is lock-only and
  within every dependent's semver range. The
  [publication dossier](docs/evidence/publication-dossier-2026-10-17.md)
  records the remaining gates; nothing is published.
- Manage constraint triggers with the routines they bind (`batter-n2s1`,
  [ADR-013](docs/adr/013-managed-database-objects.md), amended 2026-10-16).
  `ManagedObject::constraint_trigger(table, name, source)` declares one
  `CREATE CONSTRAINT TRIGGER` statement (`OR REPLACE` is rejected, as
  PostgreSQL rejects it) and `Retirement::constraint_trigger(table, name)`
  retires one; both reuse the trigger identity `"name" ON "schema"."table"`, so
  a plain and a constraint trigger of one identity are a duplicate. PostgreSQL
  cannot replace a constraint trigger, so apply drops the trigger holding a
  pending entry's name and re-creates it from source in the apply transaction,
  after the routines it binds; a pending plain trigger whose name a constraint
  trigger holds is dropped first too, so a kind change converges without a
  retirement. A constraint trigger therefore never binds an earlier body
  between a migration's commit and apply's, and bootstraps with a new function
  on a fresh database. A changed, drifted or adopted constraint trigger holds
  `ACCESS EXCLUSIVE` on its table until commit; an unchanged set still executes
  no DDL. Because `DROP TRIGGER` removes either kind by name, every trigger drop,
  retirement or drop-before-create, first takes that `ACCESS EXCLUSIVE` lock with
  `LOCK TABLE ONLY` (on tables and partitioned tables), reads the trigger again
  and acts only on the kind it finds, so a trigger another session replaced with
  the other kind is never dropped by a retirement, and a partition clone that a
  retired parent's trigger took with it is simply not there. The apply
  transaction now runs at READ COMMITTED whatever the session default, which a
  REPEATABLE READ default would otherwise defeat; earlier releases ran it at the
  session default. A drop must also reach the relation apply read: when another
  session replaces a view or foreign table, which apply reads without
  `LOCK TABLE`, by a same-named table holding a constraint trigger, apply fails
  with `ManagedApplyError::IdentityMismatch` and rolls the drop back. Before any
  retirement or drop, apply takes every lock its trigger statements need on
  application tables: `ACCESS EXCLUSIVE` for drops and retirements and
  `SHARE ROW EXCLUSIVE` for other pending triggers, on each trigger's table and
  every partition of a partitioned table's tree, each with its own
  `LOCK TABLE ONLY`. It uses `NOWAIT` in a savepoint and, when a relation
  refuses, rolls back and waits for that relation alone. It never waits for one
  relation while holding another of the set, so neither a reader turning writer
  nor a write routed through a partitioned table can deadlock with it. Once it
  holds the set, it reads the partition trees again and takes a new set when a
  partition was attached or removed meanwhile. Foreign partitions cannot be
  locked on their own; a plain row trigger statement on their partitioned table
  locks them later, while holding the rest. A session `lock_timeout` bounds
  each wait. Apply gives up after 32 rounds, with the last refusal (`55P03`),
  or with `ManagedApplyError::Capacity`, whose rustdoc now covers this, when
  the trees still changed in the last round. After a trigger retirement,
  apply reads the catalog again. If another session replaced a managed
  trigger meanwhile, that read can add work outside the lock set, or a drop
  where the set holds only `SHARE ROW EXCLUSIVE`. Apply refuses such work
  with `ManagedApplyError::IdentityMismatch`, whose rustdoc now covers this,
  before requesting another lock, and the whole apply rolls back.

  Each lock needs `MAINTAIN`, `UPDATE`, `DELETE` or `TRUNCATE` on its relation,
  partitions included. Earlier releases needed only what the trigger
  statements need. An applying login that holds only `TRIGGER` on a table,
  which lets it create or replace plain triggers, now fails with `42501`
  before any change, as does one that owns a partitioned table but not all of
  its partitions. The owners must grant it `MAINTAIN`; Batter grants nothing.
  Every drop precedes the routine sources and trigger creates. Verification reports kind and checksum mismatches, drift (including
  deferrability, `WHEN` and a disabled trigger) and present retirements for the
  new kind with the existing findings. Ledger kind names, the ledger shape and
  the digests of existing kinds are unchanged; the new kind is recorded as
  `constraint_trigger`. `ManagedObjectError::SourceHeader` now describes either
  header form. The PostgreSQL lifecycle example adds a deferred constraint
  trigger. The Rust API change is additive apart from the enum variant in the
  next entry. The runtime changes above are not: the `READ COMMITTED` pin,
  and the lock privileges that existing deployments may need to grant first.
  Their migration guidance is above; no Rust shim applies to them.

- Breaking: `batter_sqlx::managed::ManagedObjectKind` (also
  `batter::sqlx::managed::ManagedObjectKind`) gains `ConstraintTrigger`. Values
  of the new kind appear only for manifests that declare or retire a
  constraint trigger, but exhaustive matches no longer compile:

  ```rust,ignore
  // Before
  let label = match object.kind() {
      ManagedObjectKind::Function => "function",
      ManagedObjectKind::Procedure => "procedure",
      ManagedObjectKind::Trigger => "trigger",
  };
  // After
  let label = match object.kind() {
      ManagedObjectKind::Function => "function",
      ManagedObjectKind::Procedure => "procedure",
      ManagedObjectKind::Trigger => "trigger",
      ManagedObjectKind::ConstraintTrigger => "constraint trigger",
  };
  ```

  No one-release deprecated shim is kept: no alias or function can make an old
  exhaustive match cover a new variant, and reporting constraint triggers as
  `Trigger` would misstate how apply replaces them. The enum stays exhaustive so
  consumers review every kind apply can execute; its rustdoc carries a
  `Migration:` note. No existing call shape changes.

  Behaviour change in the same entry: catalog resolution used to report a
  constraint trigger as a plain trigger, so `Retirement::trigger` dropped a
  constraint trigger holding its name (for example one a migration created),
  and verification reported `RetiredManagedObjectPresent` until it did. A
  retirement now drops only a live object of its own kind, so that manifest
  leaves the constraint trigger in place and reports nothing. Retire it as its
  kind (the set digest changes with the retirement's kind):

  ```rust,ignore
  // Before
  Retirement::trigger(QualifiedName::new("app", "records")?, Identifier::new("records_check")?)
  // After
  Retirement::constraint_trigger(QualifiedName::new("app", "records")?, Identifier::new("records_check")?)
  ```

  No shim: making `Retirement::trigger` drop either kind would let a stale
  retirement drop a constraint trigger that later took the name.

- Enforce managed manifests' 64 MiB source limit while consuming their object
  iterators, before consuming retirements. Clarify that interrupted apply may
  have committed even when it returns no result. Add live managed-verifier
  regressions for concurrent apply, late ledger creation and malformed,
  row-secured or inherited ledgers (`batter-54d0.1`). Match their canonical live
  inventory entries to the runner's bare-name parser. Rustdoc ties transaction
  and session-lock termination to server-session termination. Public call shapes
  and persisted ledger formats are unchanged. Add a lazy object-count boundary
  regression, correct the live-runner summary to 157 cases, and clarify the
  PostgreSQL lock-before-snapshot reference (`batter-54d0.2`). Align the
  `CommitUnconfirmed` variant's rustdoc with verification's current-state,
  server-session termination and replay limits (`batter-54d0.3`).

- Add `batter_sqlx::managed` (also `batter::sqlx::managed`, feature `sqlx`):
  managed routines and the triggers that bind them, kept as source edited in
  place instead of whole bodies restated in migrations
  ([ADR-013](docs/adr/013-managed-database-objects.md)).
  `ManagedObjectManifest::new(ledger, PublicExecute::Revoke, objects,
  retirements)?` validates without a connection: each
  `ManagedObject::{function, procedure, trigger}` is exactly one
  `CREATE OR REPLACE` statement naming its declared schema-qualified identity,
  `Retirement::{function, procedure, trigger}` drops an object explicitly,
  `PublicExecute` has no default, and `digest()` is a SHA-256 set digest for
  release evidence. `PgLease::migrate_and_apply(&migrator, &manifest)` is the
  only apply path: it runs the application's SQLx migrator and then one apply
  transaction on the same connection, re-executing only new, changed or drifted
  entries, so an unchanged set executes no DDL. `VerificationPlan::managed_objects`,
  `with_managed_objects` and `verification::verify_managed_objects` verify the ledger,
  set digest, live rendered definitions, retirements and `PUBLIC` `EXECUTE`
  read-only. `batter-sqlx` gains a direct `sha2` dependency already in the
  lockfile. The PostgreSQL lifecycle example now migrates, applies one routine
  and one trigger, and gates readiness on both ledgers. Additive apart from the
  enum variants in the next entry.

- Breaking: add managed-object outcomes to four intentionally exhaustive
  verification enums. `FindingKind` gains `MissingManagedObjectLedger`,
  `ManagedObjectLedgerShape`, `MissingManagedObject`,
  `UnexpectedManagedObject`, `ManagedObjectChecksumMismatch`,
  `ManagedObjectDrift`, `ManagedObjectSetDigest`,
  `RetiredManagedObjectPresent` and `ManagedObjectPublicExecute`;
  `SupportedSurface` gains `ManagedObjects`; `UnsupportedSurface` gains
  `UnprotectedManagedObjectLedger`; `PlanError` gains
  `ManagedObjectsAlreadySelected` and `ConflictingLedger`. A plan without a
  managed-object manifest never produces them, but exhaustive matches no longer
  compile:

  ```rust,ignore
  // Before
  match error {
      PlanError::AuthorityAlreadySelected => "authority",
      PlanError::MigrationAlreadySelected => "migration",
      PlanError::SchemaInspectionAlreadySelected => "schema",
      PlanError::ConflictingRelationKind => "relation kind",
  }
  // After
  match error {
      PlanError::AuthorityAlreadySelected => "authority",
      PlanError::MigrationAlreadySelected => "migration",
      PlanError::SchemaInspectionAlreadySelected => "schema",
      PlanError::ConflictingRelationKind => "relation kind",
      PlanError::ManagedObjectsAlreadySelected => "managed objects",
      PlanError::ConflictingLedger => "ledger",
  }
  ```

  A `FindingKind`, `SupportedSurface` or `UnsupportedSurface` match adds the
  listed variants the same way, or a wildcard arm where the caller treats every
  finding alike. No one-release deprecated shim is kept: no alias or function
  can make an old exhaustive match cover a new variant, and reporting these
  outcomes as migration-ledger variants would misstate what was checked. The
  enums stay exhaustive so consumers review new outcomes; each enum's rustdoc
  carries a `Migration:` note. No runtime behavior changes for existing plans.

- Wrap and unwrap the `batter-at-rest` data key in fixed zeroizing stack
  buffers: `Keyring::seal`, `open`, `rewrap_envelope` and `rewrap` no longer
  allocate a temporary heap buffer for the 32-byte data key's ciphertext or
  plaintext. Version-1 bytes, HKDF inputs, AAD, authentication and randomness
  behavior are unchanged, as are the public API and persisted format. The AAD
  and HKDF info buffers, key-ID copies and owned outputs still allocate.
  Latency moved within a few percent in one local measurement and is not a
  promised speedup.

- Index admitted-router dispatch in `batter-axum`: assembly indexes the
  declared patterns of routers admitted through `GuardedRouter::from_router`,
  so a request that no native route matches consults only the inspection copy
  of an admitted router whose declared pattern matches its path, instead of
  every admitted router's copy in turn. Which router serves, and which requests
  reach the fallbacks, is unchanged. Such a request now costs at most one
  inspection routing whatever the number of admitted routers; the index lookup
  itself still compares, one by one, the capture and wildcard prefixes that
  declared patterns place at the same position. Lookup and teardown use stack
  bounded by the 25-capture limit, not by a pattern's literal depth. No public
  signature changes.

- Add `batter_runlimit::maintenance` (also `batter::runlimit::maintenance`,
  feature `runlimit-postgres`). `maintenance::register_in(target, name, limiter,
  ExpiryMaintenance::new(interval, maximum_rows, startup, shutdown)?)` registers
  a native `PostgresLimiter` or `PostgresGcraLimiter`'s `cleanup_expired` as a
  periodic component and returns the `MaintainedLimiter` to serve through
  `Quota::new` or the HTTP assembly; it is the only constructor of that type.
  The row bound is a `NonZeroU32`, each run's budget is derived from the native
  acquisition and operation timeouts, and every maintenance failure is retained
  as a recoverable periodic failure. Native limiters used directly are the named
  unmaintained low-level form. Additive; no existing signature changes.

- Narrow `RunlimitOperation::FixedWindowExpiryCleanup`: native fixed-window
  cleanup now identifies expired rows by primary key instead of `ctid`, so the
  selection renders `DELETE` with `SELECT` on `capacity_shard`,
  `config_fingerprint`, `subject_key` and `window_expires_at` instead of
  relation-level `SELECT`. No Rust signature changes. Compatibility: an older
  replica's cleanup still reads `ctid`, so narrow a provisioned role only after
  every replica runs this version. Until then, exact-role verification against
  a role provisioned from the old fragment reports the relation-level `SELECT`
  as excess authority. If that verification gates readiness, temporarily add
  relation-level `SELECT` on `runlimit_fixed_windows` to the application's
  manifest with `DeclarationPurpose::AllowedOnly` (for example, through
  `GrantFragment::with_relations`). Keep that allowance while upgrading every
  replica, then narrow the role and remove the allowance. Perform the revocation
  and replacement grants together in one transaction so running replicas never
  observe missing column grants: `REVOKE SELECT ON TABLE ... runlimit_fixed_windows` also
  removes that table's column `SELECT` grants, so re-apply the newly rendered
  grant plan afterwards.

- Add `Startup::protected` (also `batter::startup::Startup::protected`): protected
  startup initializers are owned async closures whose future the library boxes,
  and their `Ok` value is typed output delivered with the running owner by the
  new `StartingSupervisor::wait_with_output`. A failed startup never yields
  output; output returned while drain, cancellation or deadline is observed is
  destroyed before cleanup. `startup::Initializer<E>` names the `Send` initializer
  bound in return types. `startup::ApplicationError` lets `?` combine unrelated
  errors in an initializer and keeps the boxed cause downcastable, and
  `InitializationError<ApplicationError>` implements `std::error::Error`, so
  anyhow-style roots need no adapter type. The HTTP example now returns its bound
  address from startup. This is not a hard cut: `Startup::scoped`, its
  `StartupFuture` closures, `ScopedStartup::start`,
  `DeferredScopedStartup::start` and `service::start` keep their generic
  parameter lists and, for existing closures, their return types;
  `StartingSupervisor` gains a defaulted third type parameter, and its `wait`
  remains for unit output. `service::start` accepts unit-output async
  initializers but not typed output.

- Breaking: backfill the `Quota::run` work-future bound added after 0.0.1
  (also `batter::runlimit::Quota::run`). Work futures must be `Send`; generic
  wrappers must add `+ Send` to their `Fut: Future<Output = Result<T, E>>` bound.
  In a generic wrapper's `where` clause:

  ```rust,ignore
  // Before:
  Fut: Future<Output = Result<T, E>>,
  // After:
  Fut: Future<Output = Result<T, E>> + Send,
  ```

  For a future that retains shared data across an await, migrate captures:

  ```rust,ignore
  // Before: an Rc capture made the work future non-Send.
  let shared = std::rc::Rc::new(String::from("record"));
  quota.run(&context, checks, move |_| async move {
      tokio::task::yield_now().await;
      Ok::<_, std::convert::Infallible>(shared.len())
  }).await;

  // After: Arc<String> is Send; owned Send data is another option.
  let shared = std::sync::Arc::new(String::from("record"));
  quota.run(&context, checks, move |_| async move {
      tokio::task::yield_now().await;
      Ok::<_, std::convert::Infallible>(shared.len())
  }).await;
  ```

  The factory closure itself has no `Send` bound. A synchronous factory can
  extract owned `Send` data from thread-local state before creating the future.
  No deprecated shim is retained: Rust cannot overload `run` by the future's
  `Send` bound, and restoring its old bound would require a non-`Send` erased
  work future, breaking the canonical handler/task composition. A separately
  named local runner would be a new API, not source compatibility for old calls.
- Native Runledger PostgreSQL fixtures delegate host-port selection and binding
  to Docker on IPv4 loopback, matching the fixture URL and avoiding all-interface
  binds that can collide with unrelated outbound host sockets on Docker Desktop.
  Failed starts await cleanup of their uniquely identified container and
  anonymous volumes, honoring explicit keep mode. Cleanup failure retains the
  startup diagnostic. Docker publication can still fail; no application
  database operation is retried.
- Re-export `PgScopeError` from `batter_runledger` (also `batter::runledger`),
  the error `PgIntentScope::application` returns, so a consumer that handles it
  needs no direct `batter-sqlx` import. A whole-application fresh-agent
  baseline, recorded under `docs/evidence/whole-app-baseline-2026-10-09`, first
  failed to build on that missing re-export and four `CorrelationId` versus
  `&CorrelationId` type mismatches. The baseline also informed documentation
  repairs: how to run bounded startup work inside the protected scope,
  that `run_atomic` is bounded by wrapping it in `context.run`, that definition
  sync precedes `register_in`, that the runtime `sqlx::query_as` function is not
  a `PgNativeQuery`, that `AdmittedRequest::correlation_id` returns a reference,
  and that a handler test ends its invocation explicitly.
- Backfill the operation-authority hard cut and add a supported-path choice
  table to the facade's rustdoc front page; see the `Breaking (backfilled note)`
  entry below and `docs/evidence/public-api-diff-2026-10-09`.
- Supervise recurring maintenance as registered periodic components.
  `periodic::register_periodic_in` takes a sealed `RegistrationTarget`, a
  validated `PeriodicPolicy` and a per-run factory, and returns a read-only
  `PeriodicReader`. Batter owns the serial schedule, the per-run deadline and
  cancellation lineage, startup acknowledgement, the stopping class and the
  bounded diagnostics; the application owns the operation. One component owns
  one directly polled serial future, so invocations never overlap and nothing
  is spawned per run. The first invocation is immediate and later ones follow a
  fixed interval with missed ticks skipped, so one overdue invocation may run
  immediately after an overrun without replaying a burst; the semantics are
  recorded against pinned Tokio 1.53.1 timers in `docs/references.md`.
  The library realigns even short intervals within Tokio's five-ms Skip
  tolerance; sub-millisecond timer precision is not promised.
  A run budget longer than the interval is valid.

  `PeriodicStartup::immediate` acknowledges that the loop is initialized;
  `PeriodicStartup::after_first_success(allowance)` acknowledges only after one
  run succeeds within a validated total allowance, whose expiry is a distinct
  retained `PeriodicInitializationExpired` initialization failure that
  initiates drain. `PeriodicShutdown` requires one of two library-owned
  classes: `StopAtDrain` destroys an active run on drain, while
  `SupportThroughDrain` keeps running while ordinary components, admitted
  finite work and descendants, or native settlement still need it — a joined
  ordinary panic or abort, and pending or uncooperative native settlement, are
  not stopped proofs, and an unreached boundary closes support at the existing
  forced-cancellation step.
  `PeriodicFailure` converts a propagated application error into its
  recoverable variant, so `?` keeps the schedule and only an explicit
  `PeriodicFailure::Fatal` initiates drain; a recoverable failure or an expired
  run deadline permits only the next scheduled invocation.

  **Source-compatibility hard cut.** `ShutdownReport` gains a public
  `periodic: Vec<PeriodicRecord>` field carrying bounded, redacted evidence
  retained independently of each runner, including the concrete terminal cause
  of an escalated run or an expired initialization allowance. Code that constructs or exhaustively
  destructures `ShutdownReport` must be updated; match it with `..`. The
  coordinator is the only constructor, and no existing field changes meaning.
  For initialization expiry, the terminal sample's invocation counts admitted
  runs before expiry (zero if none), rather than identifying a failed run.

  The facade `worker` example replaces its hand-written drain-aware interval
  loop with both stopping classes and asserts that pruning admits nothing after
  drain while lease renewal keeps supporting a finite batch that is still
  draining. Registration authorizes recurrence only: it establishes no remote
  rollback, idempotence, successful renewal, fencing or absence of a timed-out
  effect, and lease-loss, freshness and durable witness policy stay
  application- or native-owned. No durable scheduler, retry framework,
  lease protocol or native maintenance SQL is added; `HealthMonitor` and OTLP
  scheduling are unchanged.

- Add `batter_runledger::job_phases` (also `batter::runledger::job_phases`), the
  one bridge from a native `JobExecution` and an explicit final-state reserve to
  `OperationPhases`. Work ends at the worker's own absolute deadline minus the
  reserve; finalization keeps that deadline; `Duration::ZERO` selects no reserve.
  The native invocation's exit cancels both phases through its own exit hook,
  with no caller guard or forwarding task. Derivation returns a typed
  `JobPhasesRejection` (`Unsupported`, `Ended`, `Exhausted`, `Reserve`) before
  any application work, including for custom execution services that make no
  exit claim.
- Add `OperationContext::split_finalization`, the zero-reserve form of
  `reserve_finalization`. It returns the same sibling `OperationPhases`, with
  both phases ending at the context's deadline, and shares the one
  work-deadline computation. `reserve_finalization` still rejects
  `Duration::ZERO`; no existing call changes behavior.
- Batch session settings and verify idle pooled sessions at acquisition instead
  of resetting them a second time. Reset uses five policy
  SQL statements (ROLLBACK, DISCARD ALL, native SET ROLE, one array-bound `set_config`
  statement, one verification); ordinary idle acquisition uses one verification,
  plus one batched native assignment check when custom settings are declared.
  Native role switching preserves effective-role-only function EXECUTE grants.
  Both role and custom-parameter SET authority are rechecked. These are statement counts,
  not measured network round trips or latency. Large profiles retain fixed query
  width, and live controls cover idle role/schema/parameter revocation, acquisition
  cost and timeout units. PostgreSQL 18 is explicitly the minimum supported server
  for both profile constructors and all PostgreSQL integrations.
- Give probe responses a selected private-response policy.
  `HttpBoundary::with_probe_response_policy(PrivateResponsePolicy)` applies that
  policy to every probe response, including the method rejection that answers an
  unsupported method on a probe path before any renderer runs. A probe sits
  outside every route group, so a group's `BrowserPolicy` never reached it, and
  those `405` responses previously carried no `Cache-Control`,
  `Referrer-Policy` or `X-Content-Type-Options`. A private administrative
  surface had to keep a post-assembly `PrivateResponsePolicy` layer over a taken
  router; the boundary now installs the policy around the probe routers during
  assembly, so that surface keeps the sealed `AssembledHttp` through
  registration instead. This is the capability `batter-probe-response-policy-2wz4`
  required before sealing.

  The selection changes response headers only. Probe status, the readiness
  decision, the rejection's `Allow` header, correlation, the single observation
  and probes staying outside application admission are unchanged; it applies to
  empty-body and application-rendered probes alike, and header values are set
  rather than appended, so a renderer applying the same policy leaves them
  single-valued. It is opt-in and scoped to probes: a boundary without it keeps
  probe responses exactly as before, unmatched paths remain the guarded or
  rendered fallback's concern, and guarded routes keep their own group's policy.
  Selecting twice keeps the last policy.

- Breaking: seal HTTP assembly and move deliberately caller-ordered composition
  into `batter_axum::low_level` (also `batter::axum::low_level`).
  `AssembledHttp::into_router` is removed. Assembly now has exactly two
  outcomes: `register_in`/`register_with_connect_info_in` for protected
  serving, and the new consuming `AssembledHttp::in_process`, which returns an
  opaque cloneable `InProcessClient` with
  `request(&self, Request<Body>) -> Response`. The client keeps its router
  private: it has no router conversion or accessor, no `Deref`/`AsRef`, no
  public field, no route or layer method, no Tower `Service`/`MakeService`
  implementation and no serving conversion, so no layer or route can be added
  outside the observer and nothing can be handed to `axum::serve`. Each of
  those escapes, and each relocated helper name at the crate root, has its own
  compile-fail control rather than sharing a grouped one, including bound
  controls for `Into<Router>`, `AsRef<Router>` and `Deref` and a control over
  the private router field. It prepares
  the router once, with the same `Router::with_state(())` preparation pinned
  Axum 0.8.9 performs in `into_make_service` and
  `into_make_service_with_connect_info`, then clones that prepared router per
  request; make-service stays unexposed. A request is polled, and its future
  destroyed, inside the caller's task under the boundary's existing dispatch
  and observation ownership, with no detached spawn. An in-process response
  establishes response construction only, never serving, connections, peers,
  body streaming or detached descendants.

  Eleven public helpers move to `low_level` with no crate-root aliases:
  `observe_http`, `request_admission`, `request_scope`, `operational_http`,
  `operational_http_with_quota`, `readiness`, `liveness`,
  `dependency_readiness`, `register_http`, `register_http_in` and
  `register_http_with_connect_info_in`. Their behavior and caller-obligation
  documentation are unchanged. Everything else keeps its existing supported
  path, including `RequestPolicy`, `ResponseConstructionBudget`,
  `HttpObservationLevel`, `HttpFailure`, `RequestInterruptionResponder`,
  `ReadinessPolicy`, `ReadinessDecision`, `AdmittedRequest`,
  `AdmittedRequestRejection`, `CorrelationId`,
  `render_infrastructure_failure`, `readiness_status`,
  `default_readiness_level`, the `browser` transport primitives and
  `quota_observation`.

  This is a hard cut with no deprecated root aliases and no `into_router` shim,
  so an invalid API path cannot be restored as a rollback mechanism. Migrate as
  follows.

  Taking the router for in-process requests:

  ```rust
  // Before
  let app: axum::Router = boundary.assemble(guarded).await?.into_router();
  let response = app.clone().oneshot(request).await.unwrap();

  // After
  let client = boundary.assemble(guarded).await?.in_process();
  let response = client.request(request).await;
  ```

  Importing a raw middleware or registration helper:

  ```rust
  // Before
  use batter_axum::{observe_http, operational_http, request_admission, request_scope};
  use batter_axum::{dependency_readiness, liveness, readiness};
  use batter_axum::{register_http, register_http_in, register_http_with_connect_info_in};
  use batter_axum::operational_http_with_quota;

  // After
  use batter_axum::low_level::{
      dependency_readiness, liveness, observe_http, operational_http,
      operational_http_with_quota, readiness, register_http, register_http_in,
      register_http_with_connect_info_in, request_admission, request_scope,
  };
  ```

  Through the facade, `batter::axum::register_http_in` becomes
  `batter::axum::low_level::register_http_in`; the same move applies to every
  helper above.

  A direct-peer composition that registered a taken router keeps the peer
  choice fused to assembly:

  ```rust
  // Before
  let router = boundary.assemble(guarded).await?.into_router();
  batter_axum::register_http_with_connect_info_in(scope, "http", listener, router)?;

  // After
  boundary
      .assemble(guarded)
      .await?
      .register_with_connect_info_in(scope, "http", listener)?;
  ```

  Runlimit's protected assembly is the one production adapter composition that
  deliberately orders this middleware itself, because authenticated quota
  checking has to sit between admission and body extraction. It now reaches
  `operational_http_with_quota`, `request_admission` and
  `register_http_with_connect_info_in` through `low_level`; its public
  `PreparedHttp`/`TestClient` surface, native quotas and
  `quota_observation` are unchanged.

  A composition that wrapped a taken router with an outer
  `operational_http_with_quota` has no sealed equivalent: the boundary's own
  observer is outermost and allocates no quota record, so a boundary request
  carries no quota writer at all. Build such a composition entirely from
  `low_level`, as Runlimit's assembly does.

- Compose native Runledger and Runlimit grant requirements into one exact-role
  manifest. `batter::sqlx::verification::GrantFragment` carries only object
  declarations, and `FragmentObjectPolicy` carries the application's explicit
  schema, PUBLIC-delivery, row-type and ownership choices for the objects a
  fragment declares; the current-database declaration, discovery scope and
  defaults, role ceilings and the ownership guard stay on the manifest.
  `ExactRoleManifest::with_fragment` consumes the manifest, so a rejected
  fragment or a capacity overflow leaves no partial grant set, and overlapping or
  reordered selections reach the one existing compiler and renderer as one
  deterministic plan. `runledger_postgres::grants` and `batter_runlimit::grants`
  publish this source version's operation-to-object inventory through it, reached
  as `batter::runledger::grants` and `batter::runlimit::grants`. Selections name
  operations, never role names, and there is no administrator preset. The
  inventories are frozen in offline oracles beside pinned native migration
  identities, and PostgreSQL 18.6 cases provision non-owner, non-superuser logins
  from the fragments and run both the real native operations and the real
  forbidden operations under them. Adoption is grant-only: rendered statements
  cannot remove privileges a provisioned role already holds.

- Reach Runledger and Runlimit through one `batter` dependency. The facade's
  feature bridges now expose every native library package as the native package
  itself: `batter::runledger::native::{core, postgres, runtime}` under
  `runledger`, `batter::runledger::native::test_support` under the new
  `runledger-test-support`, `batter::runlimit::memory` under `runlimit-memory`,
  and `batter::runlimit::native_transport::{http, axum}` under the new
  `runlimit-native-http` and `runlimit-native-axum`. A consumer needs no direct
  native-package declaration and no `[patch]` section; facade and direct paths
  keep one type identity. The default feature set stays empty, each namespace is
  absent without its feature, and `runlimit-native-axum` selects the Axum crate
  its native layer needs but never `batter-axum` or `batter::runlimit::http`.
  Reachability moves no ownership: `batter::runledger::register_in` remains the
  protected registration path and `batter::runlimit::http` the protected
  quota-before-body assembly, while each native module documents the obligations
  its callers take on. `runledger-tui` is binary-only and has no namespace.
  `consumers/single_facade_consumer.rs` is the executed acceptance consumer, and
  the [single-dependency recipe](docs/reference-compatibility.md#single-dependency-recipe)
  documents the selection. Publication order is unchanged and nothing is
  published.

- Extract the admitted request through one typed extractor. Guarded handlers
  and route layers take `AdmittedRequest`, whose `context()`,
  `correlation_id()` and `interruption_responder()` return the request's
  `OperationContext`, the `CorrelationId` that its operational wrapper generated
  and the admission policy's `RequestInterruptionResponder`. Admission records
  it privately, and only inside `operational_http`, which `HttpBoundary` and
  Runlimit's protected assembly always install. The value has no public
  constructor and is not `Clone`, so applications can neither construct it nor
  insert it into request extensions, and inserting or replacing the raw
  extensions cannot change it. Extracting it where admission recorded nothing,
  such as a route added after assembly or `request_admission` without
  `operational_http` outside it, answers the new `AdmittedRequestRejection`: the
  fixed 500 Problem JSON of `HttpFailure::Internal`, instead of Axum's
  missing-extension text that names the type. Renderer metadata also drops the
  admission record, so a request redispatched from it is not admitted. The
  facade HTTP example and the reference service use the extractor. Migration:
  replace `Extension<OperationContext>`, `Extension<CorrelationId>` and
  `Extension<RequestInterruptionResponder>` handler arguments with one
  `AdmittedRequest`; admission still inserts the raw extensions, so existing
  handlers keep working.
- Isolate private observation ownership at every application renderer boundary.
  Probe, failure/interruption and browser rejection metadata retain public
  correlation and application extensions but exclude the quota writer, shared
  observation state and operational ownership marker. Redispatching cloned
  metadata through operational middleware creates an independent request rather
  than altering the original request's correlation or quota observations.
- Render probe bodies from the readiness decision. `HttpBoundary` adds
  `with_rendered_liveness(path, render)` and
  `with_rendered_readiness(path, readiness, render)`: the application renderer
  receives the request metadata, and for readiness one fresh
  `ReadinessDecision`, and returns the body and headers, for example
  OpenAPI-documented JSON. The boundary keeps the probe path, its placement
  outside admission and every probe-collision check. It then sets liveness to
  200 with the default INFO completion severity, and sets readiness to
  `readiness_status` and replaces the `ReadinessDecision` and
  `HttpObservationLevel` extensions with the decision and the policy's
  severity, so a renderer cannot report an unready process as ready or change
  how the probe's completion is logged. `ReadinessEvaluator::with_condition`
  and `ReadinessPolicy::with_condition` add application readiness conditions,
  named by a validated `ReadinessCondition`, whose synchronous checks run only
  while the dependency is ready and can only turn a ready decision into the new
  `ReadinessUnreadyReason::Condition(name)`, rendered 503 and WARN by default.
  Migration: exhaustive matches on `ReadinessUnreadyReason` must add a
  `Condition(_)` arm; `default_readiness_level` already maps it to WARN. Replace
  probe handlers mounted beside the boundary with the rendered probe methods,
  and an application readiness check inside such a handler with
  `with_condition`. Existing empty-body probes are unchanged.
- Admit routers built by another router builder, for example an OpenAPI router
  converted with `Router::from`, through
  `GuardedRouter::from_router(router, RouteInventory::new(patterns)?)`. The
  application declares the route patterns the router may serve. Assembly
  checks each declared pattern and rejects probe collisions through a
  library-owned inspection copy of the router that calls or polls no
  application handler, fallback or middleware service, returning the new
  `BoundaryAssemblyError::RouteInventoryMismatch` for a pattern the router does
  not serve. Declared patterns join the group overlap check, and the new
  `OverlappingRouteInventory` rejects declared routes that share a path with
  other routes of their own group. An admitted router is never merged into the
  native router: requests that no native route matches are offered to it at
  the native router's root fallback, ahead of the default group's fallbacks,
  and it serves only those it routes to a declared pattern, inside its group's
  policy, so undeclared routes and its own fallbacks never serve. Assembly
  prepares each admitted router once, so its layers are built once rather than
  per request. `GuardedRouter` moves into its own module without changing its
  public path. Existing compositions are unchanged.
- Move the reference service's HTTP surface onto `HttpBoundary`, with lifecycle
  admission and the request deadline before trusted peer metadata and bearer
  authentication, following the repository owner's 2026-09-28 decision. Its
  `http::register_in` and `http::in_process_client` are now async:
  `register_in` registers through `AssembledHttp::register_with_connect_info_in`
  and returns `HttpRegistrationError`, and `in_process_client` returns
  `Result<InProcessRequestClient, BoundaryAssemblyError>`. Requests to a
  draining or not-yet-ready process, including those without credentials, now
  receive the admission rejection (503 `service_unavailable`) instead of 401;
  unmatched paths receive the 404 fallback only after admission instead of
  outside it; authentication now runs within the request deadline.
- Add named `HttpBoundary` route groups. `with_group(RouteGroup::new(name,
  policy, routes))` gives a set of guarded routes its own `RequestPolicy`, for
  example a longer upload budget, and an optional `BrowserPolicy`. The boundary
  installs one fixed order per group: `PrivateResponsePolicy` headers outside the
  group's admission, so admission, deadline, method and mutation rejections
  receive them; `MutationPolicy` checks with an application renderer on every
  method except GET, HEAD, OPTIONS and TRACE inside admission; then application
  layers. `HttpBoundary::new` also accepts a `GroupPolicy` for the default
  group, and existing `HttpBoundary::new(RequestPolicy)` callers are unchanged.
  Only the default group may declare fallbacks. `with_group` rejects invalid or
  reused names, empty groups and named-group fallbacks, and assembly rejects
  probes or groups that can match one request path, even with different
  methods, confirming each shared path by native routing without polling
  application code. The adapter README carries migration notes for per-handler
  Origin/CSRF checks, hand-written private-response middleware and separately
  layered budgets.
- Bound canonical adapter futures. `PgLease::acquire`, the `PgQueryHandle`
  helpers and every `run_atomic*` runner allocate their operation once on first
  poll (24-376 bytes, previously up to 79 KB). `Quota::run` erases its native
  check and admitted work, and `AttemptRunner::run` its native admission (about
  1.1 KB, previously up to 28 KB). Quota-protected Axum handlers composing these
  adapters now compile at rustc's default recursion limit and run on the default
  test-thread stack without local limits, `RUST_MIN_STACK` or boxing.
  `Quota::run` now requires a `Send` work future. Size tests run unoptimized and
  optimized, and an external composed-handler consumer runs in facade checks.
- Add checked owned-process wait and shutdown methods that propagate unsuccessful
  reports and coordinator failures through `?` while retaining successful report
  evidence. Explicit raw-report methods coexist with unchanged legacy signatures.
- Migrate the runnable service, worker, quota, SQLx, and reference consumers to
  checked completion. Preserve body and shutdown failures together in the finite
  process example and add external `anyhow`/`BoxError` lifecycle controls.
- Add public `batter-core::service` protected completion and the shared
  `telemetry::metrics::catalog`. The facade exposes service completion and an
  opt-in `otlp` feature for the new public `batter-otlp` package. The adapter
  guards catalog keys and descriptions before a shared SDK `ManualReader`, exports serial bounded
  OTLP/HTTP snapshots, and retains typed collector outcomes. It rejects
  unsupported response media types, including empty-body responses.
- Add the unpublished reference service's opt-in `metrics-export` consumer.
  Its `runtime::run` returns a retained `ServiceCompletion`, and `runtime::start`
  returns a `ServiceOwner` with cloneable observers. Final export follows the
  retained service result and closes normally afterward; metrics diagnostics
  never change the service result or exit classification.
- Add `batter_axum::browser::PrivateResponsePolicy`, an exhaustive choice
  between `SameOriginReferrer` (the default) and `NoReferrer` for private
  responses. The policy is one Tower layer and also applies directly to a
  `HeaderMap`; `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`
  stay fixed and weaker referrer policies are unrepresentable. Every
  application, including the unchanged compatibility `private_response` and
  `apply_private_response_headers`, now keeps an existing all-`no-referrer`
  field instead of replacing it with `same-origin`, so an outer layer cannot
  weaken an inner `NoReferrer` choice. Rustdoc explains the `Referer` and
  `Origin` consequences for document navigations and HTML form mutations.

- Breaking: add `ReadinessUnreadyReason::Condition(ReadinessCondition)` to the
  intentionally exhaustive readiness-reason enum (backfilled migration note).
  Exhaustive matches must handle application-condition denials as well as
  lifecycle and dependency reasons:

  ```rust
  // Before
  match reason {
      ReadinessUnreadyReason::Starting => "starting",
      ReadinessUnreadyReason::Draining => "draining",
      ReadinessUnreadyReason::Stopped => "stopped",
      ReadinessUnreadyReason::Dependency(_) => "dependency",
  }
  // After
  match reason {
      ReadinessUnreadyReason::Starting => "starting",
      ReadinessUnreadyReason::Draining => "draining",
      ReadinessUnreadyReason::Stopped => "stopped",
      ReadinessUnreadyReason::Dependency(_) => "dependency",
      ReadinessUnreadyReason::Condition(_) => "condition",
  }
  ```

  No one-release deprecated shim is kept: a function or alias cannot make the
  old exhaustive match cover a new variant. Collapsing a condition into an old
  reason would misrepresent why readiness was denied. The enum remains
  exhaustive so consumers explicitly review their response policy; the default
  HTTP mapping is still 503 and WARN. This note changes no runtime behavior.

- Breaking: separate operation cancellation authority from contexts (backfilled
  note; cut landed 2026-09-22 in `9f04aeb`, task `batter-tc9w.1`).
  `OperationContext::new`, `OperationContext::at`, `OperationContext::under` and
  `OperationContext::cancel` are removed from the public API, as is
  `OperationAdmission::admit`, and `OperationPhases` no longer implements
  `Clone`. A context is now observation and execution capability only; the new
  `OperationOwner` holds cancellation authority for one root or derived child,
  `RootDeadline` names an explicitly independent deadline, and
  `OperationContext::child` now returns `Result<OperationOwner, ConfigurationError>`
  instead of `Result<OperationContext, ConfigurationError>`.
  `OperationAdmission::admit_root` admits a process-linked root and returns its
  owner. No `#[deprecated]` shim is kept for any removed item: a context that can
  cancel itself, or a root created without a visible owner, is the state this
  cut removed, and the core keeps compile-fail controls that reject each old
  constructor. `docs/usage.md` ("Operation ownership and migration") explains
  how to decide between a root and a child. Migrate as follows.

  Creating an independent root budget:

  ```rust
  // Before
  let context = OperationContext::new(Duration::from_secs(2))?;
  let context = OperationContext::at(deadline);

  // After
  let context = OperationOwner::new(Duration::from_secs(2))?.into_context();
  let context = OperationOwner::at(RootDeadline::at(deadline)).into_context();
  ```

  Keeping the ability to cancel:

  ```rust
  // Before
  let context = OperationContext::new(budget)?;
  let shared = context.clone();
  context.cancel();

  // After
  let owner = OperationOwner::new(budget)?;
  let shared = owner.context().clone();
  owner.cancel();
  ```

  Deriving a child that inherits deadline and cancellation:

  ```rust
  // Before
  let child = OperationContext::under(step_deadline, &parent_token);

  // After
  let child = parent_context.child(step_budget)?; // an OperationOwner
  let context = child.context();
  ```

  Updating an existing `child` call:

  ```rust
  // Before
  let child: OperationContext = parent.child(step_budget)?;
  let value = child.run("step", work).await?;

  // After
  let child: OperationContext = parent.child(step_budget)?.into_context();
  let value = child.run("step", work).await?;
  // Retain the returned owner and borrow owner.context() instead when explicit
  // cancellation authority is needed.
  ```

  No deprecated same-name shim is kept for `child`: Rust cannot overload a method
  by return type, and restoring the old return would hide the child's owner on
  the canonical derivation path. `into_context()` is the explicit transition to
  an execution handle without cancellation authority.

  Admitting a request-scoped root from the process lifecycle:

  ```rust
  // Before
  let context = admission.admit(deadline)?;

  // After
  let owner = admission.admit_root(RootDeadline::at(deadline))?;
  let context = owner.into_context();
  ```

  Sharing phases with tasks:

  ```rust
  // Before
  let phases = context.reserve_finalization(reserve)?;
  tokio::spawn(run(phases.clone()));

  // After
  let phases = context.reserve_finalization(reserve)?;
  tokio::spawn(run(phases.work().clone()));
  ```

- Record the public-API diff between the 0.0.1 release and the current head
  under `docs/evidence/public-api-diff-2026-10-09`. It confirms that the
  operation-authority cut and the sealed HTTP assembly are the only removed or
  renamed public items in the Batter packages since 0.0.1, and that the native
  Runledger and Runlimit packages removed none; the two Runledger dead-letter
  re-exports it lists moved modules and keep their public paths. Every public
  hard cut now follows the rule in `AGENTS.md`: a `Breaking:` entry with
  before/after code, the removed name in the replacement's rustdoc, and a
  recorded shim decision.

## 0.0.1 — 2026-09-22

- Prepare the first crates.io release of the eight Batter packages. All internal
  path dependencies carry registry versions, native Runledger is coordinated at
  0.13.0, and native Runlimit is coordinated at 0.4.0. The two executable
  example packages remain unpublished.

- Add optional PostgreSQL `AttemptRunner`: native pre-authentication admission,
  bounded credential verification, transaction-fenced final decisions and atomic
  attempt/application completion. Durable rejections preserve failure audit;
  stale claims cannot invoke application work. Native uncertainty is retained,
  with no automatic replay or traffic-quota refund.
- Add `batter-sqlx::run_atomic_in` to preserve acknowledged atomic outcomes under
  an existing operation deadline. No caller-owned completion retention is needed.
- Update the Runlimit adapter for native typed subjects, capacities, exhaustive
  denials and separate single/batch errors. Existing quota workflows keep their
  semantics; custom native type integrations require source migration. Add memory
  GCRA batch-error compatibility and explicit independent-consumer source patches.

- Refresh the landing documentation, package descriptions and hosted evidence
  after the owned PostgreSQL scope cutover (`batter-q1t`).
- Breaking: replace the Runledger resource-view bridge with consuming owned
  transactions. `batter-sqlx` provides XID continuity, savepoint recovery,
  cancellation retirement and explicit completion evidence without a Runledger
  dependency. Runledger builds domain operations on this foundation; the optional
  Batter integration reexports that owner and snapshot-authoritative verification.
  No view compatibility module remains. These unpublished feature branches use
  coordinated Git pins and a consumer-root foundation patch pending a separate
  publication decision; see `docs/reference-compatibility.md`.

- Add consuming `PgLease::migrate` to run application-selected native SQLx
  migrations without exposing replaceable connection identity. Preserve native
  migration errors behind redacted `SqlxFailure` and retire on interruption.
- Supersede the PR #19 borrowed-resource composition API. Native shutdown retains
  its owned signal and consuming,
  unforgeable settlement; reference retirement retains `CommitUnconfirmed`.
  Bound the reference query future's private layout without changing ownership.

- Make PostgreSQL lease disposition unambiguous: native work either uses the
  result-driven `with_connection` path or the consuming
  `with_retiring_connection` path. Both expose only an opaque `PgSession` and
  opaque transactions with executor-only access, preventing safe replacement of
  the physical connection before disposition. An unfinished transaction keeps
  session-owned completion state dirty, so dropping or forgetting it retires the
  connection even after an application `Ok`. Since the native executor can issue
  raw transaction-control SQL, pool return additionally requires a private proof
  created only after a same-connection normalization handshake ends in a
  successful `ROLLBACK`; open or failed raw transactions are synchronized to
  idle without warning on the ordinary already-idle path, while cleanup failure retires the
  connection without changing the application result. The reference submission
  path uses a typed phase state to retain an acknowledged commit result or
  rolled-back failure across later cleanup interruption; only interruption while
  transaction disposition is unknown is reported as uncertain.
  `OperationContext::run_resolved` applies that retained disposition before
  telemetry finalization, keeping the observed outcome aligned with the public
  submission result. The verifier's lease-owned query phase is boxed at its
  ownership boundary so both ordinary and `test-support` SQLx package test
  targets stay within rustc's default type-layout query depth. Add
  `RunledgerTransaction` for
  atomic application writes and direct enqueue through the upstream capability;
  the facade's `runledger` feature also exposes its required `sqlx` capability.
- Keep every guarded `HttpBoundary` fallback behind lifecycle admission while
  only opaque, validated literal `ProbePath` values can select routes outside
  it. Repeated probe paths across liveness and readiness declarations now return
  a sanitized configuration error before Axum can panic. `GuardedRouter` now
  retains route identities across route, merge and typed nesting, while opaque
  nested services cannot enter the protected path. Async assembly rejects any
  retained guarded route identity that can match a reserved probe path by
  querying a library-owned inert inventory; application handlers, fallbacks and
  middleware are never polled. Nested operational/quota wrappers now share one
  library-owned observation state, retaining quota facts on the sole outer
  completion event in either supported wrapper order. Distinct observer and
  operational markers preserve correlation when plain `observe_http` is
  outermost. Document and test the non-commutative boundary: correlation must
  remain outside admission, deadlines and `request_scope`.
- Enforce that facade-owned runnable examples import the public `batter` path.

- Soften public status language now that the library is in internal use: drop
  not-production-validated framing while keeping unpublished-package,
  evidence-scope, and API-limit facts.

- Add `batter_axum::browser` with validated HTTPS/loopback origins,
  duplicate-aware opaque named-cookie reads, fixed host-only/root-path cookie
  set and removal, exact Origin/custom-marker/Fetch Metadata/JSON mutation
  signals, sanitized application-mappable rejections, and private-response
  headers. Every custom-marker policy automatically requires exact same-origin
  Fetch Metadata; an accepted marker name alone is not treated as browser or
  preflight provenance. Authentication, authorization, CORS, CSRF-token design,
  session persistence/revocation, and application error envelopes remain
  outside the adapter.
- Replace Axum-owned `ReadinessReason::Dependency(HealthStatus)`, which could
  represent a healthy dependency failure, with foundation-owned exhaustive
  classifications. `HealthStatus` now projects explicitly to
  `DependencyReadiness`; `ReadinessEvaluator` combines that result with lifecycle
  observation into `ReadinessDecision::Ready` or
  `Unready(ReadinessUnreadyReason)`.
  Axum carries the valid decision in response extensions and maps only transport
  status/severity. Migrate `ReadinessPolicy::reason()` to `decision()` and update
  `with_level` callbacks to accept `ReadinessDecision`. Replace
  `ReadinessReason::Ready` with `ReadinessDecision::Ready`, `.status()` with
  `readiness_status(decision)`, and `.level()` with
  `default_readiness_level(decision)`. Response-extension lookups must request
  `ReadinessDecision` and call `.unready_reason()` for its optional payload. The
  old `ReadinessReason` name is absent from both `batter_axum` and
  `batter::readiness`, so a stale typed extension lookup cannot be revived by
  changing only its import and then silently return `None`. Match
  `batter::readiness::ReadinessUnreadyReason` only inside an unready decision.
  No invalid or ambiguously named compatibility variant is retained.
- Replace clone-wide `ShutdownHandle::mark_ready` with one-shot readiness
  ownership. `Supervisor::start` and `run_until` consume approval automatically;
  only explicitly exceptional `start_unapproved` and `run_until_unapproved`
  paths withhold it. They return non-cloneable `UnapprovedSupervisor` and
  `UnapprovedDriver` owners; consume `approve_readiness()` to choose the normal
  transition. The caller-owned form remains movable after polling without
  separating the approval capability. Canonical
  `Startup` performs the same transition after successful initialization, while
  `without_readiness_approval()` changes its successful handoff type so deferred
  policy cannot be mistaken for an approved driver. Standalone lifecycles can
  explicitly construct a paired `ReadinessApproval`. Repeated approval, approval
  through shutdown/status/admission projections, and approval after transition
  no longer compile.
  Migration: callers that intentionally withheld approval must replace ordinary
  `start`/`run_until` with the corresponding explicitly unapproved path.
  `ShutdownHandle::new` and `Default` are removed: use `new_unapproved` only for
  a deliberately permanent Starting lifecycle, or construct the handle together
  with its one-shot approval for standalone admission.
- Split root lifecycle authority from consumer projections. `ShutdownHandle`
  now requests shutdown and constructs purpose-qualified views:
  `LifecycleStatus` for readiness/status, `OperationAdmission` for creating a
  readiness-gated downward-cancelled `OperationContext`, and `ShutdownSignal`
  for drain/cancellation observation. `RequestPolicy` now requires
  `OperationAdmission`; `ReadinessPolicy` and the status-only Axum readiness
  handler require `LifecycleStatus`. Migrate `handle.readiness()`,
  `handle.wait_ready()`, `handle.is_draining()`, and `handle.draining()` to
  `handle.status().readiness()`, `handle.status().wait_ready()`,
  `handle.status().is_draining()`, and `handle.signal().draining()` respectively;
  pass `handle.operation_admission()` to request policy. Raw `operation_token()`
  has no direct replacement: transient work uses `OperationAdmission::admit`
  while Ready, component drain work observes `ShutdownSignal::cancelled()`, and
  cleanup uses an independent bounded context. Managed supervision receives
  private coordinator authority directly and can no longer recover it through a
  read-only signal.
- Replace provenance-dependent `ShutdownSignal::mark_started` with a linear
  component-start boundary. `Supervisor::register` and constrained
  `Registration::register` factories now receive a non-cloneable
  `ComponentStartup`; consume `acknowledge_started()` after actual initialization
  and retain its returned non-cloneable `RunningComponent` while running, then
  return its `stopped()` proof after work ends.
  `HealthMonitor::run` no longer acknowledges a registered component: prefer
  `HealthMonitor::register_in`, or explicitly acknowledge the factory's
  `ComponentStartup` and pass `running.signal()` to `run`, retaining the running
  capability for its final `stopped()` proof. Passing a clone from
  `startup.shutdown()` deliberately leaves readiness pending.
- Add an opt-in `RetryOptions` / `execute_with_options` boundary for per-attempt
  deadline caps and optional injected equal jitter. Attempt deadlines are
  recomputed after backoff, cannot exceed the input total/work context, and have
  a distinct non-exhaustive `RetryExecutionError` outcome. Legacy `execute`,
  `execute_with_jitter`, `RetryError`, `StopReason`, and `Interruption` remain
  source-compatible and keep their previous meanings. Options are consumed once
  and intentionally are not clonable, preventing accidental duplication of a
  stateful jitter stream across concurrent executions. The default sampler
  specialization implements `Default`, so `RetryOptions::default()` needs no
  sampler type annotation. Retry completion now reconciles cancellation of the
  public attempt scope before accepting a value or classifying an error, even
  when the factory cancels that scope and returns in the same poll. This applies
  consistently to the legacy and options entrypoints without changing their
  error-enum shapes or the meanings of existing variants. Composite attempt
  results are mapped at the operation-owned observation boundary so application
  errors remain WARN `failed`, cancellation remains INFO `cancelled`, and neither
  can be mislabeled as a successful attempt. An error returned in the same poll
  as terminal cancellation remains available as `last_error` without being
  classified or replayed.
- Hard-cut SQLx verification to compiled inputs. `AuthorityPolicyBuilder::build`
  now returns the only executable generic authority value, migration construction
  and mutation are fallible, and non-empty `VerificationPlan` constructors plus
  fallible cross-axis composition feed `verify`. This removes
  `VerificationPolicy`, `VerificationRequest`, `verify_request`, mutable executable
  policy fields, `CompiledExactRole::authority_policy`,
  `VerificationError::InvalidPolicy`, and
  `PolicyError::EmptyVerificationRequest`. Relation/sequence namespace conflicts,
  duplicate/contradictory declarations, capacities, and migration-ledger kind
  conflicts now fail during pure construction. Catalog-identity failures retain
  their typed `PolicyError` cause, and report coverage has canonical ordering.
  `MigrationPolicy` and `MigrationExpectation` fields are now private; use their
  fallible constructors, modifiers, and read-only accessors instead of struct
  literals or field access. Exhaustive matches must handle the new
  `PolicyError::{ConflictingRelationKind, InvalidObjectPrivilege,
  ContradictoryAuthorityPrivilege}` and
  `VerificationError::{CatalogIdentity, CatalogPolicyExpansion}` variants.
  Object-kind-invalid privileges now fail as `InvalidObjectPrivilege`, including
  invalid required privileges that previously surfaced as
  `ContradictoryRequiredPrivilege`; duplicate legacy PUBLIC privilege atoms with
  conflicting grant-option values now fail instead of being merged. Internal
  catalog-policy expansion now checks identities and appends normalized defaults
  during cooperative traversal instead of synchronously sorting and re-indexing
  the full generated policy in one executor poll.
- Replace the reference package's erased `ConfigMode`/`RootSettings` boundary
  with disjoint `ServingSettings` and `MaintenanceSettings`. Serving now follows
  the inert `runtime::prepare` to consuming `runtime::run` handoff, and the
  infallible router consumes `PreparedHttp`. Maintenance accepts only its
  database capability, ignores known serving-only environment values and has no
  serving conversion. Ambient `PG*` conflicts now fail as settings errors during
  preparation, before protected startup acquires resources.
- Replace blocking retained-panic inspection with `PanicPayload::try_inspect`;
  concurrent or recursive inspection now returns `PanicPayloadBusy`.
- Advance the root workspace's immutable Runledger source from Git revision
  `d57ec6be61e9f00ccce373b19ca356cafe98f206` to PR #15 revision
  `638ee3480f69962597147f5d7bd52822267560b7`, which supplies
  `PgTransactionExecutor` for the executor-only transaction bridge; no sibling
  development checkout is required. The frozen consumer-evidence workspace
  retains the historical revision it evaluated.
- Capture finite-command interruption at the final poll before future destruction.
  Distinguish native termination before initialization from process drain in
  managed reports, and correct the reference root's empty-registry description.
- Managed stop-control failures are published immediately, including later clock
  tightening while native settlement remains pending.
- The offline retirement CLI emits structured, redacted recovery facts and keeps
  the cancellation job ID, original refusal facts and cleanup disposition.
- Reference worker settings expose validated configuration rather than a native
  supervisor builder. The composition root prepares and transfers native work to
  managed registration; the configuration consumer test follows that ownership.
- Hard-cut the reference runtime startup error to
  `ProtectedRuntimeStartupFailure`, removing `RuntimeStartupFailure` rather than
  retaining a deprecated alias. Add `RuntimePoolCleanupFailure` with typed report
  access when an otherwise successful shutdown omits the required
  `postgres.pool` cleanup record.
- PostgreSQL lifecycle pool construction is now lazy and registers close ownership
  before return. Connectivity failures therefore identify the explicit
  `postgres.probe` stage rather than the former eager acquisition step.
- Reference process controls retain simultaneous assertion and shutdown errors,
  and executable TERM/INT cases require a signal-specific acknowledgement from a
  dedicated test binary plus the separate production binary's unchanged
  empty-stdout/fixed-stderr exit contract. The production entrypoint has no test
  signal mode, and the package explicitly retains that entrypoint as Cargo's
  default run target.

- Native runtime integration now accepts `PreparedSupervisor`, an owned validated
  launch value, instead of an arbitrary application closure. Registration rejection
  and dropping an unstarted process cannot start native work. Native preparation
  errors remain part of owned application startup and retain its cleanup report.

- Add `command::Command` for finite callbacks with independently owned LIFO
  finalization, concrete work results, retained panic/cleanup outcomes and optional
  absolute total reservation. Command cancellation stays below the parent and
  cannot cancel cleanup. Migrate the UDP finite-command example to this owner.
- Add adapter-facing managed component registration with library-owned native
  initialization acknowledgement, independently retained settlement and cleanup
  eligibility. Preserve reports after wrapper abortion, including native/future
  failures. Anchor process shutdown phases to one first-stop timestamp. Native
  adapter accepts inert native preparation and the reference root keeps readiness
  unapproved until the delivery handler exists. Remove production startup-control
  jobs, leases, owner epochs and reconciliation pools; preserve applied migrations.
  Add explicit offline retirement with identity/quiescence checks, scoped native
  cancellation and retained ambiguous outcomes. Linux acceptance includes the
  production-root HTTP probe and retirement command on both supported toolchains.
- Add `batter::settings` for explicit bounded sources, integer/duration parsing
  and redacted diagnostics. HTTP and reference roots own their schemas and native
  constructors.
- Add the optional `batter-sqlx` PostgreSQL lease, probe and pool-close adapter,
  plus unpublished postgres-lifecycle and reference-service example packages.
  Isolated fixtures are opt-in under `batter-sqlx/test-support`; provisioning stays
  in the external harness.
- Add owned `startup::Startup`, `health::HealthMonitor`, and Axum operational
  helpers (`operational_http`, `ReadinessPolicy`, `register_http`).
- Retain operation, HTTP and process-task tracing parents when diagnostic INFO
  spans are filtered.
- Add component-ownership comparisons and HTTP/1.1 transport/lifetime suites on
  the shared Unix process harness.
- Add the `finite_command` example; finite work and service startup have distinct
  owned lifecycle entrypoints.
- Add `ShutdownCause::FiniteTaskExit` for shutdown initiated by admitted finite
  task failures. `ComponentExit` remains specific to registered critical
  components. Downstream exhaustive matches must add the new variant; code that
  handled finite failures under `ComponentExit` must handle `FiniteTaskExit`.
- Remove `ShutdownHandle::observer`; obtain completion observers through
  `RunningSupervisor::observer` after `Supervisor::start` so every observer has
  an owned completion publisher.
- Split the virtual workspace into `batter`, `batter-axum`, `batter-sqlx`,
  independent generic test support, and two unpublished example packages. HTTP
  imports move to `batter_axum`; the old `axum` and `postgres-example` features
  are removed. Keep PostgreSQL provisioning external and retain Rust 1.94 for
  every package.
- Expose `batter::telemetry::with_current_dispatch` for adapter futures while
  preserving dispatch during polling and destruction. Keep the combined HTTP
  `RequestPolicy` readiness/deadline contract unchanged.
- Adopt Jig with locked core/all-feature/doctest gates and example-only SQLx
  database tooling disabled. Keep the existing two-toolchain and HTTP checks.
- Move shared example support to `support.rs`, extract HTTP event assertions,
  and isolate cleanup log-capture tests without changing failure assertions.
- Exclude transient Jig backups from static package link inspection.
- Select the official Jig v0.3.0 release and document revision-preserving updates.
- Fix manual CI file-budget comparisons, cache the selected Jig runtime, and
  surface conflicting Markdown plan edits. Add CI/cache/merge/archive regressions.
- Retain Rust/ExecPlan tooling and one Jig policy workflow alongside the
  existing Rust matrix. Remove unused
  frontend/proxy settings, duplicate workflows, and the checkout helper.
- Include Jig's launcher and durable records in source archives while excluding
  local runtime/cache data and the deprecated adoption receipt.

- Enable workspace Clippy complexity/length limits of 20/100. Extract private
  cleanup/shutdown helpers while preserving behavior, including shutdown-future
  ownership through cleanup after component failure.

- Distinguish completed-but-unobserved tasks from abort targets, and arm emergency
  shutdown before a caller-owned driver is first polled.
- Preserve scoped tracing during future destruction as well as polling, including
  nested HTTP spans and task abort; no downstream wrapper is required.
- Report and log every skipped cleanup hook through one LIFO path, including
  the first hook skipped after budget exhaustion.
- Fix critical early-exit classification racing with drain; require explicit
  startup acknowledgements before publishing readiness.
- Add bounded finite process-owned tasks and a separately driven shutdown
  coordinator with repeatable, cancellation-safe completion observation.
- Add application-controlled HTTP infrastructure rendering and default-visible
  operation/HTTP/cleanup observations with sanitized fields and scoped tracing.
- Add explicit finalization reserves and caller-sampled deterministic-testable
  jitter, preserving total deadlines and provider lower bounds.
- Add process-owned and operation-budget examples and startup/drop/race,
  rendering/redaction, reserve/jitter regressions; extend local HTTP smoke to
  SIGINT, error envelopes, deadlines, and ordinary INFO telemetry.
- Upgrade all nine direct Rust dependencies to their latest stable releases and
  refresh Cargo.lock; see [exact versions](docs/references.md#dependency-refresh-2026-09-07).
- Pin the default Rust toolchain to 1.98.1. Raise the workspace minimum to 1.94
  for SQLx 0.9.0 and update the CI matrix to 1.94.0, 1.98.1, and stable.
- Normalize rustfmt output, remove a redundant cleanup-test closure, and escape
  a generic type in HTTP rustdoc. Failure-path assertions and runtime contracts
  remain unchanged.
- Optional adapter live cases have Linux
  evidence; the historical 58-entry reference inventory and separate maintenance
  probe passed on both supported toolchains. The current 64-entry inventory has
  five Linux-executed offline entries; its 59 database probes remain unexecuted.

## 0.1.0 — Initial source snapshot — 2026-09-07

Added critical-task lifecycle supervision, explicit phased shutdown/readiness,
LIFO asynchronous cleanup reports, deadline/cancellation contexts, opted-in
bounded retry policy, process-local concurrency admission, minimal tracing,
optional Axum middleware/probes, and a small test-support crate.

Added worker, HTTP, and native SQLx lifecycle examples; 67 authored failure-contract
tests; verification/packaging scripts; CI definition; architecture decisions;
Effect v4 design rationale; integration boundaries and a prioritized agent backlog.

Validation limitation: no Rust toolchain or dependency resolution was available
in the authoring environment. Compilation, tests, formatting, lints, documentation
builds, live examples, and MSRV compatibility remain unverified. Packages remained unpublished.
