# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog], and this project adheres to
[Semantic Versioning].

## [Unreleased]

### Added

- `AttemptSubject::counter_key` returns the attempt storage identity, the
  policy fingerprint and the subject key, as `Check::counter_key` does for
  quota checks. The memory and PostgreSQL attempt backends key, shard and lock
  by it instead of rebuilding it from `into_unbound_subject_key`. The attempt
  policy fingerprint, the attempt subject key and the PostgreSQL attempt
  capacity shard now have golden-vector tests computed independently of the
  crate, and `hash_for` and `hash_attempt_for` share one private keyed
  encoding. Every derived byte is unchanged (`batter-39wm.3`).
- `runlimit_core::attempts::AttemptState<T>` is the one attempt lease
  lifecycle: `Clear`, `Leased(AttemptLease<T>)` or `Failed(FailedAttempts)`.
  Its pure `settle`, `admit` and `complete` own lease expiry counted as a
  failure at the deadline, the quiet-period reset, the busy and backoff denials
  and the failure delay, and return `AdmitEvaluation` (`Grant(PendingLease)`,
  `Denied(DeniedAdmission<T>)` or `DeadlineOverflow`) and `CompleteEvaluation`
  (`Applied(AppliedCompletion)` or `Stale`). A lease is granted for the whole
  policy lease or not at all: within one lease of `Duration::MAX` a subject
  that would be admitted gets `DeadlineOverflow`. A grant and a refused
  deadline both carry the subject's `UnleasedAttempt` state at that instant,
  so a backend that cannot issue the lease can still store it. Only a
  transition builds a
  `DeniedAdmission` or an `AppliedCompletion`, and each carries the narrower
  state it can leave, `HeldAttempt<T>` or `UnleasedAttempt`, beside the
  `LeaseDenial` or `AttemptCompletion` that state gives, so a backend stores
  one and reports the other without checking them against each other. `T` is
  the backend's lease token, which `complete` requires to be
  `Eq`; `StorageFenced` stands in for it where storage compares the token
  itself, and a state decoded with it cannot complete. `AttemptPolicy::delay` is the
  only backoff derivation, and the new `AttemptPolicy::delay_schedule`
  tabulates it for storage that must apply a failure inside one statement.
  This is backend-author API, as `runlimit_core::gcra::evaluate` is for GCRA;
  the limiter APIs and their results are unchanged (`batter-ctk7.6`).
- `PostgresAttemptLimiter` implements `Debug` and has `config()`, and
  `PostgresAttemptLimiter::new` is a `const fn`, matching `PostgresLimiter` and
  `PostgresGcraLimiter` (`batter-2oht`).

### Fixed

- `PostgresGcraLimiter::cleanup_expired` advanced every candidate shard to the
  most advanced candidate's watermark instead of each shard's own clamped
  clock. After a database clock regression, one cleanup pass could therefore
  delete still-active counters in other shards and admit a fresh burst for
  their keys. Each shard now advances to the later of the database clock and its
  own watermark. Without a clock regression behavior is unchanged; no schema,
  migration or public item changes. Admission is not changed: a multi-shard
  `check_all` batch still evaluates every member at the most advanced batch
  shard's watermark and writes that watermark to each of its shards, so after a
  clock regression it can still advance another shard's clock
  (`batter-70qx.1`).

### Changed

- The `runlimit-postgres` package no longer ships `src/attempt_complete.sql`
  or `src/cleanup_expired.sql`. No code path executed either statement; both
  stay byte-identical, with their digests, as provenance records in
  `runlimit/provenance/runlimit-postgres/`, outside every package. The
  executed statements, SQL behaviour and persisted state are unchanged
  (`batter-39wm.2`).
