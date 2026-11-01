# ADR-014: Reuse after two application consumers

## Status

Accepted, 2026-10-31. Owning Bead: `batter-hq3`. The repository owner approved
the assessment before this record was written. This record is a decision only.
It changes no public API. Its two accepted extractions are delivered under
their own Beads.

## Context

Speculative caches, batching, construction graphs and adapter wrappers increase
API surface before shared semantics are known. The
[integration contract](../integrations.md) therefore requires two different
application composition roots before an adapter is promoted. `batter-hq3` asks
which construction- or framework-level reuse, if any, that evidence justifies.
It also asks how stable adapter promotion differs from the focused experimental
extraction already delivered under `batter-7r3` and the `consumer-feedback`
Beads.

### Evidence

On 2026-10-31, three independently maintained downstream Tokio/Axum/SQLx/
PostgreSQL applications that pin Batter by Git revision were surveyed
read-only. As with the [design evidence](../design-evidence.md), their
identities, paths and commits stay outside this repository.

- **Consumer A** is pinned at `1e78d46` (2026-10-05). That is 165 commits and 10
  `Breaking` changelog entries behind `ae000c7`.
- **Consumer B** is pinned at `2e4aa19` (2026-10-17), 37 commits and 2
  `Breaking` entries behind.
- A and B are the consumers recorded on the Bead on 2026-09-27. Both compose
  serving, worker and finite-command processes through Batter startup,
  lifecycle, command, HTTP, SQLx and Runledger APIs.
- **Consumer C** is pinned at `ae000c7` and was not part of the earlier survey.
  It reaches only native Runledger and Runlimit, through the facade's native
  namespaces. It uses no Batter startup, lifecycle, command, periodic, HTTP or
  SQLx adapter path. It therefore counts as an adopter of the single-dependency
  native namespaces only, not of the abstractions assessed here.

Deployment is not established. `batter-qjxi` withdrew the earlier claim that A
and B were deployed. Two of the three carry deployment configuration, but that
is not evidence of operation. The threshold applied here is two actual
application composition roots, which A and B are. Facade examples and the
reference service are not application adopters.

The survey was source-only. No consumer was built, run or modified, and the
quantities below were measured at the inspected consumer commits, not in this
repository. Whether the consumers exercise partial startup and shutdown
failures was not re-inspected, and no fresh-agent evaluation was run.

## Decision

Batter adds no declarative application root, dependency-injection container,
construction graph, service-replacement mechanism, cache, batching or
deduplication layer, bounded work queue, circuit breaker, generic receipt store
or generic provider interface.

Two narrow shared cores are justified and become concrete delivery Beads: an
application-installed redacting panic hook (`batter-4q78`) and bounded blocking
work on `Bulkhead` (`batter-0772`).

Five further candidates stay on a watch list with explicit triggers. Stable
adapter promotion is decided per adapter against the integration contract, and
this record does not gate it.

### Rejected: a declarative application root

All three consumers follow one coarse sequence:

1. settings and budgets
2. protected startup with Unix signals
3. cleanup reservation, then pool construction
4. schema and authority verification
5. listener bind, components and HTTP registration
6. checked completion and exit mapping

The parts a root framework would have to own differ, and each difference is
application policy:

- **Startup diagnostics.** A projects 18 typed stages with reason codes in
  about 340 lines. B uses 42 distinct inline stage strings and has no stage
  type. C attaches error-context strings outside Batter startup.
- **Settings authority.** A reads a bounded file with an environment overlay
  and per-process allow-lists that refuse another process's keys. B reads
  per-feature environment closures. C reads about 49 environment variables
  directly.
- **Exit mapping.** All three map process outcomes to success or failure only.
  A's launcher uses distinct exit codes for its own pre-exec failures. There is
  no shared exit taxonomy to own.

Most repetition is inside a single consumer:

- B copies its startup sequence 3 times and its command sequence 7 times, and
  opens a database through the same reservation and verification steps at 17
  call sites.
