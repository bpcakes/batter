# Provider-backed reference delivery service and compatibility probes

Unpublished, Unix-only application package. Its runnable root initializes the
atomic delivery command and registers native Runledger through `batter::runledger`.
Native preparation is inert; Batter owns launch, initialization acknowledgement,
settlement observation and dependency cleanup. The production registry installs
the delivery handler and ordinary readiness is approved after component
initialization; fresh PostgreSQL health remains independent. No production startup
control job or reconciliation loop runs.
The [compatibility manifest](../../docs/reference-compatibility.md) records version
contracts and execution evidence. The earlier 58-entry live inventory passed on
Linux with both supported toolchains. The prior 64-entry revision passed on macOS
with both supported toolchains. The 66-entry state-first provider suite
passed on macOS with Rust 1.98.1 and exact 1.94.0 against PostgreSQL 18.6. The
current 68-entry suite, adding the two metrics export cases, passed on macOS with
Rust 1.98.1 against PostgreSQL 18.6; its Rust 1.94.0 run belongs to CI.
Two exact legacy test aliases were removed rather than retained through the hard cutover.

## Staged worker and atomic delivery command

Run with an explicit PostgreSQL endpoint, worker identity, owner and bearer token:

```sh
DATABASE_URL='postgres://service:password@127.0.0.1:5432/service?sslmode=disable' \
JOBS_WORKER_ID='reference-worker' \
BATTER_AUTH_OWNER_ID='00000000-0000-0000-0000-000000000001' \
BATTER_AUTH_TOKEN='replace-with-an-opaque-token' \
BATTER_PROVIDER_BASE_URL='https://provider.example/' \
BATTER_PROVIDER_TOKEN='replace-with-a-provider-token' \
  cargo run -p batter-example-reference-service --locked
```

An optional positional settings-file path uses the literal format below.
Environment values override that file; no file is discovered implicitly.
Protected startup selects `.with_unix_signals("signals")`, so SIGTERM/SIGINT
listeners exist before the initializer runs. The initializer publishes pool close
through `batter::sqlx::pool_in`, acquires a connection, applies native and
forward-only application migrations, checks compatibility, synchronizes the
delivery producer, binds HTTP and registers handler-bearing native preparation.
A signal during initialization drains startup and awaits cleanup; after handoff
the library-owned listeners belong to the running driver. `runtime::run` and the
observer-capable `runtime::start` return a retained `ServiceCompletion`: its
`service()` result keeps the original error, and `ServiceFailure::error()`
downcasts a startup failure to `ProtectedRuntimeStartupFailure`. The
earlier wrapper was removed in the coordinated hard cutover. Generic running
failures retain `batter::lifecycle::ShutdownFailure`; an otherwise successful
report missing its required pool record downcasts to
`RuntimePoolCleanupFailure`, whose accessor exposes that report without rendering
it. Metrics diagnostics are a separate `metrics()` field and never change
`exit_code()`. A failed production executable prints only `Error: reference service failed`
and exits 1. Listener discovery never writes stdout. To discover a child-selected
port, bind a Unix datagram receiver before launching the child and set
`BATTER_LISTENER_ANNOUNCEMENT_PATH` to its absolute socket path. After TCP bind,
the child sends one datagram containing its actual SocketAddr, without a newline.
This acknowledges binding, not readiness; publication failure fails startup and
backpressure remains inside the startup deadline. Without that setting, serving
performs no announcement. A separately built test-fixture executable owns the process-local signal
acknowledgements used by the integration suite; the production entrypoint has no
witness mode or test environment switch.
One 20-second startup allowance covers initialization. Shutdown has ten seconds
of drain, one second of cancellation, one second of abort observation and separate
bounded pool cleanup; native settlement shares those process intervals.

Native loop initialization requires no queue mutation or durable execution proof.
Fresh PostgreSQL health comes from a separate bounded sample, and application
approval follows handler registration. `/ready` requires all three facts. The
native worker claims `records.delivery.execute`; durable execution and crash
reconciliation are tested only in isolated probes.
The application pool limit includes native runtime and health usage; there are no
extra production control/reconciliation pools. Applied legacy migrations and the
owner-epoch sequence remain intact. Offline retirement passed fixture acceptance
and explicit CLI execution; startup does not run it. This does not establish
retirement of any deployed database.

