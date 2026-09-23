# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog], and this project adheres to
[Semantic Versioning].

## [Unreleased]

## [0.4.0] - 2026-09-22

### Changed

- **Breaking:** `permits_request()` is the only boolean admission predicate.
  Remove `Decision::is_allowed()`, `is_denied()`, `would_deny()`,
  `is_enforced_denial()`, and `is_shadow_denied()`, and the `would_deny()`,
  `is_enforced_denial()`, and `is_shadow_denied()` predicates on
  `BatchDecision`. A predicate that is true for shadow denials compiled
  cleanly as a rejection guard and turned shadow mode into enforcement. Use
  `!permits_request()` for enforcement and match `view()` for anything else.
- Add `Admitted` and `AdmittedView`, a decision type that can only hold an
  allowed or shadow-denied outcome, and `Decision::admit()`, which splits a
  decision into `Result<Admitted, Denial>` without loss. Code that receives an
  `Admitted` value never re-checks enforcement.
- **Breaking:** add validated `Capacity` and `QuotaPeriod` newtypes.
  `RateLimitPolicy::quota()` and `capacity()` return `Capacity` and
  `quota_period()` returns `QuotaPeriod`, so a third-party policy cannot report
  a zero or oversized value and nothing downstream re-validates one.
  `FixedWindowPolicy::limit()` and `window()` and `GcraPolicy::quota()`,
  `period()`, and `burst_capacity()` return the same types;
  `window_millis()` and `period_millis()` are replaced by
  `QuotaPeriod::millis()`. `CheckError::CostExceedsCapacity` carries a
  `Capacity`.
- **Breaking:** custom check cost now has one construction path:
  `Check::new(policy_subject).with_cost(cost)`. The associated
  `Check::with_cost(policy, subject, cost)` constructor and the duplicate
  `try_with_cost` builder are removed; `with_cost` is the fallible,
  self-consuming builder.
- **Breaking:** make `Allowance` public and use it wherever a check is known to
  be allowed. `Allowance::new(Capacity, available, replenishes_after)` returns
  `Result` and checks only that `available` does not exceed the capacity;
  there is no panicking constructor and no `try_new`. `QuotaDenial::new`
  takes a `Capacity` and cannot fail, so `QuotaDenial::try_new` and
  `DecisionError::InvalidCapacity` are removed. `Decision::allowed` takes an
  `Allowance` and `Decision::try_allowed` is removed. `DecisionView::Allowed`
  and `AdmittedView::Allowed` carry an `allowance` field, and
  `BatchDecisionView::Allowed` carries `allowances: &[Allowance]`. A denied
  member of an allowed batch is no longer representable, so
  `BatchDecision::try_allowed` and
  `DecisionError::DeniedDecisionInAllowedBatch` are removed. The Serde
  `allowed` batch object carries `allowances`, a list of `Allowance` objects
  with `capacity`, `available`, and `replenishes_after`, instead of
  `decisions`; `Allowance` also serializes on its own.
- **Breaking:** `BatchDecision::allowed`, `denied`, and `shadow_denied` return
  `Result<_, DecisionError>`; the panicking spellings and the `try_denied` and
  `try_shadow_denied` siblings are removed. `allowed` rejects an empty list
  with the new `DecisionError::EmptyBatch`, and `denied` and `shadow_denied`
  take `(index, batch_size, denial)` and return
  `DecisionError::DeniedIndexOutOfRange` when `index` is not below
  `batch_size`. `BatchDecisionView::Denied` and `ShadowDenied` expose
  `batch_size` as a `NonZeroUsize`, and the Serde `denied` and
  `shadow_denied` batch objects gain a required `batch_size` field that
  deserialization validates the index against.
- **Breaking:** empty batches are rejected. `validate_batch` and every
  backend's `check_all` return `BatchError::EmptyBatch` for an empty slice
  instead of an allowed batch with no allowances, so a caller that filtered
  every check out fails closed.
