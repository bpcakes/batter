# Runlimit

Runlimit is a framework-neutral Rust library for keyed rate limiting. It
provides anchored fixed windows in memory or across PostgreSQL-backed replicas,
plus hard-bounded memory and PostgreSQL GCRA backends for continuously replenished
quotas.

The 0.x API is intentionally pre-stable. Its application boundary is exercised
by authentication throttling consumers.

## Crates

| Crate | Responsibility |
| --- | --- |
| `runlimit-axum` | Caller-controlled Axum/Tower admission middleware. |
| `runlimit-core` | Validated policies, HMAC-derived subject keys, checks, and structured decisions. |
| `runlimit-http` | Framework-neutral IETF draft RateLimit response-field encoding. |
| `runlimit-memory` | Sharded, hard-bounded process-local storage with bounded cleanup work. |
| `runlimit-postgres` | Replica-safe SQLx/PostgreSQL storage, migrations, and bounded maintenance. |

These five native packages are members of Batter's root workspace, imported from
master `12e035dac504a1d348c2058ee7ade8e61f2e7974`. The source versions remain
coordinated at 0.4.0 and their manifests restrict publication to crates.io.
Use one Batter revision for direct native packages and the optional facade.
See [provenance](IMPORT.md) and the [workspace decision](../docs/adr/012-native-runlimit-workspace.md).

```toml
[dependencies]
runlimit-core = { path = "../batter/runlimit/runlimit-core" }
runlimit-memory = { path = "../batter/runlimit/runlimit-memory" }
# Optional backends/adapters:
# runlimit-postgres = { path = "../batter/runlimit/runlimit-postgres" }
# runlimit-http = { path = "../batter/runlimit/runlimit-http" }
# runlimit-axum = { path = "../batter/runlimit/runlimit-axum" }
```

## Memory backend example

The following admission checks a client and a normalized identity atomically.
If either quota is unavailable, neither counter is consumed.

```rust
use std::{env, error::Error, time::Duration};

use runlimit_core::{
    BatchDecisionView, Check, Denial, FixedWindowPolicy, KeyHasher, PolicyId, ScopeId,
};
use runlimit_memory::{MemoryStore, MemoryStoreConfig};

fn main() -> Result<(), Box<dyn Error>> {
    let client_policy = FixedWindowPolicy::new(
        PolicyId::new("auth.login")?,
        ScopeId::new("client")?,
        40,
        Duration::from_secs(60),
    )?;
    let identity_policy = FixedWindowPolicy::new(
        PolicyId::new("auth.login")?,
        ScopeId::new("identity")?,
        8,
        Duration::from_secs(60),
    )?;

    // Load one stable, random secret (at least 32 bytes) from secret storage.
    let secret = env::var("RUNLIMIT_KEY_SECRET")?;
    let key_hasher = KeyHasher::new(secret.as_bytes())?;

    // Address extraction and subject normalization remain application-owned.
    let client = key_hasher.hash_for(&client_policy, b"client-network:192.0.2.4");
    let identity = key_hasher.hash_for(&identity_policy, b"user@example.test");

    let config = MemoryStoreConfig::new(50_000)?.with_shard_count(64)?;
    let limiter = MemoryStore::new(config);
    let checks = [Check::new(client), Check::new(identity)];

    let decision = limiter.check_all(&checks)?;
    match decision.view() {
        BatchDecisionView::Allowed { allowances } => {
            assert_eq!(allowances.len(), checks.len());
            println!("request admitted");
        }
        BatchDecisionView::Denied {
            index,
            denial: Denial::QuotaExceeded(quota),
            ..
        } => {
            let seconds = quota.retry_after().seconds();
            println!("check {index} denied; retry after {seconds} seconds");
        }
        BatchDecisionView::Denied {
            index,
            denial: Denial::StorageCapacity { .. },
            ..
        } => println!("check {index} denied; backend storage is full"),
        BatchDecisionView::ShadowDenied { index, .. } => {
            println!("request admitted after check {index} was shadow denied");
        }
    }

    Ok(())
}
```

Run `python3 scripts/check_runlimit_consumer.py` from the repository root to execute
the original `smoke/native_consumer.rs` and the facade quota consumer from a
Git-free source copy. The standalone consumer needs no dependency patches or
database; it uses a public fixture key. It preserves the selected toolchain and
locked external versions. Prefix with `RUSTUP_TOOLCHAIN=1.94.0` to check the minimum
compiler. Root verification also runs this check.

## Decision model

`Decision::view()` returns an exhaustive `DecisionView`. An enforced denial is
an exhaustive `Denial` enum naming its reason, so every outcome and every
reason is a named match arm and none of them hides behind an `Option`:

```rust
use runlimit_core::{Decision, DecisionView, Denial};

fn describe(decision: &Decision) -> String {
    match decision.view() {
        DecisionView::Allowed { allowance } => format!(
            "admitted; {} of {} left",
            allowance.available(),
            allowance.capacity(),
        ),
        DecisionView::ShadowDenied { denial } => format!(
            "admitted; quota of {} would have denied for {}s",
            denial.capacity(),
            denial.retry_after().seconds(),
        ),
        DecisionView::Denied {
            denial: Denial::QuotaExceeded(quota),
        } => format!("rejected; retry after {}s", quota.retry_after().seconds()),
        DecisionView::Denied {
            denial: Denial::StorageCapacity { retry_after },
        } => match retry_after {
            Some(retry_after) => format!("rejected; backend full for {}s", retry_after.seconds()),
            None => "rejected; backend full".to_owned(),
        },
    }
}
```

This example is exercised in `runlimit-core/tests/readme_decision_model.rs`.

Every outcome carries a validated value type. Quotas and capacities are
`Capacity` values and periods are `QuotaPeriod` values, validated once when a
policy is built and never re-checked downstream; `RateLimitPolicy` returns
them, so a third-party policy cannot report a zero or oversized number.
`QuotaDenial::new` takes a `Capacity` and cannot fail. `Allowance::new` checks
the one remaining relation, that `available` does not exceed the capacity, and
returns a `Result`; there is no panicking spelling. Only a validated
`QuotaDenial` can be shadowed, and an allowed batch is a nonempty
`Vec<Allowance>`, so a denied member is not representable. Serialization does
not perform further metadata validation.

Backend-measured durations are `Delay` values: a quota denial's `retry_after`
and an allowance's `replenishes_after` both feed the same whole-second header
fields, so both carry the same rounding rule. `Delay::seconds()` rounds up to
the whole seconds an HTTP `Retry-After` or `RateLimit` field needs, and
`Delay::duration()` keeps the exact backend measurement. A storage-capacity
denial reports a delay only when the backend knows its earliest expiry.

`permits_request()` is the only boolean admission predicate: it is true for
allowed and shadow-denied decisions and false for every enforced denial. When
the outcome itself is needed, `Decision::admit()` splits a decision into the
`Admitted` value a handler may proceed with or the `Denial` it must enforce;
`Admitted` cannot hold an enforced denial, so code that receives one never
re-checks enforcement. Everything else, including telemetry that distinguishes
shadow denials from allowances, matches `view()`. There are no optional
accessors that answer for every outcome at once.

## Generic backend API

Async application adapters can be generic over `runlimit_core::Limiter` and
swap storage backends without depending on an async-runtime abstraction:

```rust
use runlimit_core::{BatchDecision, Check, Limiter};

async fn admit<L: Limiter>(
    limiter: &L,
    checks: &[Check<'_, L::Policy>],
) -> Result<BatchDecision, L::CheckAllError> {
    limiter.check_all(checks).await
}
```

`Limiter` uses static dispatch with no required future boxing and returns
`Send` futures. It is intentionally not object-safe; use a generic parameter
for test-time backend substitution, or implement `Limiter` on an
application-owned enum for runtime selection. Single checks and batches have
separate error types, `L::CheckError` and `L::CheckAllError`, so a single
check's error never carries a variant that only a batch can produce.

The inherent `MemoryStore::check` and `MemoryStore::check_all` APIs remain
synchronous. In generic code the trait methods are selected automatically. To
request the async trait method from a concrete memory store, use a fully
qualified call such as `Limiter::check(&store, &check).await`.

Trait calls evaluate and consume quota only when their returned future is
first polled. Creating and dropping an unpolled future leaves quota unchanged.

## Axum admission middleware

`runlimit-axum` provides `RateLimitLayer` for checks that must run before an
Axum handler or request-body extractor. The application supplies a synchronous
key extractor and a rejection mapper. The key extractor receives the request
and policy, while the mapper owns the status, body, and headers for missing
trusted metadata, enforced denials, and backend failures.

The adapter does not interpret `Forwarded`, `X-Forwarded-For`, `ConnectInfo`,
cookies, or application identities. Establish trust and normalize identities
at the application boundary. A closure extractor returns an already-opaque
`SubjectKey`, which the adapter binds to its configured policy. A named
`ExtractSubjectKey` implementation that calls `KeyHasher::hash_for` explicitly
converts the result with `PolicySubject::into_unbound_subject_key`; the adapter
then binds that key to its configured policy. The extractor can choose the
opaque subject identity, but its result type cannot replace the policy the
layer evaluates and records.