The authenticated routes are:

- `POST /records/{record_id}/deliveries` with `expected_generation`,
  `idempotency_key`, and `payload`.
- `GET /deliveries/{delivery_id}` for the owner-scoped durable projection.
- `GET /delivery-commands/{idempotency_key}` for reconciliation when the caller
  did not receive the POST response.

The application-owned `http::register_in` boundary builds the trusted-peer
router and inseparably selects native `ConnectInfo<SocketAddr>` registration, so
the production root cannot choose those two requirements independently.
The separate `http::in_process_client` test seam returns an opaque request
client rather than a `Router`; it cannot enter a production serving operation
and requires an explicit synthetic peer for every request.
`TrustedRequestMetadata` combines that direct peer IP
with Batter's server-generated `CorrelationId`, separately from the authenticated
`OwnerId` and request `OperationContext`. Application and infrastructure bodies
that carry `request_id` agree with the generated `x-request-id` response header.
`Forwarded`, `X-Forwarded-For`, `X-Real-IP`, `traceparent`, `tracestate` and
client request IDs are ignored; behind a proxy the direct peer is the proxy.
There is no trusted-proxy configuration, task-local/spawn propagation, quota
backend, or durable correlation storage in this stage.

The idempotency key is 1–128 URL-safe ASCII bytes (`A-Z`, `a-z`, `0-9`, `.`,
`_`, `:`, `-`); the meaningful JSON payload is at most 16 KiB encoded. The
application retains the owner, record, expected positive generation, canonical
JSONB payload, stable delivery UUID, and immutable Runledger enqueue inputs.
An exact retry returns the same delivery without another enqueue. Changed
record/generation/payload input conflicts. Another authenticated owner may reuse
the same key independently and cannot observe the first owner's rows.

The first submission uses the READ COMMITTED `run_atomic` runner.
Application preparation, enqueue and application completion use protected
savepoint scopes. The runner owns commit/rollback and releases output or rejection
only after acknowledgement; uncertain results retain the domain output/error.
All completion paths retire the session; cancellation still leaves uncertainty.
The service never retries automatically: reconcile by original owner/key, since
absence while an old session may still settle is not rollback proof. Completion
evidence is retained inside the operation boundary before telemetry finalization.

`pending`, `in_flight`, `succeeded`, `dead_lettered`, and `cancelled` are a
closed application projection of the locked Runledger status vocabulary.
Provider outcome is a separate closed projection: `awaiting_attempt`,
`retryable_undispatched`, `reconcile_needed`, `confirmed`, `business_denied`,
`manual_resolution`, or `exhausted`. Command acceptance still does not mean that
an external effect succeeded.

The application creates one provider key and canonical versioned request in the
same transaction as the command, delivery and Runledger job. The key derives from
the stable delivery UUID and remains unchanged across attempts or restart. The
selected protocol uses `POST /effects` plus `GET /effects/{key}`: same key and
payload returns the retained effect, while changed payload conflicts. Its key
retention assumption is exactly 24 hours. Unknown lookup or expiry becomes bounded
manual resolution; this is not an exactly-once claim beyond that protocol window.

The worker loads the retained provider state before provider admission. Terminal
redelivery completes without taking provider capacity. Fresh work may apply
generation policy before dispatch; `reconcile_needed` must perform keyed lookup
first, and only a retained-window absence permits a later generation denial or
replay. That newly authorized POST receives a new 24-hour resolution window
immediately before polling; the prior uncertain attempt's deadline is retained
until absence is proven. Every SQL transition constrains its legal source state.
The shared terminal writer takes a privately validated unresolved-state value;
both manual resolution and business denial reject all terminal sources before
database acquisition. A matching SQL source predicate alone is not transition
authorization.
The state module is the only mutation boundary: it first locks the exact
Runledger `job_queue` row, rechecks the unexpired job/run/attempt/worker lease
after acquiring that lock, performs one legal transition, and commits before
returning. It revalidates before commit as well, so later effect-row lock waits
cannot publish an expired mutation. Heartbeat, reaping and application state writes therefore serialize
on the same native row. Lease loss is distinct from retained-data corruption.