- **Breaking:** `Denial` is the exhaustive denial enum, with variants
  `QuotaExceeded(QuotaDenial)` and `StorageCapacity { retry_after }`. The
  opaque `Denial` value, `DenialView`, `Denial::view()`,
  `Denial::quota_exceeded()`, `Denial::storage_capacity()`, and
  `Decision::quota_denied()` are removed; `Decision::denied` accepts anything
  convertible into `Denial`. `DecisionView::Denied` and
  `BatchDecisionView::Denied` carry a `Denial` by value, and `DecisionView` no
  longer has a lifetime. Every denial reason is a named match arm, so a future
  reason fails to compile in every consumer instead of landing in a fallback
  branch.
- **Breaking:** add `Delay`, the one type for backend-measured durations that
  feed whole-second header fields. `QuotaDenial::retry_after()` and
  `Allowance::replenishes_after()` both return it, and
  `Denial::StorageCapacity` carries an optional one. `Delay::seconds()` rounds
  up for `Retry-After` and `RateLimit` fields and `Delay::duration()` keeps
  the exact measurement; the removed `retry_after_seconds()` accessors and
  `runlimit-http`'s private rounding are gone.
- **Breaking:** remove `DenialKind` and the reason-agnostic accessors
  `Denial::kind()`, `quota()`, `capacity()`, `retry_after()`, and
  `retry_after_seconds()`. Match `Denial` instead.
- **Breaking:** remove the optional accessors `Decision::capacity()`,
  `available()`, `replenishes_after()`, `retry_after()`,
  `retry_after_seconds()`, `denial()`, and `quota_denial()`, and
  `BatchDecision::allowed_decisions()`, `denied_index()`, `denial()`, and
  `quota_denial()`, together with `BatchDecision::try_into_allowed()` and
  `try_into_single_decision()`. Match `DecisionView` and `BatchDecisionView`
  instead: `try_into_allowed().is_ok()` was an `is_allowed()` predicate that
  is false for a shadow denial, and no backend converts a batch of one into a
  single decision any more. Shadow outcomes
  store `QuotaDenial` directly, making shadowed storage-capacity denials
  unrepresentable internally, and the Serde `shadow_denied` object now parses
  only a `quota_exceeded` denial instead of parsing any denial and rejecting a
  `storage_capacity` reason afterwards. The Serde wire representation of
  single decisions is otherwise unchanged.
- **Breaking:** observations expose their enums instead of optional
  accessors. `AdmissionOperation::Check` carries an `AdmissionPolicy` with
  the policy identifier, scope, and fingerprint together, and
  `AdmissionOperation::Batch` carries `batch_size` and the relevant
  `Option<AdmissionPolicy>`; `AdmissionObservation::batch_size()`,
  `policy_id()`, `scope_id()`, and `policy_fingerprint()` are removed.
  `AdmissionObservation::failed_batch(checks, consumption, elapsed)` is the
  only failed-batch factory: it derives the batch size from the input and
  includes policy metadata exactly when the batch contains one check.
  `CleanupObservation::new(requested, CleanupOutcome, elapsed)` replaces the
  `confirmed`, `definitely_no_effect`, and `outcome_unknown` factories, and
  `CleanupObservation::outcome()` replaces `removed()` and `consumption()`,
  which had reused `ConsumptionStatus::Consumed` to mean "rows were deleted".
  `CapacityObservation::new` takes and `shard_index()` returns a plain
  `usize`: every backend that reports capacity is sharded, so the index was
  never absent.
- **Breaking:** `Observation`, `AdmissionOperation`, `AdmissionOutcome`, and
  `ConsumptionStatus` are no longer `#[non_exhaustive]`, and neither is any
  other public enum: `DecisionError`, `runlimit_postgres::CheckError`,
  `MaintenanceError`, `PostgresConfigError`, and `EncodingError` drop the
  attribute too. A consumer names every variant, so a new outcome or error is
  a compile error instead of a silently unmetered or mishandled wildcard arm.
