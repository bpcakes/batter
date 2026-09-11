# Reference compatibility probes

Unpublished, Unix-only application package for `batter-4t6`. Its native SQLx
pool/connection/transaction seams compile against pinned Runledger Git sources.
The [compatibility manifest](../../docs/reference-compatibility.md) records exact
versions, API contracts, executed evidence and limits.

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

The recorded nineteen-case live passes predate these handoff corrections.
Current live revalidation of the combined forty-case inventory on both toolchains remains pending in
`batter-5pm`; the implementation's offline checks do not complete that acceptance.

The external harness owns four kinds of lease cleanup, and each application pool
closes before lease disposal. The runner checks prerequisites, requires all forty
named ignored cases to exist and run, and uses the existing bounded Unix process
owner. Ordinary workspace tests report these cases as ignored and require no
database. The failure injection locks a shared system catalog, so keep other
workloads off the endpoint; the runner executes cases serially. A failed or watchdog-terminated run does not establish cleanup.

The migration fixtures and witness job are minimal probes. They do not implement
the later business command, durable provider, HTTP host. Reusable native pool/lease ownership comes from the optional
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
held-handler worker concurrency, and failed startup closing its pool before the
native fixture lease. See [validation](../../docs/validation.md) for execution
status. Full command and worker hosting remain separate delivery tasks.

Offline native-option, IPv6 and worker-builder checks execute in cleared child
processes. The normal matrix repeats this target with hostile parent PG* values;
no test mutates process-wide environment. Supported encoded identifier and
application-name spaces remain literal after one decoding pass. Setup omission
of worker identity still forbids building a worker; the pinned JobsConfig has no
cross-field validation rule beyond the bounds already checked in both modes.