A known non-dispatch outcome and its absolute provider retry eligibility commit
together. The state API takes the delay and returns the native scheduling result;
the handler does not coordinate two independent values. Every initial/replay POST
authorization checks eligibility under the effect-row lock. After a crash between
the application commit and native completion, a recovered attempt defers without
POSTing. That attempt still consumes native retry budget. Returned storage errors
retain known delay, but process death before any successful persistence can lose
the remote response; this is not an exactly-once or remote-acknowledgement guarantee.

Provider admission waits on the purpose-specific `BATTER_PROVIDER_CAPACITY`
inside the handler work deadline. Runledger necessarily claims and increments the
native attempt before invoking the handler and exposes no attempt-neutral
refund/defer operation, but its validated global handler concurrency bounds the
waiting population. No second claim engine or unbounded waiter set is introduced.
Before POST polling, the handler commits `reconcile_needed` and a resolution
deadline. Timeout, cancellation, lost response or SIGKILL then remains
indeterminate. Only exact structured protocol responses can establish known
non-dispatch or business denial; response text never authorizes replay. The
complete response, including bounded body streaming, remains inside that work
deadline. Both POST and GET acceptance require the exact canonical request echo;
a 200 with a missing or mismatched echo cannot confirm an effect. A reqwest
connect-phase failure is known not dispatched, while timeout and every
post-connect/body failure remain indeterminate. The worker rechecks authoritative owner/record/generation before
dispatch and confirmation. Confirmation takes a shared record-row lock before
the native job and effect locks, evaluates generation in a fresh statement, and
holds that record lock through commit. If replacement wins the record lock,
accepted old-generation work becomes manual resolution; if confirmation wins,
replacement waits until confirmation commits. Confirmed work remains attached
to its original identity; it does not mutate the replacement. Record-lock waits
do not hold the heartbeat's job lock, and lease authority is still revalidated
after lock acquisition and immediately before commit. No transaction spans
provider I/O.

`exhausted` and `manual_resolution` deliberately retain different facts.
Exhaustion means the native attempt budget ended and may still have
`acceptance_possible=true`; manual resolution means automatic action is unsafe
because retained truth expired or conflicted, or the target changed.

Both owner-scoped reads and exact submission replay use the same validated
command loader. The query reads job and effect facts in one SQL snapshot. If a
handler cannot persist its last outcome, unresolved effects on a dead-lettered
job with a spent attempt budget project `exhausted`; cancellation, early terminal
failure, or unexpected native success project `manual_resolution`. These reads
preserve acceptance uncertainty and provider identity without changing retained
rows. Missing effect state is an invariant error on both query routes.

Each provider exchange consumes its admission permit and releases it before
returning to post-response SQL. POST admission also covers its short durable
pre-dispatch marker; GET and a subsequent POST acquire separately. The worker
retains static diagnostic codes for request failure, body failure, oversized
response, deadline and cancellation, without storing provider error text.
The reference deliberately accepts a provider retry lower bound of up to 24
hours. This application retry policy is separate from key retention even though
both currently select 24 hours; accepted delays are never shortened.

Provider capacity and database pool capacity count different lifetimes, so there
is no required equality or ordering between their limits. Native global handler
concurrency bounds active handlers and admission waiters. The shared pool also
serves HTTP and health probes; size it and its acquisition budget for measured
database latency and request load. Provider permits no longer remain occupied
while post-response state writes wait for that pool.

Run offline compilation and the native seam doctest:

```sh
cargo check -p batter-example-reference-service --all-targets --all-features --locked
cargo test -p batter-example-reference-service --doc --locked
```

The ordinary configuration tests require enabled IPv6 loopback (`::1`), IPv4
loopback socket permissions and Unix subprocess permissions, including inside
containers. Their IPv6 PostgreSQL protocol fixture owns its listener and requires
no external database. Enable IPv6 before running the matrix; the test is mandatory.