- **Breaking:** `Limiter` has separate `CheckError` and `CheckAllError`
  associated types in place of `Error`, so a single check's error type never
  carries a batch-only variant. `runlimit-axum`'s `RateLimitRejection` is
  parameterized by `L::CheckError`.
- **Breaking (`runlimit-memory`):** each operation has an error type listing
  only the failures it can produce. `MemoryStore::check`, `stats`, and `clear`
  return `PoisonedShardError`; `MemoryStore::check_all` returns
  `MemoryBatchError` with `InvalidBatch`, `BatchExceedsShardCapacity`, and
  `PoisonedShard`; `GcraStore::check` returns `GcraCheckError` and
  `GcraStore::check_all` returns `GcraBatchError`. `MemoryStoreError` and
  `GcraStoreError` are removed. Custom clocks are selected before construction
  with `MemoryStore::builder(config).with_clock(clock).build()` or the
  corresponding `GcraStore` builder, so a built store cannot reinterpret live
  timestamps under another clock. `with_observer` remains a store builder.
- **Breaking (`runlimit-postgres`):** a single check is shaped directly from
  the database response instead of being converted from a batch of one, and
  every decision is built before commit or rollback. `CheckError::InvalidBatch`,
  `ResponseInvariant`, and `CommittedResponseInvariant` are removed;
  `check_all` returns the new `BatchCheckError` with `InvalidBatch` and
  `Check(CheckError)` variants. `CheckError::consumption()` and
  `BatchCheckError::consumption()` return a `ConsumptionStatus` and replace
  `may_have_consumed_quota()`. `CheckError::TimedOutBeforeCommit` and
  `MaintenanceError::TimedOutBeforeCommit` carry a typed `CheckPhase` or
  `CleanupPhase` instead of a string. `PostgresLimiter::with_config` is a
  builder on `PostgresLimiter::new(pool)`. Query-result fields that cannot be
  decoded into the protocol's expected types are reported as pre-commit
  `StorageInvariant` failures and leave quota definitely unconsumed. Their
  `StorageInvariantError` payload preserves the originating SQLx decode error;
  its constructors remain backend-owned so callers cannot fabricate or erase
  that distinction.
- **Breaking (`runlimit-http`):** `draft_11::service_limit` takes anything
  convertible into the new `QuotaState`, an `Allowance`, a `QuotaDenial`, or
  an `Admitted` outcome, instead of a `Decision`, so a storage-capacity denial
  cannot reach it. `EncodingError::UnsupportedDecision`, `ZeroQuotaPeriod`,
  and `InvalidHeaderValue` are removed as unreachable.
- **Breaking:** `KeyHasher::hash(policy_id, scope_id, subject)` is removed;
  `KeyHasher::hash_for(&policy, subject)` is the only derivation and returns a
  `PolicySubject` retaining that exact policy reference. `Check::new` accepts
  only this bound value, so the normal construction path has no independent
  policy argument to substitute. `PolicySubject::into_unbound_subject_key`
  explicitly leaves that path for adapter boundaries; `Check::subject` and
  `CounterKey::subject` likewise expose an unbound key that can be rebound.
  `SubjectKey::from_digest` remains a test and already-opaque-digest escape
  hatch; callers must bind it explicitly before constructing a check. The
  packaged external-consumer smoke test extracts and compiles the exact README
  GCRA example.
- **Breaking (`runlimit-axum`):** `ExtractSubjectKey::extract_subject_key`
  returns only a `SubjectKey`, not a `PolicySubject`. `RateLimitLayer` binds
  that opaque key to its configured policy, making it impossible for an
  extractor to replace the policy the layer evaluates while the admission
  records another. Named extractors using `KeyHasher::hash_for` explicitly
  call `PolicySubject::into_unbound_subject_key` at this adapter boundary.
- **Breaking (`runlimit-axum`):** `RateLimitRejection` is no longer
  `#[non_exhaustive]`, and `RateLimitRejection::Denied` carries a
  `runlimit_core::Denial` instead of a `Decision`, so a rejection mapper names
  every rejection category and never handles an allowed or shadow-denied arm.