- The three PostgreSQL limiters now share one private shell for their pool,
  bounds, observer, migration, bounded connections and transactions, and both
  quota limiters share one response finisher. No public item is removed,
  renamed or re-typed; SQL, lock order and persisted state are unchanged. Two
  observable differences follow here, and the next entry records a third
  (`batter-2oht`):
  - `PostgresAttemptLimiter::migrate` returns its connection to the pool after
    a successful run and unlock, as the other two limiters do. It previously
    closed the connection on every outcome. An error or cancellation still
    closes it.
  - `PostgresGcraLimiter`'s `Debug` output ends with `..`, as
    `PostgresLimiter`'s does, because the observer is reported only as
    `has_observer`.
- **Breaking:** a deadline reached exactly now counts as exhausted when the
  timeout refresh before a statement or a commit is computed, as a deadline
  already passed does (`batter-2oht`). A zero remaining budget was previously
  treated as budget: the refresh was sent with 1 ms server timeouts and could
  retire a reusable connection when it timed out. The operation now fails
  before sending that refresh and its connection stays reusable. This is a
  runtime break: nothing stops compiling, but an admission that could return a
  denial now fails. If the deadline was reached at the refresh before a later
  statement, and that refresh and every remaining statement finished before
  the client timer fired, `PostgresLimiter::check` and `check_all`,
  `PostgresGcraLimiter::check` and `check_all`, and
  `PostgresAttemptLimiter::admit` returned their denial. That includes a
  shadow denial, which permits the request, and the attempt limiter's busy,
  backoff and storage-capacity denials. They now return
  `CheckError::TimedOutBeforeCommit` with the phase of the statement that was
  about to run. Allowances, attempt admissions and completions, and cleanup
  need a commit, which already failed once the deadline was reached. Quota
  consumption certainty is unchanged: nothing was consumed either way. The
  window is a single clock reading wide.

  Newly failing, with its handling:

  ```rust,ignore
  // Before: the operation deadline was reached exactly as the last
  // statement's timeout refresh was computed, and that statement finished
  // before the client timer fired. Each call returned its denial; a shadow
  // denial permitted the request.
  let decision = limiter.check(&check).await?;
  let batch = limiter.check_all(&checks).await?;
  let admission = attempts.admit(subject).await?;

  // After: the same calls fail before sending the refresh. Nothing was
  // consumed and no lease was taken, so fail closed as for any other
  // pre-commit timeout. A single check and an attempt admission report
  // `CheckError` directly:
  match limiter.check(&check).await {
      Ok(decision) => apply(decision),
      Err(CheckError::TimedOutBeforeCommit { phase }) => fail_closed(phase),
      Err(error) => fail_closed_for(error),
  }
  match attempts.admit(subject).await {
      Ok(admission) => apply_admission(admission),
      Err(CheckError::TimedOutBeforeCommit { phase }) => fail_closed(phase),
      Err(error) => fail_closed_for(error),
  }
  // A batch wraps the same error in `BatchCheckError::Check`:
  match limiter.check_all(&checks).await {
      Ok(batch) => apply_batch(batch),
      Err(BatchCheckError::Check(CheckError::TimedOutBeforeCommit { phase })) => {
          fail_closed(phase)
      }
      Err(error) => fail_closed_for_batch(error),
  }
  ```

  Log text an operator may match. A call that returned a denial and logged no
  error can now end as "PostgreSQL rate-limit check timed out while
  preflighting counter batch; quota was not consumed", or with another phase
  in place of "preflighting counter batch"; an attempt admission reports
  "acquiring counter row lock" there, and a batch prints the same text. A
  statement that follows a refresh which itself used up the budget is still
  attempted, as before (`batter-pp9j`).
