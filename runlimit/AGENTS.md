# Native Runlimit packages

## Purpose
Five native packages in the root Batter workspace own policy validation, key derivation,
quota/attempt storage and transport adapters. Follow the root guide and Beads tracker.
Keep coordinated version 0.4.0, native licenses, strict lints, and publication
restricted to crates.io. These packages
cannot depend on Batter or introduce its facade into native consumption.

## Key entrypoints
`runlimit-core` owns validated policies/decisions and opaque subject derivation;
`runlimit-memory` owns bounded process-local stores; `runlimit-postgres` owns
transactions, migration families and bounded cleanup; `runlimit-http` owns response
metadata; `runlimit-axum` owns the explicitly caller-controlled native layer.

## Edit here for X
Native algorithms and persistence belong here. Operational factory execution and
protected authenticated assembly belong in `crates/batter-runlimit`. Applications
own credentials, identity normalization, proxy trust, response bodies and policies.
Never rewrite published migrations or persistent lock/key derivation protocols.
The import inventory retains original SQL/license digests; new schema work adds
forward migrations instead. Source-copy consumer work is tracked by batter-isdr.2.

## Invariants

- The memory fixed-window and PostgreSQL backends implement the same anchored
  fixed-window semantics. `GcraStore` implements only `GcraPolicy`.
- A policy configuration fingerprint is part of every storage key. Changing
  any storage-relevant configuration never reinterprets an existing counter.
- Numbers are validated once, where they enter. `Capacity` is nonzero and at
  most `MAX_LIMIT`; `QuotaPeriod` is nonzero, whole milliseconds, and at most
  `MAX_WINDOW`. `RateLimitPolicy` and the built-in policy accessors return
  these types, `QuotaDenial::new` takes a `Capacity` and cannot fail, and
  nothing downstream re-validates a quota, capacity, or period. The only
  relation checked at decision construction is `available <= capacity`.
- Enforced and shadow-denied checks do not consume quota. Storage-capacity
  denials are always enforced, and the decision types enforce this: only a
  validated `QuotaDenial` can be shadowed, and the Serde `shadow_denied` wire
  object only parses a `quota_exceeded` reason.
- Decision metadata is validated where it is constructed, never where it is
  serialized. Every constructible `Decision`, `Denial`, and `BatchDecision`
  is reportable and serializable; serialization must not reject their metadata,
  and new invariants belong in their constructors. Constructors that check an
  invariant return `Result`; there is no panicking sibling with a shorter
  name. A batch denial carries its batch size and its index is validated
  below that size, and an allowed batch carries at least one allowance, so a
  batch decision never names an input it did not contain.
- An empty batch is a `BatchError::EmptyBatch` failure, never a vacuous
  allowance. A caller that filtered every check out must fail closed rather
  than admit the request without evaluating a policy.
- The public API is designed for AI-agent consumers. No public enum is
  `#[non_exhaustive]`, errors included: consumers match errors to choose
  fail-closed responses and retry behavior, so a new variant is a compile
  error in every consumer rather than a fallback arm, and adding one is a
  semver-signaled change. Every dispatch point is exhaustive: `DecisionView`,
  `BatchDecisionView`, `Denial`, `AdmittedView`, the observation enums
  `Observation`, `AdmissionOperation`, `AdmissionOutcome`, `ConsumptionStatus`,
  and `CleanupOutcome`, the adapter rejection `RateLimitRejection`, and the
  HTTP `QuotaState`. Do not add accessors that answer for several outcomes at
  once with an `Option`, and do not make a field optional that every producer
  fills in: `CapacityObservation` always names its shard because every bounded
  backend that reports capacity is sharded. Expose new metadata on the view
  variant it belongs to, and encode header rules such as rounding up in types
  like `Delay` rather than in documentation. `Delay` is the one type for every
  backend-measured duration that feeds a whole-second header field.
- An error enum contains only variants its operation can produce. Single
  checks and batches have separate error types, `Limiter::CheckError` and
  `Limiter::CheckAllError`, so a single check's error never carries a
  batch-only variant such as a duplicate key. A defensive variant for a state
  the code cannot reach is deleted, not documented.