- **Breaking (`runlimit-axum`):** admitted requests carry an `Admissions`
  request extension instead of a `runlimit_core::Decision`. Every
  `RateLimitLayer` the request passed appends an `Admission` naming its policy
  identifier, scope, and fingerprint together with the `Admitted` outcome, so
  stacked layers no longer overwrite each other's decision. Look a layer's
  outcome up with `Admissions::get(&policy)` or iterate them in evaluation
  order. Code passing adapter outcomes to
  `runlimit_http::draft_11::service_limit` passes `admission.decision()`
  directly and, in a rejection mapper, the `QuotaDenial` from a matched
  `Denial::QuotaExceeded`.
- Document why atomic batches reject mixed quota modes and how to shadow one
  policy of a multi-policy batch during a rollout.
- **Breaking:** upgrade `runlimit-postgres` to SQLx 0.9.0. Applications passing
  SQLx pools or handling SQLx errors must also upgrade to SQLx 0.9.
- Raise the workspace minimum supported Rust version from 1.88 to 1.94.

### Fixed

- Defer `MemoryStore` and `GcraStore` async `Limiter` checks until their
  futures are first polled. Creating and dropping an unpolled single or batch
  check no longer consumes quota. **Breaking:** the futures returned by these
  backends' `Limiter::check` and `Limiter::check_all` methods no longer implement
  `Unpin`. Callers that require `Unpin` must pin these futures first, for example
  with `std::pin::pin!` or `Box::pin`; ordinary `.await` calls are unaffected.

## [0.3.0] - 2026-08-24

### Added

- Add borrowed `DecisionView` and `BatchDecisionView` enums so callers can
  exhaustively match valid decision states without reconstructing them from
  optional accessors. Existing accessors and Serde representations remain
  unchanged.
- Add `AdmissionObservation::from_check` and
  `AdmissionObservation::from_batch` constructors for consistently deriving
  backend-neutral outcome, consumption, and relevant-policy metadata from
  completed admission decisions.

### Changed

- **Breaking:** replace the raw seven-field `AdmissionObservation::new`
  constructor with semantic failed-check and failed-batch factories. Policy ID,
  scope, and configuration-fingerprint metadata are now derived together from
  a validated check, while failure factories require an explicit consumption
  status. The existing `Debug` field representation remains unchanged.
- **Breaking:** replace the raw `CleanupObservation::new` constructor with
  `confirmed`, `definitely_no_effect`, and `outcome_unknown` factories so a
  confirmed removal count cannot be paired with contradictory consumption
  certainty. Existing getters and the `Debug` field representation remain
  unchanged.
- **Breaking:** replace directly constructible decision and denial enum states
  with validated constructors and read-only accessors. Shadow denials now
  require `QuotaDenial`, allowed batches reject denied members, and the Serde
  wire representation remains unchanged.
- **Breaking:** remove `From<PolicyError> for GcraPolicyError`, whose public
  conversion could panic for constructible fixed-window limit errors. Policy
  constructors continue to report the same algorithm-specific validation
  errors in the same order.
- **Breaking:** return `GcraStoreError` from GCRA admission operations and move
  the GCRA-only arithmetic-overflow failure out of `MemoryStoreError`. Common
  bounded-storage failures are available through `GcraStoreError::Store`.
- Share the bounded shard, expiration, capacity, locking, and recovery
  machinery used by the in-memory fixed-window and GCRA stores while retaining
  their separate policy algorithms and existing behavior.
- Reorganize the PostgreSQL backend into focused configuration, error,
  protocol, admission, and maintenance modules without changing its public
  paths, SQL protocol, or published migrations.

## [0.2.0] - 2026-07-25

### Added

- Add a generic async `Limiter` trait with an associated policy type for
  statically dispatched storage and adapter substitution.
