# Staged worker, reference delivery command and compatibility probes

Unpublished, Unix-only application package. Its runnable root initializes the
authenticated delivery command and hosts a probe-only Runledger worker as one
Batter critical component. The control handler is witnessed through durable
success, but the delivery handler is deliberately absent and application
readiness remains unapproved until `batter-8q8.2` installs the real provider.
The package also retains the compatibility and fixture probes created for
`batter-4t6`.
The [compatibility manifest](../../docs/reference-compatibility.md) records exact
versions, API contracts, executed evidence and limits.

## Staged worker and atomic delivery command

Run the staged service with an explicit PostgreSQL endpoint, worker identity,
owner and opaque bearer token. Startup executes `jobs.startup.control` through
the probe registry and keeps driving the native supervisor, but it never claims
`records.delivery.execute`:

```sh
DATABASE_URL='postgres://service:password@127.0.0.1:5432/service?sslmode=disable' \
JOBS_WORKER_ID='reference-probe' \
BATTER_AUTH_OWNER_ID='00000000-0000-0000-0000-000000000001' \
BATTER_AUTH_TOKEN='replace-with-an-opaque-token' \
  cargo run -p batter-example-reference-service --locked
```

An optional positional settings-file path uses the literal format described
below. Environment values override that file; no file is discovered implicitly.
Startup applies the pinned Runledger migration history, the forward-only
application history, the upstream compatibility check, and the handler-free
`records.delivery.execute` producer definition. It then witnesses the control
handler and hands off a running driver without approving application readiness.
Consequently `/ready` and admission-protected business routes remain unavailable
in this stage. Signal sources are installed before the first awaited pool acquisition and
polled throughout schema, binding and worker preparation, then transferred to the critical signal component at
handoff, so SIGTERM/SIGINT can still initiate owned startup cleanup.

The private control registry uses one dedicated PostgreSQL session advisory lock
per database. A second staged probe host is rejected until the first native
driver has completed and released its session; rolling overlap is intentionally
not supported during a healthy lease. The owner checks that same session every
second throughout database preparation and native execution, bounds each check
to two seconds, and sets a session-local ten-second idle limit; loss requests
native shutdown and remains an explicit unproven termination. A successor may
acquire after server-confirmed session loss before that shutdown is observed, so
each control handler accepts only its own witness generation. A predecessor that
claims the successor's witness returns an authorized delayed retry instead of
completing or terminally failing it. Each such retry consumes one durable
attempt, so the control carries no finite attempt budget: the witness is bounded
by its deadline alone, and stale controls are canceled by the next owner rather
than dead-lettered by attempt exhaustion.
Preparation runs under an independent owner before lease acquisition. Dropping
its waiter requests cancellation while cleanup continues on the live runtime.
The owner records bounded unlock and local closure separately; errors and timeouts
remain unconfirmed release, and client close never proves backend exit. Inspect
TerminationGate::settlement after dependency cleanup; preparation errors are shared
through Arc so waiter loss cannot discard them. RuntimeStartupFailure and
RuntimeShutdownFailure retain these outcomes alongside the outer and nested reports.
One 14-second reserve includes native shutdown, abort drain, lease release and margin.

BATTER_POOL_MAX_CONNECTIONS is the application pool limit. The control session
adds one connection while running. Preparation temporarily adds a separate
one-connection pool for the native catalog/cancellation/enqueue helpers; its
connections close on return and it closes before native construction. This avoids
SQLx return-to-pool checks waiting behind cancelled queries. Witness reads use
PgLease disposition. These allocations do not bound residual remote sessions.
Before enqueueing its unique witness, a new owner cancels every pending or leased
control left by an earlier owner. An already-terminal cancellation race is
accepted only after a readback proves its terminal state. A rejected Batter registration returns an
error that retains the already-stopping host so its driver can still be awaited.
Dropping an unstarted Batter supervisor also drops that registered owner, requests
native shutdown and leaves the independent observer to publish completion.
This staging contract requires a direct or session-sticky PostgreSQL connection;
transaction-pooling middleware that reassigns sessions is unsupported.

The authenticated routes are:

- `POST /records/{record_id}/deliveries` with `expected_generation`,
  `idempotency_key`, and `payload`.
- `GET /deliveries/{delivery_id}` for the owner-scoped durable projection.
- `GET /delivery-commands/{idempotency_key}` for reconciliation when the caller
  did not receive the POST response.

The idempotency key is 1–128 URL-safe ASCII bytes (`A-Z`, `a-z`, `0-9`, `.`,
`_`, `:`, `-`); the meaningful JSON payload is at most 16 KiB encoded. The
application retains the owner, record, expected positive generation, canonical
JSONB payload, stable delivery UUID, and immutable Runledger enqueue inputs.
An exact retry returns the same delivery without another enqueue. Changed
record/generation/payload input conflicts. Another authenticated owner may reuse
the same key independently and cannot observe the first owner's rows.

The first submission uses one `PgLease`, one READ COMMITTED transaction, and the
native `enqueue_job_with_outcome_tx` API. Acknowledged commit returns the lease;
acknowledged rollback returns it after rollback; interruption after `BEGIN` or a
missing commit/rollback acknowledgement retires it and returns an uncertain
outcome. The service never retries a transaction automatically. The caller must
query its original owner/key; absence while the original database session may
still settle does not prove rollback.

`pending`, `in_flight`, `succeeded`, `dead_lettered`, and `cancelled` are a
closed application projection of the locked Runledger status vocabulary. This
command initially creates `pending` work only. The probe worker does not
register the delivery type, so acceptance never means that an external effect
succeeded.

Run offline compilation and the native seam doctest:

```sh
cargo check -p batter-example-reference-service --all-targets --all-features --locked
cargo test -p batter-example-reference-service --doc --locked
```