The rejection mapper receives an exhaustive `RateLimitRejection`: a key
extraction error, an enforced `Denial`, or a backend error. Allowed and
shadow-denied outcomes never reach it. Enforced quota and capacity denials
short-circuit before the inner service is called or the request body is
consumed.

Admitted requests carry an `Admissions` request extension. Every layer the
request passed through appends an `Admission` naming its policy and the
`Admitted` outcome, so stacking a client gate and an identity gate keeps both
decisions. Read it with `Extension<Admissions>` and look up a layer's outcome
with `admissions.get(&policy)`.

## HTTP response metadata

`runlimit-http` encodes caller-selected policy and decision metadata using the
versioned `draft_11` module. The active Internet-Draft uses the Structured
Field values `RateLimit-Policy: "name";q=N;w=S` and
`RateLimit: "name";r=N;t=S`.

The helpers return typed HTTP header names and values. They do not select a
response status or body, emit `Retry-After`, expose partition keys, or decide
which policies an application should disclose. Policy periods advertised in
`RateLimit-Policy` must be exact whole seconds; dynamic service durations are
rounded up. Encoding also rejects quota values outside RFC 9651's Structured
Field integer range.

`draft_11::service_limit` accepts only the states a `RateLimit` field can
describe: an `Allowance`, a `QuotaDenial`, or an `Admitted` outcome, each of
which converts into `draft_11::QuotaState`. A handler passes
`admission.decision()` straight through; a shadow denial then exposes the
service value that enforcement would have applied. A rejection mapper matches
its `Denial` and passes the `QuotaDenial` from the `QuotaExceeded` arm. A
storage-capacity denial has no quota service metadata and is not accepted, so
it cannot become an encoding error in the response path.

## Optional Serde support

The core and storage backend crates have an opt-in `serde` feature. Enabling it
on a backend also enables it for `runlimit-core`:

```toml
[dependencies]
runlimit-memory = { version = "0.4.0", features = ["serde"] }
```

The feature serializes validated policy and scope identifiers as strings;
fixed-window and GCRA policies without their derived fingerprints; quota mode;
`Allowance`; tagged `Decision`/`Denial`/`BatchDecision` values; and backend
configuration values. `MemoryStoreStats` is also serializable for telemetry.
Durations retain their exact seconds and nanoseconds. Deserialization rejects
unknown fields and values that violate Runlimit's constructors or decision
invariants.

`SubjectKey`, `CounterKey`, `PolicyFingerprint`, `KeyHasher`, and live backend
instances deliberately do not implement Serde traits. Keep opaque storage keys
and hashing secrets out of generic configuration and telemetry paths.

## Fixed-window semantics

Runlimit implements **anchored** fixed windows. The first allowed check for a
storage key starts its window; subsequent checks use that anchor until the
entire duration elapses. Windows are not aligned to wall-clock boundaries such
as calendar minutes.

- Denied checks do not consume quota.
- A batch is all-or-nothing and allowed decisions retain input order.
- An empty batch is rejected with `BatchError::EmptyBatch` rather than
  vacuously allowed, so a caller that filtered every check out fails closed
  instead of admitting the request without evaluating any policy.
- Every check in a batch uses the same quota mode. A mixed enforced/shadow
  batch is rejected before backend work begins; see the shadow-mode section
  for the rollout consequence.
- A denied batch names the failing input index and the batch size. The index
  is validated below the size at construction and on deserialization.
- Duplicate storage keys in a batch are rejected as caller errors.
- The policy identifier, scope, limit, and window are fingerprinted into the
  storage key. Changing any of them starts an independent counter instead of
  reinterpreting existing state.
- Core accepts limits through `runlimit_core::MAX_LIMIT` and exact
  whole-millisecond windows through `runlimit_core::MAX_WINDOW`. These shared
  portable bounds ensure every core-valid fixed-window policy is representable
  by both fixed-window storage backends.
- Decisions expose durations measured by the backend. Memory durations are
  exact at evaluation time. PostgreSQL measures elapsed evaluation time with
  its authoritative database clock and can conservatively overstate the
  remaining time at the caller by commit and transport latency. Use
  `Delay::seconds()` for an HTTP `Retry-After` value rounded up to the next
  whole second.

The memory and PostgreSQL backends intentionally implement these same
semantics.

## GCRA semantics

`GcraPolicy` and `GcraStore` provide a continuously replenished quota without
fixed-window boundary bursts:

