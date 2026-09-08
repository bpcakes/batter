# Implementation status

Updated: 2026-09-08. The local Rust verification matrix passes on 1.94.0 and
1.98.1 with the refreshed lockfile. There are no hidden implemented adapters
behind the planned rows. See [validation](validation.md) for execution evidence
and limits; local checks do not establish production or hosted CI validation.

| Concern | Source status | Evidence / boundary |
| --- | --- | --- |
| Jig harness | Locally validated | Official v0.3.0 installation, MCP, five configured targets, and guide checks; CI comparison, cache reuse, plan merging, and archive regression tests. Hosted cache reuse remains unverified. Use `update --recopy` to retain the selected release. See [validation](validation.md). |
| Clippy complexity and length limits | Enabled | Both workspace packages inherit warning-level lints; thresholds 20 / 100, enforced by `-D warnings`. |
| Dependency/toolchain refresh | Locally validated | Latest stable direct dependencies, SQLx 0.9.0, Rust 1.94 minimum, default toolchain 1.98.1; [versions](references.md#dependency-refresh-2026-09-07). |
| Typed execution errors | Implemented | `operation` preserves E; interruption is separate. Tests authored in [operation](../tests/operation.rs). |
| Total deadline and child deadline clamp | Implemented | Tokio Instant, no serialization; cancellation-first boundary precedence. |
| Child cancellation on operation finish/drop | Implemented | CancellationToken drop guard; does not join spawned children. |
| Critical process task supervision | Implemented | Registered factories start inside JoinSet tasks; observed early Ok, error, and panic. |
| Startup-acknowledged readiness and drain/cancel separation | Implemented | Application approval plus all critical startup acknowledgements; one-way transitions. |
| Finite process-owned work | Implemented | Synchronous bounded admission, active-scope descendants during drain, typed receipts independent of work lifetime, bounded failure retention. |
| Owned shutdown driver | Implemented | Waiter/last-owner drop tests, retained report/coordinator JoinError; runtime must remain alive. |
| Bounded drain, cancellation, abort observation | Implemented | Ready-result harvesting before escalation; only unfinished abort targets; unpolled driver drop signals shutdown. Cooperative scheduler assumption remains. |
| Ordered explicit asynchronous teardown | Implemented | LIFO, total/per-hook budgets, errors retained, one report/event per skip, skipped capture destruction in dependency order. |
| Generic async resource acquisition scope | Not implemented | No Effect-style interruption mask or acquire/register atomic protocol. |
| Partial-startup cleanup | Pattern implemented | `take_cleanup`; [SQLx example](../examples/postgres_lifecycle.rs). Must be explicitly driven. |
| Concurrency admission | Implemented | Native semaphore permits; no waiter-count or memory bound. |
| Retry classification and replay authorization | Implemented | No timeout retries; provider lower bound; last error retained. |
| Finalization reserves and injected jitter | Implemented | Sibling phase contexts, deterministic samples, provider floor; neither masking nor fleet coordination. |
| Attempt deadlines, retry tokens, circuit breaking, fallback | Not implemented | Remaining BTR-040 scope. |
| Tracing conventions | Implemented | INFO/WARN completion events, HTTP status/outcome/latency; scoped dispatch survives owned-future destruction and nested spans. No exporter or metric backend. |
| Axum middleware/probes | Implemented, optional | Bounds response construction, not streaming; fixed server budget. |
| Typed HTTP infrastructure errors | Implemented | Sanitized default problems or configured application renderer with request parts; trust/domain mapping remains external. |
| Configuration framework / secret types | Not implemented | Only operational argument validation and example env parsing. |
| SQLx native pool integration | Example only | Connection, probe, pool close, partial-startup pattern; no pool wrapper. |
| SQLx transactional Runledger reference path | Not implemented | BTR-020. No business transaction or durable job test in this package. |
| Runledger host/trace adapter | Not implemented | Preserve upstream supervision; do not recreate worker loops. |
| Runlimit admission/observer adapter | Not implemented | Preserve key trust boundary and consumption certainty. |
| postgres-test-harness app adapter | Not implemented | No database/container provisioning code is copied here. |
| Scripted fake / error-preserving teardown helper | Implemented | [Test support](../crates/batter-test-support/src/lib.rs). |
| Live PostgreSQL integration tests | Not implemented or executed | No test silently skips a missing database. |
| Schema/OpenAPI/client generation | Not implemented | BTR-050; existing ecosystem integration first. |
| Durable correlation / distributed tracing | Not implemented | No serialization of cancellation tokens or monotonic deadlines. |
| Request-scoped child-task joining | Not implemented | Ordinary composed futures recommended; no fake structured-concurrency guarantee. |
| Cache/batching/service graph | Deliberately deferred | Extract only after two real consumers establish common semantics. |
| Memory safety/Send/Sync/Result replacement | Out of scope | Use Rust directly. |

## Release evidence still required

Hosted CI execution, a real HTTP traffic/streaming/disconnect hardening suite,
non-yielding child-process tests, and a real PostgreSQL setup remain outstanding.
Startup/drop/race regressions and local HTTP signal/envelope/telemetry checks are
recorded separately in validation; they do
not establish database integration or detached-descendant shutdown guarantees.

See [validation](validation.md) for what was actually checked, and
[roadmap](roadmap.md) for scoped follow-on work.