The ordinary configuration tests require enabled IPv6 loopback (`::1`), IPv4
loopback socket permissions and Unix subprocess permissions, including inside
containers. Their IPv6 PostgreSQL protocol fixture owns its listener and requires
no external database. Enable IPv6 before running the matrix; the test is mandatory.

For live probes, select **two dedicated disposable local PostgreSQL 18 servers**.
The primary requires superuser authority, SCRAM host authentication and autovacuum
with `track_counts` enabled and `autovacuum_naptime <= 5s`
(use `postgres -c autovacuum_naptime=1s`). The
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
in its URL. The native Setup constructor also permits operator-selected passwordless
endpoints for focused probes; neither preflight nor configured probes read a passfile. Python owns only command
budgets and exact test inventory, not a second URL parser or client policy.

The live fixture URL path accepts `localhost` and `127.0.0.1`. IPv6 literals
fail preflight before connection work because the locked SQLx URL parser retains
their brackets in TCP lookup. The separate native-options constructor below
still supports IPv6. The live handoff canonicalizes selected host, database and
TLS values and explicitly retains empty passwords so fixture parsing does not
fall back to a passfile.

The recorded forty- and 42-case live passes predate worker hosting. The current
runner requires 54 entries (52 live probes and two offline signal entries), including command/reconciliation, configured-root,
hosted-worker witness, exclusive probe ownership and lease loss, retained driver
observation, drain, timeout and attempt-accounting cases; see validation for the distinction
between compiled inventory and live execution evidence.

The external harness owns four kinds of lease cleanup, and each application pool
closes before lease disposal. The runner checks prerequisites, requires all 54
named entries to exist and run, and uses the existing bounded Unix process
owner. Ordinary workspace tests ignore the 52 database cases, execute the two
offline entries, and require no
database. The failure injection locks a shared system catalog, so keep other
workloads off the endpoint; the runner executes cases serially. A failed or watchdog-terminated run does not establish cleanup.

The original migration fixtures remain narrow compatibility probes. The staged
root adds a probe-only worker host but no durable delivery provider. Reusable
native pool/lease ownership comes from the optional
`batter-sqlx/test-support` feature; template isolation and lock-operation probes
exercise it without importing application policy into the adapter.

The minimal fixture installs a separately budgeted session observer before
acquisition. Bounded observation failure retains the database until explicit retry;
all attempt errors remain in its final report. Native detach and adapter retirement
controls demonstrate why pool close alone is insufficient. Separate tests retain
body, handled pool, consuming-cleanup and deferred-drain errors, and distinguish
live waiter loss from actual runtime destruction and native lease Drop.

## Typed settings and native constructors

`config::RootSettings` loads application-owned policy before resource acquisition.
`from_process(mode, selected_file, overrides)` captures environment once and reads
only that optional path (64 KiB limit). `from_sources` accepts injected sources.
Precedence is defaults < selected file < captured environment < explicit overrides.
Every source is structurally validated before values merge. Invalid winners fail;
invalid shadowed scalar values may be replaced, but malformed/duplicate/unknown
file or override entries cannot. Unknown BATTER_/JOBS_ environment names and all
PG* entries fail; unrelated environment names are ignored.

The shared literal dotenv dialect is UTF-8 LF/CRLF `KEY=VALUE` with ASCII identifier
keys, outer space/tab trimming, full-line comments and matching optional outer
quotes. Unquoted whitespace, quotes or `#` fail. Dollar and backslash stay literal;
there is no interpolation, escaping, export, multiline or inline-comment syntax.

| Setting | Initial default / requirement |
| --- | --- |
| BATTER_BIND | 127.0.0.1:3000; port 0 permitted |
| BATTER_REQUEST_TIMEOUT_MS | 2000; positive, at most one year |
| BATTER_BULKHEAD_CAPACITY | 32; 1..=Tokio Semaphore::MAX_PERMITS |
| BATTER_PROCESS_CAPACITY | 32; independent finite-task bound, same range |
| BATTER_AUTH_OWNER_ID | Required non-nil UUID in Serve; optional only as an owner/token pair in Setup |
| BATTER_AUTH_TOKEN | Required visible-ASCII token, 1..=256 bytes in Serve; redacted from application diagnostics |
| BATTER_POOL_MAX_CONNECTIONS | 8; positive u32 |
| BATTER_POOL_MIN_CONNECTIONS | 0; u32, at most maximum |
| BATTER_POOL_ACQUIRE_TIMEOUT_MS | 3000; positive, at most one year |
| DATABASE_URL | Required; supported TCP URL subset below |
| JOBS_WORKER_ID | Required and nonblank in Serve; optional in Setup |
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

Serve requires an explicit nonempty password. Setup permits a passwordless
endpoint selected by the operator and may omit worker identity; attempting a
worker constructor then fails. Both modes validate every supplied setting.
`PoolSettings::new`/`from_source` and `WorkerSettings::from_source` expose the same
section validation to current probes without pretending they are serving roots.

Use `request_policy`, `pool_options`, `connect_options_from_process`, `bulkhead`, `supervisor`
and `worker().builder` directly at their native consumer boundaries. The returned
native connection options can expose secrets through Debug and URL conversion.
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
deadline, Bulkhead, and finite-process effects. See
[validation](../../docs/validation.md) for execution status. Probe-only worker
hosting is implemented; delivery-provider execution remains a separate task.

Offline native-option, IPv6 and worker-builder checks execute in cleared child
processes. The normal matrix repeats this target with hostile parent PG* values;
no test mutates process-wide environment. Supported encoded identifier and
application-name spaces remain literal after one decoding pass. Setup omission
of worker identity still forbids building a worker; the pinned JobsConfig has no
cross-field validation rule beyond the bounds already checked in both modes.