Offline legacy controls retire through `retirement::prepare` or the
`retire_startup_controls` example. Stop old deployments and revoke restart first;
use a direct PostgreSQL endpoint and the expected cluster identifier/database OID
from the known deployment target. Production startup never runs retirement.

The `service.env` path below is a dedicated database-only file, not the serving
process file. A minimal file contains only
`DATABASE_URL=postgres://operator@localhost/database?sslmode=disable`. The command
unsets the higher-precedence environment `DATABASE_URL` so this file selects the
operator endpoint.

```bash
env -u DATABASE_URL cargo run -p batter-example-reference-service --example retire_startup_controls -- \
  "$EXPECTED_SYSTEM_IDENTIFIER" "$EXPECTED_DATABASE_OID" service.env
```

The command rejects a mismatched target, other client/unknown backends, prepared
transactions and physical connection replacement. It disables the native legacy
definition and cancels only its global nonterminal jobs through Runledger. Terminal
history, domain rows, migrations and the epoch sequence remain. An absent
definition is reported as absent; it is not a durable disabled tombstone.
Failures remain failures even if later `retirement::readback` observes a cancelled
job. Retain the primary command report before running that separate read-only
command; cancelling readback cannot remove the already-published primary cause.
The CLI emits one JSON object: successful reports go to stdout (exit 0), failures
to stderr (exit 1). Keep that object. It includes the command stage, work/refusal
kind, actual target identity, quiescence counts, prior cancellation count and
cleanup disposition when available. Cancellation failures include `job_id`,
`readback_attempted: false`, and sanitized native classification; a session-replacement
wrapper retains these under `original`. Pass that job ID and the independently
verified target to `retirement::readback`. Native SQL/error/panic bodies are never
formatted by this projection. Preparation failures give the fixed usage without printing
configuration contents. JSON facts do not authorize retry or replace the retained
library report. The report describes database observations, not deployment completion.

For live probes, select **two dedicated disposable local PostgreSQL 18 servers**.
The primary requires superuser authority, SCRAM host authentication,
`max_prepared_transactions > 0`, and autovacuum
with `track_counts` enabled and `autovacuum_naptime <= 5s`
(use `postgres -c autovacuum_naptime=1s -c max_prepared_transactions=10`). The
secondary must be a different cluster for the wrong-server observation control
and permit `pg_control_system()`; preflight rejects equal cluster identities.
PostgreSQL 18.4 permits this by default; if access was revoked, grant
`EXECUTE ON FUNCTION pg_catalog.pg_control_system()` to the secondary login in
the selected administrative database.
Then run from the workspace root:

```sh
POSTGRES_TEST_ADMIN_URL='postgres://postgres:fixture@127.0.0.1:5432/postgres?sslmode=disable' \
POSTGRES_TEST_OBSERVER_URL='postgres://postgres:fixture@127.0.0.1:5433/postgres?sslmode=disable' \
  bash scripts/test_reference_live.sh
```

Preflight is the read-only `reference_preflight` Rust example. It shares the
application URL validator and fixture endpoint policy and uses native SQLx to
check authentication, server version and privileges before any database fixture.
Use explicit username, database and sslmode=disable. PG* environment entries and
unsupported query keys are rejected for both endpoints before either connection
opens. The complete suite requires SCRAM on the primary and explicit credentials
in its URL. `MaintenanceSettings` also permits operator-selected passwordless
endpoints for focused probes; neither preflight nor configured probes read a passfile. Python owns only command
budgets and exact test inventory, not a second URL parser or client policy.

The live fixture URL path accepts `localhost` and `127.0.0.1`. IPv6 literals
fail preflight before connection work because the locked SQLx URL parser retains
their brackets in TCP lookup. The separate native-options constructor below
still supports IPv6. The live handoff canonicalizes selected host, database and
TLS values and explicitly retains empty passwords so fixture parsing does not
fall back to a passfile.