- A reserves cleanup and then constructs a pool at 5 sites.

Those are local refactors. A framework that absorbed them would also have to
absorb both consumers' different stage and settings policies.

The cross-consumer parts are already focused tasks:

- `batter-od4b`: A and B each wrap `Command` with a local signal installer.
- `batter-m9rp`: neither uses `service::start`. A still delivers its bound
  address through a oneshot channel because its pin predates `batter-syah`;
  B only logs the address.

### Accepted: an application-installed redacting panic hook

All three consumers install a process panic hook.

- B (one helper, three binaries) and C (two binaries) log the formatted
  `PanicHookInfo`, whose Display includes the payload. They then call the
  default hook, which prints the payload to stderr again.
- A writes only the location, never delegates to the default hook and covers
  this with a subprocess test. Its records note that an earlier version leaked
  the payload.
- Installation timing has gaps of its own. A installs after configuration
  loading, some B modes return before installing, and one B operator
  binary installs none.

[Guarantees](../guarantees.md) and [operations](../operations.md) already state
that native panic-hook output carries no redaction promise, and they ask
applications to keep credentials out of panic messages. Two of three consumers
still emit payloads at panic time. Under [ADR-010](010-agent-only-consumption.md),
a repeated caller instruction that is not followed is a signal to change the
API.

The library still never installs a hook. A constructor returns a hook value
that the application installs at its root. It cannot prove that the hook was
installed, that it was installed early enough, or that the process is
sanitized. Those limits stay explicit in `batter-4q78`.

### Accepted: bounded blocking work

All three consumers hand-write the same helper: acquire a permit, move it into
a `spawn_blocking` closure so that capacity stays held until the closure
returns, then await the join.

- B does this over Batter's `Bulkhead`. It also has unbounded blocking sites
  whose comments say the work finishes unseen.
- C does this over a Tokio semaphore.
- A wraps it in a capacity-2 executor.

A permit released when the awaiting future is dropped would let cancelled
callers oversubscribe the blocking pool. The correct ordering is a local
ownership transfer, so the library can own it. `Bulkhead::enter` returns a
native permit with the instruction to hold it for the work's full lifetime, and
nothing on the canonical path performs that hold for blocking work.

`batter-0772` adds a library-owned blocking run that makes the early release
unrepresentable. It must not claim to cancel blocking work: an abandoned caller
discards the result while the closure runs to completion.

