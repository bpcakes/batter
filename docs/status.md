# Implementation status

This page describes capabilities in the source tree. It does not establish a
successful verification run against the current checkout. Record executed
commands, revisions, platforms and limitations in the owning Bead.

Rust 1.98.1 is the pinned toolchain and every package's declared minimum.
Linux and macOS are in scope; other Unix targets remain unverified and Windows
is unsupported. PostgreSQL integrations require PostgreSQL 18 or newer.

[Documentation index](README.md) · [Verification](testing.md#verification) ·
[Delivery tracker](roadmap.md)

## Implemented capabilities

| Area | Source capability and boundary |
| --- | --- |
| Package ownership | Twenty-two libraries, the Runledger TUI, the unpublished `batter-flags` operator command and two unpublished example packages share a virtual workspace. The facade re-exports one core implementation and optional adapters. Native Runledger and Runlimit retain their own ownership. See [architecture](architecture.md) and the [package table](../README.md#packages). |
| Canonical agent integration | Protected construction, consuming transitions, opaque capabilities and exhaustive outcomes enforce locally expressible invariants. Escape hatches disclose weaker caller obligations. [ADR-010](adr/010-agent-only-consumption.md) owns the review policy. |
| Service startup and completion | Protected initialization retains failures and cleanup, transfers typed output once, acknowledges readiness and retains diagnostic completion independently of waiters. [Startup contract](guarantees.md#owned-startup-contract). |
| Process ownership | Critical components and bounded finite work have observed direct exits, phased drain/cancel/abort and retained reports. Detached descendants and runtime death remain outside the guarantee. [Process contract](guarantees.md#process-ownership-and-shutdown). |
| Finite commands | Library-owned work and finalization retain concrete results and failures through cancellation and owner loss while the runtime lives. Executable roots select native Unix signals. [Command contract](guarantees.md#owned-finite-commands). |
| Operation budgets | Parent-clamped deadlines, downward cancellation, tracked invocation evidence and owned work/finalization phases preserve concrete application errors. No remote rollback or arbitrary child-task joining is implied. [Execution contract](guarantees.md#execution-boundary). |
| Replay and admission | Retry requires explicit replay authorization and failure classification. Async bulkhead runs own admission and retain permits through future destruction; raw `enter` is removed, and `run_resolved` retains application outcome handling; blocking work retains its permit inside the closure and can outlive its waiter. [Retry](guarantees.md#retry-contract), [admission](guarantees.md#admission-contract). |
| Cleanup and native acquisition | LIFO finalizers require explicit awaiting. Facade guidance selects owned phases and supported native/adapter acquisition. Semaphore and UDP acquisition own cleanup registration before publishing a handle; manual push/register remain lower-level caller obligations (`batter-sdzz`). Conservative cleanup skipping preserves uncertain task settlement. [Cleanup contract](guarantees.md#cleanup-contract). |
| Maintenance and readiness | Registered serial periodic operations have run budgets and owned stopping behavior. Dependency sampling feeds read-only freshness snapshots; application conditions only narrow readiness. [Maintenance](guarantees.md#periodic-maintenance-components), [health](guarantees.md#dependency-health-sampling). |
| Settings and diagnostics | Explicit bounded settings sources and validated values retain redacted diagnostics. Applications own schemas. An application root installs the payload-free panic hook; libraries never install it. [Settings](guarantees.md#explicit-settings-and-redacted-diagnostics), [panic hook](guarantees.md#application-installed-redacting-panic-hook). |
| Telemetry and export | Owned futures retain tracing dispatch through polling and destruction, including uncalled async bulkhead captures and nested spans while admission is pending (`batter-f9kq.10`). Bounded outcome metrics and opt-in OTLP export use protected service completion. No global subscriber installation or durable trace propagation is supplied. [OTLP integration](integrations.md#opentelemetry-metrics-export-protected-adapter). |
| Inbound HTTP | Sealed boundary assembly preserves route inventory, per-group request/browser policy, admitted authority, probe status, fallback placement, direct-peer serving and in-process testing. The request boundary ends at response construction. [HTTP contract](guarantees.md#http-boundary). |
| Browser credentials | Validated origins, opaque cookies, mutation signals and private-response headers enforce transport policy. Credential meaning, authorization and CORS remain application-owned. [Browser adapter](../crates/batter-axum/README.md). |
| Outbound HTTP | The experimental leaf owns one fresh TCP/TLS connection and low-level HTTP/1 driver per exchange, gates handoff and writes, bounds response collection and returns exhaustive transport evidence. No pool, reuse, redirects or replay; provider meaning remains downstream. [Outbound contract](guarantees.md#outbound-provider-exchange). |
| PostgreSQL scopes and pools | Profiled pools register closure before return. Opaque lease/query/atomic/snapshot scopes retain completion and commit uncertainty without replacing native SQLx. [SQLx integration](integrations.md#sqlx-owned-transactions-and-session-operations). |
| Schema and authority verification | Read-only verification checks declared schema and grant requirements. It neither provisions databases nor proves remote effects. [Verifier guide](../crates/batter-sqlx/README.md#read-only-schema-and-authority-verification). |
| Managed database objects | Application migrations precede declared routines/triggers and scoped application SQL on one lease. Validated release revisions fence downgrades/conflicts; applications own numbering, grants, source and rollout. [ADR-013](adr/013-managed-database-objects.md). |
| Runledger | `PhasedJob` owns borrowed work/finalization with a positive reserve, retained returns and separate destruction panics. Optional `settle_late` receives an opaque capability only for a successful value returned after the work deadline; recovery requires checked finalization and live native authority. Ordinary phases remain strict. The private driver uses one explicit execution-state enum. Reference interruption probes delegate production settlement and assert nested versus outer finalization selection (`batter-f9kq.6`). Native initialization, invocation exit, descendant settlement, atomic intent/enqueue and scoped reconciliation remain native-owned. [Integration](integrations.md#runledger-optional-native-lifecycle-adapter). |
| Runlimit | Native policy/storage and atomic quota decisions feed protected quota-before-work/authenticated HTTP composition. Maintained PostgreSQL construction registers native expiry cleanup. Both native PostgreSQL quota algorithms subtract database-measured processing time from retry/replenishment delays without changing evaluated quota state; no automatic refund or replay is supplied. [Integration](integrations.md#runlimit-optional-protected-native-quota-adapter). |
| Typed flags (experimental) | One catalog of release flags, mode flags and kill switches with typed handles, consuming set-once declaration policies and `requires`, `conflicts` and `both_or_none` invariants. `declared::resolve` reads flags only from the root's settings source and returns a redacted startup report. Snapshots validate and publish or hold rows one by one; evaluation is clock-free, and decisions can be pinned and replayed. HTTP route-group, job and periodic gates; `TestFlags`. Seams in existing packages: route-group gates rendering `HttpFailure::Disabled`, `periodic::register_wakeable_in` and `PgAtomicTransaction::commit_unless`. [ADR-016](adr/016-typed-flags-control-plane.md), [contract](guarantees.md#typed-flags-experimental-adr-016), [usage](usage.md#typed-flags-and-kill-switches-experimental). |
| Flags control plane (experimental) | The audited PostgreSQL store in a dedicated `flags` schema owns compare-and-set writes with ordered refusals, history, archive and restore, presence, pruning, the fleet view and the write-side invariant check (`WritePolicy`). `control_plane::register_in` registers refresh, heartbeat, prune and an optional LISTEN wake as process components, persists an optional last-good file and refuses a base an engaged governor would break. It validates pool cleanup ownership, preserves terminal interrupt state and owns persistence selection through blocking completion (`batter-jkw6.36`). The `batter-flags` operator command writes through the store. No hot-standby operation, per-subject rollout or agreement across process kinds is implemented. [Operations](operations.md#flags-control-plane-experimental), [integration](integrations.md#flags-control-plane-over-postgresql), [assessments](evidence/api-assessments/flags.md). |
| Fixtures and consumers | External provisioning feeds optional SQLx fixtures. Native Runledger retains Docker test support. Single-dependency source-copy consumers verify facade reachability. The reference HTTP command validates before database capacity admission; its admitted submission retains settled outcomes across interruption. The reference provider handler uses application-owned exchange execution to record interruption evidence at admission, fence and provider entry; consumers cannot set progress markers. Recovery actions accept only unresolved source states (`batter-f9kq.8`). Retained-state invariant recovery uses the finalization reserve; bounded process-task migration preserves outer fatal failures (`batter-f9kq.7`). [Coverage](testing.md#authored-coverage-map), [consumer recipe](reference-compatibility.md#single-dependency-recipe). |
| Public API migrations | Hard cuts require a Breaking changelog entry, replacement rustdoc naming the removed item and a shim decision. Retained [surface diffs](evidence/public-api-diff-2026-10-09/README.md) support review; name-only inventories cannot detect every signature change. |
| Documentation | Current guides and source checks are indexed; superseded research, static inspection output and obsolete consumer snapshots are removed. Docs-only checks are recorded in `batter-9xqi`. |

## Limits of current evidence

Historical Linux x86_64 and macOS arm64 results on Rust 1.94.0 and 1.98.1
describe their recorded revisions. Hosted run
[35580602864](https://github.com/bpcakes/batter/actions/runs/35580602864), for
`56814038f2a9cf6a34688ee39cd9f0e433487a1e`, predates later changes including
the outbound package. Its owned-connection transport has separately recorded
Linux verification in `batter-u387`; earlier macOS evidence predates that
transport. Neither record verifies the current checkout.

CI definitions are not execution evidence. Ordinary verification does not run
the ignored external SQLx/reference live suites or the flags PostgreSQL 18
suites; the `native-flags-postgres` CI job runs the store, loader and
operator-command suites, and each local or hosted run is recorded on its flags
Bead. Local socket tests do not prove provider behavior, arbitrary descendant
termination, server-side rollback or public-CA acceptance. See [coverage limits](testing.md#limits-of-current-coverage).

## Unimplemented or outside scope

Request-scoped child-task joining, durable distributed correlation, generated
OpenAPI/client contracts and retry-token budgets are not implemented. No DI
container, application root framework, cache, queue or workflow replacement is
planned by the current design; [ADR-014](adr/014-reuse-after-two-consumers.md)
records the extraction boundary. The experimental flags packages change none
of this: a kill switch's safe state is not a circuit breaker or fallback, since
no dependency failure, latency or error class engages it and nothing probes or
half-opens; `Pinned<T>` serializes no cancellation token, deadline, clock value
or trace context; and the flag snapshot is not a cache, since one validated
snapshot is published per control-plane version and evaluation never loads,
evicts or misses per key ([ADR-016](adr/016-typed-flags-control-plane.md)).
Delivery decisions belong in [Beadroll](roadmap.md).