<!-- runlimit-readme-gcra:start -->
```rust
use std::{env, time::Duration};

use runlimit_core::{Check, GcraPolicy, KeyHasher, PolicyId, ScopeId};
use runlimit_memory::{GcraStore, MemoryStoreConfig};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let policy = GcraPolicy::new(
    PolicyId::new("api.read")?,
    ScopeId::new("client")?,
    10,                       // units replenished
    Duration::from_secs(1),   // during this period
    20,                       // maximum immediate burst
)?;
let key_hasher = KeyHasher::new(env::var("RUNLIMIT_KEY_SECRET")?.as_bytes())?;
let limiter = GcraStore::new(MemoryStoreConfig::new(50_000)?);
let decision = limiter.check(&Check::new(
    key_hasher.hash_for(&policy, b"client-network:192.0.2.4"),
))?;
assert!(decision.permits_request());
# Ok(())
# }
```
<!-- runlimit-readme-gcra:end -->

The backend uses exact scaled-integer arithmetic rather than a floating-point
token count. Policy periods must be exact whole milliseconds; reported retry
and full-replenishment durations round up to the next whole millisecond.
`quota`, `period`, and `burst_capacity` are all part of the policy fingerprint.
Each key uses constant-size state, and it becomes eligible for bounded cleanup
once its full burst capacity has replenished.

`GcraStore` is process-local. For shared continuously replenished quotas, use
`runlimit_postgres::PostgresGcraLimiter`; `PostgresLimiter` keeps its existing
fixed-window contract.

### Durable GCRA

`PostgresGcraLimiter::new(pool)` implements `Limiter<Policy = GcraPolicy>` and
accepts the same policy-bound checks as `GcraStore`. Call its `migrate()` before
serving traffic and schedule `cleanup_expired(maximum_rows)` to reclaim expired
storage. Its separate `gcra-migrations` stream installs only GCRA tables;
existing fixed-window consumers need no schema or workflow change. Strict host
migrators may vendor `CREATE_RUNLIMIT_GCRA_SQL` followed by
`INDEX_RUNLIMIT_GCRA_SHARD_EXPIRY_SQL` instead. The canonical `migrate()` applies
both in order, including when upgrading a database with the first migration
already installed. Shared SQLx migration
histories require every participating migrator to ignore unrelated versions.

The memory and PostgreSQL GCRA implementations share one pure, exact evaluator
in `runlimit_core::gcra`. PostgreSQL supplies whole-millisecond time and retains
a committed time watermark per shard, preventing backward clock adjustments
from replenishing quota early. Delays are measured at evaluation time and may
conservatively include later database/transport latency. Allowed batches commit
every member together, preserve input order, and consume nothing on enforced or
shadow denial. The existing `CheckError` and `BatchCheckError` distinguish
definite non-consumption from uncertain commit; uncertain operations are never
automatically replayed. Cancellation while committing is likewise uncertain.

The independent GCRA v1 storage protocol has 256 shards, a database-enforced
ceiling of 65,536 rows per shard, and configurable lower operational bounds via
`PostgresConfig`. Admission locks affected ledger rows in ascending shard order
before reading or writing counters, including absent keys. This deliberately
serializes keys in the same shard. Cleanup follows the same order, skips busy
shards, and deletes at most its requested bound. Discovery makes at most 256
shard/expiry index probes under each shard's clamped clock. Deletion reads at
most the requested number of eligible entries per locked shard before selecting
the globally oldest requested rows; unrelated active entries are outside these
index ranges. Active rows are never evicted;
expired rows still occupy a slot until cleanup, but remain reusable for their
existing key. The shard derivation and locking order are persistent protocol
and cannot change in place during rolling deploys.

Run the opt-in PostgreSQL GCRA suite with:

```sh
RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql://... \
  cargo test -p runlimit-postgres --test gcra -- --ignored --test-threads=1
```

## Shadow mode

Use `with_quota_mode(QuotaMode::Shadow)` to warm and observe a policy before
enforcing it. Quota exhaustion then returns a shadow-denied decision:
`permits_request()` is true, `admit()` returns an `Admitted` value, and
`view()` reports `DecisionView::ShadowDenied` with the quota details that
enforcement would have applied. A shadow denial does not consume quota.
Storage-capacity denials and backend errors always remain fail-closed.

Quota mode is deliberately excluded from the policy fingerprint, so switching
a warmed policy to enforcement keeps its counter state.

Atomic batches reject mixed enforcement and shadow modes with
`BatchError::MixedQuotaModes`, and a denied or shadow-denied batch consumes
nothing. This has an operational consequence for the most natural rollout
step, shadowing one new policy inside an existing multi-policy batch: that
batch fails closed until every policy in it shares a mode. The rule exists
because a batch is all-or-nothing. If a shadow policy could be exhausted inside
an enforced batch, the shadow denial would have to either consume the enforced
members, breaking "shadow denials consume nothing", or consume none of them,
silently stopping the enforced policies from counting whenever the shadow one
is exhausted. To roll out a new policy in shadow mode next to enforced ones,
check it separately with its own `check()` call, or shadow every policy in the
batch together and switch them to enforcement together.