- Add `GcraPolicy` and a hard-bounded process-local `GcraStore` with exact
  scaled-integer replenishment, weighted checks, atomic batches, and bounded
  cleanup.
- Add per-policy enforce and shadow modes. Shadow quota denials are explicit,
  permit the application request, and retain warmed counter state for a later
  switch to enforcement.
- Add backend-neutral operational observers for admission outcome, consumption
  certainty, latency, cleanup work, and capacity headroom.
- Add `runlimit-axum`, caller-controlled Axum/Tower middleware that checks
  admission before the inner service without interpreting forwarding headers
  or defining application responses.
- Add `runlimit-http` with versioned helpers for the active IETF
  `RateLimit-Policy` and `RateLimit` draft-11 response fields.
- Add replica-safe PostgreSQL cardinality enforcement with 256 stable capacity
  shards, a transactionally maintained ledger, a configurable lower
  operational bound, and a database-enforced rolling-deployment ceiling.
- Add opt-in, invariant-preserving Serde support for public policy, decision,
  backend configuration, and memory telemetry value types.
- Add `Debug` for `MemoryStore`, `Clone` for `KeyHasher`, explicit poisoned
  shard recovery, and public constants for each bundled PostgreSQL migration.

### Changed

- Generalize checks over `RateLimitPolicy`, and rename algorithm-neutral
  decision metadata from limit/remaining/reset to
  capacity/available/replenishes-after.
- Precompute `KeyHasher`'s zeroizing HMAC state so each subject derivation
  avoids rebuilding the SHA-256 key schedule.
- Use already-uniform counter-key material directly for memory shard selection
  and entry hashing, and make quota arithmetic fail closed if stored usage ever
  exceeds its invariant.
- Resolve PostgreSQL's authoritative clock explicitly through `pg_catalog` and
  give pool acquisition a separate timeout so a connection survivor receives
  a fresh database-operation budget.
- Mark PostgreSQL `CheckError`, `MaintenanceError`, `PostgresConfigError`, and
  core `BatchDecision` as non-exhaustive so 0.x releases can add variants
  without breaking consumers.
- Reject in-memory atomic batches that can never fit in a target shard with a
  structural `MemoryStoreError` instead of a retryable capacity denial.
- Route PostgreSQL single and batch admission through the same deterministic
  logical-key, counter-row, and capacity-shard lock protocol, and configure
  transaction-local timeouts once after `BEGIN`.
- Reserve heap space for HOT counter updates with an additive PostgreSQL
  `fillfactor` migration and document workload-specific autovacuum monitoring
  and tuning.

### Fixed

- Preserve a valid PostgreSQL denial when rollback confirmation fails or its
  client deadline expires; the affected connection is discarded.
- Sample PostgreSQL cleanup time once in a materialized CTE so the expiry
  predicate remains an index range condition and bounded cleanup does not scan
  every active row.

## [0.1.0] - 2026-07-24

### Added

- Add framework-neutral fixed-window policies, opaque HMAC-derived subject
  keys, structured decisions, and all-or-nothing ordered batch checks.
- Add a hard-bounded, sharded in-memory backend with bounded cleanup work and
  fail-closed capacity exhaustion.
- Add a replica-safe SQLx/PostgreSQL backend with database-authoritative time,
  transactional batch admission, bundled migrations, and bounded expired-row
  cleanup.
- Add Rust 1.88 compatibility checks, disposable PostgreSQL integration tests,
  and an external-consumer smoke test for packaged crates.

[Keep a Changelog]: https://keepachangelog.com/en/1.1.0/
[Semantic Versioning]: https://semver.org/spec/v2.0.0.html
[Unreleased]: https://github.com/bpcakes/batter/compare/runlimit-v0.4.0...HEAD
[0.4.0]: https://github.com/bpcakes/batter/releases/tag/runlimit-v0.4.0
[0.3.0]: https://github.com/bpcakes/runlimit/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/bpcakes/runlimit/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/bpcakes/runlimit/releases/tag/v0.1.0