Delivered under `batter-0772` as `Bulkhead::run_blocking`, returning
`Result<T, BlockingError>`; see its
[ADR-010 assessment](010-agent-only-consumption.md#bounded-blocking-work-assessment-batter-0772).
Shutdown settlement stays on the watch list below with its trigger unchanged.

### Watch list

Each item below lacks a second consumer that needs it through Batter. A later
survey that meets an item's trigger files a concrete Bead citing this record.
These items are not backlog.

| Candidate | Current evidence | Trigger |
| --- | --- | --- |
| Finalizers that skip when a dependency's termination is unconfirmed | Only A has a real case: it skips its pool close unless an external subprocess cleanup confirmed termination, using a shared flag and an error from the finalizer. C skips its pool close after an unsettled Runledger report, which reimplements Batter's existing conservative skipping outside Batter. | A second root using Batter cleanup that needs a named dependency skip reason. |
| Shutdown settlement of outstanding blocking work | Only A registers its blocking executor as managed work so dependent cleanup waits for settlement. | A second consumer of the `batter-0772` path that needs it. |
| Transaction-local session settings inside atomic scopes | Only B scopes authority settings with transaction-local `set_config`. C uses transaction-local settings only for lock timeouts. | A second consumer scoping such settings through Batter atomic scopes. |
| A migration-ledger mode that tolerates newer versions during mixed-version deploys | C tolerates foreign and newer versions in its own native SQLx check, outside Batter verification. A and B select `SqlxLedgerMode::Exact` for serving. B's documented stopped-service upgrade comes from an authority cutover, not from the ledger mode. | Two Batter verification consumers that need mixed-version serving under an application-owned expand/contract discipline. |
| Output publication for finite commands | Only B runs stdout writers that a signal can abandon, in two copies. A prints results to stdout without a dedicated writer. | Remains a non-goal of `batter-od4b` until a second concrete need appears. |

### Rejected or not demonstrated

- **Operation receipts and ledgers.** All three keep many, each with its own
  schema, key, retention and domain meaning. A has several replay and receipt
  tables, B has 11 receipt tables, 4 operation tables and a delivery ledger,
  and C has about 15 tables. These are application persistence protocols, and
  a shared store would be the repository or outbox abstraction this repository
  excludes.
- **Provider adapters.** A and C use the same email delivery provider with
  stable idempotency keys, but different retry owners: A uses the in-process
  `retry` with deduplicated replay, C uses durable Runledger retries. Only C
  records the provider's key-retention window. B's providers use a dispatch
  fence, an unknown outcome and reconciliation instead of idempotency keys, and
  each provider defines this pattern separately. The failure contracts do not
  match.
  The 2026-09-15 adapter note on `batter-hq3` stays conditional: two roots must
  show the same retry owner, key-retention assumption and ambiguity handling,
  checked against the provider's primary documentation.
- **Caches, batching and deduplication.** A has a byte-budgeted cache with a
  per-key load gate, B has one token cache, and C has a readiness single-flight
  and a key-manifest cache. Their keys, invalidation and failure behaviour
  differ.
- **Bounded in-process work queues.** B has one channel and C one capped join
  set. A has none and rejects work with non-waiting admission instead.
- **Construction graphs, dependency injection and service replacement.** No
  consumer uses a container. All three compose with closures, trait objects,
  builders and test-only constructors at the root.
- **Circuit breakers and fallback.** None is present. B disables automatic
  provider retry and documents that it has no automatic fallback. No dependency
  need was measured.

### Stable adapter promotion

Three tracks are kept distinct:

1. **Focused experimental extraction** under `batter-7r3` and the
   `consumer-feedback` Beads proceeds under its own Beads. This record neither
   blocks nor recreates it.
2. **Stable or public promotion** of an adapter follows the
   [integration contract](../integrations.md): two application composition
   roots, exercised partial-startup and shutdown failures, and a documented
   external version contract. Two-consumer evidence applies only to the APIs
   actually proposed for release (`batter-ddc`).
3. **Speculative frameworks** are rejected above.

The survey's usage status is input to the second track, not a promotion
decision:

| API | A | B |
| --- | --- | --- |
| `HttpBoundary` route groups, rendered probes and listener registration | used | used |
| `batter-sqlx` pools, atomic scopes and verification | used | used |
| `batter-runledger::register_in` | used | used |
| `batter-runledger::job_phases` | pin predates it | used |
| Periodic components | pin predates them | used |
| `Command` and `check_command` | used, with local signal wrapper | used, with local signal wrapper |
| `batter-at-rest` | used | used |
| Supervised Runlimit expiry maintenance | not used | used |
| `service::start` and `batter-otlp` | not used | not used |

Usage at those pins does not establish the current API, because both pins
predate later hard cuts.

## Consequences

- The "Cache/batching/service graph" [status](../status.md) row stays
  unimplemented and cites this record.
- `batter-4q78` and `batter-0772` each carry their own ADR-010 invalid-state
  review, rustdoc, example, failure-path tests and `scripts/verify.sh` evidence.
- Watch-list items live here, not as backlog Beads.
- A future promotion assessment for a specific adapter repeats the survey at
  the consumers' then-current pins.
- A third consumer adopting the lifecycle layer, or consumers converging on
  stage and settings policy, could reopen the application-root question. Only
  shared failure semantics can do so; line counts cannot.