- There is one way to reach each datum and one way to derive each value.
  `KeyHasher::hash_for` is the only subject-key derivation and returns a
  `PolicySubject` that retains the exact policy reference. `Check::new`
  accepts only that bound value, so the normal derivation-to-check path has no
  second policy argument. Operations that expose an unbound `SubjectKey` are
  explicit escape hatches and must say that the key can then be rebound; an
  already-opaque `SubjectKey` must be bound explicitly before it can become a
  check. An Axum extractor returns only an opaque `SubjectKey`, and the layer
  alone binds that key to the configured policy it evaluates and records.
  `Denial` is one enum, not an opaque value plus a view. Constructors are named
  `new`; every `with_*` method is a builder that takes `self`.
- `permits_request()` is the only boolean admission predicate on `Decision`
  and `BatchDecision`. Do not add sibling predicates such as `would_deny()`:
  a predicate that is true for shadow denials compiles cleanly as a rejection
  guard and silently turns shadow mode into enforcement. The same rule covers
  fallible conversions: `BatchDecision` has no `try_into_allowed()`, whose
  `is_ok()` was an `is_allowed()` predicate that is false for a shadow denial,
  and no `try_into_single_decision()`, because every backend shapes a single
  decision directly. `Decision::admit()` is the typed split into `Admitted` or
  `Denial`; everything else goes through `view()`. Types that can only hold a
  subset of outcomes use the narrower type: an allowed batch view is
  `&[Allowance]`, an adapter rejection carries `Denial`, an admitted request
  carries `Admitted`, and the HTTP service-limit encoder accepts only
  `QuotaState`, so a storage-capacity denial cannot reach it.
- HTTP adapters must record every stacked layer's admitted decision. A request
  that passes several `RateLimitLayer`s carries one `Admissions` extension
  with an `Admission` per layer in evaluation order; a layer must never
  overwrite another layer's entry.
- Multi-check operations are all-or-nothing and preserve the caller's input
  order in returned decisions. A batch rejects mixed quota modes because a
  shadow denial consumes nothing: a shadow policy exhausted inside an
  otherwise enforced batch would either consume the enforced members or stop
  them counting. Shadow one policy of a multi-policy batch by checking it
  separately; document this trap wherever shadow mode is introduced.
- `Limiter::check` and `Limiter::check_all` futures do no work before their
  first poll. The type system cannot express this, so every backend keeps a
  regression test that drops an unpolled future and proves nothing was
  evaluated or consumed.
- Retry and replenishment delays are measured from the backend's authoritative
  evaluation time and round up when converted to whole-second headers.
  PostgreSQL measures elapsed evaluation time with its database clock and may
  conservatively overstate the delay at the caller by commit and transport
  latency.
- PostgreSQL builds every decision from the database response before the
  transaction is finalized. A malformed response is a pre-commit
  `StorageInvariant` failure, never a decision that may already have consumed
  quota, and a single check is shaped directly rather than converted from a
  batch of one.
- Raw subjects must not enter storage, logs, or error messages. Applications
  should derive subject keys with a secret of at least 32 bytes.
  `SubjectKey::from_digest` exists for tests and already-opaque digests only.
- PostgreSQL 0.1 storage is not hard-cardinality-bounded. Deploy it behind a
  bounded local gate, schedule expired-row cleanup, and monitor table growth.
- PostgreSQL 0.2 storage is hard-bounded per persistent capacity shard. Keep
  the shard derivation and database ceiling stable, schedule expired-row
  cleanup to reclaim slots, and monitor shard skew and table growth.


## Common commands

`python3 scripts/check_runlimit_consumer.py` runs native and facade consumers
from a Git-free source copy with the selected compiler and no consumer patches.
Run from the repository root:

```sh
bash scripts/verify.sh
python3 scripts/test_matrix.py runlimit
python3 scripts/check_runlimit_workspace.py
RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql://... cargo test -p runlimit-postgres --tests --all-features --locked -- --ignored --test-threads=1
```
Local verification uses the root pinned toolchain; exact Rust 1.94.0 verification
belongs in CI. Use only disposable PostgreSQL fixtures for live tests. CI retains PostgreSQL 16;
record actual versions and executed platforms. No database is needed to compile.
See [import provenance](IMPORT.md); use the root tracker, not upstream administration.