## Operational observations

`MemoryStore`, `GcraStore`, and `PostgresLimiter` accept an optional
`runlimit_core::Observer`. Admission observations classify outcome, quota
consumption certainty, and elapsed time; cleanup observations report bounded
work. `Observation`, `AdmissionOperation`, `AdmissionOutcome`,
`ConsumptionStatus`, and `CleanupOutcome` are exhaustive enums, so an observer
that maps outcomes to metrics names every variant and a new outcome is a
compile error rather than a silently unmetered wildcard arm. The memory stores
also report per-shard capacity headroom. Observations intentionally omit
subject keys, backend error text, and other sensitive high-cardinality values.

`AdmissionOperation::Check` always carries an `AdmissionPolicy` with the
policy identifier, scope, and fingerprint together; `AdmissionOperation::Batch`
carries the batch size and the policy of the one relevant check when a batch
singles one out. A cleanup observation's `CleanupOutcome` says whether the
pass confirmed a removal count, definitely had no effect, or failed with an
unknown effect; it never borrows the quota-consumption vocabulary.

Callbacks run synchronously after memory locks are released or database
transactions are finalized. Keep them fast and hand expensive export work to
another thread. Runlimit catches observer panics so telemetry cannot change an
admission result. PostgreSQL observations preserve the distinction between
definite non-consumption and a commit result that may already have consumed
quota.

## Subject keys and secret rotation

Applications normalize subjects before passing them to Runlimit. Construct
stored keys with `KeyHasher::hash_for`, which uses HMAC-SHA-256 domain-separated
by the policy the key will be checked against and requires a secret of at
least 32 bytes. It is the only derivation method and returns a `PolicySubject`
that retains the exact policy reference. `Check::new` accepts only that bound
value, so the normal derivation-to-check path has no second policy argument
that can disagree with the derivation namespace. Adapter and backend APIs can
deliberately expose an unbound key: `PolicySubject::into_unbound_subject_key`,
`Check::subject`, and `CounterKey::subject` all return a `SubjectKey` that can
then be explicitly rebound. Those are escape hatches, not part of the normal
construction path. `SubjectKey::from_digest` bypasses derivation entirely and
exists only for tests and for input that is already an opaque, secret-keyed
32-byte digest; bind it explicitly with `SubjectKey::bind` before constructing
a check, and never feed it raw or padded identities.
Raw emails, account IDs, session IDs, and IP addresses should not enter
storage, logs, metrics labels, or errors.
`KeyHasher` precomputes key-equivalent HMAC state instead of retaining the raw
secret or rebuilding the key schedule for every subject. Treat a live hasher
and its clones as secret material; their debug output is redacted and their
underlying SHA-256 state and buffered input are wiped when dropped.

Use one stable secret across every replica sharing a backend. Rotating the
secret starts fresh counters because every derived subject key changes. Deploy
a rotation to all replicas together: replicas using old and new secrets at the
same time can each admit traffic against different counters.

## Memory backend behavior

`MemoryStore` and `GcraStore` are process-local and hard bounded:

- The store defaults to one shard so all of `max_keys` is available regardless
  of key distribution.
- Configuring multiple shards divides `max_keys` into fixed per-shard
  capacities. This increases lock concurrency, but an uneven key distribution
  can fill one shard and deny a new key while other shards still have unused
  capacity.
- Active entries are never evicted to make room for new subjects.
- Each check removes at most the configured number of expired entries from its
  shard. An atomic batch receives that allowance once for each check on the
  shard targeted by that check. A fixed-window entry expires at its anchored
  deadline; a GCRA entry expires when its full capacity has replenished.
- An atomic batch targeting more distinct keys at one shard than that shard can
  ever hold returns
  `MemoryBatchError::BatchExceedsShardCapacity { shard_index, key_count,
  capacity }`. This is a structural error: waiting for expiry cannot make the
  batch fit.
- A new key that cannot fit in its shard is denied with
  `Denial::StorageCapacity`, optionally including the earliest known retry
  delay when the same operation may fit after existing entries expire.

If application code panics while holding a shard lock, that shard remains
poisoned. Every later operation touching it returns `PoisonedShardError`
without resetting its counters. With the default one-shard configuration, this
makes the entire store unavailable.
Keep admissions failed closed and alert operators. As an explicit availability
tradeoff, `recover_poisoned()` on either store atomically empties only poisoned
shards, clears their poison flags, preserves healthy-shard counters, and
returns the number recovered. Resetting those counters can admit requests that
their lost state would have denied; replacing the store is a broader reset
with the same security consequence for every shard.