- **Breaking:** `PostgresLimiter::cleanup_expired` and
  `PostgresGcraLimiter::cleanup_expired` commit the way a check does: refresh
  the server timeouts, confirm that budget remains, then attempt the commit
  (`batter-2oht`). Cleanup previously confirmed the budget first and attempted
  the commit after the refresh without looking at the clock again. This is a
  runtime break: nothing stops compiling, but one cleanup that could succeed
  now fails. Three cases at the deadline are affected differently:
  - The deadline has been reached or passed on arriving at commit, so no
    refresh is sent. The error is still `TimedOutBeforeCommit` and no rows are
    removed, but its phase is `CleanupPhase::PreparingCommit` instead of
    `CleanupPhase::StartingCommit`.
  - The refresh itself reports a timeout, from the client deadline or from
    the server. Unchanged: `TimedOutBeforeCommit` with
    `CleanupPhase::PreparingCommit`.
  - The refresh succeeds and the budget check that follows finds the deadline
    passed. Cleanup used to attempt the commit anyway, with three possible
    results: `Ok(removed)` when the server acknowledged before the client
    timer fired, `CommitOutcomeUnknown` when `SQLx` returned an error first (a
    connection lost during commit, for example), and `CommitTimedOut`
    otherwise. The last two are uncertain outcomes. It now stops before the
    attempt with `TimedOutBeforeCommit` and `CleanupPhase::StartingCommit`,
    and removes no rows.

  A cleanup that used to succeed can also end with
  `CleanupPhase::PreparingCommit`. Cleanup read the clock twice before commit.
  If the first reading still had budget and the second, for the refresh,
  found the deadline reached exactly, the refresh was sent with 1 ms timeouts
  and the commit could follow and succeed. A deadline reached exactly now
  counts as exhausted, as the previous entry records.

  Newly failing, with its handling:

  ```rust,ignore
  // Before: the commit's timeout refresh succeeded with the operation
  // deadline already passed, and the server acknowledged the commit before
  // the client timer fired. This returned the number of removed rows.
  let removed = limiter.cleanup_expired(1_000).await?;

  // After: the same run stops before commit and removes nothing. Expired rows
  // stay eligible, so the next scheduled pass repeats the work. Raise
  // `PostgresConfig::with_operation_timeout` if it recurs.
  match limiter.cleanup_expired(1_000).await {
      Ok(removed) => record(removed),
      Err(MaintenanceError::TimedOutBeforeCommit {
          phase: CleanupPhase::PreparingCommit | CleanupPhase::StartingCommit,
      }) => retry_on_the_next_pass(),
      Err(error) if error.may_have_removed_rows() => treat_as_unknown(error),
      Err(error) => treat_as_no_effect(error),
  }
  ```

  Log text an operator may match. A run that ended as "PostgreSQL
  expired-window cleanup commit timed out; rows may have been removed" or as
  "PostgreSQL expired-window cleanup commit outcome is unknown; rows may have
  been removed" can now end as "PostgreSQL expired-window cleanup timed out
  while starting expired-window cleanup commit; no rows were removed", and a
  run that succeeded can now end that way or as "timed out while preparing
  cleanup commit". A run that reached commit after its deadline reported
  "timed out while starting expired-window cleanup commit" and now reports
  "timed out while preparing cleanup commit". A unit test proves that an
  exhausted budget never polls the commit; the timing of a real refresh
  against the deadline is not reproduced by a test.
- `MemoryStore` and `GcraStore` now bind each check or batch to the shards it
  locks. Fixed-window batches stop at the first quota or capacity denial. GCRA
  checks the remaining members for arithmetic errors before cleanup. Both
  stores check capacity for the allowed prefix. Fixed-window batches charge
  the validated keys in a second pass under the same locks. GCRA applies its
  saved updates after preflight succeeds. The session adds no allocation to single
  checks. The two-check batches measured in `batter-ctk7.10` used fewer
  allocations.
