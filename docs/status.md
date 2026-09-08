# Implementation status

Updated: 2026-09-08. The Rust verification matrix passes on 1.94.0 and
1.98.1 on Linux x86_64 and macOS arm64 with the refreshed lockfile. There are no hidden implemented adapters
behind the capability rows. See [validation](validation.md) for execution evidence
and limits; local checks do not establish production or hosted CI validation.
The [Effect v4 reconciliation](effect-v4-reconciliation.md) explains current
capabilities and deliberate differences from the broader proposal. This page
records implemented facts and limits; [Beads](roadmap.md) owns delivery status,
priorities, acceptance and dependencies.

| Concern | Source status | Evidence / boundary |
| --- | --- | --- |
| Workspace package boundaries | Implemented | Virtual root, separate `batter` / `batter-axum` / generic test-support libraries, and unpublished SQLx example package; independent manifests retain Rust 1.94. PostgreSQL harness stays external. [ADR-006](adr/006-workspace-packages.md); execution evidence in [validation](validation.md). |
| Jig harness | Locally validated | Official v0.3.0 installation, MCP, five configured targets, and guide checks; CI comparison, cache reuse, plan merging, and archive regression tests. Hosted cache reuse remains unverified. Use `update --recopy` to retain the selected release. See [validation](validation.md). |
| Clippy complexity and length limits | Enabled | All workspace packages inherit warning-level lints; thresholds 20 / 100, enforced by `-D warnings`. |
| Platform scope | Unix-only | Linux and macOS remain in scope. Windows is unsupported and not planned; no Windows CI or non-Unix implementation fallbacks. Linux x86_64 and macOS arm64 have full verification and HTTP smoke evidence. The updated macOS CI job and other Unix targets remain unverified. [ADR-007](adr/007-unix-platform-scope.md). |
| Dependency/toolchain refresh | Locally validated | Latest stable direct dependencies, SQLx 0.9.0, Rust 1.94 minimum, default toolchain 1.98.1; [versions](references.md#dependency-refresh-2026-09-07). |
| Typed execution errors | Implemented | `operation` preserves E; interruption is separate. Tests authored in [operation](../crates/batter/tests/operation.rs). |
| Total deadline and child deadline clamp | Implemented | Tokio Instant, no serialization; cancellation-first boundary precedence. |
| Child cancellation on operation finish/drop | Implemented | CancellationToken drop guard; does not join spawned children. |
| Critical process task supervision | Implemented | Registered factories start inside JoinSet tasks; observed early Ok, error, and panic. |
| Startup-acknowledged readiness and drain/cancel separation | Implemented | Application approval plus all critical startup acknowledgements; one-way transitions. |
| Finite process-owned work | Implemented | Synchronous bounded admission, active-scope descendants during drain, typed receipts independent of work lifetime, bounded failure retention. |
| Owned shutdown driver | Implemented | Waiter/last-owner drop tests, retained report/coordinator JoinError; runtime must remain alive. |
| Bounded drain, cancellation, abort observation | Implemented | Ready-result harvesting before escalation; only unfinished abort targets; unpolled driver drop signals shutdown. Cooperative scheduler assumption remains. |
| Non-yielding subprocess evidence | Tested on Linux and macOS | [Watchdog tests](testing.md#non-yielding-subprocess-tests) cover cooperative completion, unjoined work with skipped finalizers, blocked runtime destruction/timers, parent death, emergency deadlines, launch identity, observed SIGKILL status, bounded output and rejection controls. Bounded complete event records own capture timestamps; a pure wait policy separates startup and observation without depending on polling time. Synchronization derives its complete startup-plus-observation bound from the scenario policy. Exited children have capture joined before final event lookup, and live event overflow proves continued draining with a distinct retained cause. Live kill timing has a bounded lateness check. A later captured panic preserves established startup timing while final validation still rejects it; deterministic and live delayed-poll regressions cover this ordering. Deterministic controls cover deadline boundaries, fragmented/incomplete output, overflow and actual kill-request timing; real processes cover missing startup, premature extended-deadline kills and the actual wait deadline for both blocked scenarios. Reader-thread controls cover interruption, EOF, partial-output I/O errors and panics with destruction joined before finish returns. The unwind control rejects waiting for emergency exit instead of prompt kill/reap. Optimized Python probe controls reject wrong exit status and forbidden cleanup evidence. Linux-only reaping probes remain platform-specific. The updated macOS CI job is unexecuted. Process termination does not establish application finalization. |
| Ordered explicit asynchronous teardown | Implemented | LIFO, total/per-hook budgets, errors retained, one report/event per skip, skipped capture destruction in dependency order. |
| Generic async resource acquisition scope | Not implemented | No Effect-style interruption mask or acquire/register atomic protocol. |
| Partial-startup cleanup | Pattern implemented | `take_cleanup`; [SQLx example](../examples/postgres-lifecycle/src/main.rs). Must be explicitly driven. |
| Concurrency admission | Implemented | Native semaphore permits; no waiter-count or memory bound. |
| Retry classification and replay authorization | Implemented | No timeout retries; provider lower bound; last error retained. |
| Finalization reserves and injected jitter | Implemented | Sibling phase contexts, deterministic samples, provider floor; neither masking nor fleet coordination. |
| Attempt deadlines and retry tokens | Not implemented | Current retry execution has one total budget. |
| Circuit breaking and fallback | Not implemented | No measured consumer requirement has established a shared abstraction. |
| Tracing conventions | Implemented | INFO/WARN completion events, HTTP status/outcome/latency; scoped dispatch survives owned-future destruction and nested spans. No exporter or metric backend. |
| Public adapter dispatch seam | Implemented | `telemetry::with_current_dispatch` captures the current subscriber at the call, preserves polling/destruction, and accepts borrowed/non-Send work; [direct tests](../crates/batter/tests/scoped_dispatch.rs). It does not capture the current span or own a task. |
| Axum middleware/probes | Implemented, separate package | `batter_axum` retains combined readiness/deadline policy. Bounds response construction, not streaming; fixed server budget. |
| Typed HTTP infrastructure errors | Implemented | Stable codes and basic Problem JSON or a configured application renderer; no app-wide domain taxonomy or comprehensive RFC conformance test. |
| HTTP handler panic recovery | Not implemented | Owned task/cleanup panic observation does not convert handler panics into HTTP 500s. Default panic-hook output remains separate. |
| Request metadata / ambient context | Partial example only | Explicit deadline/cancellation extension plus example-generated request IDs; no task-local tenant/principal context or inbound trace-parent handling. |
| Configuration framework / secret types | Not implemented | Only operational argument validation and example env parsing. |
| SQLx native pool integration | Example package only | `batter-example-postgres-lifecycle`: connection, probe, pool close, partial-startup pattern; no pool wrapper or SQLx dependency in the foundation. |
| SQLx transactional Runledger reference path | Not implemented | No business transaction or durable job test in this package. |
| Runledger host/trace adapter | Not implemented | Preserve upstream supervision; do not recreate worker loops. |
| Runlimit admission/observer adapter | Not implemented | Preserve key trust boundary and consumption certainty. |
| postgres-test-harness app adapter | Not implemented | No database/container provisioning code is copied here. |
| Scripted fake / error-preserving teardown helper | Implemented | [Test support](../crates/batter-test-support/src/lib.rs). |
| Live PostgreSQL integration tests | Not implemented or executed | No test silently skips a missing database. |
| Schema/OpenAPI/client generation | Not implemented | No generated API/client pipeline exists. |
| Durable correlation / distributed tracing | Not implemented | No serialization of cancellation tokens or monotonic deadlines. |
| Request-scoped child-task joining | Not implemented | Ordinary composed futures recommended; no fake structured-concurrency guarantee. |
| Cache/batching/service graph | Not implemented | Shared extraction requires two actual consumers with common semantics. |
| Memory safety/Send/Sync/Result replacement | Out of scope | Use Rust directly. |

## Limits of current evidence

Non-yielding subprocess and rebuilt HTTP smoke evidence covers Linux x86_64 and
macOS arm64 hosts on the toolchains recorded in [validation](validation.md).
The updated hosted macOS CI job remains unexecuted. Abrupt-owner-death adoption
and reaping probes are Linux-only; macOS covers the shared stdin-EOF path.
These results imply neither general task preemption nor application finalization
after process termination. A real HTTP traffic/streaming/disconnect hardening
suite and a real PostgreSQL setup remain outstanding. Startup/drop/race and
HTTP signal/envelope/telemetry checks do not establish database integration or
detached-descendant shutdown guarantees.

See [validation](validation.md) for what was actually checked, and
[Beads](roadmap.md) for delivery work.