Earlier live-suite results retain their historical scope. The current executable
inventory is defined in `scripts/reference_live.py` and checked against the Rust
target; its native initialization, ownership, durable execution, callback and
business-failure cases replace the retired hosted-control protocol cases.
The earlier 58 runner entries (56 database and two offline signal cases), plus the
separate maintenance-session probe, passed on both supported toolchains on Linux.
The current inventory has 68 entries: 63 database probes, four offline signal
controls and the private child entry; the two metrics export probes require the
`metrics-export` feature, which the runner selects. Its eight protected-startup rows are
described in [testing](../../docs/testing.md#protected-startup-consumer-process-cases);
the full baseline executed on PostgreSQL 18.6. The expanded assertions for
resumed generation replacement, uncertain admission interruption and terminal
redelivery also passed on PostgreSQL 18.6 with both supported Rust toolchains.
Ordinary tests keep database cases ignored and run offline signal controls without
PostgreSQL.

The external harness owns lease cleanup and each application pool closes before
lease disposal. The runner checks prerequisites and exact named results through
bounded Unix process ownership. Database failure injection requires dedicated
endpoints and serial execution. A failed or watchdog-terminated run does not
establish cleanup.

The original migration fixtures remain narrow compatibility probes. The production
root starts native Runledger with the delivery handler and ordinary readiness
approval. Reusable
native pool/lease ownership comes from the optional
`batter`'s `sqlx-test-support` feature; template isolation and lock-operation probes
exercise it without importing application policy into the adapter.

The minimal fixture installs a separately budgeted session observer before
acquisition. Bounded observation failure retains the database until explicit retry;
all attempt errors remain in its final report. Native detach and adapter retirement
controls demonstrate why pool close alone is insufficient. Separate tests retain
body, handled pool, consuming-cleanup and deferred-drain errors, and distinguish
live waiter loss from actual runtime destruction and native lease Drop.

## Opt-in metrics export

Build with `--features metrics-export` and set `BATTER_METRICS_OTLP_ENDPOINT` to
export the bounded foundation catalog (operations including
`http.response_construction` and `provider.dispatch`, admission decisions, task
exits, cleanup hooks and the shutdown outcome) over OTLP/HTTP protobuf:

```sh
BATTER_METRICS_OTLP_ENDPOINT='http://127.0.0.1:4318/v1/metrics' \
  cargo run -p batter-example-reference-service --features metrics-export --locked
```

Without the setting nothing is installed and no collector is contacted; default
successful runs stay silent. The value must be `http` to a literal loopback
address at exactly `/v1/metrics`, without credentials, query or fragment.
An explicit value in a build without the feature, a malformed value, or any
ambient `OTEL_*` variable while export is enabled fails configuration before
acquisition. Preparation rechecks the live environment before upstream builders
that read `OTEL_*` values, without mutating it. The resource carries only
`service.name`; there is no proxy discovery, redirect, TLS or remote collector.

`runtime::start` installs one guarded recorder through Batter's canonical
`telemetry::metrics::install`, which publishes catalog descriptions, before
protected startup. A second process-wide installation is rejected: the rejected
recorder, exporter and provider are closed explicitly and the service runs
without export (`MetricsExport::InstallationRejected`). The guard admits only the
foundation catalog with its exact label shapes and vocabularies and at most
`MAX_SERIES` complete keys for the process lifetime, rejecting everything else
before the `metrics-exporter-otel` registry, SDK callbacks or metadata allocate.
Rejections are counted, never logged.

One application-owned serial loop collects cumulative snapshots from a shared
SDK `ManualReader` every ten seconds and exports each under a three-second
deadline. There is no `PeriodicReader`, background SDK thread, request queue or
retry; missed ticks are coalesced and counted, and a collector outage keeps
aggregating the same bounded series. Histograms use twelve fixed boundaries
(5 ms to 30 s, thirteen buckets). Requests above 2 MiB are refused before
dispatch; responses above 64 KiB are rejected. The worst-case full catalog with
maximum-length names encodes to about 0.55 MB. Refusal, header or body stalls,
non-success status, OTLP partial rejection, malformed or oversized responses and
deadline expiry each become a typed `ExportFailure`; only a decoded response
without rejected points is `Acknowledged`, and that claims nothing about durable
downstream storage. Upstream error text and collector bodies are never retained.

Finalization is owned by the orchestration, not by readiness, drain or cleanup
hooks. After protected startup failure cleanup, or complete driver settlement
including the shutdown metric recorded after `Stopped`, scheduling stops, any
in-flight periodic attempt settles under its own deadline, and one separate
five-second allowance covers the final snapshot, its export and exactly-once
exporter/provider closure. Service drain and cleanup budgets never wait on the
collector. A report with unjoined tasks, uncertain native settlement or skipped
or unjoined cleanup marks `FinalCoverage::Incomplete`. The combined
`ServiceCompletion` keeps the original service result and report beside the
`MetricsExport` diagnostics; the executable's exit code and stderr follow the
service result only. Dropping the `ServiceOwner` requests ordinary drain; waiter
cancellation requests nothing, and an observer taken earlier still receives the
retained completion. Runtime death, SIGKILL, blocking or panicking recorder code
and remote rollback of a cancelled request are outside the guarantee.

## Typed settings and native constructors

`config::ServingSettings` and `config::MaintenanceSettings` load distinct
application-owned command schemas before resource acquisition. Their
`from_process(selected_file, overrides)` constructors capture environment once
and read only that optional path (64 KiB limit). `from_sources` accepts injected sources.
Precedence is defaults < selected file < captured environment < explicit overrides.
Every source is structurally validated before values merge. Invalid winners fail;
invalid shadowed scalar values may be replaced, but malformed/duplicate/unknown
file or override entries cannot. Unknown BATTER_/JOBS_ environment names and all
PG* entries fail; unrelated environment names are ignored. Maintenance also
ignores known serving-only environment names without parsing them, while its
dedicated file and overrides reject those same names.

The shared literal dotenv dialect is UTF-8 LF/CRLF `KEY=VALUE` with ASCII identifier
keys, outer space/tab trimming, full-line comments and matching optional outer
quotes. Unquoted whitespace, quotes or `#` fail. Dollar and backslash stay literal;
there is no interpolation, escaping, export, multiline or inline-comment syntax.

| Setting | Initial default / requirement |
| --- | --- |
| BATTER_BIND | 127.0.0.1:3000; port 0 permitted |
| BATTER_LISTENER_ANNOUNCEMENT_PATH | Unset by default; optional absolute Unix datagram receiver path, validated without I/O. After binding any configured port, send one datagram containing only the actual SocketAddr (no newline). The receiver must exist before startup. Failure fails startup; backpressure remains inside its existing deadline. This announces binding, not readiness. No stdout discovery protocol. |
| BATTER_REQUEST_TIMEOUT_MS | 2000; positive, at most one year |
| BATTER_BULKHEAD_CAPACITY | 32; 1..=Tokio Semaphore::MAX_PERMITS |
| BATTER_PROCESS_CAPACITY | 32; independent finite-task bound, same range |
| BATTER_AUTH_OWNER_ID | Required non-nil UUID in serving; outside the maintenance schema |
| BATTER_AUTH_TOKEN | Required visible-ASCII token, 1..=256 bytes in serving; redacted from application diagnostics; outside the maintenance schema |
| BATTER_PROVIDER_BASE_URL | Required HTTPS origin, or cleartext HTTP only for literal IPv4/IPv6 loopback fixtures; DNS names including localhost are not cleartext exemptions; credentials, path, query and fragment forbidden |
| BATTER_PROVIDER_TOKEN | Required visible-ASCII token, 1..=256 bytes; redacted from application diagnostics; outside the maintenance schema |
| BATTER_PROVIDER_CAPACITY | 32; provider exchanges plus the short POST marker, 1..=Tokio Semaphore::MAX_PERMITS; excludes post-response SQL; waiting callers are bounded by JOBS_MAX_GLOBAL_CONCURRENCY |
| BATTER_POOL_MAX_CONNECTIONS | 8; positive u32 |
| BATTER_POOL_MIN_CONNECTIONS | 0; u32, at most maximum |
| BATTER_POOL_ACQUIRE_TIMEOUT_MS | 3000; positive, at most one year |
| DATABASE_URL | Required; supported TCP URL subset below |
| JOBS_WORKER_ID | Required and nonblank in serving; outside the maintenance schema |
| JOBS_POLL_INTERVAL_MS | 500; positive, at most one year |
| JOBS_CLAIM_BATCH_SIZE | 16; 1..=native JOBS_CLAIM_BATCH_SIZE_MAX (1000) |
| JOBS_LEASE_TTL_SECONDS | 60; positive i32 |
| JOBS_MAX_GLOBAL_CONCURRENCY | 32; 1..=Tokio Semaphore::MAX_PERMITS |
| JOBS_REAPER_INTERVAL_SECONDS | 15; positive, at most one year |
| JOBS_SCHEDULE_POLL_INTERVAL_SECONDS | 30; positive, at most one year |
| JOBS_REAPER_RETRY_DELAY_MS | 30000; positive i32 |

Numeric strings are unsigned decimal, without signs/whitespace; checked conversions
reject overflow. Native validation remains mandatory. Lease TTL 1 and retry delay
1 are accepted, matching typed Runledger validation rather than its environment
loader's clamps. Intent promotion derives polling/batch values from JobsConfig;
there is no JOBS_INTENT_PROMOTER_* second source path.

`DATABASE_URL` is secret as a whole. Require postgres/postgresql, explicit host,
username and database, port 1..=65535 (default 5432), no fragment, and explicit
sslmode. Only password, sslmode and application_name query keys are accepted,
once each; conflicting password sources fail. Percent encodings must be valid
UTF-8 and are decoded once; query `+` means space. DNS, IPv4 and IPv6 TCP hosts
are supported; socket URLs and additional libpq parameters are outside this
reference subset. Application name defaults to `reference-service`.

Serving requires an explicit nonempty password. Maintenance permits a
passwordless endpoint selected by the operator and recognizes only DATABASE_URL.
Pool, HTTP, authentication, worker and lifecycle keys are rejected
by that schema rather than parsed into optional capabilities. Both types validate
every supplied setting, and maintenance has no conversion into serving.
`PoolSettings::new`/`from_source` and `WorkerSettings::from_source` expose the same
section validation to current probes without pretending they are serving roots.

The protected service path consumes `ServingSettings` through `runtime::prepare`,
which creates a must-use, non-cloneable `PreparedServing` without connecting,
binding, migrating or spawning. `runtime::run` and `runtime::start` accept only
that value; a never-polled `run` future installs, connects and spawns nothing. The router
similarly consumes an opaque `PreparedHttp`; it cannot reconstruct missing
authentication or accept maintenance inputs. Offline commands consume
`MaintenanceSettings::prepare` and receive only native database inputs.
Focused probes retain explicit low-level accessors for the validated serving
values, but those accessors are not the canonical runtime handoff. Native SQLx
options can expose secrets through Debug and URL conversion.
The selected graph has no SQLx TLS backend: settings preserve requested modes,
but TLS connection execution requires the application's native SQLx TLS feature;
no TLS negotiation result is claimed. SQLx 0.9's URL formatter cannot represent
bare setter-provided IPv6 hosts. Offline tests verify that endpoint and credentials
through a controlled native PostgreSQL handshake rather than that formatter.

The reference rejects all PG* environment settings and rechecks before constructing
SQLx options. It uses `new_without_pgpass` and explicit setters, with no native URL
parser or passfile lookup. Keep environment unchanged during construction. These
wrappers do not sanitize upstream logs, deliberate source-chain inspection or
panic hooks, and do not encrypt or erase memory.

Offline tests cover source/bounds/redaction/native constructors and partial startup.
The explicit live inventory additionally includes configured pool timeout/reuse,
held-handler worker concurrency, failed startup closing its pool before the
native fixture lease, atomic delivery/reconciliation, and command-root pool,
deadline, Bulkhead, and finite-process effects. Provider-backed
delivery execution is implemented; external-provider adoption remains separate.

Offline native-option, IPv6 and worker-builder checks execute in cleared child
processes. The normal matrix repeats this target with hostile parent PG* values;
no test mutates process-wide environment. Supported encoded identifier and
application-name spaces remain literal after one decoding pass. Maintenance
does not parse worker settings at all; serving constructs and validates the native
JobsConfig before preparation. The pinned JobsConfig has no additional
cross-field validation rule beyond those checked by the serving schema.