- `MemoryAttemptLimiter` stores `AttemptState` instead of separate failure,
  timestamp and optional-lease fields, and evaluates admission, completion and
  cleanup through it. Admissions, denials, completions, delays, observations,
  the 16-entry cleanup scan and allocation behavior are unchanged for each
  subject, with one exception on an injected clock within one failure delay
  of `Duration::MAX`: the retry deadline saturates there, and a failed
  completion and its `Completed` observation now report the wait that remains
  until it, as the next `Backoff` denial does, where they reported the full
  policy delay. Admission on such a clock within one lease of `Duration::MAX`
  now fails, as the first **Breaking** entry below describes. Two effects on
  stored state are new. `complete` with a receipt whose
  lease has expired still returns and observes `Stale`, but now records the
  expiry as a failure at once rather than at the next admission, and removes
  the subject when its quiet period has also passed. That frees its capacity
  slot without a cleanup scan, so in a full store a new subject can be
  admitted, and observed as `Admitted`, where it was denied `StorageCapacity`
  until a scan reached the expired entry. A cleanup scan that retains an entry
  whose lease has expired stores that failure instead of recomputing it on
  every later scan (`batter-ctk7.7`).
- `PostgresAttemptLimiter::admit` decodes the locked row once into
  `AttemptState` and evaluates it with the transition the memory backend calls.
  Completion executes `src/attempt_complete_scheduled.sql`, which reads the
  delay from the bound `AttemptPolicy::delay_schedule` instead of restating the
  formula; the imported `src/attempt_complete.sql` is retained unchanged as a
  provenance record that no code path executes. No migration, schema, public
  item, lock protocol or fence changes. Storage is compatible in both
  directions: rows written by earlier releases decode unchanged, and every row
  this release writes is a shape earlier releases already read. While a lease
  is held, `last_failure_ms` and `retry_at_ms` now hold the admission instant;
  neither release reads them until the lease ends. A new live test,
  `memory_and_postgres_share_attempt_transition_semantics`, holds both backends
  to one scripted lifecycle (`batter-ctk7.8`).

  For rows this library writes, replicas of both releases reach the same
  decisions while the server clock does not step backwards. After a backwards
  step they can differ during a rolling upgrade in one case: a row without a
  lease and without failures is `Clear`. After a success both timestamps hold
  the completion instant, so an earlier release denies `Backoff` until the
  clock passes that instant again, and this release admits.

  One clock-step behavior is unchanged from earlier releases and is now
  documented and pinned by the live test
  `a_backwards_clock_step_revives_an_unreplaced_expired_lease`: a `Busy` or
  `Backoff` denial and a stale completion leave the subject's row unchanged, so
  after a step behind a lease deadline a lease already reported expired is
  live again until that deadline (`batter-ctk7.22`). A `StorageCapacity`
  denial still commits the bounded cleanup of expired rows, as before.
- **Breaking:** `MemoryAttemptError` has a new variant, `ClockExhausted`, so
  an exhaustive `match` on it stops compiling until it handles the variant.
  `MemoryAttemptLimiter::admit` returns it when the clock is within one policy
  lease of `Duration::MAX` and the subject would otherwise be admitted. Only a
  clock injected through `MemoryAttemptLimiter::with_clock` reaches that
  range. Earlier releases admitted there with a lease cut short at
  `Duration::MAX`; at `Duration::MAX` itself that lease was already expired,
  so its receipt was stale and a second admission was granted beside it.
  Denials are unchanged there, and a lease that ends exactly at
  `Duration::MAX` still expires at that instant. The error displays as
  `attempt clock exhausted its representable range`.

  ```rust
  // Before: every admission error was one of two variants.
  match limiter.admit(subject) {
      Ok(admission) => handle(admission),
      Err(MemoryAttemptError::Poisoned | MemoryAttemptError::SequenceExhausted) => fail_closed(),
  }
  // After: fail closed for the clock range as well.
  match limiter.admit(subject) {
      Ok(admission) => handle(admission),
      Err(
          MemoryAttemptError::Poisoned
          | MemoryAttemptError::SequenceExhausted
          | MemoryAttemptError::ClockExhausted,
      ) => fail_closed(),
  }

  // Before, with an injected clock at Duration::MAX: Ok(Admitted(receipt)),
  //         twice in a row for one subject, each receipt already stale.
  // After:  Err(MemoryAttemptError::ClockExhausted) both times.
  limiter.admit(subject)
  ```