Treat every error from a store as an admission failure; they are distinct
from a normal quota denial returned in a `Decision` or `BatchDecision`. Each
operation has an error type that lists only the failures it can produce: a
single `MemoryStore` check fails only with `PoisonedShardError`, a batch with
`MemoryBatchError`, a single `GcraStore` check with `GcraCheckError`, and a
GCRA batch with `GcraBatchError`, whose `Store` variant wraps the
bounded-storage failures shared with `MemoryStore`.

Both stores use `new(config)` with the system clock. Deterministic tests choose
their clock before the store exists with
`MemoryStore::builder(config).with_clock(clock).build()` or the corresponding
`GcraStore` builder. A built store cannot replace its clock because its stored
timestamps belong to that clock's coordinate system. `with_observer` remains a
store builder because attaching telemetry does not reinterpret quota state.

## Outcome-aware authentication attempts

`runlimit_core::attempts::AttemptPolicy` is separate from ordinary request quota
policies. It admits at most one active attempt per opaque policy-bound subject.
A failure or explicit abandonment increments consecutive failures and doubles
the retry delay up to the configured cap. Full success resets retry state.
Inactive failure state expires after the configured quiet period; application
audit history is independent and is never deleted by Runlimit. Quiet expiry
cannot erase an active lease, even when the lease exceeds the quiet period.

Derive subjects with `KeyHasher::hash_attempt_for(&policy, normalized_subject)`.
`MemoryAttemptLimiter` and `PostgresAttemptLimiter` return an opaque, consuming
receipt on admission. Dropping it does not refund the attempt: its admission
lease expires as a failure, evaluated at the lease deadline. A late success
cannot reset a newer reservation. PostgreSQL samples its authoritative clock
after locks; process-local storage clamps backwards clock readings. PostgreSQL
uses server wall time, so a server clock correction can extend or shorten
waiting periods; deploy with disciplined database time.

The policy validates nonzero whole-millisecond initial delay, delay cap, quiet
period, and admission lease. The cap must be at least the initial delay, and
quiet expiry must be at least the cap. Failure counts saturate at `u32::MAX`;
delay arithmetic saturates at the configured cap. Every parameter participates
in the storage fingerprint. These are application-selected security parameters,
not defaults chosen by the library.

`runlimit_postgres::attempts::PostgresAttemptLimiter` installs only its own
`ATTEMPTS_MIGRATOR` / `CREATE_RUNLIMIT_ATTEMPTS_SQL`. Existing fixed-window
consumers need no new tables. PostgreSQL enforces 256 shards with at most 65,536
slots each using a checked slot range and unique `(capacity_shard, capacity_slot)`.
`PostgresConfig` may lower the operational bound (4,096 slots by default).
Admission reclaims at most 16 expired rows in its shard, never live rows. Memory
storage has an explicit total capacity and similarly bounded cleanup. Full
storage fails closed; quotas and failure state are never evicted to admit a key.

Standalone `complete` returns `AttemptCompletionResult` only after acknowledged
commit. `CheckError` preserves uncertain-commit classification; never replay an
uncertain admission or completion automatically. Dropping a pending future gives
the caller no acknowledgement and can race with commit. Existing ordinary quota
admission remains non-refundable.

For session/audit atomicity, an application transaction owner such as Batter
uses the explicitly low-level PostgreSQL seam:

1. `claim_in(executor, receipt)` locks and validates the live reservation.
2. Run authoritative application checks and writes in that transaction.
3. `finish_in(executor, claim, final_outcome)` stages success or failure.
4. Commit, then publish the result. Roll back on `Stale`, errors, or cancellation.

A claim is fenced to the exact PostgreSQL transaction ID as well as the opaque
receipt token, which is rotated transactionally during claim. Rolling back a
containing savepoint reverts the token and invalidates an escaped claim even
though the outer transaction ID remains unchanged. Releasing a savepoint retains
the claim and its row lock. After a valid claim, time elapsed during application work does
not expire its held row lock. Using the claim in a different transaction returns
`Stale`. No transaction remains open during expensive credential verification.
The low-level API cannot own commit acknowledgement; its distinct
`StagedAttemptCompletion` is provisional. A direct `complete_in` operation is
also available for owners whose outcome is already final. An autocommit executor
must not be used for application atomicity.

Attempt observations use their own enum and panic-isolated observer. Standalone
owners report admitted, denied, completed, stale, and uncertain-commit outcomes;
low-level staging deliberately emits no committed event. The host transaction
owner is responsible for reporting its final disposition. Observations never
include raw subjects or lease tokens.

## PostgreSQL backend

