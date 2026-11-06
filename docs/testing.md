# Testing and failure-contract coverage

[Documentation index](README.md) · [Contracts](guarantees.md) ·
[Coverage map](#authored-coverage-map) · [Live suites](#explicit-reference-compatibility-probes)

## Verification

From the repository root, with the pinned Rust 1.98.1 toolchain:

```sh
bash scripts/verify.sh
```

Rust 1.98.1 is also the declared minimum; there is no separate MSRV run.
Prerequisites include Python 3.9+, Docker for native Runledger PostgreSQL 18
tests, loopback TCP, Unix subprocess permissions and a host certificate store.
Initial dependency and file-budget-runtime downloads require network access.

The script owns the complete local sequence: formatting, file budgets, four
Clippy configurations, six test-matrix parts, rustdoc with warnings denied and
all five HTTP smoke profiles. It fails on any unsuccessful command and preserves
the Cargo-generated lockfile. Do not repeat a successful matrix unless later
changes or environment differences invalidate its result. Record outcomes and
limitations in the owning Bead. Backend changes require the complete run.

Run it from a foreground shell. A background job in a non-interactive POSIX shell
can inherit ignored SIGINT/SIGQUIT dispositions, breaking the signal tests; reset
them to defaults before exec if a background launch is necessary.

The standalone [at-rest portability gate](../scripts/check-batter-at-rest-portability.sh)
runs separately in CI. Hosted Rust job selection belongs to
[ci_plan.py](../scripts/ci_plan.py); a planner failure selects the full run.
Weekly CI checks floating stable. A workflow definition is not evidence that it ran.

<a id="verification-commands"></a>

### Focused commands

```sh
python3 scripts/test_matrix.py workspace
python3 scripts/test_matrix.py no-default-features
python3 scripts/test_matrix.py doctests
python3 scripts/test_matrix.py consumers
python3 scripts/test_matrix.py runlimit
python3 scripts/test_matrix.py scripts
cargo fmt --all -- --check
bash scripts/check_file_budget.sh
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

Use a focused part while developing; it does not replace a required complete run.

| Matrix part | Scope |
| --- | --- |
| `workspace` | All-feature workspace tests, reference-runner controls, hostile configuration and optimized adapter future-size tests. |
| `no-default-features` | Minimal core/facade, standalone outbound leaf, and the shipped outbound example without dev-only reqwest features. |
| `doctests` | Workspace all-feature rustdoc examples, including compilation failures for invalid compositions. |
| `consumers` | Facade feature checks, native/source-copy consumers and graph/tool controls. |
| `runlimit` | Native default-feature tests, optimized corrupt-state regression and workspace graph checks. |
| `scripts` | Bounded process, smoke, live-runner and consumer controls. |

[test_matrix.py](../scripts/test_matrix.py) owns the exact inventory and ordered
batches. Do not run conflicting reference executable feature builds together:
Cargo can replace the shared binary while a peer test is still launching it.
The matrix rejects those schedules. Its bounded subprocess runner retains every
outcome, stops later batches after failure, and treats overflow, interruption or
incomplete output as failure. These bounds do not prove detached descendants stopped.

Jig is used only for its pinned standalone file-budget checker. Local checks
compare against `origin/master`; use
`scripts/jig file-budget check --strict-inventory` for all governed files.
Keep the 800-line Rust ceiling and exact imported-file ceilings intact. No Jig
plans, receipts, gates or MCP registration are required.

## Authored coverage map

This is a navigation map, not an assertion that every suite passed on this tree.
Tests and the matrix own the executable inventory; contracts live in
[guarantees](guarantees.md) and package rustdoc.

| Contract | Owning tests |
| --- | --- |
| Deadline clamping, refusal, cancellation, tracked invocation and retry | [Core tests](../crates/batter-core/tests/), especially `operation`, `tracked_operation` and `retry`. |
| Owned phases, finalization and acquisition publication | [Phased operation](../crates/batter-core/tests/phased_operation.rs), [native acquisition](../crates/batter-core/tests/native_acquisition.rs), [cleanup acquisition](../crates/batter-core/tests/cleanup_acquisition.rs). |
| Critical/finite task ownership, shutdown, readiness and retained errors | [Core tests](../crates/batter-core/tests/), especially `lifecycle`, `process_ownership`, `shutdown_causes`, `protected_startup` and `driver_observer`. |
| Commands, signals, cleanup and dispatch destruction | [Command](../crates/batter-core/tests/command.rs), [command signals](../crates/batter-core/tests/command_signals.rs), core `cleanup` and `scoped_dispatch` targets. |
| Serial maintenance, health and freshness | [Periodic](../crates/batter-core/tests/periodic.rs), core health/readiness targets. |
| Panic-hook ownership and redacted reports | [Hook ownership](../crates/batter/tests/panic_hook_ownership.rs), core `panic_hook` and `report_redaction` targets. |
| Settings and diagnostic exclusion | Core settings tests, reference `configuration`, SQLx example diagnostics. |
| Browser transport and HTTP composition | [Browser](../crates/batter-axum/tests/browser.rs), [route groups](../crates/batter-axum/tests/route_groups.rs), [operational](../crates/batter-axum/tests/operational.rs), and adapter rustdoc. |
| HTTP/1.1 traffic, streaming and disconnect limits | [HTTP transport tests](../crates/batter-axum/tests/), [ADR-008](adr/008-http-transport-ownership.md) and [ADR-009](adr/009-http-lifetime-observations.md). |
| Send bounds, future size and composed consumers | Core `send_futures`, [SQLx future sizes](../crates/batter-sqlx/tests/future_size.rs), Runlimit future sizes and facade consumer targets. |
| Profiled SQLx scopes, query/transaction completion and uncertainty | [SQLx tests](../crates/batter-sqlx/tests/), including ignored `atomic_live`, profile and verification targets. |
| Managed routines/triggers, release fences and scoped application SQL | [Managed live tests](../crates/batter-sqlx/tests/managed_objects_live.rs). |
| Native invocation phases, settlement and handler testing | [Runledger adapter tests](../crates/batter-runledger/tests/), native [runtime tests](../runledger/runledger-runtime/tests/). |
| Quota, authenticated attempts and maintenance | [Runlimit adapter tests](../crates/batter-runlimit/tests/) and native package tests. |
| Outbound wire behavior and phase-owned evidence | [Outbound tests](../crates/batter-outbound-http/tests/) and facade `outbound_exchange` example tests. |
| External single-dependency adoption | [Consumer sources](../consumers/README.md) and [source-copy checker](../scripts/check_single_facade_consumer.py). |
| Verification selection and process failure behavior | [Script controls](../scripts/), especially `test_verify`, `test_parallel_process`, `test_ci_plan` and live-runner controls. |

## Owned phase and acquisition handoff regression

The phased-operation and native-acquisition suites cover failed work,
cancellation, destruction panics, unpolled owners, retained publication, LIFO
cleanup and private success proofs. They verify locally owned resources and
outcomes; they do not establish remote effect settlement. See the
[assessment](adr/010-agent-only-consumption.md#owned-phase-and-acquisition-handoff-assessment-batter-jxj2).

## Testing a job handler without a worker

```sh
cargo test -p batter-runledger --features test-support --locked
```

`test_support::HandlerInvocation` drives native handler dispatch without a queue,
worker or database. `handler_invocation` and `handler_progress` cover first-poll
budgets, retained progress, returned failures, panic handling and invocation exit.
This facility applies no durable persistence or retry policy.

## Managed routines and triggers

The SQLx managed live suite covers migration-before-apply, routine/trigger
replacement, constraint-trigger identity, privileges, lock ordering, release
fences, verifier integrity and interruption. The exact cases are selected by
[sqlx_live.py](../scripts/sqlx_live.py); ordinary tests leave external cases ignored.
See [ADR-013](adr/013-managed-database-objects.md).

## Explicit reference compatibility probes

Provision disposable external fixtures and satisfy the prerequisites documented
in the [SQLx guide](../crates/batter-sqlx/README.md) and
[reference compatibility manifest](reference-compatibility.md), then run:

```sh
bash scripts/test_sqlx_live.sh
bash scripts/test_reference_live.sh
```

The runners fail closed on missing prerequisites or inventory mismatches. SQLx
requires `DATABASE_URL`, `BATTER_SQLX_AUTH_ACCEPT_URL` and
`BATTER_SQLX_ADMIN_URL`; the reference runner checks independent cluster identity
and its selected schema/authority policy. Never point these commands at a
non-disposable database. Provisioning remains external to Batter.

Native Runledger's PostgreSQL 18 Docker tests are different: they run in ordinary
workspace verification through its own test-support package. Do not infer an
external Batter live-suite result from those native tests or from a runner-control test.

For the reference suite, set `POSTGRES_TEST_ADMIN_URL` and
`POSTGRES_TEST_OBSERVER_URL` to two dedicated disposable local PostgreSQL 18
clusters. The primary requires superuser authority, SCRAM host authentication,
`autovacuum` and `track_counts` enabled, `autovacuum_naptime <= 5s`, and
`max_prepared_transactions > 0`. Provision it with, for example,
`postgres -c autovacuum_naptime=1s -c max_prepared_transactions=10`.
The secondary must have a different cluster identity and allow
`pg_catalog.pg_control_system()`; if access was revoked, grant EXECUTE to its
observer login in the selected administrative database. No target fixture
databases are created on the secondary. Keep unrelated workloads and connection
producers off both endpoints; failure controls briefly lock system catalogs.

Use explicit URL credentials. The native preflight rejects ambient `PG*`
entries, missing/remote/TLS-required endpoints, unsupported query parameters,
equal cluster identities and insufficient authority before fixtures run.
Both endpoints are validated before either connection opens. The startup probe
checks actual SCRAM authentication for its disposable database; preflight alone
cannot certify database-specific `pg_hba` rules. Passfiles are not consulted.

The reference runner builds both executables with `metrics-export`, verifies the
exact inventory declared in [reference_live.py](../scripts/reference_live.py),
executes it serially, and requires the separate maintenance-session and provider
state probes. Zero tests, skipped cases, inventory mismatches and watchdog kills
are failures. Inventory size belongs to the runner rather than this guide.

## Managed application SQL regression

The maintained `managed_objects_live/application_sql` and
`application_failures` cases exercise restricted callers, atomic publication,
callback rejection, integrity changes and uncertain transaction outcomes. They
are included in the explicit SQLx live inventory.

## Seeded scheduling exploration

The core scheduling suite exercises seeded multi-worker lifecycles, finite
admission, accounting, cancellation and escalation with bounded subprocess
observation. Seeds reproduce scenario choices, not Tokio scheduling. Mutation
and delayed-start controls reject vacuous accounting and watchdog success.
Use [scheduling tools](../scripts/stress_scheduling.py) for the maintained runner;
a finite corpus is not exhaustive concurrency evidence.

## Non-yielding subprocess tests

Blocking polls run in subprocesses owned by an external watchdog. The private
[process machinery](../test-support/README.md) retains output and termination
evidence while bounding the parent test. Linux-only orphan/reaping cases do not
establish macOS behavior. Process kill is not evidence of application finalization
or general task preemption.

## Browser credential transport

The adapter browser and route-group suites cover origin validation, cookie
transport, mutation signals, browser-policy rejection and private-response
headers. They test transport policy, not credential meaning, authorization or CORS.

## HTTP route groups

`route_groups` and rustdoc cover inventory agreement, route matching,
group-specific policies, browser credentials, admitted extraction and invalid
composition rejection. Test expectations distinguish public probes, protected
routes and unmatched fallback behavior.

## Axum operational defaults

The operational suite covers readiness decisions, trusted renderer metadata,
admission precedence, generated correlation and transport peer identity.
Real-socket cases compare the peer with the client's own socket address;
forwarded headers cannot supply that authority.

## Sealed HTTP assembly and the in-process client

Compile-fail examples reject access to an assembled raw router and unregistered
serving. In-process tests select an explicit synthetic peer through the opaque
client. They exercise boundary assembly without claiming socket provenance.

## Probe response policy

Operational probe cases verify library-selected statuses, application-rendered
bodies, dependency freshness and response policy outside ordinary admission.
Readiness tests distinguish starting, running, draining and stopped states.

## Serving over a generic listener

The adapter's operational and TLS-serving targets cover native registration,
startup acknowledgement, listener abandonment, pending handshakes and graceful
streaming drain. Application-owned TLS configuration and listener-spawned
descendants remain outside the registration guarantee.

## HTTP/1.1 instrumented lifetime observations

The adapter transport suite separately observes handler/response, body,
connection and server-wrapper lifetimes. Streaming-body survival after wrapper
abort is an explicit limit, not successful cleanup. See
[ADR-009](adr/009-http-lifetime-observations.md) for measured scope.

## Fixture infrastructure regression and mutation checks

SQLx fixture tests and live-runner controls cover acquisition/finish ownership,
retained failures, endpoint prerequisites, cache identity and interruption.
Fixture provisioning belongs to the external harness; fixture tests do not prove
the termination of arbitrary server sessions or detached work.

## Provider lease and retry boundary probes

Reference provider tests exercise application-owned idempotency, payload
matching, lease fencing, lookup-before-replay and terminal outcomes against a
loopback fixture. These protocols do not establish arbitrary provider behavior
or exactly-once execution. Database-backed cases remain explicit live probes.

The reference worker's `panic_policy` unit regression exercises its production
failure classifier for construction, polling and destruction panics in either
phase, including a retryable finalization error. The ignored
`provider_state_lock_and_retry_boundaries` test also drives production work
against a blocked dispatch-fence transaction, covering outer and nested deadline
ordering. It verifies finalization of both an uncommitted fence and a committed
fence whose work result was withheld. The latter models retained commit
uncertainty; it does not emulate a lost wire-level COMMIT acknowledgement.

## Outbound exchange facade example

```sh
cargo test -p batter --no-default-features --features outbound-http --example outbound_exchange --locked
```

The example is also part of ordinary verification. It runs the owned transport
without the outbound package's dev-only reqwest features. Package wire tests
separately control handoff, TLS writes, deadline/cancellation and response bounds.

## HTTP process smoke test

```sh
python3 scripts/check_http_smokes.py
```

The runner builds the HTTP example and owns five loopback process profiles:
normal SIGTERM, SIGINT, deadline, WARN-filtered and WARN-filtered deadline.
They assert readiness, fallback, response envelopes, generated correlation,
completion fields and checked shutdown. The smoke launcher owns its fixture
settings instead of consuming ambient `BATTER_*` values. These are not load,
streaming or external deployment tests.

## Typed settings verification

Core settings and reference configuration tests cover bounded explicit sources,
native constructors, invalid/non-Unicode values, secret exclusion and hostile
ambient environment. Applications still own their settings schema.

## Reference metrics export

The OTLP package tests use a local collector and protected service completion to
exercise bounded export, final collection, retained failures and cleanup order.
The reference package's opt-in metrics cases are compiled by the matrix; its
database-backed export cases run only in the explicit reference live suite.

## Native lifecycle and offline retirement acceptance

Core managed-component tests and Runledger adapter tests distinguish inert
preparation, native initialization, readiness, stop clocks and complete
settlement. Offline definition retirement remains a separate administrative
operation; queued work does not inherit a request's deadline or cancellation.

### Protected startup consumer process cases

The reference live inventory covers SIGTERM/SIGINT during schema work and
startup owner/waiter loss. Its offline acquisition and executable controls cover
signals during a withheld connection handshake and actual binary startup.
They assert acknowledged phase boundaries and retained cleanup outcomes;
watchdog termination cannot count as successful shutdown.

## Native Runledger workspace verification

The native packages share local SQLx dependencies. Workspace and consumer
checks reject duplicate/remote foundation sources and reversed ownership;
native PostgreSQL tests retain their own Docker support. See
[native package guidance](../runledger/AGENTS.md).

## Runledger standalone sources and cache maintenance

[check_runledger_consumer.py](../scripts/check_runledger_consumer.py) builds an
independent native consumer from a Git-free source copy.
[refresh_runledger_sqlx.py](../scripts/refresh_runledger_sqlx.py) owns native
offline query-cache refresh; it requires an explicitly provisioned database.
Graph and tool controls run in the ordinary matrix.

## Single-dependency facade consumer

[check_single_facade_consumer.py](../scripts/check_single_facade_consumer.py)
builds the [external sources](../consumers/README.md) from temporary manifests
declaring only `batter`. It checks feature reachability, native type identity and
protected composition. The consumer owns a disposable PostgreSQL container and
runs in its own matrix batch. This is executable adoption evidence, not a
population-wide agent-usability result.

## Native Runlimit workspace verification

Native tests cover policy validation, bounded memory behavior, PostgreSQL
storage and transport. The matrix includes default-feature tests and the
optimized corrupt-state regression; graph/consumer checks reject unwanted
dependencies. Adapter tests cover quota-before-work, authenticated attempts and
registered expiry maintenance. See [native guidance](../runlimit/AGENTS.md).

## Enqueue-intent reconciliation

Native `intent_reconciliation` and `grants` tests exercise exact scope,
restricted reads, own-write visibility, rejection rollback, concurrent
promotion/retention and a dropped COMMIT acknowledgement. Compile-fail rustdoc
keeps removed pool getters unavailable. Observed absence does not prove remote rollback.

## Limits of current coverage

Builds, unit tests, loopback transports, native Docker tests and external live
suites provide different evidence. No passing suite proves arbitrary remote
effects, general task preemption, detached-descendant termination or exhaustive
race coverage. Handler-panic recovery and comprehensive RFC/OpenAPI conformance
are not implemented guarantees.

The outbound wire suites use loopback and controlled I/O. Production platform
verification rejects self-signed certificates, but public-CA acceptance is
untested. DNS/setup failure behavior, operating-system scheduling and blocking
certificate verification retain the limits recorded in the
[outbound contract](guarantees.md#outbound-provider-exchange).

Use [status](status.md#limits-of-current-evidence) and the owning Bead for actual
execution scope. Missing database prerequisites must fail explicitly, and a
runner-control pass must never be reported as an executed live suite.

## Reporting convention

Record the command, revision, toolchain, lockfile hash, platform, result and
skipped prerequisites in the owning Bead. Source inspection is not a passing
test, a successful build is not an operational audit, and a local timeout does
not establish database rollback or provider non-dispatch.