- **Breaking:** this is a run-time break for hand-edited attempt rows only;
  nothing stops compiling and no row this library writes is affected.
  `PostgresAttemptLimiter::admit` now fails closed with
  `CheckError::StorageInvariant`, before commit and without consuming anything,
  for a row that fits no lease state. The subject stays refused until the row
  is corrected or admission's bounded cleanup deletes it after its quiet period.

  ```sql
  -- A delay on one subject with no failures and no lease, under a quiet
  -- period longer than the 60 seconds.
  UPDATE runlimit_attempts SET failures = 0, retry_at_ms = last_failure_ms + 60000
  WHERE config_fingerprint = $1 AND subject_key = $2 AND lease_until_ms IS NULL;
  ```

  ```rust
  // Before: Ok(AttemptAdmission::Denied(AttemptDenial::Backoff { .. })) until
  //         retry_at_ms, or until the quiet period after last_failure_ms ended
  //         if that came first.
  // After:  Err(CheckError::StorageInvariant(_)), detail
  //         "attempt without failures carries a retry delay".
  limiter.admit(subject).await
  ```

  To hold a subject back by hand, store at least one failure with the delay,
  for that one unleased row: `SET failures = GREATEST(failures, 1),
  retry_at_ms = ...` with the same `WHERE` clause. As before, the hold ends at
  `retry_at_ms` or when the quiet period after `last_failure_ms` ends,
  whichever comes first, and the two ends differ. At `retry_at_ms` admission
  resumes with the stored failure count, so the next failure takes the next,
  longer delay. Only the end of the quiet period clears the failures and lets
  cleanup delete the row, which is why a later `retry_at_ms` cannot hold
  longer. The `Backoff` denial now
  reports the wait until that earlier instant; earlier releases reported the
  wait until `retry_at_ms` even when the quiet period ended first, so replicas
  of the two releases report different delays for such a row and admit at the
  same instant. To hold for up to one quiet
  period from now, also set `last_failure_ms` to the current server time in
  milliseconds. Moving
  `retry_at_ms` earlier still releases a subject and keeps its failures, and
  stored instants before 1970 keep their exact meaning. An operator matching
  logs can match that detail string; `"invalid attempt failure count"` is
  unchanged (`batter-ctk7.8`).
- PostgreSQL GCRA retry and replenishment delays now subtract database-measured
  elapsed time through the final statement of the decision path, saturating at
  zero, matching fixed-window delay semantics. The evaluation instant, persisted
  quota state, public API and commit-uncertainty behavior are unchanged.
- Raise the minimum supported Rust version from 1.94 to 1.98.1, the Batter
  workspace's pinned toolchain. No public item changes.
- `MemoryAttemptLimiter::admit` borrows the subject's policy and clones it
  only when it inserts a new subject; it no longer constructs an entry for an
  occupied key. Its bounded expiry cleanup copies the at most 16 examined keys
  into a fixed stack array instead of a heap vector. Busy, backoff and
  storage-capacity denials and existing-subject admissions therefore allocate
  nothing; the 16-entry scan bound, cursor progression and wrap, policy
  fingerprints and public APIs are unchanged. Latency was not measured.
- `PostgresLimiter::cleanup_expired` identifies expired windows by their
  primary key instead of the `ctid` system column. PostgreSQL checks a
  system-column read against relation-level `SELECT`, so a cleanup login no
  longer needs to read every counter column; `SELECT` on `capacity_shard`,
  `config_fingerprint`, `subject_key` and `window_expires_at`, `UPDATE` on
  `window_expires_at` for the row lock, and `DELETE` suffice. The bounded,
  `SKIP LOCKED`, shard-ordered protocol and its results are unchanged. The
  imported `src/cleanup_expired.sql` is retained unchanged as a provenance
  record; the executed statement is `src/cleanup_expired_by_key.sql`.

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