Use `runlimit-postgres` when a quota must be shared across replicas or survive
process restarts. `PostgresLimiter` uses PostgreSQL time as its authority,
locks batch keys in deterministic order, and commits an allowed batch in one
transaction. Atomic batches use set-based lock, preflight, and update phases,
so the number of SQL phases stays fixed as the bounded batch size grows.
Authoritative time calls are explicitly resolved as
`pg_catalog.clock_timestamp()`, so a caller-controlled `search_path` cannot
substitute another clock function.

`PostgresConfig::pool_acquire_timeout` bounds waiting for a pooled connection.
After acquisition, the check or cleanup receives a fresh
`operation_timeout` budget for transaction begin, statements, lock waits,
rollback, and commit. If an application also imposes an outer timeout, allow
for both configured budgets plus scheduling overhead; cancelling around commit
can leave its outcome unknown.

Runlimit's `pg_advisory_xact_lock(bigint)` protocol uses a database-wide
namespace shared by every application and role connected to that database. An
unrelated session holding the same numeric lock can delay an affected counter
until its operation deadline. Use a trusted or dedicated database boundary, or
ensure unrelated roles cannot execute the advisory-lock functions.

Version 0.2 hard-bounds PostgreSQL cardinality with 256 persistent capacity
shards. `PostgresConfig::maximum_rows_per_shard` defaults to 4,096 and can be
lowered or raised through the database-enforced maximum of 65,536; apply it
with `PostgresLimiter::new(pool).with_config(config)`. Admission locks the
affected ledger shards and reserves all missing batch keys in the same
transaction as quota consumption. A full shard denies new keys with
`Denial::StorageCapacity`; existing keys remain usable and active rows are
never evicted. The migration's trigger-maintained ledger also caps inserts
from older replicas at 65,536 rows per shard during a rolling deployment.

The bound is per shard, so skew can deny a new key while other shards retain
headroom. Expiry does not itself release a ledger slot:
`cleanup_expired(maximum_rows)` must delete the row before its capacity becomes
reusable. Keep scheduling bounded cleanup and monitor shard headroom, table and
index size, cleanup throughput, and storage-capacity denials. A coarse
hard-bounded memory gate is still recommended before body parsing and
PostgreSQL acquisition to protect application and pool capacity.

Before serving traffic, apply the bundled migrations with
`PostgresLimiter::migrate()`. In a database where application and library
migrations share SQLx's `_sqlx_migrations` table, every participating migrator
must enable `Migrator::set_ignore_missing(true)` so each one tolerates versions
owned by the others. The exported raw `MIGRATOR` keeps SQLx's strict default
and is suitable only when Runlimit exclusively owns that migration history and
connection. An application that must retain a strict host migrator should
vendor the bundled Runlimit SQL as application-owned migrations and not run
either Runlimit migrator against the shared history. The exact bundled
statements are available as `CREATE_RUNLIMIT_FIXED_WINDOWS_SQL`,
`SET_RUNLIMIT_FIXED_WINDOWS_FILLFACTOR_SQL`, and
`BOUND_RUNLIMIT_FIXED_WINDOW_CARDINALITY_SQL` so hosts can vendor them without
reaching into crate source files.

Periodically call `cleanup_expired(maximum_rows)` to bound maintenance work;
expired rows do not affect correctness before cleanup. The cleanup query
materializes one PostgreSQL clock sample so the expiry predicate remains an
index range condition; when no rows are expired it does not filter a full scan
of active windows.

The counter table is update-heavy. Each admission after a counter row's initial
insert updates that row and leaves a dead heap tuple. In-window increments of
`used` can use PostgreSQL HOT updates, but starting a new anchored window also
changes the indexed `window_expires_at` value, so window renewals cannot be HOT
and also leave dead index entries. Deleting expired rows adds more dead tuples.
One bundled additive migration sets the table `fillfactor` to 80, reserving
page space for HOT counter updates at the cost of a larger live heap; it does
not remove the need for autovacuum. It remains separate from 0.1.0's
create-table migration so existing SQLx histories retain the published
checksum. The following additive migration installs the capacity ledger,
generated shard column, and statement-level maintenance triggers without
changing that published migration.

Tune autovacuum for churn rather than accepting its table-wide defaults
unchanged. The following is a reasonable starting point, not a capacity
guarantee:

```sql
ALTER TABLE runlimit_fixed_windows SET (
    autovacuum_vacuum_threshold = 500,
    autovacuum_vacuum_scale_factor = 0.01,
    autovacuum_analyze_threshold = 500,
    autovacuum_analyze_scale_factor = 0.02
);
```

PostgreSQL schedules vacuum after approximately the threshold plus the scale
factor times the estimated live rows. Lower the values for a large or
high-throughput counter table, and ensure the cluster's autovacuum workers and
cost limits can keep up. Monitor the live/dead tuple estimates, HOT-update
ratio, vacuum cadence, and heap/index bytes together:

```sql
SELECT
    n_live_tup,
    n_dead_tup,
    n_tup_upd,
    n_tup_hot_upd,
    last_autovacuum,
    autovacuum_count,
    pg_size_pretty(pg_relation_size(relid)) AS heap_size,
    pg_size_pretty(pg_indexes_size(relid)) AS index_size
FROM pg_stat_user_tables
WHERE relid = 'runlimit_fixed_windows'::regclass;
```

Alert when dead tuples or relation bytes keep rising across completed
autovacuums while live cardinality is stable. Check for long-running
transactions that prevent tuple removal before adding more vacuum capacity.
Ordinary vacuum makes dead space reusable but does not return relation files to
the operating system. The fillfactor migration likewise does not rewrite pages
created by 0.1.0; if an existing table or expiry index already needs repacking,
plan `REINDEX INDEX CONCURRENTLY` for a confirmed bloated index or an online
table repack. Use locking `VACUUM FULL` only in a separate maintenance window.

No running database is needed to compile Runlimit or its smoke consumer.
Database integration tests should use a disposable PostgreSQL instance.

Database errors must fail closed. `runlimit_postgres::CheckError` distinguishes
operations that definitely did not consume quota from commit outcomes that may
have consumed it, and `CheckError::consumption()` reports that certainty as a
`ConsumptionStatus`. Do not blindly retry an unknown commit as part of a
non-idempotent operation. A single check goes straight to the database and
returns `CheckError`; a batch returns `BatchCheckError`, which adds the
structural `InvalidBatch` failure that only a batch can produce. A timeout
names the `CheckPhase` it interrupted, and every phase precedes commit, so a
timeout never consumed quota. A decision is always built from the database
response before commit or rollback, so a malformed response is a pre-commit
failure rather than a decision that may already have consumed quota. When a
read-only denial has already been produced, a rollback failure does not replace
it with an error; Runlimit returns the denial and discards that connection.
`MaintenanceError` makes the corresponding distinction for cleanup; inspect
`may_have_removed_rows()` before deciding whether an unconfirmed cleanup needs
to be retried.

## Production topology

The memory backend enforces a per-process quota, not a fleet-wide quota. In a
multi-replica service, use PostgreSQL for authoritative shared identity,
account, or global limits, while retaining a coarse hard-bounded memory policy
as the first-line client/cardinality gate. Give the two gates distinct policy
identifiers: the local gate protects parsing and database capacity, while the
PostgreSQL policy enforces the shared quota.

All replicas sharing PostgreSQL must use the same policies, normalization rules,
and HMAC secret. Readiness checks should verify the dependencies required by
the chosen fail-closed path.

## Application-owned boundaries

Runlimit owns counter mechanics and storage. Applications continue to own:

- trusted-proxy configuration and client-address extraction;
- IPv4/IPv6 aggregation and identity normalization;
- route policy selection and HTTP/gRPC response formats;
- concurrency bulkheads such as Tokio semaphores;
- durable password lockouts, resend cooldowns, and business quotas.

In particular, Runlimit does not inspect forwarding headers. Only the
application knows which network peers are trusted to supply them. Concurrency
limits also remain separate because they bound simultaneous work rather than
work admitted during a time window.

## Development

Run from the Batter repository root. Rust 1.94 is the minimum; the default
workspace toolchain is 1.98.1. Root `scripts/verify.sh` includes all native tests,
default-feature tests/lint, strict all-feature lint, doctests, and the release-mode
memory corruption/fail-closed regression. Use the root Beads tracker and
[testing guide](../docs/testing.md#native-runlimit-workspace-verification).

```sh
cargo test -p runlimit-core -p runlimit-memory -p runlimit-postgres -p runlimit-http -p runlimit-axum --locked
python3 scripts/check_runlimit_workspace.py
RUNLIMIT_POSTGRES_TEST_DATABASE_URL=postgresql://... cargo test -p runlimit-postgres --tests --all-features --locked -- --ignored --test-threads=1
```

The explicit URL must point to a disposable database; the ignored suite owns
isolated schemas and exercises locks and migrations. CI preserves the upstream
PostgreSQL 16 fixture. Provisioning stays outside the library. Existing published
migrations and persistent lock/key protocols remain immutable across rolling
deployments; add forward migrations for schema changes.

Registry publication and release automation are outside this workspace migration.
Historical release tooling, the original backlog and development plan remain at
the [immutable upstream snapshot](https://github.com/bpcakes/runlimit/tree/12e035dac504a1d348c2058ee7ade8e61f2e7974).

## License

Licensed under either the Apache License, Version 2.0 or the MIT license, at
your option.
